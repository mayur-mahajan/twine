//! `cargo xtask ci [--quick]`: every check CI runs, in order, stopping at the first failure.
//!
//! A summary table is printed at the end. `--quick` skips `nostd`, `safety-exclusions`,
//! `stack-budget`, `doc`, `bench-build`, `miri` and `firmware`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde_json::Value;

use crate::cmd::{
    divisions, firmware, fonts, guidelines, images, layers, miri, nostd, progress, safety, shifts, sim,
    snapshots, stack, style_props, todo, traceability,
};
use crate::util::{R, cargo, cargo_ci, output, run as run_cmd, warn};

/// The safety profile's feature (`safety`, on the facade and every profile crate) is left out of
/// the all-features and `defmt` passes too: it refuses `perf-monitor`, `fs`, `ttf` and
/// `img-jpeg`, which those passes enable. The `clippy-safety` stage lints it.
///
/// `(crate, feature)` pairs left out of the all-features clippy pass because they cannot be
/// linted meaningfully on the host (`defmt` would win over `log`, hiding the `log` backend, and
/// needs a target-side logger to link tests). The `defmt` code paths are linted by the
/// `clippy-defmt` stage instead (see [`defmt_config`]).
pub const CLIPPY_ALL_FEATURES_EXCLUDE: &[(&str, &str)] = &[
    ("twine-core", "defmt"),
    ("twine", "defmt"),
    ("twine-view", "defmt"),
    ("twine-hal", "defmt"),
    ("twine-drivers", "defmt"),
    ("twine-embassy", "defmt"),
    ("twine-demos", "defmt"),
    ("twine-reactive", "defmt"),
    ("twine-render", "defmt"),
    ("twine-text", "defmt"),
    ("twine-image", "defmt"),
    ("twine-vector", "defmt"),
    ("twine-anim", "defmt"),
    ("twine-style", "defmt"),
    ("twine-layout", "defmt"),
    ("twine-engine", "defmt"),
    ("twine-theme", "defmt"),
    ("twine-widgets", "defmt"),
    ("twine-widgets-ext", "defmt"),
    ("twine-fs", "defmt"),
    ("twine-lottie", "defmt"),
    ("twine-extra", "defmt"),
    ("twine-accel-stm32", "defmt"),
    ("twine-embedded-graphics", "defmt"),
    // `stm32-metapac` accepts exactly one chip feature; the all-features pass keeps `stm32f429zi`.
    ("twine-accel-stm32", "stm32f746ng"),
    ("twine-accel-stm32", "stm32h743zi"),
    // Needs SDL2; checked by the `eg-sim` stage when it is installed.
    ("twine-embedded-graphics", "eg-sim"),
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
            if feat == "default"
                || feat == safety::FEATURE
                || CLIPPY_ALL_FEATURES_EXCLUDE.contains(&(name, feat.as_str()))
            {
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

/// Whether `feat` itself conflicts with the `clippy-defmt` configuration: `log` (a second
/// logging backend; `defmt` would win, and nobody ships the pair) or a `std` feature (`std`, or a
/// name ending in `-std` such as `platform-std`; `defmt` targets `no_std` firmware).
fn is_defmt_conflict(feat: &str) -> bool {
    feat == "log" || feat == "std" || feat.ends_with("-std")
}

/// Feature tables of the workspace packages, by package name.
pub type FeatureTables<'a> = BTreeMap<&'a str, &'a serde_json::Map<String, Value>>;

/// Whether feature `feat` of package `pkg` is, or enables directly or through other features of
/// the workspace, a feature for which `hit(package, feature)` holds (on `pkg` or on a
/// dependency). `dep:` entries are not followed.
pub fn feature_implies(
    features: &FeatureTables<'_>,
    pkg: &str,
    feat: &str,
    hit: &dyn Fn(&str, &str) -> bool,
    depth: u32,
) -> bool {
    if hit(pkg, feat) {
        return true;
    }
    // Feature graphs are acyclic (cargo rejects cycles); the bound only guards malformed input.
    if depth > 64 {
        return false;
    }
    let Some(entries) = features
        .get(pkg)
        .and_then(|f| f.get(feat))
        .and_then(Value::as_array)
    else {
        return false;
    };
    entries.iter().filter_map(Value::as_str).any(|e| {
        if e.starts_with("dep:") {
            false
        } else if let Some((dep, dep_feat)) = e.split_once('/') {
            // `dep?/feat` only applies when `dep` is enabled; it still enables `feat` then.
            feature_implies(features, dep.trim_end_matches('?'), dep_feat, hit, depth + 1)
        } else {
            feature_implies(features, pkg, e, hit, depth + 1)
        }
    })
}

/// Whether feature `feat` of package `pkg` enables, directly or through other features of the
/// workspace, a feature for which [`is_defmt_conflict`] holds (on `pkg` or on a dependency).
fn implies_defmt_conflict(features: &FeatureTables<'_>, pkg: &str, feat: &str) -> bool {
    feature_implies(features, pkg, feat, &|_, f| is_defmt_conflict(f), 0)
}

/// A package name and the features to enable on it.
type CrateFeatures = (String, Vec<String>);

/// The `clippy-defmt` configuration, derived from the workspace manifests: one `(package,
/// features)` entry per library crate with a `defmt` feature. `features` is `defmt` plus every
/// optional feature a `no_std` + `defmt` build can combine with it: all except `default`, those
/// that are or imply a conflict ([`is_defmt_conflict`]) and the [`CLIPPY_ALL_FEATURES_EXCLUDE`]
/// pairs (one chip per PAC, SDL2-only features), so the `defmt::Format` impls and defmt log calls
/// of optional code are linted too.
fn defmt_config(metadata: &str) -> Result<Vec<CrateFeatures>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(metadata)?;
    let pkgs = meta["packages"]
        .as_array()
        .ok_or("metadata: missing `packages`")?;
    let features: FeatureTables<'_> = pkgs
        .iter()
        .filter_map(|p| Some((p["name"].as_str()?, p["features"].as_object()?)))
        .collect();
    let mut config = Vec::new();
    for pkg in pkgs {
        let name = pkg["name"].as_str().unwrap_or_default();
        let is_lib = pkg["targets"].as_array().is_some_and(|ts| {
            ts.iter()
                .any(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "lib")))
        });
        let Some(feats) = features.get(name) else {
            continue;
        };
        if !is_lib || !feats.contains_key("defmt") {
            continue;
        }
        let list: Vec<String> = feats
            .keys()
            .filter(|feat| {
                let excluded =
                    *feat != "defmt" && CLIPPY_ALL_FEATURES_EXCLUDE.contains(&(name, feat.as_str()));
                *feat != "default"
                    && *feat != safety::FEATURE
                    && !excluded
                    && !implies_defmt_conflict(&features, name, feat)
            })
            .cloned()
            .collect();
        config.push((name.to_owned(), list));
    }
    config.sort();
    Ok(config)
}

