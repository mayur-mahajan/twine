//! `cargo xtask bench [--iai] [--save-baseline]`: runs the renderer and text benchmarks.
//!
//! Without `--iai` the criterion benches run and print their table. With `--iai` (Linux +
//! valgrind + `iai-callgrind-runner`) the instruction-count benches run; their counts are
//! compared with each crate's `benches/baseline/iai.json` (a regression of more than 5 % fails)
//! or, with `--save-baseline`, written to it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::util::{R, cargo, output, run as run_cmd, warn, workspace_root};

/// Benchmarked crates: `(package, criterion bench, iai bench)`.
pub const BENCHES: &[(&str, &str, &str)] = &[
    ("twine-render", "render", "iai"),
    ("twine-text", "text", "text_iai"),
];

/// Allowed instruction-count growth before `--iai` fails.
pub const MAX_REGRESSION_PERCENT: u64 = 5;

fn baseline_path(krate: &str) -> PathBuf {
    workspace_root()
        .join("crates")
        .join(krate)
        .join("benches/baseline/iai.json")
}

/// Parses iai-callgrind's text output into `(benchmark id, instructions)`.
///
/// Benchmark headers are unindented lines such as `render::scene fill_565:setup(0)` (id
/// `fill_565`) or `reactive::get_untracked` (id `get_untracked`); the count is the first
/// number of the following indented `Instructions:` line.
#[must_use]
pub fn parse_iai(text: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && line.contains("::") {
            let id = match line.split_once(' ') {
                Some((_, rest)) => rest.split(':').next().unwrap_or(rest).trim().to_string(),
                None => line.rsplit("::").next().unwrap_or(line).trim().to_string(),
            };
            current = Some(id);
            continue;
        }
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("Instructions:") {
            let num: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == ',')
                .filter(char::is_ascii_digit)
                .collect();
            if let (Some(id), Ok(n)) = (current.take(), num.parse::<u64>()) {
                out.push((id, n));
            }
        }
    }
    out
}

/// Regressions of `new` against `base` above [`MAX_REGRESSION_PERCENT`] (and ids missing from
/// the baseline).
#[must_use]
pub fn compare(base: &BTreeMap<String, u64>, new: &[(String, u64)]) -> (Vec<String>, Vec<String>) {
    let mut regressions = Vec::new();
    let mut missing = Vec::new();
    for (id, n) in new {
        match base.get(id) {
            Some(&b) if *n * 100 > b * (100 + MAX_REGRESSION_PERCENT) => {
                regressions.push(format!(
                    "{id}: {n} instructions vs baseline {b} (+{:.1} %)",
                    pct(*n, b)
                ));
            }
            Some(_) => {}
            None => missing.push(id.clone()),
        }
    }
    (regressions, missing)
}

#[allow(clippy::cast_precision_loss)] // display only
fn pct(n: u64, b: u64) -> f64 {
    (n as f64 - b as f64) * 100.0 / (b.max(1) as f64)
}

/// Runs the benchmarks.
pub fn run(iai: bool, save_baseline: bool) -> R {
    if !iai {
        if save_baseline {
            return Err("bench: --save-baseline applies to --iai".into());
        }
        for (krate, bench, _) in BENCHES {
            run_cmd(cargo().args(["bench", "-p", krate, "--bench", bench]))?;
        }
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        return Err("bench --iai: iai-callgrind needs Linux with valgrind and iai-callgrind-runner".into());
    }
    let mut failures = Vec::new();
    for (krate, _, bench) in BENCHES {
        if let Err(e) = run_iai(krate, bench, save_baseline) {
            failures.push(format!("{krate}: {e}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

fn run_iai(krate: &str, bench: &str, save_baseline: bool) -> R {
    let text = output(cargo().args(["bench", "-p", krate, "--bench", bench]))?;
    print!("{text}");
    let counts = parse_iai(&text);
    if counts.is_empty() {
        return Err("bench --iai: no instruction counts found in the output".into());
    }
    let path = baseline_path(krate);
    if save_baseline {
        let map: BTreeMap<_, _> = counts.into_iter().collect();
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
        std::fs::write(&path, serde_json::to_string_pretty(&map)? + "\n")?;
        println!(
            "bench: baseline written to {} ({} benchmarks)",
            path.display(),
            map.len()
        );
        return Ok(());
    }
    let base: BTreeMap<String, u64> = if let Ok(s) = std::fs::read_to_string(&path) {
        serde_json::from_str(&s)?
    } else {
        warn(&format!(
            "bench: no baseline at {}; run `cargo xtask bench --iai --save-baseline`",
            path.display()
        ));
        BTreeMap::new()
    };
    let (regressions, missing) = compare(&base, &counts);
    for id in &missing {
        warn(&format!("bench: `{id}` has no baseline entry"));
    }
    if regressions.is_empty() {
        println!(
            "bench: {} benchmarks within {MAX_REGRESSION_PERCENT} % of the baseline",
            counts.len()
        );
        Ok(())
    } else {
        Err(format!("instruction-count regressions:\n  {}", regressions.join("\n  ")).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
render::scene fill_fullscreen_565:setup(0)
  Instructions:                 12,345|12000               (+2.87500%) [+1.02875x]
  L1 Hits:                      15000|N/A                  (*********)
render::rotate rotate_320x40_565:RotateChunk::new()
  Instructions:                  99000|N/A                  (*********)
reactive::get_untracked
  Instructions:                   1734|1734                 (No change)
";

    #[test]
    fn parses_ids_and_counts() {
        assert_eq!(
            parse_iai(SAMPLE),
            vec![
                ("fill_fullscreen_565".to_string(), 12_345),
                ("rotate_320x40_565".to_string(), 99_000),
                ("get_untracked".to_string(), 1734),
            ]
        );
    }

    #[test]
    fn regression_threshold() {
        let base: BTreeMap<String, u64> = [("a".to_string(), 1000), ("b".to_string(), 1000)].into();
        let (r, m) = compare(&base, &[("a".into(), 1050), ("b".into(), 1051), ("c".into(), 5)]);
        assert_eq!(r.len(), 1);
        assert!(r[0].starts_with("b:"));
        assert_eq!(m, vec!["c".to_string()]);
    }
}
