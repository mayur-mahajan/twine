//! `cargo xtask stack-budget [--update]`: the stack the engine's recursive tree walks need for
//! the tree depth the safety profile is checked at (R5.S05).
//!
//! Drawing, the opaque-cover search, hit testing and layout recurse over the widget tree;
//! `Limits::max_tree_depth` bounds the depth. The test `twine-engine/tests/stack_depth.rs`
//! measures, in release mode, the stack each path costs per tree level and its constant part
//! (`twine-stack:` lines); this command computes each path's worst case at
//! [`REFERENCE_DEPTH`] levels and fails when one exceeds [`BUDGET_BYTES`]. `--update` writes
//! the measured table into `docs/safety/profile.md` § 10.4 (the numbers are those of the host
//! and toolchain that ran it, which the table names; CI checks only the budget, since another
//! host or compiler version gives slightly different frames).

use std::fmt::Write as _;
use std::process::Command;

use crate::util::{R, cargo_ci, output, workspace_root};

/// The tree depth the budget is checked at: the `Limits::max_tree_depth` of the reference
/// safety configuration. A product sizes its UI task's stack from the per-level numbers and its
/// own limit (`base + per_level × max_tree_depth` per path, measured on its target).
pub const REFERENCE_DEPTH: u64 = 32;

/// Stack budget for any recursive path at [`REFERENCE_DEPTH`] levels, in bytes (host,
/// release profile).
pub const BUDGET_BYTES: u64 = 32 * 1024;

/// The structural maximum of the depth (`Limits::DEFAULT.max_tree_depth`), reported for
/// information.
const DEFAULT_DEPTH: u64 = 255;

/// The document holding the measured table.
const PROFILE_DOC: &str = "docs/safety/profile.md";
const BEGIN: &str = "<!-- stack-budget:begin (cargo xtask stack-budget --update) -->";
const END: &str = "<!-- stack-budget:end -->";

/// One measured path.
#[derive(Debug, PartialEq, Eq)]
pub struct PathCost {
    /// The path's name (`draw`, `layout_content`, …).
    pub name: String,
    /// Bytes per tree level.
    pub per_level: u64,
    /// Bytes from the caller of the engine to the first level, plus the leaf's own frame.
    pub base: u64,
}

impl PathCost {
    /// The stack the path needs at `depth` levels.
    #[must_use]
    pub fn worst(&self, depth: u64) -> u64 {
        self.base + self.per_level * depth
    }
}

/// Parses the `twine-stack:` lines of the measuring test's output.
///
/// # Errors
/// A `twine-stack:` line without a `path`, `per_level` or `base` field.
pub fn parse(out: &str) -> Result<Vec<PathCost>, String> {
    let mut paths = Vec::new();
    // With `--nocapture` the first line can follow the harness's `test … ... ` prefix.
    for line in out
        .lines()
        .filter_map(|l| l.find("twine-stack:").map(|i| &l[i + "twine-stack:".len()..]))
    {
        let field = |key: &str| {
            line.split_whitespace()
                .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
                .ok_or_else(|| format!("stack-budget: `{key}` missing in `{line}`"))
        };
        let num = |key: &str| -> Result<u64, String> {
            field(key)?
                .parse()
                .map_err(|e| format!("stack-budget: bad `{key}` in `{line}`: {e}"))
        };
        paths.push(PathCost {
            name: field("path")?.to_owned(),
            per_level: num("per_level")?,
            base: num("base")?,
        });
    }
    Ok(paths)
}

/// The paths whose worst case at `depth` exceeds `budget`, as messages.
#[must_use]
pub fn over_budget(paths: &[PathCost], depth: u64, budget: u64) -> Vec<String> {
    paths
        .iter()
        .filter(|p| p.worst(depth) > budget)
        .map(|p| {
            format!(
                "{}: {} B at depth {depth} ({} B per level + {} B) exceeds the budget of {budget} B",
                p.name,
                p.worst(depth),
                p.per_level,
                p.base
            )
        })
        .collect()
}

