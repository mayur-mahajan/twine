//! `cargo xtask miri [crate…]`: runs the tests of the given crates under Miri (nightly) to
//! detect undefined behaviour.

use std::process::Command;

use crate::util::{R, run as run_cmd, workspace_root};

/// Crates checked when none are given.
pub const DEFAULT_CRATES: &[&str] = &["twine-core", "twine-reactive"];

/// Single test targets also checked when no crates are given: `(crate, integration test)`.
/// Covers `unsafe` in crates that are otherwise too heavy for Miri (`twine-testing`'s
/// `CountingAllocator`).
pub const DEFAULT_TESTS: &[(&str, &str)] = &[("twine-testing", "alloc_count")];

/// Unit-test subsets also checked when no crates are given: `(crate, test name filter)`, run as
/// `cargo miri test -p <crate> --lib -- <filter>`. `twine-engine`'s tree (arena links, iterators)
/// and event dispatch (handlers taken out and put back while they run) run under Miri; its
/// rendering tests are too slow for it. `twine-view`'s raw task waker runs under Miri too.
pub const DEFAULT_FILTERED: &[(&str, &str)] = &[
    ("twine-engine", "tree::"),
    ("twine-engine", "handlers::"),
    // The `Ui`'s task waker (a `RawWaker` over a `&'static UiWaker`).
    ("twine-view", "ui::tests"),
];

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
            cmd.args(["--lib", "--", f]);
        }
        run_cmd(&mut cmd)?;
    }
    println!("miri: {} target(s) clean", targets.len());
    Ok(())
}