/// Clippy over every library crate with a `defmt` feature, configured the way firmware uses it:
/// no default features, `defmt` as the logging backend, every compatible optional feature (see
/// [`defmt_config`]).
///
/// One `cargo clippy -p <crate>` per crate rather than one invocation for all: cargo applies
/// `pkg/feat` arguments inconsistently when several packages are selected (it routes them through
/// dev-dependency edges), and a per-crate build also proves that each crate's own `defmt` feature
/// forwards to every dependency whose log macros it expands (feature unification across crates
/// would hide a missing forward). Host target: `defmt` needs its logger symbols only when a
/// binary links, and clippy does not link.
fn clippy_defmt() -> Result<Outcome, Box<dyn std::error::Error>> {
    let meta = output(cargo().args(["metadata", "--format-version", "1", "--no-deps"]))?;
    let config = defmt_config(&meta)?;
    if config.is_empty() {
        return Ok(Outcome::Skipped("no crate has a `defmt` feature".into()));
    }
    for (package, features) in &config {
        run_cmd(cargo_ci().args([
            "clippy",
            "-p",
            package,
            "--lib",
            "--no-default-features",
            "--features",
            &features.join(","),
            "--",
            "-D",
            "warnings",
        ]))?;
    }
    Ok(Outcome::Ok)
}

/// Set once the `test` stage ran: its `cargo test --workspace` already compared every
/// snapshot, so the `snapshots` stage does not run the whole suite a second time.
static TESTS_RAN: AtomicBool = AtomicBool::new(false);