/// The Markdown table of `paths` (with the host that measured them).
#[must_use]
pub fn render_table(paths: &[PathCost], host: &str) -> String {
    let mut t = format!(
        "\nMeasured with `cargo xtask stack-budget --update` on `{host}` (release profile).\n\n\
         | Path | Per level (B) | Base (B) | At depth {REFERENCE_DEPTH} (B) | At depth {DEFAULT_DEPTH} (B) |\n\
         |------|--------------:|---------:|------------:|------------:|\n"
    );
    for p in paths {
        let _ = writeln!(
            t,
            "| `{}` | {} | {} | {} | {} |",
            p.name,
            p.per_level,
            p.base,
            p.worst(REFERENCE_DEPTH),
            p.worst(DEFAULT_DEPTH)
        );
    }
    t
}

/// The range between the table markers of `doc`.
fn table_in(doc: &str) -> Option<(usize, usize)> {
    let start = doc.find(BEGIN)? + BEGIN.len();
    let end = doc[start..].find(END)? + start;
    Some((start, end))
}

/// The host triple and compiler version (`rustc -vV`).
fn host() -> Result<String, Box<dyn std::error::Error>> {
    let v = output(Command::new("rustc").arg("-vV"))?;
    let get = |k: &str| {
        v.lines()
            .find_map(|l| l.strip_prefix(k))
            .map_or("?", str::trim)
            .to_owned()
    };
    Ok(format!("{}, rustc {}", get("host:"), get("release:")))
}

/// Measures (release build of the engine's `stack_depth` test), checks the budget and, with
/// `update`, rewrites the table in [`PROFILE_DOC`].
pub fn run(update: bool) -> R {
    let out = output(cargo_ci().args([
        "test",
        "-p",
        "twine-engine",
        "--release",
        "--test",
        "stack_depth",
        "--",
        "--nocapture",
        "--test-threads=1",
        "stack_cost_per_level",
    ]))?;
    let paths = parse(&out)?;
    if paths.is_empty() {
        return Err("stack-budget: the test printed no `twine-stack:` lines".into());
    }
    let table = render_table(&paths, &host()?);
    print!("{table}");
    if update {
        let path = workspace_root().join(PROFILE_DOC);
        let doc = std::fs::read_to_string(&path)?;
        let (start, end) =
            table_in(&doc).ok_or_else(|| format!("{PROFILE_DOC}: stack-budget markers missing"))?;
        std::fs::write(&path, format!("{}{table}{}", &doc[..start], &doc[end..]))?;
        println!("stack-budget: {PROFILE_DOC} updated");
    }
    let errors = over_budget(&paths, REFERENCE_DEPTH, BUDGET_BYTES);
    if errors.is_empty() {
        println!(
            "stack-budget: ok ({} paths within {BUDGET_BYTES} B at depth {REFERENCE_DEPTH})",
            paths.len()
        );
        Ok(())
    } else {
        for e in &errors {
            println!("stack-budget: {e}");
        }
        Err(format!("stack-budget: {} path(s) over budget", errors.len()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "running 1 test\n\
        test stack_cost_per_level_of_every_recursive_path ... twine-stack: path=draw per_level=288 base=12320 profile=release\n\
        twine-stack: path=layout_content per_level=656 base=4048 profile=release\n\
        test stack_cost_per_level_of_every_recursive_path ... ok\n";

    #[test]
    fn parses_the_measured_lines() {
        let p = parse(OUT).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            p[1],
            PathCost {
                name: "layout_content".into(),
                per_level: 656,
                base: 4048
            }
        );
        assert_eq!(p[0].worst(32), 12320 + 288 * 32);
        assert!(parse("twine-stack: path=draw base=1").is_err());
    }

    #[test]
    fn a_path_over_budget_fails() {
        let p = parse(OUT).unwrap();
        assert!(over_budget(&p, 32, 32 * 1024).is_empty());
        let e = over_budget(&p, 255, 32 * 1024);
        assert_eq!(e.len(), 2);
        assert!(e[1].starts_with("layout_content: 171328 B at depth 255"));
    }

    #[test]
    fn table_lists_every_path_between_the_markers() {
        let p = parse(OUT).unwrap();
        let t = render_table(&p, "x86_64-unknown-linux-gnu, rustc 1.0.0");
        assert!(t.contains("| `draw` | 288 | 12320 | 21536 | 85760 |"));
        let doc = format!("a\n{BEGIN}\nold\n{END}\nb");
        let (s, e) = table_in(&doc).unwrap();
        assert_eq!(&doc[s..e], "\nold\n");
    }
}
