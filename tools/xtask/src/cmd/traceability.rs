//! `cargo xtask traceability [--check]`: the requirements → tests matrix
//! `docs/safety/traceability.md` (R5.S08).
//!
//! Requirements are the rows `| SAF-nnn | text | source | other evidence |` of
//! `docs/safety/requirements.md`. Tests name what they verify with a doc line `req: SAF-nnn[, …]`:
//! `/// req: …` on a test function (the next `fn` after the doc block), or `//! req: …` at the
//! top of a test file (every test in it). The scan covers `crates/*/tests/**/*.rs`.
//!
//! Fault-injection tests also carry ``/// fault: `<FaultKind>` `` lines (the kinds they inject and
//! observe); the matrix gets a second table, fault kind → injection tests, and every kind of
//! `twine_core::fault::FaultKind::ALL` (read from `crates/twine-core/src/fault.rs`) must have one.
//!
//! `--check` fails when the file is out of date, a test names an unknown requirement or fault
//! kind, a requirement has neither a test nor other evidence, or a fault kind has no injection
//! test.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::util::{R, workspace_root};

/// The requirement list (input).
pub const REQUIREMENTS: &str = "docs/safety/requirements.md";
/// The generated matrix (output).
pub const MATRIX: &str = "docs/safety/traceability.md";

/// One requirement row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// `SAF-nnn`.
    pub id: String,
    /// The requirement text.
    pub text: String,
    /// Evidence that is not a test (CI checks, analyses); may be empty.
    pub other: String,
}

