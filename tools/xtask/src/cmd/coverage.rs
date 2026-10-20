//! `cargo xtask coverage [--mcdc]`: per-crate coverage (R5.S08 evidence).
//!
//! - Without flags: line coverage of the workspace with the per-crate minimums of [`MIN`]
//!   (stable toolchain).
//! - `--mcdc`: MC/DC (modified condition/decision coverage) of the safety-relevant core crates
//!   [`MCDC_CRATES`], with line and branch coverage next to it — `cargo +nightly llvm-cov
//!   --mcdc`, which needs a nightly toolchain with the `llvm-tools-preview` component (rustc's
//!   MC/DC instrumentation is unstable). Without it the command says exactly what is missing
//!   and falls back to line coverage of the same crates on stable. Report only: no MC/DC
//!   minimums yet (the first measured numbers set them).
//!
//! Requires `cargo-llvm-cov` (`cargo install cargo-llvm-cov`); nothing is installed by the
//! command.

use std::collections::BTreeMap;
use std::process::Command;

use serde_json::Value;

use crate::util::{R, cargo_ci, run as run_cmd, warn, which, workspace_root};

/// The crates whose MC/DC is measured by `cargo xtask coverage --mcdc` (plan R5.S08).
pub const MCDC_CRATES: &[&str] = &[
    "twine-core",
    "twine-reactive",
    "twine-engine",
    "twine-render",
    "twine-layout",
    "twine-style",
];

/// Minimum line coverage (percent) per crate.
pub const MIN: &[(&str, f64)] = &[
    ("twine-core", 85.0),
    ("twine-reactive", 85.0),
    ("twine-render", 85.0),
    ("twine-text", 85.0),
    ("twine-style", 85.0),
    ("twine-layout", 85.0),
    ("twine-engine", 85.0),
    ("twine-view", 85.0),
    ("twine-widgets", 75.0),
    ("twine-widgets-ext", 75.0),
];

/// Crates with fewer instrumented lines than this are reported but not enforced.
pub const ENFORCE_MIN_LINES: u64 = 200;

/// Line counts of one crate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrateLines {
    /// Instrumented lines.
    pub count: u64,
    /// Covered lines.
    pub covered: u64,
}

impl CrateLines {
    /// Coverage in percent (100 when there is nothing to cover).
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // line counts are far below 2^52
    pub fn percent(self) -> f64 {
        if self.count == 0 {
            100.0
        } else {
            self.covered as f64 * 100.0 / self.count as f64
        }
    }
}

/// Extracts the crate name from a source path (`…/crates/<name>/…` or `…/tools/<name>/…`).
fn crate_of(path: &str) -> Option<String> {
    let norm = path.replace('\\', "/");
    for marker in ["/crates/", "/tools/"] {
        if let Some(pos) = norm.rfind(marker) {
            let rest = &norm[pos + marker.len()..];
            return rest.split('/').next().map(str::to_string);
        }
    }
    None
}

/// Parses `cargo llvm-cov --json --summary-only` output into per-crate line counts.
pub fn parse_summary(json: &str) -> Result<BTreeMap<String, CrateLines>, Box<dyn std::error::Error>> {
    let v: Value = serde_json::from_str(json)?;
    let mut map: BTreeMap<String, CrateLines> = BTreeMap::new();
    for data in v["data"].as_array().ok_or("llvm-cov: missing `data`")? {
        for file in data["files"].as_array().ok_or("llvm-cov: missing `files`")? {
            let Some(name) = file["filename"].as_str().and_then(crate_of) else {
                continue;
            };
            let lines = &file["summary"]["lines"];
            let e = map.entry(name).or_default();
            e.count += lines["count"].as_u64().unwrap_or(0);
            e.covered += lines["covered"].as_u64().unwrap_or(0);
        }
    }
    Ok(map)
}

/// Covered / total counts of one coverage kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Total items (lines, branches, conditions).
    pub count: u64,
    /// Covered items.
    pub covered: u64,
}

impl Counts {
    fn add(&mut self, v: &Value) {
        self.count += v["count"].as_u64().unwrap_or(0);
        self.covered += v["covered"].as_u64().unwrap_or(0);
    }

    /// `covered/count (percent)`, or `-` when there is nothing to cover.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // counts are far below 2^52
    pub fn display(self) -> String {
        if self.count == 0 {
            "-".into()
        } else {
            format!(
                "{}/{} ({:.1} %)",
                self.covered,
                self.count,
                self.covered as f64 * 100.0 / self.count as f64
            )
        }
    }
}