/// The whole workspace test suite (snapshot comparisons included, the run reported).
fn test() -> Result<Outcome, Box<dyn std::error::Error>> {
    TESTS_RAN.store(true, Ordering::Relaxed);
    ok(snapshots::run(snapshots::Action::Check))
}

/// Tests of feature-gated backends that the default `cargo test --workspace` does not compile.
fn test_features() -> Result<Outcome, Box<dyn std::error::Error>> {
    run_cmd(cargo_ci().args(["test", "-p", "twine-core", "--features", "log"]))?;
    // The safety profile (R5.S04): the arena's 48-bit generations, the animation and timer
    // tables, the reactive runtime and the engine (limits, handler serials, faults) in that
    // profile — unit and integration tests; the doctests show the default profile's API (e.g.
    // `expect_context`, non-preallocated `Limits`), which the profile removes or refuses.
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-core",
        "--features",
        "safety",
        "--lib",
        "--tests",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-anim",
        "--features",
        "safety",
        "--lib",
        "--tests",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-reactive",
        "--features",
        "safety",
        "--lib",
        "--tests",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-engine",
        "--features",
        "safety",
        "--lib",
        "--tests",
    ]))?;
    // The view layer in the profile (R5.S05): its tests use the fallible forms (`try_build`,
    // `use_context`, `use_navigator`) and `twine_testing::HARNESS_LIMITS` (preallocated
    // there); the tests of the removed panicking APIs are `cfg(not(feature = "safety"))`.
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-view",
        "--features",
        "safety",
        "--lib",
        "--tests",
    ]))?;
    // Display integrity (R5.S06): opt-in, so the default runs above build without it; its
    // tests (`tests/integrity.rs`), doctests and the gated unit tests run here.
    for pkg in [
        "twine-render",
        "twine-text",
        "twine-engine",
        "twine-widgets",
        "twine-view",
    ] {
        run_cmd(cargo_ci().args(["test", "-p", pkg, "--features", "display-integrity"]))?;
    }
    // Partitioned runtimes (R5.S07, ADR-001, SAF-034): opt-in and not in `full`, so the runs
    // above build without them. The runtime's slots, claims, ownership and seal domains (with
    // the host-only cross-partition check of `debug-checks`), the core's partition table, and
    // the view's fault-injection isolation test with its control run — in the default profile
    // and in the safety profile (which adds the `no-panic-runtime` misuse case).
    run_cmd(cargo_ci().args(["test", "-p", "twine-core", "--features", "partitions", "--lib"]))?;
    run_cmd(cargo_ci().args(["test", "-p", "twine-reactive", "--features", "partitions"]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-reactive",
        "--features",
        "partitions,debug-checks",
        "--test",
        "partitions",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-reactive",
        "--features",
        "safety,partitions",
        "--lib",
        "--tests",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-view",
        "--features",
        "partitions,display-integrity",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-view",
        "--features",
        "safety,partitions",
        "--lib",
        "--tests",
    ]))?;
    // The engine's parts the profile excludes, which its dev-dependencies no longer enable.
    run_cmd(cargo_ci().args(["test", "-p", "twine-engine", "--features", "perf-monitor,fs"]))?;
    // The reactive runtime's `no-panic-runtime` behaviour (tests/no_panic_runtime.rs) and the
    // rest of its suite with the feature on.
    run_cmd(cargo_ci().args(["test", "-p", "twine-reactive", "--features", "no-panic-runtime"]))?;
    // Layout warnings (invalid grid cells) captured through `log`.
    run_cmd(cargo_ci().args(["test", "-p", "twine-layout", "--features", "log"]))?;
    // SVG images: vectors with `svg`, the placeholder without it.
    run_cmd(cargo_ci().args(["test", "-p", "twine-widgets", "--test", "svg_image"]))?;
    run_cmd(cargo_ci().args([
        "test",
        "-p",
        "twine-widgets",
        "--features",
        "svg",
        "--test",
        "svg_image",
    ]))?;
    // DMA2D bit layout cross-checked against the chip PAC (DMA2D v1 and v2).
    for chip in ["stm32f429zi", "stm32h743zi"] {
        run_cmd(cargo_ci().args(["test", "-p", "twine-accel-stm32", "--lib", "--features", chip]))?;
    }
    // Decoders and glyph readers in the release profile (R5.S09, SAF-032; R5.S10, SAF-033): there
    // an out-of-range shift silently masks its amount instead of panicking, so the header
    // validation that keeps every header-derived shift amount in range — and refuses zero-size
    // images — is checked without debug assertions too.
    run_cmd(cargo_ci().args([
        "test",
        "--release",
        "-p",
        "twine-image",
        "--lib",
        "--test",
        "shift_amounts",
        "--test",
        "decoders",
        "--test",
        "gif",
        "--test",
        "zero_size",
    ]))?;
    run_cmd(cargo_ci().args([
        "test",
        "--release",
        "-p",
        "twine-text",
        "--lib",
        "--test",
        "shift_amounts",
    ]))?;
    Ok(Outcome::Ok)
}

