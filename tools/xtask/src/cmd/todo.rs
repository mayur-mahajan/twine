//! `cargo xtask todo-check`: rejects untracked work markers (P9).
//!
//! Scans `crates/`, `tools/`, `examples/` and `firmware/` (`*.rs`, `*.toml`, skipping `target/`)
//! and fails on `todo!(`, `unimplemented!(`, `TODO`, `FIXME`, `XXX`, `HACK`, `dbg!(` and on any
//! `NOTE(` that is not followed by a step id `Pxx.Syy)`. The only allowed marker is
//! `NOTE(Pxx.Syy): …`. This file is exempt (it has to spell the patterns).

use std::path::{Path, PathBuf};

use crate::util::{R, workspace_root};

const ROOTS: &[&str] = &["crates", "tools", "examples", "firmware"];
/// Paths (relative to the workspace root, `/`-separated) exempt from the check.
const EXEMPT: &[&str] = &["tools/xtask/src/cmd/todo.rs"];
const FORBIDDEN: &[&str] = &[
    "todo!(",
    "unimplemented!(",
    "TODO",
    "FIXME",
    "XXX",
    "HACK",
    "dbg!(",
];

/// A violation found on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 1-based line number.
    pub line: usize,
    /// The offending pattern.
    pub pattern: &'static str,
    /// The trimmed line text.
    pub text: String,
}

/// Whether `rest` (the text right after `NOTE(`) starts with `Pdd.Sdd)`.
fn note_has_step_id(rest: &str) -> bool {
    let b = rest.as_bytes();
    b.len() >= 8
        && b[0] == b'P'
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3] == b'.'
        && b[4] == b'S'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b')'
}

/// Checks the content of one file.
pub fn check_source(content: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate() {
        for pat in FORBIDDEN {
            if line.contains(pat) {
                out.push(Finding {
                    line: i + 1,
                    pattern: pat,
                    text: line.trim().to_string(),
                });
            }
        }
        let mut rest = line;
        while let Some(pos) = rest.find("NOTE(") {
            let after = &rest[pos + 5..];
            if !note_has_step_id(after) {
                out.push(Finding {
                    line: i + 1,
                    pattern: "NOTE( without step id",
                    text: line.trim().to_string(),
                });
                break;
            }
            rest = after;
        }
    }
    out
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                collect(&path, files)?;
            }
        } else if path.extension().is_some_and(|e| e == "rs" || e == "toml") {
            files.push(path);
        }
    }
    Ok(())
}

/// Runs the check over the workspace.
pub fn run() -> R {
    let root = workspace_root();
    let mut files = Vec::new();
    for r in ROOTS {
        let dir = root.join(r);
        if dir.is_dir() {
            collect(&dir, &mut files)?;
        }
    }
    files.sort();
    let mut count = 0usize;
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap_or(f);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if EXEMPT.contains(&rel_str.as_str()) {
            continue;
        }
        let content = std::fs::read_to_string(f)?;
        for finding in check_source(&content) {
            println!(
                "{rel_str}:{}: [{}] {}",
                finding.line, finding.pattern, finding.text
            );
            count += 1;
        }
    }
    if count == 0 {
        println!("todo-check: {} files clean", files.len());
        Ok(())
    } else {
        Err(format!(
            "todo-check: {count} forbidden marker(s); only `NOTE(Pxx.Syy): …` is allowed (plan README §1 rule 4)"
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_todo_macro() {
        let f = check_source("fn a() {\n    todo!()\n}\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 2);
        assert_eq!(f[0].pattern, "todo!(");
        for src in [
            "// TODO later",
            "// FIXME",
            "// XXX",
            "// HACK",
            "dbg!(x);",
            "unimplemented!()",
        ] {
            assert!(!check_source(src).is_empty(), "{src}");
        }
    }

    #[test]
    fn allows_note_with_step_id() {
        assert!(check_source("// NOTE(P03.S10): faster path").is_empty());
        assert!(check_source("// NOTE: plain notes are fine").is_empty());
        assert!(check_source("// NOTE(P01.S02): a NOTE(P02.S03): b").is_empty());
    }

    #[test]
    fn flags_note_without_step_id() {
        assert_eq!(check_source("// NOTE(later): x").len(), 1);
        assert_eq!(check_source("// NOTE(P1.S2): x").len(), 1);
        assert_eq!(check_source("// NOTE(P01.S02 x").len(), 1);
    }
}