/// Line, branch and MC/DC counts of one crate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrateCoverage {
    /// Lines.
    pub lines: Counts,
    /// Branches (`--branch`, nightly).
    pub branches: Counts,
    /// MC/DC conditions (`--mcdc`, nightly).
    pub mcdc: Counts,
}

/// Parses `cargo llvm-cov --json --summary-only` output into per-crate line, branch and MC/DC
/// counts (absent kinds count 0).
pub fn parse_full_summary(json: &str) -> Result<BTreeMap<String, CrateCoverage>, Box<dyn std::error::Error>> {
    let v: Value = serde_json::from_str(json)?;
    let mut map: BTreeMap<String, CrateCoverage> = BTreeMap::new();
    for data in v["data"].as_array().ok_or("llvm-cov: missing `data`")? {
        for file in data["files"].as_array().ok_or("llvm-cov: missing `files`")? {
            let Some(name) = file["filename"].as_str().and_then(crate_of) else {
                continue;
            };
            let sum = &file["summary"];
            let e = map.entry(name).or_default();
            e.lines.add(&sum["lines"]);
            e.branches.add(&sum["branches"]);
            e.mcdc.add(&sum["mcdc"]);
        }
    }
    Ok(map)
}

/// What the MC/DC run can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McdcSupport {
    /// `cargo +nightly llvm-cov --mcdc` can run.
    Available,
    /// It cannot; the reason names what is missing.
    Missing(String),
}

/// Whether a nightly toolchain with `llvm-tools` is installed (`cargo-llvm-cov` is checked
/// separately).
#[must_use]
pub fn mcdc_support() -> McdcSupport {
    let Ok(out) = Command::new("rustup")
        .args(["component", "list", "--toolchain", "nightly", "--installed"])
        .output()
    else {
        return McdcSupport::Missing("`rustup` not found".into());
    };
    if !out.status.success() {
        return McdcSupport::Missing(
            "no nightly toolchain (`rustup toolchain install nightly --component llvm-tools-preview`)".into(),
        );
    }
    let list = String::from_utf8_lossy(&out.stdout);
    if list.lines().any(|l| l.starts_with("llvm-tools")) {
        McdcSupport::Available
    } else {
        McdcSupport::Missing(
            "the nightly toolchain has no `llvm-tools` component \
             (`rustup component add llvm-tools-preview --toolchain nightly`)"
                .into(),
        )
    }
}

fn package_args(crates: &[&str]) -> Vec<String> {
    crates
        .iter()
        .flat_map(|c| ["-p".to_string(), (*c).to_string()])
        .collect()
}

/// MC/DC (and line/branch) coverage of [`MCDC_CRATES`]; falls back to line coverage on
/// stable when nightly MC/DC is not available.
fn run_mcdc() -> R {
    let out = workspace_root().join("target/coverage-mcdc.json");
    let support = mcdc_support();
    let mut cmd = cargo_ci();
    let nightly = support == McdcSupport::Available;
    if let McdcSupport::Missing(why) = &support {
        warn(&format!(
            "MC/DC needs nightly rustc with llvm-tools: {why}; falling back to line coverage on stable"
        ));
    }
    if nightly {
        cmd.arg("+nightly");
    }
    cmd.arg("llvm-cov").args(package_args(MCDC_CRATES));
    if nightly {
        cmd.args(["--mcdc", "--branch"]);
    }
    cmd.args([
        "--json",
        "--summary-only",
        "--output-path",
        &out.to_string_lossy(),
    ]);
    run_cmd(&mut cmd)?;
    let map = parse_full_summary(&std::fs::read_to_string(&out)?)?;
    println!(
        "\n{:<16} {:>24} {:>24} {:>24}",
        "crate", "lines", "branches", "MC/DC conditions"
    );
    println!("{}", "-".repeat(92));
    for c in MCDC_CRATES {
        let e = map.get(*c).copied().unwrap_or_default();
        println!(
            "{c:<16} {:>24} {:>24} {:>24}",
            e.lines.display(),
            e.branches.display(),
            e.mcdc.display()
        );
    }
    if !nightly {
        println!("(branches and MC/DC need nightly: not measured)");
    }
    Ok(())
}

/// Evaluates thresholds; returns the table rows and the failures.
fn evaluate(map: &BTreeMap<String, CrateLines>) -> (Vec<String>, Vec<String>) {
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (name, lines) in map {
        let min = MIN.iter().find(|(n, _)| n == name).map(|(_, m)| *m);
        let pct = lines.percent();
        let status = match min {
            None => "no threshold".to_string(),
            Some(_) if lines.count < ENFORCE_MIN_LINES => "not yet enforced".to_string(),
            Some(m) if pct + f64::EPSILON < m => {
                failures.push(format!("{name}: {pct:.1} % < {m:.0} %"));
                format!("FAIL (min {m:.0} %)")
            }
            Some(m) => format!("ok (min {m:.0} %)"),
        };
        rows.push(format!(
            "{name:<26} {:>7} {:>7} {pct:>7.1} %  {status}",
            lines.covered, lines.count
        ));
    }
    (rows, failures)
}

