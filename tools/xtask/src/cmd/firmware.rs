//! `cargo xtask firmware [example|all] [--strict]`: builds the example firmware in `firmware/`
//! and reports flash and static RAM sizes.
//!
//! Every directory of `firmware/` with a `Cargo.toml` is an example. Its manifest lists the
//! feature sets to build under `[package.metadata.twine] feature-sets` (`""` = the default
//! features; any other entry is built with `--no-default-features --features <entry>`); the
//! target comes from its `.cargo/config.toml` and the toolchain from its `rust-toolchain.toml`
//! (else the workspace's). An example whose target or toolchain is missing is skipped with a
//! warning, or fails the command with `--strict`.
//!
//! Xtensa examples (toolchain `esp`) need Espressif's linker on `PATH`: when it is not, the
//! variables of `~/export-esp.sh` (written by `espup install`) are applied to the build.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{R, display_command, warn, which, workspace_root};

/// One built binary and its section sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeRow {
    /// Example directory name.
    pub example: String,
    /// Feature set (`default` or the `--features` list).
    pub features: String,
    /// Sizes of the ELF file.
    pub sizes: ElfSizes,
}

/// `size`-style totals of an ELF file: code and read-only data, initialized data, zeroed data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ElfSizes {
    /// Allocated, non-writable sections (code, read-only data, vectors).
    pub text: u64,
    /// Allocated, writable sections with contents (initialized statics; stored in flash too).
    pub data: u64,
    /// Allocated, writable sections without contents (zeroed statics, heap arrays, stacks).
    pub bss: u64,
}

impl ElfSizes {
    /// Bytes stored in flash.
    #[must_use]
    pub const fn flash(&self) -> u64 {
        self.text + self.data
    }

    /// Bytes of static RAM.
    #[must_use]
    pub const fn ram(&self) -> u64 {
        self.data + self.bss
    }
}

/// Section header flags and types used by [`elf_sizes`].
const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHT_NOBITS: u32 = 8;

/// Sums the allocated sections of a 32- or 64-bit little-endian ELF file like `size` (Berkeley
/// format): non-writable → text, writable with contents → data, writable without → bss.
/// Placeholder sections that only reserve address space (`*_dummy`, esp-hal's linker scripts)
/// are skipped.
pub fn elf_sizes(elf: &[u8]) -> Result<ElfSizes, String> {
    let u16_at = |o: usize| -> Result<u16, String> {
        elf.get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| "truncated ELF".to_string())
    };
    let u32_at = |o: usize| -> Result<u32, String> {
        elf.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| "truncated ELF".to_string())
    };
    let u64_at = |o: usize| -> Result<u64, String> {
        elf.get(o..o + 8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
            .ok_or_else(|| "truncated ELF".to_string())
    };
    if elf.get(..4) != Some(b"\x7fELF".as_slice()) {
        return Err("not an ELF file".into());
    }
    if elf.get(5) != Some(&1) {
        return Err("big-endian ELF is not supported".into());
    }
    let is64 = match elf.get(4) {
        Some(1) => false,
        Some(2) => true,
        _ => return Err("unknown ELF class".into()),
    };
    let (shoff, shentsize, shnum, shstrndx) = if is64 {
        (
            u64_at(0x28)? as usize,
            usize::from(u16_at(0x3A)?),
            usize::from(u16_at(0x3C)?),
            usize::from(u16_at(0x3E)?),
        )
    } else {
        (
            u32_at(0x20)? as usize,
            usize::from(u16_at(0x2E)?),
            usize::from(u16_at(0x30)?),
            usize::from(u16_at(0x32)?),
        )
    };
    let section_offset = |i: usize| -> Result<usize, String> {
        let h = shoff + i * shentsize;
        Ok(if is64 {
            u64_at(h + 0x18)? as usize
        } else {
            u32_at(h + 0x10)? as usize
        })
    };
    let strtab = if shstrndx != 0 && shstrndx < shnum {
        Some(section_offset(shstrndx)?)
    } else {
        None
    };
    let name = |off: u32| -> &[u8] {
        strtab
            .and_then(|t| elf.get(t + off as usize..))
            .map_or(&[][..], |b| {
                &b[..b.iter().position(|c| *c == 0).unwrap_or(b.len())]
            })
    };
    let mut s = ElfSizes::default();
    for i in 0..shnum {
        let h = shoff + i * shentsize;
        if name(u32_at(h)?).ends_with(b"_dummy") {
            continue;
        }
        let sh_type = u32_at(h + 4)?;
        let (flags, size) = if is64 {
            (u64_at(h + 8)?, u64_at(h + 0x20)?)
        } else {
            (u64::from(u32_at(h + 8)?), u64::from(u32_at(h + 0x14)?))
        };
        if flags & SHF_ALLOC == 0 {
            continue;
        }
        if flags & SHF_WRITE == 0 {
            s.text += size;
        } else if sh_type == SHT_NOBITS {
            s.bss += size;
        } else {
            s.data += size;
        }
    }
    Ok(s)
}