/// Parses the requirement table.
///
/// # Errors
/// Duplicate ids or a malformed row.
pub fn parse_requirements(md: &str) -> Result<Vec<Requirement>, String> {
    let mut out: Vec<Requirement> = Vec::new();
    for line in md.lines() {
        let Some(rest) = line.strip_prefix("| SAF-") else {
            continue;
        };
        let row = format!("SAF-{rest}");
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            return Err(format!("malformed requirement row: {line}"));
        }
        let id = cells[0].to_string();
        if !valid_id(&id) {
            return Err(format!("malformed requirement id {id:?}"));
        }
        if out.iter().any(|r| r.id == id) {
            return Err(format!("duplicate requirement {id}"));
        }
        out.push(Requirement {
            id,
            text: cells[1].to_string(),
            other: cells.get(3).copied().unwrap_or_default().to_string(),
        });
    }
    Ok(out)
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("SAF-")
        .is_some_and(|n| n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The ids named on a `<key>:` doc line (`None` when the line has no `<key>:`).
fn tag_ids(doc: &str, key: &str) -> Option<Vec<String>> {
    let (_, list) = doc.trim().split_once(key)?;
    Some(
        list.split([',', ' '])
            .map(|s| s.trim().trim_end_matches('.').trim_matches('`'))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// What a tag line names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tag {
    /// `req: SAF-nnn`.
    Req,
    /// `fault: <FaultKind>`.
    Fault,
}

impl Tag {
    const ALL: [(Tag, &'static str); 2] = [(Tag::Req, "req:"), (Tag::Fault, "fault:")];
}

/// A test (or test file) and the requirements or fault kinds it names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tagged {
    /// `crates/<crate>/tests/<file>.rs` or `…::<fn>`.
    pub test: String,
    /// What the ids are.
    pub tag: Tag,
    /// The ids.
    pub ids: Vec<String>,
}

/// The `req:` tags of one test file's source (`path` is its display path).
#[must_use]
pub fn scan_source(path: &str, src: &str) -> Vec<Tagged> {
    let mut out = Vec::new();
    let mut pending: Vec<(Tag, Vec<String>)> = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if let Some(doc) = t.strip_prefix("//!") {
            for (tag, key) in Tag::ALL {
                if let Some(ids) = tag_ids(doc, key) {
                    out.push(Tagged {
                        test: path.to_string(),
                        tag,
                        ids,
                    });
                }
            }
        } else if let Some(doc) = t.strip_prefix("///") {
            for (tag, key) in Tag::ALL {
                if let Some(ids) = tag_ids(doc, key) {
                    pending.push((tag, ids));
                }
            }
        } else if t.starts_with("#[") || t.is_empty() {
            // Attributes and blank lines between the doc block and the function.
        } else if let Some(name) = fn_name(t) {
            for (tag, ids) in pending.drain(..) {
                out.push(Tagged {
                    test: format!("{path}::{name}"),
                    tag,
                    ids,
                });
            }
        } else {
            pending.clear();
        }
    }
    out
}

fn fn_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("pub ").unwrap_or(line);
    let rest = rest.strip_prefix("async ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every tagged test of the workspace.
fn scan_workspace() -> Result<Vec<Tagged>, Box<dyn std::error::Error>> {
    let root = workspace_root();
    let mut files = Vec::new();
    for c in std::fs::read_dir(root.join("crates"))?.filter_map(Result::ok) {
        rust_files(&c.path().join("tests"), &mut files);
    }
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&root).unwrap_or(&f).display().to_string();
        out.extend(scan_source(&rel, &std::fs::read_to_string(&f)?));
    }
    Ok(out)
}

/// The variant names of `FaultKind::ALL` in `fault.rs` source.
///
/// # Errors
/// When the constant is not found.
pub fn fault_kinds(src: &str) -> Result<Vec<String>, String> {
    let start = src
        .find("pub const ALL: [FaultKind;")
        .ok_or("`FaultKind::ALL` not found")?;
    let body = &src[start..];
    let open = body.find("= [").ok_or("`FaultKind::ALL` has no list")?;
    let close = body.find("];").ok_or("`FaultKind::ALL` is not closed")?;
    Ok(body[open + 3..close]
        .split(',')
        .filter_map(|s| s.trim().strip_prefix("FaultKind::"))
        .map(str::to_string)
        .collect())
}

/// Builds the matrix text; returns it and the problems found.
#[must_use]
pub fn render(reqs: &[Requirement], kinds: &[String], tagged: &[Tagged]) -> (String, Vec<String>) {
    let known: BTreeSet<&str> = reqs.iter().map(|r| r.id.as_str()).collect();
    let mut problems = Vec::new();
    let mut by_req: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut by_kind: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for t in tagged.iter().filter(|t| t.tag == Tag::Fault) {
        for id in &t.ids {
            if kinds.iter().any(|k| k == id) {
                by_kind.entry(id.as_str()).or_default().insert(t.test.as_str());
            } else {
                problems.push(format!("{} names unknown fault kind {id:?}", t.test));
            }
        }
    }
    for t in tagged.iter().filter(|t| t.tag == Tag::Req) {
        for id in &t.ids {
            if known.contains(id.as_str()) {
                by_req.entry(id.as_str()).or_default().insert(t.test.as_str());
            } else {
                problems.push(format!("{} names unknown requirement {id:?}", t.test));
            }
        }
    }
    let mut s = String::from(
        "# Traceability: requirements → tests\n\n\
         Generated by `cargo xtask traceability` from `requirements.md` and the `req:` doc lines \
         of the tests (`crates/*/tests/`); do not edit. CI runs `cargo xtask traceability --check`.\n\n\
         A test named `file.rs` verifies the requirement with every test in that file; `file.rs::name` \
         with that test.\n\n\
         | Requirement | Tests | Other evidence |\n|-------------|-------|----------------|\n",
    );
    let mut untested = 0;
    for r in reqs {
        let tests = by_req.get(r.id.as_str());
        if tests.is_none() {
            untested += 1;
            if r.other.is_empty() {
                problems.push(format!("{} has neither a test nor other evidence", r.id));
            }
        }
        let list = tests.map_or_else(
            || "—".to_string(),
            |t| {
                t.iter()
                    .map(|x| format!("`{x}`"))
                    .collect::<Vec<_>>()
                    .join("<br>")
            },
        );
        let other = if r.other.is_empty() {
            "—"
        } else {
            r.other.as_str()
        };
        let _ = writeln!(s, "| **{}** {} | {list} | {other} |", r.id, r.text);
    }
    let _ = writeln!(
        s,
        "\n{} requirements, {} with tests, {} by other evidence only.",
        reqs.len(),
        reqs.len() - untested,
        untested,
    );
    s.push_str(
        "\n## Fault injection: fault kind → tests\n\n\
         Each test injects the fault through a test double (`MemoryDisplay` failures and hangs, \
         mock input health, failing panels and accelerators, small `Limits`, a sealed UI, a time \
         budget, a corrupted panel, …) and checks the record the application receives \
         (``/// fault: `<Kind>` `` lines; SAF-031).\n\n\
         | Fault kind | Injection tests |\n|------------|-----------------|\n",
    );
    for k in kinds {
        let list = by_kind.get(k.as_str()).map_or_else(
            || "—".to_string(),
            |t| {
                t.iter()
                    .map(|x| format!("`{x}`"))
                    .collect::<Vec<_>>()
                    .join("<br>")
            },
        );
        if !by_kind.contains_key(k.as_str()) {
            problems.push(format!(
                "fault kind {k} has no injection test (``/// fault: `{k}` ``)"
            ));
        }
        let _ = writeln!(s, "| `{k}` | {list} |");
    }
    (s, problems)
}

/// Writes (or with `check`, verifies) the matrix.
pub fn run(check: bool) -> R {
    let root = workspace_root();
    let req_path = root.join(REQUIREMENTS);
    let Ok(md) = std::fs::read_to_string(&req_path) else {
        println!("traceability: {REQUIREMENTS} not found (docs are local); nothing to check");
        return Ok(());
    };
    let reqs = parse_requirements(&md)?;
    let kinds = fault_kinds(&std::fs::read_to_string(
        root.join("crates/twine-core/src/fault.rs"),
    )?)?;
    let tagged = scan_workspace()?;
    let (text, problems) = render(&reqs, &kinds, &tagged);
    if !problems.is_empty() {
        return Err(format!("traceability:\n  {}", problems.join("\n  ")).into());
    }
    let out = root.join(MATRIX);
    if check {
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current != text {
            return Err(format!("{MATRIX} is out of date: run `cargo xtask traceability`").into());
        }
        println!(
            "traceability: {} requirements, {} fault kinds, up to date",
            reqs.len(),
            kinds.len()
        );
    } else {
        std::fs::write(&out, text)?;
        println!("wrote {MATRIX}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_rows_parse() {
        let md = "| ID | x |\n| SAF-001 | Faults. | R0 | |\n| SAF-002 | Waits. | R0 | CI x |\n";
        let r = parse_requirements(md).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].other, "CI x");
        assert!(parse_requirements("| SAF-001 | a | b | |\n| SAF-001 | a | b | |").is_err());
        assert!(parse_requirements("| SAF-01 | a | b | |").is_err());
    }

    #[test]
    fn tags_attach_to_files_and_functions() {
        let src = "//! Tests.\n//!\n//! req: SAF-001\n\n/// Does a thing.\n///\n/// req: SAF-002, SAF-003\n/// fault: FlushError\n#[test]\nfn thing() {}\n\n/// No tag.\n#[test]\nfn other() {}\n/// req: SAF-004\nconst X: u8 = 1;\nfn after() {}\n";
        let t = scan_source("f.rs", src);
        let tagged = |test: &str, tag, ids: &[&str]| Tagged {
            test: test.into(),
            tag,
            ids: ids.iter().map(|s| (*s).to_string()).collect(),
        };
        assert_eq!(
            t,
            vec![
                tagged("f.rs", Tag::Req, &["SAF-001"]),
                tagged("f.rs::thing", Tag::Req, &["SAF-002", "SAF-003"]),
                tagged("f.rs::thing", Tag::Fault, &["FlushError"]),
            ]
        );
    }

    #[test]
    fn unknown_and_unverified_requirements_are_problems() {
        let reqs = parse_requirements("| SAF-001 | a | s | |\n| SAF-002 | b | s | CI |\n").unwrap();
        let kinds = vec!["FlushError".to_string()];
        let tag = |tag, id: &str| Tagged {
            test: "t.rs".into(),
            tag,
            ids: vec![id.into()],
        };
        let (_, problems) = render(
            &reqs,
            &kinds,
            &[tag(Tag::Req, "SAF-009"), tag(Tag::Fault, "Nope")],
        );
        // unknown requirement, unknown kind, SAF-001 unverified, FlushError without a test
        assert_eq!(problems.len(), 4, "{problems:?}");
        let (text, problems) = render(
            &reqs,
            &kinds,
            &[tag(Tag::Req, "SAF-001"), tag(Tag::Fault, "FlushError")],
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert!(text.contains("| **SAF-001** a | `t.rs` | — |"));
        assert!(text.contains("| `FlushError` | `t.rs` |"));
    }

    #[test]
    fn fault_kinds_are_read_from_the_source() {
        let src = std::fs::read_to_string(workspace_root().join("crates/twine-core/src/fault.rs")).unwrap();
        let kinds = fault_kinds(&src).unwrap();
        assert_eq!(kinds.first().map(String::as_str), Some("FlushError"));
        assert!(kinds.len() >= 18);
    }
}
