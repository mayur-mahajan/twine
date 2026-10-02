//! `cargo xtask miri [crate…]`: runs the tests of the given crates under Miri (nightly) to
//! detect undefined behaviour.
//!
//! **Policy.** Undefined behaviour needs `unsafe` code, so Miri only checks the crates that may
//! contain it. Every library crate of the workspace either declares `#![forbid(unsafe_code)]`
//! (which no inner `#[allow]` can lift: adding `unsafe` there fails to compile) or has a Miri
//! target in [`DEFAULT_CRATES`], [`DEFAULT_TESTS`] or [`DEFAULT_FILTERED`].
//! [`check_policy`] (run by the `layers` CI stage) enforces both directions: a crate that
//! allows `unsafe` without a Miri target, or a Miri target on a crate that forbids `unsafe`,
//! fails CI.

use std::process::Command;

use crate::util::{R, no_incremental, run as run_cmd, workspace_root};

/// Crates checked when none are given (whole test suites).
pub const DEFAULT_CRATES: &[&str] = &["twine-reactive"];

/// Single test targets also checked when no crates are given: `(crate, integration test)`.
/// Covers `unsafe` in crates that are otherwise too heavy for Miri (`twine-testing`'s
/// `CountingAllocator`).
pub const DEFAULT_TESTS: &[(&str, &str)] = &[("twine-testing", "alloc_count")];

/// Unit-test subsets also checked when no crates are given: `(crate, test name filter)`, run as
/// `cargo miri test -p <crate> --lib -- <filter>` (an empty filter runs every unit test), for a
/// crate whose `unsafe` is covered by light unit tests while its integration tests are too
/// heavy for Miri. None at present: `twine-view` forbids `unsafe` since the safe runtime token
/// (R3.S02) replaced its `bind_to_current_context` forwarding.
pub const DEFAULT_FILTERED: &[(&str, &str)] = &[];

/// Proptest cases per property under Miri (it is ~1000× slower than native).
pub const PROPTEST_CASES: &str = "8";

/// Per-crate settings: `(crate, features, extra MIRIFLAGS, proptest cases)`.
/// `twine-reactive` runs its thread-local (`std`) runtime under strict provenance, with more
/// cases for its model test.
pub const CRATE_SETTINGS: &[(&str, &str, &str, &str)] =
    &[("twine-reactive", "std", "-Zmiri-strict-provenance", "16")];

/// `(features, extra MIRIFLAGS, proptest cases)` for `krate`.
fn settings(krate: &str) -> (&'static str, &'static str, &'static str) {
    CRATE_SETTINGS
        .iter()
        .find(|(c, ..)| *c == krate)
        .map_or(("", "", PROPTEST_CASES), |&(_, f, m, p)| (f, m, p))
}

/// `MIRIFLAGS` for the run: proptest's failure persistence reads the working directory, which
/// Miri's isolation forbids, so isolation is disabled (tests stay deterministic: proptest
/// seeds come from its own RNG, not from the host). User-provided flags are appended.
fn miri_flags(crate_flags: &str) -> String {
    let extra = std::env::var("MIRIFLAGS").unwrap_or_default();
    format!("-Zmiri-disable-isolation {crate_flags} {extra}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `cargo +nightly miri` is usable.
#[must_use]
pub fn miri_available() -> bool {
    Command::new("rustup")
        .args(["+nightly", "component", "list", "--installed"])
        .current_dir(workspace_root())
        .output()
        .is_ok_and(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.starts_with("miri"))
        })
}