/// What an example needs to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    /// Directory name.
    pub name: String,
    /// Directory.
    pub dir: PathBuf,
    /// Package name (binary name).
    pub package: String,
    /// `[build] target` of `.cargo/config.toml`.
    pub target: String,
    /// `channel` of the example's `rust-toolchain.toml` (`None`: the workspace's).
    pub toolchain: Option<String>,
    /// Feature sets (`""` = defaults).
    pub feature_sets: Vec<String>,
}

/// Reads an example directory.
pub fn read_example(dir: &Path) -> Result<Example, String> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("bad directory")?;
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let manifest: toml::Table =
        toml::from_str(&read(&dir.join("Cargo.toml"))?).map_err(|e| format!("{name}/Cargo.toml: {e}"))?;
    let package = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("{name}/Cargo.toml: no package name"))?
        .to_string();
    let feature_sets = manifest
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("twine"))
        .and_then(|t| t.get("feature-sets"))
        .and_then(toml::Value::as_array)
        .map_or_else(
            || vec![String::new()],
            |a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        );
    let config: toml::Table = toml::from_str(&read(&dir.join(".cargo/config.toml"))?)
        .map_err(|e| format!("{name}/.cargo/config.toml: {e}"))?;
    let target = config
        .get("build")
        .and_then(|b| b.get("target"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("{name}/.cargo/config.toml: no [build] target"))?
        .to_string();
    let toolchain = match std::fs::read_to_string(dir.join("rust-toolchain.toml")) {
        Ok(s) => toml::from_str::<toml::Table>(&s)
            .ok()
            .and_then(|v| v.get("toolchain")?.get("channel")?.as_str().map(str::to_string)),
        Err(_) => None,
    };
    Ok(Example {
        name,
        dir: dir.to_path_buf(),
        package,
        target,
        toolchain,
        feature_sets,
    })
}

/// Every example in `firmware/`, sorted by name.
pub fn examples() -> Result<Vec<Example>, String> {
    let root = workspace_root().join("firmware");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    for e in entries.flatten() {
        let dir = e.path();
        if dir.join("Cargo.toml").is_file() {
            out.push(read_example(&dir)?);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Why an example cannot be built here (`None`: it can).
fn missing_prerequisite(ex: &Example, env: &[(String, String)]) -> Option<String> {
    let toolchain = ex.toolchain.as_deref();
    let mut list = Command::new("rustup");
    list.args(["toolchain", "list"]);
    let toolchains = list
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if let Some(t) = toolchain {
        if t != "stable"
            && !toolchains.lines().any(|l| {
                l.split_whitespace()
                    .next()
                    .is_some_and(|n| n == t || n.starts_with(&format!("{t}-")))
            })
        {
            return Some(format!(
                "toolchain `{t}` is not installed (`espup install` for `esp`)"
            ));
        }
    }
    if toolchain == Some("esp") {
        let has_linker = |bin: &str| {
            which(bin)
                || env
                    .iter()
                    .any(|(k, v)| k == "PATH" && std::env::split_paths(v).any(|d| d.join(bin).is_file()))
        };
        if !has_linker("xtensa-esp-elf-gcc")
            && !has_linker("xtensa-esp32-elf-gcc")
            && !has_linker("xtensa-esp32s3-elf-gcc")
        {
            return Some("Xtensa linker not found (run `espup install`, then `. ~/export-esp.sh`)".into());
        }
        return None;
    }
    let mut targets = Command::new("rustup");
    targets
        .args(["target", "list", "--installed"])
        .current_dir(&ex.dir);
    let installed = targets
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if !installed.lines().any(|l| l.trim() == ex.target) {
        return Some(format!(
            "target `{}` is not installed (`rustup target add {}`)",
            ex.target, ex.target
        ));
    }
    None
}

/// The variables of `~/export-esp.sh` (`export K="V"` lines, `$PATH` expanded), or none.
fn esp_export_env() -> Vec<(String, String)> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(PathBuf::from(home).join("export-esp.sh")) else {
        return Vec::new();
    };
    let path = std::env::var("PATH").unwrap_or_default();
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("export "))
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim_matches('"').replace("$PATH", &path)))
        .collect()
}

