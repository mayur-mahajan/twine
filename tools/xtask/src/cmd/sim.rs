//! `cargo xtask sim <example>` runs a simulator example: a Cargo example of the `twine` crate
//! (`crates/twine/examples/`), or one of [`FEATURE_EXAMPLES`];
//! `cargo xtask sim-smoke` runs all of them headless (CI job `sim-smoke`).

use std::path::PathBuf;

use crate::util::{R, cargo, run as run_cmd, workspace_root};

/// The package whose Cargo examples are the simulator examples.
pub const PACKAGE: &str = "twine";

/// Directory of the simulator examples, relative to the workspace root.
pub const EXAMPLES_DIR: &str = "crates/twine/examples";

/// Examples that run in another simulator (`embedded-graphics-simulator`, SDL2), each a Cargo
/// example of another package behind a feature: `(example, package, feature)`. They have no
/// headless mode, so `sim-smoke` skips them (CI builds them in the `eg-sim` stage when SDL2 is
/// installed).
pub const FEATURE_EXAMPLES: &[(&str, &str, &str)] = &[("eg_simulator", "twine-embedded-graphics", "eg-sim")];

/// Names of the Cargo examples of [`PACKAGE`] (`examples/<name>.rs` and
/// `examples/<name>/main.rs`, as Cargo discovers them), sorted.
#[must_use]
pub fn examples() -> Vec<String> {
    let dir = workspace_root().join(EXAMPLES_DIR);
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "rs") || p.join("main.rs").is_file())
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Every runnable example: [`examples`] and the [`FEATURE_EXAMPLES`], sorted.
#[must_use]
pub fn all_examples() -> Vec<String> {
    let mut names = examples();
    names.extend(FEATURE_EXAMPLES.iter().map(|(e, _, _)| (*e).to_string()));
    names.sort();
    names
}

/// The directory of the SDL2 library (`sdl2-config --prefix`/lib), if SDL2 is installed.
#[must_use]
pub fn sdl2_lib_dir() -> Option<PathBuf> {
    let out = std::process::Command::new("sdl2-config")
        .arg("--prefix")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let prefix = String::from_utf8(out.stdout).ok()?;
    Some(PathBuf::from(prefix.trim()).join("lib"))
}

/// Points the linker at SDL2 (Homebrew installs it outside the default search path).
pub fn add_sdl2_env(cmd: &mut std::process::Command, lib: &std::path::Path) {
    let mut paths = vec![lib.to_path_buf()];
    if let Some(old) = std::env::var_os("LIBRARY_PATH") {
        paths.extend(std::env::split_paths(&old));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.env("LIBRARY_PATH", joined);
    }
}

/// The headless script of `example`, if `crates/twine/examples/scripts/<example>.twinescript`
/// exists.
#[must_use]
pub fn smoke_script(example: &str) -> Option<PathBuf> {
    let p = workspace_root()
        .join(EXAMPLES_DIR)
        .join("scripts")
        .join(format!("{example}.twinescript"));
    p.is_file().then_some(p)
}

/// How to run an example.
#[derive(Debug, Clone, Default)]
pub struct SimOptions {
    /// Build in release mode.
    pub release: bool,
    /// Run without a window.
    pub headless: bool,
    /// Headless script.
    pub script: Option<PathBuf>,
}

/// Runs `example`, forwarding `args` to it and `RUST_LOG` (default `twine=info`).
pub fn run(example: &str, opts: &SimOptions, args: &[String]) -> R {
    let available = all_examples();
    if !available.iter().any(|e| e == example) {
        let list = if available.is_empty() {
            "(none yet)".to_string()
        } else {
            available.join(", ")
        };
        return Err(format!("sim: unknown example `{example}`; available: {list}").into());
    }
    let mut cmd = cargo();
    if let Some((_, package, feature)) = FEATURE_EXAMPLES.iter().find(|(e, _, _)| *e == example) {
        cmd.args(["run", "-p", package, "--example", example]);
        let Some(lib) = sdl2_lib_dir() else {
            return Err(format!(
                "sim: `{example}` needs SDL2 (`brew install sdl2` / `apt install libsdl2-dev`)"
            )
            .into());
        };
        cmd.args(["--features", feature]);
        add_sdl2_env(&mut cmd, &lib);
    } else {
        cmd.args(["run", "-p", PACKAGE, "--example", example]);
    }
    if opts.release {
        cmd.arg("--release");
    }
    if !args.is_empty() {
        cmd.arg("--").args(args);
    }
    if std::env::var_os("RUST_LOG").is_none() {
        cmd.env("RUST_LOG", "twine=info");
    }
    if opts.headless {
        cmd.env("TWINE_SIM_HEADLESS", "1");
    }
    if let Some(script) = &opts.script {
        let abs = if script.is_absolute() {
            script.clone()
        } else {
            std::env::current_dir()?.join(script)
        };
        cmd.env("TWINE_SIM_SCRIPT", abs);
    }
    run_cmd(&mut cmd)
}

/// Runs every example headless; fails if any exits non-zero.
pub fn smoke() -> R {
    let all = examples();
    if all.is_empty() {
        println!("sim-smoke: no examples");
        return Ok(());
    }
    // Build once so the per-example runs only execute.
    run_cmd(cargo().args(["build", "-p", PACKAGE, "--examples"]))?;
    let mut failed = Vec::new();
    for ex in &all {
        let opts = SimOptions {
            release: false,
            headless: true,
            script: smoke_script(ex),
        };
        if let Err(e) = run(ex, &opts, &[]) {
            eprintln!("sim-smoke: {ex} failed: {e}");
            failed.push(ex.clone());
        }
    }
    if failed.is_empty() {
        println!("sim-smoke: {} example(s) ran headless", all.len());
        Ok(())
    } else {
        Err(format!("sim-smoke: failed: {}", failed.join(", ")).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_has_a_smoke_script() {
        let all = examples();
        // Single-file and multi-file (`<name>/main.rs`) examples; helper directories are not.
        assert!(all.iter().any(|e| e == "hello"));
        assert!(all.iter().any(|e| e == "test_pattern"));
        assert!(
            !all.iter()
                .any(|e| e == "common" || e == "assets" || e == "scripts")
        );
        assert!(all_examples().iter().any(|e| e == "eg_simulator"));
        assert!(smoke_script("test_pattern").is_some());
        assert!(smoke_script("no_such_example").is_none());
    }
}
