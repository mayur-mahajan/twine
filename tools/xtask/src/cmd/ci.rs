//! `cargo xtask ci [--quick]`: every check CI runs, in order, stopping at the first failure.
//!
//! A summary table is printed at the end. `--quick` skips `nostd`, `doc`, `bench-build`, `miri`
//! and `firmware`.

use std::time::Instant;

use serde_json::Value;

use crate::cmd::{firmware, fonts, images, layers, miri, nostd, snapshots, todo};
use crate::util::{R, cargo, output, run as run_cmd, warn};

/// `(crate, feature)` pairs left out of the all-features clippy pass because they cannot be
/// linted meaningfully on the host (`defmt` would win over `log`, hiding the `log` backend, and
/// needs a target-side logger to link tests).
pub const CLIPPY_ALL_FEATURES_EXCLUDE: &[(&str, &str)] = &[
    ("twine-core", "defmt"),
    ("twine-hal", "defmt"),
    ("twine-reactive", "defmt"),
    ("twine-render", "defmt"),
    ("twine-text", "defmt"),
    ("twine-image", "defmt"),
];

/// Outcome of one stage.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Ok,
    Skipped(String),
    Failed(String),
}

type StageFn = fn() -> Result<Outcome, Box<dyn std::error::Error>>;

fn ok(r: R) -> Result<Outcome, Box<dyn std::error::Error>> {
    r.map(|()| Outcome::Ok)
}

fn fmt() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo().args(["fmt", "--all", "--", "--check"])))
}

fn clippy() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])))
}

/// Every `crate/feature` of the workspace except `default`, implicit `dep:` features and the
/// excluded pairs.
fn all_features_list(metadata: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(metadata)?;
    let mut list = Vec::new();
    for pkg in meta["packages"]
        .as_array()
        .ok_or("metadata: missing `packages`")?
    {
        let name = pkg["name"].as_str().unwrap_or_default();
        let Some(features) = pkg["features"].as_object() else {
            continue;
        };
        for feat in features.keys() {
            if feat == "default" || CLIPPY_ALL_FEATURES_EXCLUDE.contains(&(name, feat.as_str())) {
                continue;
            }
            list.push(format!("{name}/{feat}"));
        }
    }
    list.sort();
    Ok(list)
}

fn clippy_all_features() -> Result<Outcome, Box<dyn std::error::Error>> {
    let meta = output(cargo().args(["metadata", "--format-version", "1", "--no-deps"]))?;
    let features = all_features_list(&meta)?;
    if features.is_empty() {
        return Ok(Outcome::Skipped("no optional features in the workspace".into()));
    }
    ok(run_cmd(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--features",
        &features.join(","),
        "--",
        "-D",
        "warnings",
    ])))
}

fn test() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo().args(["test", "--workspace"])))
}

/// Tests of feature-gated backends that the default `cargo test --workspace` does not compile.
fn test_features() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo().args([
        "test",
        "-p",
        "twine-core",
        "--features",
        "log",
    ])))
}

fn todo_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(todo::run())
}

fn layers_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(layers::run())
}

fn fonts_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(fonts::run(true))
}

fn images_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(images::gen_assets(true).and_then(|()| images::run(true)))
}

fn nostd_build() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(nostd::run())
}

/// Miri over the default crates; skipped (with a warning) when nightly Miri is not installed.
fn miri_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    if !miri::miri_available() {
        return Ok(Outcome::Skipped(
            "nightly toolchain with miri not installed (`rustup toolchain install nightly --component miri`)"
                .into(),
        ));
    }
    ok(miri::run(&[]))
}

fn doc() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo().env("RUSTDOCFLAGS", "-D warnings").args([
        "doc",
        "--workspace",
        "--no-deps",
    ])))
}

/// Compiles every benchmark without running it.
fn bench_build() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo().args(["bench", "--workspace", "--no-run"])))
}

fn snapshot_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(snapshots::run(false))
}

fn firmware_build() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(firmware::run(None))
}

/// `(name, runs in --quick mode, stage)`.
const STAGES: &[(&str, bool, StageFn)] = &[
    ("fmt", true, fmt),
    ("clippy", true, clippy),
    ("clippy-all-features", true, clippy_all_features),
    ("test", true, test),
    ("test-features", true, test_features),
    ("todo-check", true, todo_check),
    ("layers", true, layers_check),
    ("fonts", true, fonts_check),
    ("images", true, images_check),
    ("nostd", false, nostd_build),
    ("doc", false, doc),
    ("bench-build", false, bench_build),
    ("miri", false, miri_check),
    ("snapshots", true, snapshot_check),
    ("firmware", false, firmware_build),
];

/// Names of all stages, in execution order.
#[must_use]
pub fn stage_names() -> Vec<&'static str> {
    STAGES.iter().map(|(n, _, _)| *n).collect()
}

/// Runs all stages (or only those named in `only`); `quick` skips the slow ones.
pub fn run(quick: bool, only: &[String]) -> R {
    if let Some(unknown) = only.iter().find(|o| !STAGES.iter().any(|(n, _, _)| n == o)) {
        return Err(format!(
            "ci: unknown stage `{unknown}`; stages: {}",
            stage_names().join(", ")
        )
        .into());
    }
    let mut results: Vec<(&str, Outcome, f64)> = Vec::new();
    let mut failed = false;
    for (name, in_quick, stage) in STAGES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        if failed {
            results.push((name, Outcome::Skipped("earlier stage failed".into()), 0.0));
            continue;
        }
        if quick && !in_quick {
            results.push((name, Outcome::Skipped("--quick".into()), 0.0));
            continue;
        }
        eprintln!("\n\x1b[1;36m==> ci: {name}\x1b[0m");
        let start = Instant::now();
        let outcome = stage().unwrap_or_else(|e| Outcome::Failed(e.to_string()));
        if let Outcome::Skipped(why) = &outcome {
            warn(&format!("{name} skipped: {why}"));
        }
        failed = matches!(outcome, Outcome::Failed(_));
        results.push((name, outcome, start.elapsed().as_secs_f64()));
    }
    println!("\n{:<24} {:<8} {:>8}  details", "stage", "result", "time");
    println!("{}", "-".repeat(64));
    for (name, outcome, secs) in &results {
        let (label, detail) = match outcome {
            Outcome::Ok => ("\x1b[32mok\x1b[0m    ", String::new()),
            Outcome::Skipped(w) => ("\x1b[33mskipped\x1b[0m", w.clone()),
            Outcome::Failed(e) => ("\x1b[31mFAILED\x1b[0m ", e.clone()),
        };
        println!("{name:<24} {label:<8} {secs:>7.1}s  {detail}");
    }
    if failed { Err("ci: failed".into()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_features_excludes_default_and_listed_pairs() {
        let meta = serde_json::json!({ "packages": [
            { "name": "twine-core", "features": { "default": [], "log": ["dep:log"], "defmt": ["dep:defmt"], "std": [] } },
            { "name": "twine-hal", "features": {} }
        ]})
        .to_string();
        assert_eq!(
            all_features_list(&meta).unwrap(),
            vec!["twine-core/log", "twine-core/std"]
        );
    }
}