/// The plan's §5 checklist agrees with its steps (skipped when the plan is not in the checkout).
fn progress_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    if !progress::plan_exists() {
        return Ok(Outcome::Skipped(format!(
            "{} not in this checkout",
            progress::PLAN
        )));
    }
    ok(progress::run())
}

fn todo_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(todo::run())
}

fn layers_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    layers::run()?;
    ok(miri::check_policy())
}

fn fonts_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(fonts::run(true))
}

fn images_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(images::gen_assets(true).and_then(|()| images::run(true)))
}

/// The requirements → tests matrix is current and every requirement is verified (R5.S08).
fn traceability_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(traceability::run(true))
}

/// Every shift by a non-literal amount in library code carries a `// SHIFT:` proof, and the
/// inventory `docs/safety/shifts.md` is current (R5.S09).
fn shifts_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(shifts::run(true))
}

/// Every divisor that is not a literal in library code is non-zero by a `NonZero` type, a guard
/// or a `// DIV:` proof, and the inventory `docs/safety/divisions.md` is current (R5.S10).
fn divisions_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(divisions::run(true))
}

/// The measured block of the coding-guidelines register is current (R5.S10).
fn guidelines_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(guidelines::run(true))
}

fn style_props_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(style_props::run(true))
}

/// Clippy over the facade in the safety profile (its lints denied) and the lint-status
/// ratchet (`docs/safety/profile.md`).
fn clippy_safety() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(safety::clippy_safety())
}

/// Every part the safety profile excludes fails to compile together with it.
fn safety_exclusions() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(safety::check_exclusions())
}

/// The stack of the engine's recursive tree walks at the reference depth stays within its
/// budget (release build; R5.S05).
fn stack_budget() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(stack::run(false))
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

/// Features of the `doc` stage (see [`doc`]): everything a user can enable on the facade
/// (`full`, `std`, `embassy`, every platform, the drivers module with every driver), plus the
/// optional parts of crates the facade does not re-export. Default-feature docs are checked
/// separately (`doc` builds `twine` without features first), so feature-gated links must not
/// break them.
pub const DOC_FEATURES: &str = "twine/full,twine/std,twine/embassy,twine/drivers,\
twine/platform-cortex-m,twine/platform-riscv,twine/platform-std,twine/platform-embassy,\
twine/drivers-fbdev,twine/drivers-evdev,\
twine-drivers/all,twine-drivers/async,twine-drivers/testkit,\
twine-fs/fs-std,twine-fs/fs-fat,\
twine-demos/multilang-cjk,twine-demos/ttf,twine-demos/vector,twine-demos/lottie";

fn doc() -> Result<Outcome, Box<dyn std::error::Error>> {
    // The facade with its default features: what `cargo doc` gives a new user. Feature-gated
    // items are mentioned without links there (cfg-conditional doc text).
    run_cmd(
        cargo_ci()
            .env("RUSTDOCFLAGS", "-D warnings")
            .args(["doc", "--no-deps", "-p", "twine"]),
    )?;
    // Document (and link-check) every feature-gated API: everything the facade offers, every
    // driver of twine-drivers, and the optional parts of crates the facade does not re-export.
    ok(run_cmd(cargo_ci().env("RUSTDOCFLAGS", "-D warnings").args([
        "doc",
        "--workspace",
        "--no-deps",
        "--features",
        DOC_FEATURES,
    ])))
}

/// Compiles every benchmark without running it.
fn bench_build() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(run_cmd(cargo_ci().args(["bench", "--workspace", "--no-run"])))
}