/// A `cargo` command for building inside `dir` with the example's own toolchain file (the
/// variables `cargo xtask` runs with would otherwise pin the workspace toolchain).
fn example_cargo(dir: &Path, env: &[(String, String)]) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(dir);
    for k in [
        "RUSTUP_TOOLCHAIN",
        "CARGO",
        "RUSTC",
        "RUSTDOC",
        "CARGO_TARGET_DIR",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// Builds `ex` for each feature set; returns the size rows.
fn build_example(ex: &Example, env: &[(String, String)]) -> Result<Vec<SizeRow>, String> {
    let mut rows = Vec::new();
    for set in &ex.feature_sets {
        let mut cmd = example_cargo(&ex.dir, env);
        cmd.args(["build", "--release"]);
        if !set.is_empty() {
            cmd.args(["--no-default-features", "--features", set]);
        }
        let line = display_command(&cmd);
        eprintln!("\x1b[1m$ (firmware/{}) {line}\x1b[0m", ex.name);
        let status = cmd
            .status()
            .map_err(|e| format!("failed to spawn `{line}`: {e}"))?;
        if !status.success() {
            return Err(format!("firmware/{}: `{line}` failed with {status}", ex.name));
        }
        let elf = ex
            .dir
            .join("target")
            .join(&ex.target)
            .join("release")
            .join(&ex.package);
        let bytes = std::fs::read(&elf).map_err(|e| format!("{}: {e}", elf.display()))?;
        rows.push(SizeRow {
            example: ex.name.clone(),
            features: if set.is_empty() {
                "default".into()
            } else {
                set.clone()
            },
            sizes: elf_sizes(&bytes)?,
        });
    }
    Ok(rows)
}

/// `bytes` in KiB with one decimal.
fn kib(bytes: u64) -> String {
    let tenths = (bytes * 10 + 512) / 1024;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// A Markdown table of the sizes.
#[must_use]
pub fn size_table(rows: &[SizeRow]) -> String {
    let mut s = String::from(
        "| Example | Features | Flash (text + data) | Static RAM (data + bss) | text | data | bss |\n|---|---|---:|---:|---:|---:|---:|\n",
    );
    for r in rows {
        let z = r.sizes;
        let _ = writeln!(
            s,
            "| {} | {} | {} KiB | {} KiB | {} | {} | {} |",
            r.example,
            r.features,
            kib(z.flash()),
            kib(z.ram()),
            z.text,
            z.data,
            z.bss
        );
    }
    s
}

/// Builds the examples (`which`: one name, `all` or `None` = all).
pub fn run(which_example: Option<&str>, strict: bool) -> R {
    let all = examples()?;
    let selected: Vec<&Example> = match which_example {
        None | Some("all") => all.iter().collect(),
        Some(name) => {
            let found: Vec<_> = all.iter().filter(|e| e.name == name).collect();
            if found.is_empty() {
                let names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
                return Err(format!("firmware: no example `{name}` (have: {})", names.join(", ")).into());
            }
            found
        }
    };
    let esp_env = esp_export_env();
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for ex in selected {
        let env: Vec<(String, String)> =
            if ex.toolchain.as_deref() == Some("esp") && !which("xtensa-esp-elf-gcc") {
                esp_env.clone()
            } else {
                Vec::new()
            };
        if let Some(reason) = missing_prerequisite(ex, &env) {
            if strict {
                return Err(format!("firmware: {}: {reason}", ex.name).into());
            }
            warn(&format!("skipping {}: {reason}", ex.name));
            skipped.push(ex.name.clone());
            continue;
        }
        rows.extend(build_example(ex, &env)?);
    }
    let table = size_table(&rows);
    println!("{table}");
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY") {
        let text = format!("### Firmware sizes\n\n{table}\n");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(summary)
            .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
    }
    if !skipped.is_empty() {
        println!("firmware: skipped {}", skipped.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF32 with three sections: `.text` (AX), `.data` (WA), `.bss` (WA, NOBITS),
    /// plus a non-allocated `.comment`.
    fn elf32(sections: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut v = vec![0u8; 0x34];
        v[..4].copy_from_slice(b"\x7fELF");
        v[4] = 1; // 32-bit
        v[5] = 1; // little endian
        let shoff = v.len() as u32;
        v[0x20..0x24].copy_from_slice(&shoff.to_le_bytes());
        v[0x2E..0x30].copy_from_slice(&40u16.to_le_bytes());
        v[0x30..0x32].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        for &(ty, flags, size) in sections {
            let mut h = [0u8; 40];
            h[4..8].copy_from_slice(&ty.to_le_bytes());
            h[8..12].copy_from_slice(&flags.to_le_bytes());
            h[0x14..0x18].copy_from_slice(&size.to_le_bytes());
            v.extend_from_slice(&h);
        }
        v
    }

    #[test]
    fn elf_sizes_like_size() {
        let elf = elf32(&[
            (1, 0x6, 1000),
            (1, 0x2, 200),
            (1, 0x3, 30),
            (8, 0x3, 4000),
            (1, 0x30, 99),
        ]);
        let s = elf_sizes(&elf).unwrap();
        assert_eq!(
            s,
            ElfSizes {
                text: 1200,
                data: 30,
                bss: 4000
            }
        );
        assert_eq!((s.flash(), s.ram()), (1230, 4030));
        assert!(elf_sizes(b"nope").is_err());
    }

    #[test]
    fn examples_are_discovered() {
        let all = examples().unwrap();
        for ex in &all {
            assert!(!ex.feature_sets.is_empty(), "{}", ex.name);
            assert!(ex.target.contains("none"), "{}: {}", ex.name, ex.target);
        }
    }

    #[test]
    fn size_table_formats() {
        let t = size_table(&[SizeRow {
            example: "rp2040".into(),
            features: "default".into(),
            sizes: ElfSizes {
                text: 2048,
                data: 0,
                bss: 1024,
            },
        }]);
        assert!(
            t.contains("| rp2040 | default | 2.0 KiB | 1.0 KiB | 2048 | 0 | 1024 |"),
            "{t}"
        );
    }
}