/// Runs coverage: line coverage of the workspace with the thresholds, or with `mcdc` the
/// MC/DC report of [`MCDC_CRATES`] (see the module docs).
pub fn run(mcdc: bool) -> R {
    if !which("cargo-llvm-cov") {
        return Err(
            "coverage: `cargo llvm-cov` not found\n  hint: `cargo install cargo-llvm-cov` \
                    (needs the `llvm-tools-preview` component from rust-toolchain.toml)"
                .into(),
        );
    }
    if mcdc {
        return run_mcdc();
    }
    let out = workspace_root().join("target/coverage.json");
    run_cmd(cargo_ci().args([
        "llvm-cov",
        "--workspace",
        "--json",
        "--summary-only",
        "--output-path",
        &out.to_string_lossy(),
    ]))?;
    let map = parse_summary(&std::fs::read_to_string(&out)?)?;
    let (rows, failures) = evaluate(&map);
    println!(
        "\n{:<26} {:>7} {:>7} {:>9}  status",
        "crate", "covered", "lines", "coverage"
    );
    println!("{}", "-".repeat(72));
    for r in rows {
        println!("{r}");
    }
    for (name, _) in MIN {
        if !map.contains_key(*name) {
            println!("{name:<26} {:>7} {:>7} {:>9}  not yet enforced", "-", "0", "-");
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("coverage below threshold: {}", failures.join("; ")).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "type": "llvm.coverage.json.export", "version": "2.0.1",
      "data": [{
        "files": [
          { "filename": "/w/crates/twine-core/src/geometry.rs",
            "summary": { "lines": { "count": 300, "covered": 270, "percent": 90.0 } } },
          { "filename": "/w/crates/twine-core/src/lib.rs",
            "summary": { "lines": { "count": 100, "covered": 50, "percent": 50.0 } } },
          { "filename": "/w/crates/twine-render/src/lib.rs",
            "summary": { "lines": { "count": 10, "covered": 0, "percent": 0.0 } } },
          { "filename": "/w/tools/xtask/src/main.rs",
            "summary": { "lines": { "count": 50, "covered": 10, "percent": 20.0 } } }
        ],
        "totals": { "lines": { "count": 460, "covered": 330, "percent": 71.7 } }
      }]
    }"#;

    #[test]
    fn parses_branch_and_mcdc_counts() {
        let json = r#"{"data": [{"files": [
            { "filename": "/w/crates/twine-core/src/a.rs",
              "summary": { "lines": { "count": 10, "covered": 9 },
                           "branches": { "count": 4, "covered": 3 },
                           "mcdc": { "count": 6, "covered": 2 } } },
            { "filename": "/w/crates/twine-core/src/b.rs",
              "summary": { "lines": { "count": 10, "covered": 1 } } }
        ]}]}"#;
        let map = parse_full_summary(json).unwrap();
        let core = map["twine-core"];
        assert_eq!(
            core.lines,
            Counts {
                count: 20,
                covered: 10
            }
        );
        assert_eq!(core.branches, Counts { count: 4, covered: 3 });
        assert_eq!(core.mcdc, Counts { count: 6, covered: 2 });
        assert_eq!(core.mcdc.display(), "2/6 (33.3 %)");
        assert_eq!(Counts::default().display(), "-");
    }

    #[test]
    fn mcdc_crates_have_line_thresholds() {
        for c in MCDC_CRATES {
            assert!(MIN.iter().any(|(n, _)| n == c), "{c}");
        }
    }

    #[test]
    fn parses_llvm_cov_summary() {
        let map = parse_summary(SAMPLE).unwrap();
        assert_eq!(
            map["twine-core"],
            CrateLines {
                count: 400,
                covered: 320
            }
        );
        assert_eq!(
            map["twine-render"],
            CrateLines {
                count: 10,
                covered: 0
            }
        );
        assert_eq!(
            map["xtask"],
            CrateLines {
                count: 50,
                covered: 10
            }
        );
        let (rows, failures) = evaluate(&map);
        assert_eq!(rows.len(), 3);
        // core: 80 % < 85 % and ≥ 200 lines → failure; render: too small → not enforced.
        assert_eq!(failures.len(), 1);
        assert!(failures[0].starts_with("twine-core"));
        assert!(
            rows.iter()
                .any(|r| r.contains("twine-render") && r.contains("not yet enforced"))
        );
    }
}
