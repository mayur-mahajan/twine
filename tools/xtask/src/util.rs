//! Shared helpers for xtask commands: running processes, locating the workspace, probing tools.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Result type of every xtask command.
pub type R = Result<(), Box<dyn std::error::Error>>;

/// Renders `cmd` as a shell-like command line (environment overrides first).
#[must_use]
pub fn display_command(cmd: &Command) -> String {
    let mut parts = Vec::new();
    for (k, v) in cmd.get_envs() {
        if let Some(v) = v {
            parts.push(format!("{}={}", k.to_string_lossy(), v.to_string_lossy()));
        }
    }
    parts.push(cmd.get_program().to_string_lossy().into_owned());
    for a in cmd.get_args() {
        let a = a.to_string_lossy();
        if a.is_empty() || a.contains(' ') {
            parts.push(format!("'{a}'"));
        } else {
            parts.push(a.into_owned());
        }
    }
    parts.join(" ")
}

/// Runs `cmd` (inheriting stdio), printing its command line in bold first.
///
/// Fails when the process cannot be spawned or exits with a non-zero status.
pub fn run(cmd: &mut Command) -> R {
    let line = display_command(cmd);
    eprintln!("\x1b[1m$ {line}\x1b[0m");
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn `{line}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{line}` failed with {status}").into())
    }
}

/// Runs `cmd` and captures its standard output as a string (stderr is inherited).
pub fn output(cmd: &mut Command) -> Result<String, Box<dyn std::error::Error>> {
    let line = display_command(cmd);
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to spawn `{line}`: {e}"))?;
    if !out.status.success() {
        return Err(format!("`{line}` failed with {}", out.status).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// The workspace root (two levels above this crate's manifest directory).
#[must_use]
pub fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(std::path::Path::parent)
        .map_or(manifest.clone(), std::path::Path::to_path_buf)
}

/// A `cargo` command whose working directory is the workspace root.
#[must_use]
pub fn cargo() -> Command {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(workspace_root());
    cmd
}

/// Whether the rustup target `t` is installed for the active toolchain.
#[must_use]
pub fn has_target(t: &str) -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .current_dir(workspace_root())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == t))
}

/// Whether the executable `bin` can be found on `PATH`.
#[must_use]
pub fn which(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(bin);
        candidate.is_file() || (cfg!(windows) && candidate.with_extension("exe").is_file())
    })
}

/// Prints a yellow warning line.
pub fn warn(msg: &str) {
    eprintln!("\x1b[33mwarning:\x1b[0m {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_command_quotes_args_with_spaces() {
        let mut c = Command::new("cargo");
        c.args(["test", "a b"]).env("X", "1");
        assert_eq!(display_command(&c), "X=1 cargo test 'a b'");
    }

    #[test]
    fn workspace_root_contains_cargo_toml() {
        assert!(workspace_root().join("Cargo.toml").is_file());
    }
}
