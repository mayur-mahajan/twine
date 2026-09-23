//! `cargo xtask coverage`: per-crate line coverage with minimum thresholds
//! Requires `cargo-llvm-cov`.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::util::{R, cargo, run as run_cmd, which, workspace_root};

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
    for marker in ["/crates/", "/tools/", "/examples/"] {
        if let Some(pos) = norm.rfind(marker) {
            let rest = &norm[pos + marker.len()..];
            if marker == "/examples/" {
                return Some("twine-examples".into());
            }
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

/// Runs coverage over the workspace and enforces the thresholds.
pub fn run() -> R {
    if !which("cargo-llvm-cov") {
        return Err(
            "coverage: `cargo llvm-cov` not found\n  hint: `cargo install cargo-llvm-cov` \
                    (needs the `llvm-tools-preview` component from rust-toolchain.toml)"
                .into(),
        );
    }
    let out = workspace_root().join("target/coverage.json");
    run_cmd(cargo().args([
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