/// Runs `cargo +nightly miri test -p <crate>` for each crate (plus [`DEFAULT_TESTS`] when no
/// crates are given).
pub fn run(crates: &[String]) -> R {
    if !miri_available() {
        return Err(
            "miri: nightly toolchain with the miri component not found\n  hint: \
                    `rustup toolchain install nightly --component miri` or \
                    `rustup +nightly component add miri`"
                .into(),
        );
    }
    let with_defaults = crates.is_empty();
    let crates: Vec<String> = if with_defaults {
        DEFAULT_CRATES.iter().map(ToString::to_string).collect()
    } else {
        crates.to_vec()
    };
    let mut targets: Vec<(String, Option<&str>, Option<&str>)> =
        crates.iter().map(|c| (c.clone(), None, None)).collect();
    if with_defaults {
        targets.extend(
            DEFAULT_TESTS
                .iter()
                .map(|(c, t)| ((*c).to_string(), Some(*t), None)),
        );
        targets.extend(
            DEFAULT_FILTERED
                .iter()
                .map(|(c, f)| ((*c).to_string(), None, Some(*f))),
        );
    }
    for (krate, test, filter) in &targets {
        let (features, flags, cases) = settings(krate);
        // The rustup proxy (not `$CARGO`, which is pinned to the stable toolchain) resolves `+nightly`.
        let mut cmd = Command::new("cargo");
        no_incremental(&mut cmd);
        cmd.current_dir(workspace_root())
            .env_remove("RUSTUP_TOOLCHAIN")
            .env("PROPTEST_CASES", cases)
            .env("MIRIFLAGS", miri_flags(flags))
            .args(["+nightly", "miri", "test", "-p", krate]);
        if !features.is_empty() {
            cmd.args(["--features", features]);
        }
        if let Some(t) = test {
            cmd.args(["--test", t]);
        }
        if let Some(f) = filter {
            cmd.arg("--lib");
            if !f.is_empty() {
                cmd.args(["--", f]);
            }
        }
        run_cmd(&mut cmd)?;
    }
    println!("miri: {} target(s) clean", targets.len());
    Ok(())
}

/// Whether `crate_name` has a Miri target in the default lists.
fn has_miri_target(crate_name: &str) -> bool {
    DEFAULT_CRATES.contains(&crate_name)
        || DEFAULT_TESTS.iter().any(|(c, _)| *c == crate_name)
        || DEFAULT_FILTERED.iter().any(|(c, _)| *c == crate_name)
}

/// Whether a crate root forbids `unsafe` code crate-wide.
#[must_use]
pub fn forbids_unsafe(lib_rs: &str) -> bool {
    lib_rs.lines().any(|l| l.trim() == "#![forbid(unsafe_code)]")
}

/// Checks the unsafe-code policy (see the module docs) over every workspace library crate
/// (`crates/*` and `tools/*`, each with a `src/lib.rs`; examples, tests and benches are not
/// library crates).
pub fn check_policy() -> R {
    let root = workspace_root();
    let mut dirs = Vec::new();
    for parent in ["crates", "tools"] {
        for entry in std::fs::read_dir(root.join(parent))? {
            dirs.push(entry?.path());
        }
    }
    dirs.sort();
    let mut problems = Vec::new();
    let mut checked = 0usize;
    for dir in dirs {
        let lib = dir.join("src/lib.rs");
        let manifest = dir.join("Cargo.toml");
        if !lib.is_file() || !manifest.is_file() {
            continue;
        }
        let manifest = std::fs::read_to_string(&manifest)?;
        let Some(name) = manifest.lines().find_map(|l| {
            l.trim()
                .strip_prefix("name = ")
                .map(|n| n.trim_matches('"').to_string())
        }) else {
            continue;
        };
        // Crates outside the workspace (e.g. `twine-esp`, built with another toolchain).
        let rel = dir
            .strip_prefix(&root)
            .unwrap_or(&dir)
            .to_string_lossy()
            .replace('\\', "/");
        let workspace = std::fs::read_to_string(root.join("Cargo.toml"))?;
        if !workspace.contains(&format!("\"{rel}\"")) {
            continue;
        }
        checked += 1;
        let forbids = forbids_unsafe(&std::fs::read_to_string(&lib)?);
        match (forbids, has_miri_target(&name)) {
            (false, false) => problems.push(format!(
                "{name}: allows `unsafe` code but has no Miri target \
                 (add `#![forbid(unsafe_code)]` to src/lib.rs, or a Miri target in tools/xtask/src/cmd/miri.rs)"
            )),
            (true, true) => problems.push(format!(
                "{name}: forbids `unsafe` code, so its Miri target checks nothing: remove it"
            )),
            _ => {}
        }
    }
    if problems.is_empty() {
        println!("unsafe policy: {checked} library crates ok");
        Ok(())
    } else {
        for p in &problems {
            println!("unsafe policy: {p}");
        }
        Err(format!("unsafe policy: {} problem(s)", problems.len()).into())
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn forbid_is_detected_only_crate_wide() {
        assert!(forbids_unsafe("//! docs\n#![no_std]\n#![forbid(unsafe_code)]\n"));
        assert!(!forbids_unsafe("#![deny(unsafe_code)]\n"));
        assert!(!forbids_unsafe("#[forbid(unsafe_code)]\nmod m {}\n"));
    }

    #[test]
    fn workspace_follows_the_policy() {
        check_policy().unwrap();
    }
}