/// Builds (and lints) the examples that need SDL2; skipped with a warning without SDL2.
fn eg_sim() -> Result<Outcome, Box<dyn std::error::Error>> {
    let Some(lib) = sim::sdl2_lib_dir() else {
        return Ok(Outcome::Skipped(
            "SDL2 not found (`brew install sdl2` / `apt install libsdl2-dev`)".into(),
        ));
    };
    for (example, package, feature) in sim::FEATURE_EXAMPLES {
        let mut build = cargo_ci();
        build.args([
            "build",
            "-p",
            package,
            "--example",
            example,
            "--features",
            feature,
        ]);
        sim::add_sdl2_env(&mut build, &lib);
        run_cmd(&mut build)?;
        let mut lint = cargo_ci();
        lint.args([
            "clippy",
            "-p",
            package,
            "--example",
            example,
            "--features",
            feature,
            "--",
            "-D",
            "warnings",
        ]);
        run_cmd(&mut lint)?;
    }
    Ok(Outcome::Ok)
}

/// The golden-image manifests parse and hold no entry awaiting approval; the snapshot
/// comparisons themselves run with the workspace tests (here only if the `test` stage did not).
fn snapshot_check() -> Result<Outcome, Box<dyn std::error::Error>> {
    snapshots::verify()?;
    if TESTS_RAN.load(Ordering::Relaxed) {
        return Ok(Outcome::Ok);
    }
    ok(snapshots::run(snapshots::Action::Check))
}

fn firmware_build() -> Result<Outcome, Box<dyn std::error::Error>> {
    ok(firmware::run(None, false, None))
}

/// `(name, runs in --quick mode, stage)`.
const STAGES: &[(&str, bool, StageFn)] = &[
    ("fmt", true, fmt),
    ("clippy", true, clippy),
    ("clippy-all-features", true, clippy_all_features),
    ("clippy-defmt", true, clippy_defmt),
    ("clippy-safety", true, clippy_safety),
    ("test", true, test),
    ("test-features", true, test_features),
    ("todo-check", true, todo_check),
    ("progress", true, progress_check),
    ("layers", true, layers_check),
    ("fonts", true, fonts_check),
    ("images", true, images_check),
    ("style-props", true, style_props_check),
    ("traceability", true, traceability_check),
    ("shifts", true, shifts_check),
    ("divisions", true, divisions_check),
    ("guidelines", true, guidelines_check),
    ("nostd", false, nostd_build),
    ("safety-exclusions", false, safety_exclusions),
    ("stack-budget", false, stack_budget),
    ("doc", false, doc),
    ("bench-build", false, bench_build),
    ("miri", false, miri_check),
    ("eg-sim", true, eg_sim),
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

    #[test]
    fn defmt_config_keeps_no_std_features_of_defmt_libs() {
        let meta = serde_json::json!({ "packages": [
            { "name": "twine-core", "targets": [{ "kind": ["lib"] }],
              "features": { "default": [], "log": ["dep:log"], "defmt": ["dep:defmt"], "std": [] } },
            { "name": "twine-fs", "targets": [{ "kind": ["lib"] }],
              "features": { "defmt": ["twine-core/defmt"], "log": ["twine-core/log"],
                            "std": ["twine-core/std"], "fs-std": ["std"], "fs-fat": ["dep:sd"],
                            "logged": ["twine-core?/log"] } },
            { "name": "twine-accel-stm32", "targets": [{ "kind": ["lib"] }],
              "features": { "defmt": [], "stm32f429zi": [], "stm32h743zi": [], "platform-std": [] } },
            { "name": "twine-hal", "targets": [{ "kind": ["lib"] }], "features": { "log": [] } },
            { "name": "tool", "targets": [{ "kind": ["bin"] }], "features": { "defmt": [] } }
        ]})
        .to_string();
        let config = defmt_config(&meta).unwrap();
        let expect = |p: &str, f: &[&str]| (p.to_owned(), f.iter().map(|s| (*s).to_owned()).collect());
        assert_eq!(
            config,
            vec![
                expect("twine-accel-stm32", &["defmt", "stm32f429zi"]),
                expect("twine-core", &["defmt"]),
                expect("twine-fs", &["defmt", "fs-fat"]),
            ]
        );
    }
}
