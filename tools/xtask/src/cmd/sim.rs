//! `cargo xtask sim <example>` runs a simulator example from `examples/src/bin/`;
//! `cargo xtask sim-smoke` runs all of them headless (CI job `sim-smoke`).

use std::path::PathBuf;

use crate::util::{R, cargo, run as run_cmd, workspace_root};

/// Names of the available examples (file stems of `examples/src/bin/*.rs`), sorted.
#[must_use]
pub fn examples() -> Vec<String> {
    let dir = workspace_root().join("examples/src/bin");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "rs"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The headless script of `example`, if `examples/scripts/<example>.twinescript` exists.
#[must_use]
pub fn smoke_script(example: &str) -> Option<PathBuf> {
    let p = workspace_root()
        .join("examples/scripts")
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
    let available = examples();
    if !available.iter().any(|e| e == example) {
        let list = if available.is_empty() {
            "(none yet)".to_string()
        } else {
            available.join(", ")
        };
        return Err(format!("sim: unknown example `{example}`; available: {list}").into());
    }
    let mut cmd = cargo();
    cmd.args(["run", "-p", "twine-examples", "--bin", example]);
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
    run_cmd(cargo().args(["build", "-p", "twine-examples", "--bins"]))?;
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
        assert!(examples().iter().any(|e| e == "test_pattern"));
        assert!(smoke_script("test_pattern").is_some());
        assert!(smoke_script("no_such_example").is_none());
    }
}
