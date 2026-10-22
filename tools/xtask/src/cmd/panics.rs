//! `cargo xtask panics [--check]`: the panic inventory of the safety profile
//! ([`PANICS_DOC`], `docs/safety/panics.md`).
//!
//! Every place in the library code of the profile crates where the profile build can panic:
//!
//! - **from clippy** (the counting run of the safety lints, [`safety::clippy_sites`]: the facade
//!   with the profile's features, every lint forced to warn): `panic!`, `.unwrap()`,
//!   `.expect(..)`, `unreachable!`, `todo!`, `unimplemented!`, indexing and slicing, and
//!   arithmetic that can overflow or divide by zero (a panic under `overflow-checks`). These are
//!   exactly the sites compiled in the profile;
//! - **from the sources** of each profile crate (`src/`, outside `#[cfg(test)]` items, comments
//!   and string literals): `assert!`, `assert_eq!` and `assert_ne!` (not `debug_assert*`, not in
//!   `const` items), and functions marked `#[track_caller]` (entry points that report their
//!   caller's location when they panic).
//!
//! Explicit sites (`panic!`, `unwrap`, `expect`, `unreachable!`, `todo!`, `unimplemented!`,
//! `assert*!`) are listed one by one with their justification: a `// PANIC: …` comment on the
//! lines directly above the site. `--check` fails when one has none, and when the checked-in
//! file differs from the generated one (CI stage `clippy-safety`). Indexing and arithmetic sites
//! are counted per file (their removal is tracked by the lint ratchet). The document carries no
//! line numbers, so it changes only when sites change.
//!
//! **Proven sites** (R5.S11): indexing and arithmetic that the code proves in range instead of
//! leaving a lint site — `wrapping_*` arithmetic that cannot wrap and checked accesses whose
//! fallback is unreachable — carry a `// SAFE-INDEX: …` or `// SAFE-ARITH: …` proof comment
//! ([`PROOF_TAGS`]), on the line or in the comment block directly above the code it proves.
//! They are listed with their proofs (the source scan, [`scan_proofs`]); `--check` fails on an
//! empty proof or a proof comment that no code line follows.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::cmd::safety::{self, Site};
use crate::util::{R, workspace_root};

/// The checked-in inventory.
pub const PANICS_DOC: &str = "docs/safety/panics.md";

/// The comment marker that justifies an explicit panic site.
pub const REASON_TAG: &str = "PANIC:";

/// The proof comments of [`scan_proofs`] and their inventory kinds.
pub const PROOF_TAGS: &[(&str, &str)] = &[("SAFE-INDEX:", "safe_index"), ("SAFE-ARITH:", "safe_arith")];

/// Clippy lints whose sites are listed one by one (explicit panics).
const EXPLICIT_LINTS: &[&str] = &[
    "panic",
    "unwrap_used",
    "expect_used",
    "unreachable",
    "todo",
    "unimplemented",
];

/// Columns of the summary table: `(category, title)`.
const CATEGORIES: &[(&str, &str)] = &[
    ("panic", "`panic!`"),
    ("unwrap_used", "`unwrap`"),
    ("expect_used", "`expect`"),
    ("unreachable", "`unreachable!`"),
    ("todo", "`todo!`/`unimplemented!`"),
    ("assert", "`assert*!`"),
    ("track_caller", "`#[track_caller]`"),
    ("indexing_slicing", "indexing"),
    ("arithmetic_side_effects", "arithmetic"),
];

/// An entry of the inventory.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// The crate (package name).
    pub krate: String,
    /// The category: a clippy lint name, `assert` or `track_caller`.
    pub kind: String,
    /// The file, relative to the workspace root.
    pub file: String,
    /// The source line of the site (for `track_caller`: the function's signature), trimmed.
    pub code: String,
    /// The justification (`// PANIC: …` above the site), if any.
    pub reason: Option<String>,
    /// The line (used for sorting and messages only; not written to the document).
    pub line: u64,
}

/// `cargo xtask panics [--check]`: generates the inventory (clippy counting run plus source
/// scan) and writes [`PANICS_DOC`], or with `check` verifies it.
pub fn run(check: bool) -> R {
    write_or_check(&safety::clippy_sites()?, check)
}

/// Builds the inventory from the clippy `sites` and the source scan, then writes
/// [`PANICS_DOC`] (or, with `check`, fails if it is out of date or a site lacks a reason).
pub fn write_or_check(sites: &[Site], check: bool) -> R {
    let root = workspace_root();
    let crates = safety::profile_crate_roots()?;
    let mut entries = Vec::new();
    for site in sites {
        let kind = match site.lint.as_str() {
            "unimplemented" => "todo",
            other => other,
        };
        if !CATEGORIES.iter().any(|(c, _)| *c == kind) {
            continue;
        }
        let reason = if EXPLICIT_LINTS.contains(&site.lint.as_str()) {
            std::fs::read_to_string(root.join(&site.file))
                .ok()
                .and_then(|src| reason_above(&src, site.line))
        } else {
            None
        };
        entries.push(Entry {
            krate: site.krate.clone(),
            kind: kind.to_owned(),
            file: site.file.clone(),
            code: site.code.clone(),
            reason,
            line: site.line,
        });
    }
    for (name, lib) in &crates {
        let Some(src_dir) = Path::new(lib).parent() else {
            continue;
        };
        let files = rust_files(src_dir);
        let test_files: Vec<PathBuf> = files
            .iter()
            .filter_map(|f| Some((f, std::fs::read_to_string(f).ok()?)))
            .flat_map(|(f, src)| test_module_files(f, &src))
            .collect();
        for file in files {
            if test_files.iter().any(|t| file.starts_with(t)) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&file) else {
                continue;
            };
            let rel = file
                .strip_prefix(&root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            for found in scan_source(&src).into_iter().chain(scan_proofs(&src)) {
                entries.push(Entry {
                    krate: name.clone(),
                    kind: found.kind.to_owned(),
                    file: rel.clone(),
                    code: found.code,
                    reason: found.reason,
                    line: found.line,
                });
            }
        }
    }
    entries.sort_by(|a, b| (&a.krate, &a.file, a.line, &a.kind).cmp(&(&b.krate, &b.file, b.line, &b.kind)));
    let names: Vec<String> = crates.into_iter().map(|(n, _)| n).collect();
    let doc = render(&names, &entries);
    let missing: Vec<&Entry> = entries
        .iter()
        .filter(|e| is_explicit(&e.kind) && e.reason.is_none())
        .collect();
    let path = root.join(PANICS_DOC);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let mut errors = Vec::new();
    for e in &missing {
        errors.push(format!(
            "{}:{}: `{}` has no `// {REASON_TAG} …` justification on the lines above it",
            e.file, e.line, e.code
        ));
    }
    for e in entries.iter().filter(|e| is_proof(&e.kind)) {
        if e.code.is_empty() {
            errors.push(format!(
                "{}:{}: a `// {}` proof comment with no code line directly below it",
                e.file,
                e.line,
                proof_tag(&e.kind)
            ));
        } else if e.reason.as_deref().is_none_or(str::is_empty) {
            errors.push(format!(
                "{}:{}: `{}` has an empty `// {}` proof",
                e.file,
                e.line,
                e.code,
                proof_tag(&e.kind)
            ));
        }
    }
    if current != doc {
        if check {
            errors.push(format!(
                "{PANICS_DOC} is out of date (run `cargo xtask panics` and review the change)"
            ));
        } else {
            std::fs::write(&path, &doc)?;
            println!("panics: {PANICS_DOC} written");
        }
    }
    let explicit = entries.iter().filter(|e| is_explicit(&e.kind)).count();
    let proven = entries.iter().filter(|e| is_proof(&e.kind)).count();
    if errors.is_empty() {
        println!(
            "panics: ok ({explicit} explicit sites, {proven} proven sites, {} entries in {} crates)",
            entries.len(),
            names.len()
        );
        Ok(())
    } else {
        for e in &errors {
            println!("panics: {e}");
        }
        Err(format!("panics: {} problem(s)", errors.len()).into())
    }
}

/// Whether entries of `kind` are listed one by one and need a justification.
fn is_explicit(kind: &str) -> bool {
    kind == "assert" || EXPLICIT_LINTS.contains(&kind)
}

/// Whether entries of `kind` are proven sites ([`PROOF_TAGS`]).
fn is_proof(kind: &str) -> bool {
    PROOF_TAGS.iter().any(|(_, k)| *k == kind)
}

/// The proof tag of a proven-site kind.
fn proof_tag(kind: &str) -> &'static str {
    PROOF_TAGS
        .iter()
        .find(|(_, k)| *k == kind)
        .map_or("SAFE-", |(tag, _)| tag)
}

/// The proven sites of `src` (library source, outside `#[cfg(test)]` items): every
/// `// SAFE-INDEX: …` / `// SAFE-ARITH: …` comment ([`PROOF_TAGS`]) with its proof — the text after
/// the tag, continued by the following lines of its comment block up to the next tagged comment
/// (`DIV:`, `SHIFT:`, …) — and the code it proves: the code before a trailing comment, or the
/// first line after the comment block. A comment block followed by a blank line or the end of
/// the file gives an empty `code` (reported by `--check`).
#[must_use]
pub fn scan_proofs(src: &str) -> Vec<Found> {
    let code = code_only(src);
    // Test items as line ranges (`code` keeps the line breaks of `src`).
    let line_at = |pos: usize| code[..pos.min(code.len())].matches('\n').count();
    let tests: Vec<(usize, usize)> = test_items(&code, src)
        .into_iter()
        .map(|(a, b)| (line_at(a), line_at(b)))
        .collect();
    let lines: Vec<&str> = src.lines().collect();
    let mut found = Vec::new();
    for (idx, l) in lines.iter().enumerate() {
        if tests.iter().any(|&(a, b)| idx >= a && idx <= b) {
            continue;
        }
        for (tag, kind) in PROOF_TAGS {
            let marker = format!("// {tag}");
            let Some(pos) = l.find(&marker) else { continue };
            let first = l[pos + marker.len()..].trim();
            let before = l[..pos].trim();
            let (proof, code_line) = if before.is_empty() {
                // A comment line: the proof continues on the block's untagged lines; the code
                // is the first line after the block.
                let mut proof = vec![first];
                let mut k = idx + 1;
                let mut in_proof = true;
                while let Some(next) = lines.get(k) {
                    let Some(c) = next.trim_start().strip_prefix("//") else {
                        break;
                    };
                    if next.trim_start().starts_with("///") || next.trim_start().starts_with("//!") {
                        break;
                    }
                    in_proof &= !crate::cmd::audit::is_tag_line(c.trim());
                    if in_proof {
                        proof.push(c.trim());
                    }
                    k += 1;
                }
                let code_line = lines.get(k).map_or("", |c| c.trim());
                (proof.join(" "), (k, code_line))
            } else {
                (first.to_owned(), (idx, before))
            };
            found.push(Found {
                kind,
                line: code_line.0 as u64 + 1,
                code: code_line.1.to_owned(),
                reason: Some(proof.trim().to_owned()),
            });
        }
    }
    found
}

/// The `.rs` files under `dir`, sorted.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The files (or directories) of the out-of-line `#[cfg(test)] mod name;` modules declared in
/// `file` (`src`): `name.rs` or `name/` next to a `lib.rs`/`main.rs`/`mod.rs`, else in the
/// directory named after `file`.
fn test_module_files(file: &Path, src: &str) -> Vec<PathBuf> {
    let code = code_only(src);
    let Some(dir) = file.parent() else {
        return Vec::new();
    };
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let base = if matches!(stem, "lib" | "main" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    let mut out = Vec::new();
    for (a, b) in test_items(&code, src) {
        let item = code[a..b].split_once(']').map_or("", |(_, rest)| rest);
        let words: Vec<&str> = item.split_whitespace().collect();
        if let [.., "mod", name] = words.as_slice() {
            let name = name.trim_end_matches(';');
            out.push(base.join(format!("{name}.rs")));
            out.push(base.join(name));
        }
    }
    out
}

/// The justification of the site on (1-based) `line` of `src`: the text after
/// [`REASON_TAG`] in the block of `//` comment lines directly above it (or a trailing
/// comment on the line itself), continued by the following comment lines of that block.
#[must_use]
pub fn reason_above(src: &str, line: u64) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    let idx = usize::try_from(line).ok()?.checked_sub(1)?;
    let here = lines.get(idx)?;
    if let Some(pos) = here.find(&format!("// {REASON_TAG}")) {
        return Some(here[pos + 3 + REASON_TAG.len()..].trim().to_owned());
    }
    let mut block = Vec::new();
    for l in lines[..idx].iter().rev() {
        let t = l.trim_start();
        if let Some(c) = t.strip_prefix("//") {
            if t.starts_with("///") || t.starts_with("//!") {
                break;
            }
            block.push(c.trim());
        } else {
            break;
        }
    }
    block.reverse();
    let start = block.iter().position(|l| l.starts_with(REASON_TAG))?;
    let text = block[start..].join(" ");
    Some(text[REASON_TAG.len()..].trim().to_owned())
}

/// A site found by [`scan_source`].
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    /// `assert`, `track_caller`, `safe_index` or `safe_arith`.
    pub kind: &'static str,
    /// 1-based line.
    pub line: u64,
    /// The source line (for `track_caller`, the signature of the marked function), trimmed.
    pub code: String,
    /// The justification, for asserts; the proof, for proven sites.
    pub reason: Option<String>,
}

/// Replaces comments and the contents of string and character literals with spaces, keeping
/// line breaks, so that the remaining text is code only.
#[must_use]
pub fn code_only(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
        } else if (c == 'r' || (c == 'b' && next == Some('r')))
            && !b
                .get(i.wrapping_sub(1))
                .is_some_and(|p| p.is_alphanumeric() || *p == '_')
            && {
                let mut j = i + if c == 'b' { 2 } else { 1 };
                while b.get(j) == Some(&'#') {
                    j += 1;
                }
                b.get(j) == Some(&'"')
            }
        {
            // Raw string: r#"…"#.
            let mut j = i + if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while b.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            out.extend(std::iter::repeat_n(' ', j + 1 - i));
            i = j + 1;
            while i < b.len() {
                if b[i] == '"' && (0..hashes).all(|k| b.get(i + 1 + k) == Some(&'#')) {
                    out.extend(std::iter::repeat_n(' ', hashes + 1));
                    i += hashes + 1;
                    break;
                }
                out.push(blank(b[i]));
                i += 1;
            }
        } else if c == '"' {
            out.push(' ');
            i += 1;
            while i < b.len() {
                if b[i] == '\\' {
                    out.push(' ');
                    if let Some(&e) = b.get(i + 1) {
                        out.push(blank(e));
                    }
                    i += 2;
                } else if b[i] == '"' {
                    out.push(' ');
                    i += 1;
                    break;
                } else {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
        } else if c == '\'' {
            // A character literal ('x', '\n', '\u{..}'), or a lifetime / label ('a).
            let lit_end = if next == Some('\\') {
                (i + 2..b.len()).find(|&j| b[j] == '\'')
            } else if b.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            if let Some(end) = lit_end {
                out.extend(std::iter::repeat_n(' ', end + 1 - i));
                i = end + 1;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// The attribute (whitespace removed) of items the profile leaves out: the panicking
/// convenience forms (`build`, `expect_context`, …).
const NOT_IN_PROFILE: &str = "#[cfg(not(feature=\"safety\"))]";

/// The byte ranges of `#[cfg(test)]` items, and of items the profile leaves out
/// ([`NOT_IN_PROFILE`]), in `code` (code-only text): from the attribute to the item's `;` or
/// to the brace closing its body.
fn test_items(code: &str, src: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(off) = code[from..].find("#[cfg(") {
        let start = from + off;
        from = start + 6;
        // From the source, not `code`: the attribute's string literal is blanked there.
        let skip_chars = code[..start].chars().count();
        let compact: String = src
            .chars()
            .skip(skip_chars)
            .take(48)
            .filter(|c| !c.is_whitespace())
            .collect();
        if !(compact.starts_with("#[cfg(test)")
            || compact.starts_with("#[cfg(all(test,")
            || compact.starts_with(NOT_IN_PROFILE))
        {
            continue;
        }
        // The item starts after the attribute's `]`; it ends at its first `;` (a `use`, a
        // `mod x;`) when that comes before any `{`, else at the brace matching the first `{`.
        let Some(attr_end) = code[start..].find(']').map(|k| start + k + 1) else {
            break;
        };
        let rest = &code[attr_end..];
        let end = match (rest.find(';'), rest.find('{')) {
            (Some(s), Some(b)) if s < b => attr_end + s + 1,
            (Some(s), None) => attr_end + s + 1,
            (_, Some(b)) => {
                let mut depth = 0usize;
                let mut end = code.len();
                for (k, ch) in rest[b..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                end = attr_end + b + k + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                end
            }
            (None, None) => code.len(),
        };
        ranges.push((start, end));
        from = end;
    }
    ranges
}

/// The byte ranges of compile-time contexts in `code` (code-only text): inline `const { .. }`
/// blocks, and `const`/`static` items from their keyword to their `;` (not `const fn`).
fn const_contexts(code: &str) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut ranges = Vec::new();
    for kw in ["const", "static"] {
        let mut from = 0;
        while let Some(off) = code[from..].find(kw) {
            let start = from + off;
            from = start + kw.len();
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '\'' || c == '$') {
                continue;
            }
            // A const generic parameter (`<const N: usize>`), not an item.
            if matches!(code[..start].trim_end().chars().next_back(), Some('<' | ',')) {
                continue;
            }
            let rest = code[from..].trim_start();
            if !code[from..].starts_with(char::is_whitespace) {
                continue;
            }
            let inline_block = kw == "const" && rest.starts_with('{');
            if !inline_block
                && (rest.starts_with("fn ") || rest.starts_with("unsafe ") || rest.starts_with("extern "))
            {
                continue;
            }
            // Inline block: up to its matching brace; item: up to its `;` at depth 0.
            let mut depth = 0usize;
            let mut end = code.len();
            for (k, &b) in bytes.iter().enumerate().skip(from) {
                match b {
                    b'{' | b'(' | b'[' => depth += 1,
                    b'}' | b')' | b']' => {
                        if depth == 0 {
                            end = k;
                            break;
                        }
                        depth -= 1;
                        if inline_block && depth == 0 && b == b'}' {
                            end = k + 1;
                            break;
                        }
                    }
                    b';' if depth == 0 => {
                        end = k + 1;
                        break;
                    }
                    _ => {}
                }
            }
            ranges.push((start, end));
        }
    }
    ranges
}

/// Finds `assert*!` calls and `#[track_caller]` functions in `src` (library source), outside
/// comments, literals and `#[cfg(test)]` items.
#[must_use]
pub fn scan_source(src: &str) -> Vec<Found> {
    let code = code_only(src);
    let skip = test_items(&code, src);
    let in_test = |pos: usize| skip.iter().any(|&(a, b)| pos >= a && pos < b);
    let consts = const_contexts(&code);
    let in_const = |pos: usize| consts.iter().any(|&(a, b)| pos >= a && pos < b);
    let src_lines: Vec<&str> = src.lines().collect();
    let line_of = |pos: usize| code[..pos].matches('\n').count() as u64 + 1;
    let mut found = Vec::new();
    for name in ["assert!", "assert_eq!", "assert_ne!"] {
        let mut from = 0;
        while let Some(off) = code[from..].find(name) {
            let pos = from + off;
            from = pos + name.len();
            let prev = code[..pos].chars().next_back();
            if prev.is_some_and(|p| p.is_alphanumeric() || p == '_') || in_test(pos) {
                continue;
            }
            // Compile-time assertions (in `const` items, `static` initializers and inline
            // `const { .. }` blocks) cannot panic at run time.
            if in_const(pos) {
                continue;
            }
            let line = line_of(pos);
            let text = src_lines
                .get(usize::try_from(line).unwrap_or(0) - 1)
                .copied()
                .unwrap_or("");
            found.push(Found {
                kind: "assert",
                line,
                code: text.trim().to_owned(),
                reason: reason_above(src, line),
            });
        }
    }
    let mut from = 0;
    while let Some(off) = code[from..].find("track_caller") {
        let pos = from + off;
        from = pos + "track_caller".len();
        if in_test(pos) {
            continue;
        }
        // The signature of the function the attribute marks.
        let Some(fn_off) = code[pos..].find("fn ") else {
            continue;
        };
        let sig_line = line_of(pos + fn_off);
        let text = src_lines
            .get(usize::try_from(sig_line).unwrap_or(0) - 1)
            .copied()
            .unwrap_or("")
            .trim();
        let sig = text
            .split(" where")
            .next()
            .unwrap_or(text)
            .trim_end_matches('{')
            .trim();
        found.push(Found {
            kind: "track_caller",
            line: sig_line,
            code: sig.to_owned(),
            reason: None,
        });
    }
    found.sort_by_key(|f| f.line);
    found
}

/// The inventory document.
#[must_use]
pub fn render(crates: &[String], entries: &[Entry]) -> String {
    let mut s = String::from(HEADER);
    // Summary.
    s.push_str("\n## Summary\n\n| Crate |");
    for (_, title) in CATEGORIES {
        let _ = write!(s, " {title} |");
    }
    s.push_str("\n|---|");
    s.push_str(&"---:|".repeat(CATEGORIES.len()));
    s.push('\n');
    let mut totals = vec![0usize; CATEGORIES.len()];
    for krate in crates {
        let _ = write!(s, "| {krate} |");
        for (i, (cat, _)) in CATEGORIES.iter().enumerate() {
            let n = entries
                .iter()
                .filter(|e| &e.krate == krate && e.kind == *cat)
                .count();
            totals[i] += n;
            let _ = write!(s, " {n} |");
        }
        s.push('\n');
    }
    s.push_str("| **all** |");
    for t in &totals {
        let _ = write!(s, " {t} |");
    }
    s.push('\n');
    // Explicit sites.
    s.push_str(
        "\n## Explicit panic sites\n\nEach with its justification (the `// PANIC:` comment above \
         it in the source).\n",
    );
    let mut any = false;
    for krate in crates {
        let list: Vec<&Entry> = entries
            .iter()
            .filter(|e| &e.krate == krate && is_explicit(&e.kind))
            .collect();
        if list.is_empty() {
            continue;
        }
        any = true;
        let _ = write!(
            s,
            "\n### {krate}\n\n| Kind | File | Code | Justification |\n|---|---|---|---|\n"
        );
        for e in list {
            let _ = writeln!(
                s,
                "| {} | `{}` | `{}` | {} |",
                kind_title(&e.kind),
                short(&e.file, krate),
                cell(&e.code),
                e.reason
                    .as_deref()
                    .map_or_else(|| "**missing**".to_owned(), |r| r.replace('|', "\\|"))
            );
        }
    }
    if !any {
        s.push_str("\nNone.\n");
    }
    // `#[track_caller]` functions.
    s.push_str(
        "\n## `#[track_caller]` functions\n\nFunctions that report their caller's location: the \
         panicking API forms (thin wrappers over a non-panicking form) and diagnostics.\n",
    );
    for krate in crates {
        let list: Vec<&Entry> = entries
            .iter()
            .filter(|e| &e.krate == krate && e.kind == "track_caller")
            .collect();
        if list.is_empty() {
            continue;
        }
        let _ = write!(s, "\n### {krate}\n\n");
        let mut by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for e in list {
            by_file.entry(&e.file).or_default().push(&e.code);
        }
        for (file, sigs) in by_file {
            let _ = writeln!(
                s,
                "- `{}`: {}",
                short(file, krate),
                sigs.iter()
                    .map(|c| format!("`{}`", cell(c)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    // Indexing and arithmetic per file.
    s.push_str(
        "\n## Indexing and arithmetic sites per file\n\nPossible out-of-bounds indexing or slicing \
         (`clippy::indexing_slicing`) and arithmetic that can overflow or divide by zero \
         (`clippy::arithmetic_side_effects`), per file. Their removal is tracked by the lint \
         ratchet (`profile.md` § 4.3).\n\n| Crate | File | Indexing | Arithmetic |\n|---|---|---:|---:|\n",
    );
    let mut per_file: BTreeMap<(&str, &str), (usize, usize)> = BTreeMap::new();
    for e in entries {
        let slot = || (0, 0);
        match e.kind.as_str() {
            "indexing_slicing" => per_file.entry((&e.krate, &e.file)).or_insert_with(slot).0 += 1,
            "arithmetic_side_effects" => per_file.entry((&e.krate, &e.file)).or_insert_with(slot).1 += 1,
            _ => {}
        }
    }
    for ((krate, file), (i, a)) in per_file {
        let _ = writeln!(s, "| {krate} | `{}` | {i} | {a} |", short(file, krate));
    }
    render_proofs(&mut s, crates, entries);
    s
}

/// The "Proven sites" section of the inventory.
fn render_proofs(s: &mut String, crates: &[String], entries: &[Entry]) {
    s.push_str(
        "\n## Proven sites\n\nIndexing and arithmetic that the code proves in range instead of \
         checking at run time: `wrapping_*` arithmetic that cannot wrap and checked accesses whose \
         fallback is unreachable. Each carries its proof (the `// SAFE-INDEX:` or `// SAFE-ARITH:` \
         comment in the source, copied below). They cannot panic and are not lint sites; they are \
         listed as evidence for review.\n\n| Crate | `SAFE-INDEX` | `SAFE-ARITH` |\n|---|---:|---:|\n",
    );
    let count = |krate: &str, kind: &str| {
        entries
            .iter()
            .filter(|e| e.krate == krate && e.kind == kind)
            .count()
    };
    for krate in crates {
        let (i, a) = (count(krate, "safe_index"), count(krate, "safe_arith"));
        if i + a > 0 {
            let _ = writeln!(s, "| {krate} | {i} | {a} |");
        }
    }
    for krate in crates {
        let list: Vec<&Entry> = entries
            .iter()
            .filter(|e| &e.krate == krate && is_proof(&e.kind))
            .collect();
        if list.is_empty() {
            continue;
        }
        let _ = write!(
            s,
            "\n### {krate}\n\n| Kind | File | Code | Proof |\n|---|---|---|---|\n"
        );
        for e in list {
            let _ = writeln!(
                s,
                "| `{}` | `{}` | `{}` | {} |",
                proof_tag(&e.kind).trim_end_matches(':'),
                short(&e.file, krate),
                cell(&e.code),
                e.reason.as_deref().unwrap_or_default().replace('|', "\\|")
            );
        }
    }
}

/// The table title of a category.
fn kind_title(kind: &str) -> &str {
    CATEGORIES
        .iter()
        .find(|(c, _)| *c == kind)
        .map_or(kind, |(_, t)| t)
}

/// `file` without the `crates/<crate>/` prefix.
fn short<'a>(file: &'a str, krate: &str) -> &'a str {
    file.strip_prefix(&format!("crates/{krate}/")).unwrap_or(file)
}

/// Text made safe for a Markdown table cell inside backticks or plain.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('`', "'")
}

const HEADER: &str = "# Panic inventory

<!-- Generated by `cargo xtask panics`; checked by CI (stage `clippy-safety`). Do not edit. -->

Every place in the library code of the safety profile's crates where a profile build can panic
(`docs/safety/profile.md`). Generated by `cargo xtask panics`; CI (stage `clippy-safety`, `cargo
xtask panics --check`) fails when this file is out of date or when an explicit site has no
justification.

**Method.** Clippy over the `twine` facade with the profile's feature set (`safety` plus every
compatible feature: the code a profile build compiles), with the safety lints forced to warn,
reports `panic!`, `unwrap`, `expect`, `unreachable!`, `todo!`/`unimplemented!`, indexing and
slicing, and arithmetic that can overflow or divide by zero (which panics under
`overflow-checks = true`). A scan of each crate's `src/` (outside `#[cfg(test)]` items, items
the profile removes with `#[cfg(not(feature = \"safety\"))]`, comments and literals) adds `assert!`/`assert_eq!`/`assert_ne!` (not `debug_assert*`, not compile-time
`const` assertions) and the functions marked `#[track_caller]`. Each explicit site carries a
`// PANIC: …` justification in the source, copied below. Line numbers are left out, so the file
changes only when sites change.

**Not covered** (not reported by these tools): `RefCell` borrow conflicts (the runtimes keep no
borrow across user code, rule R1), allocation failure (the global allocator's handler),
shifts (proven in range by R5.S09, `shifts.md`), panics inside dependencies, and panics in the
application's own callbacks. Divisions and remainders are counted under arithmetic (a possible
overflow); that none divides by zero is proven separately (R5.S10, `divisions.md`).

**Proven sites.** Indexing and arithmetic that the code proves in range (R5.S11: `wrapping_*`
arithmetic that cannot wrap, checked accesses whose fallback is never taken) carry a
`// SAFE-INDEX: …` or `// SAFE-ARITH: …` proof comment; the source scan lists each with its proof
at the end of this file, and CI fails on an empty proof or a proof comment with no code below it.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_only_blanks_comments_and_literals() {
        let src = "let a = \"assert!(x)\"; // assert!(y)\n/* assert!(z) */ let c = '}';\nlet r = r#\"{\"#;";
        let code = code_only(src);
        assert_eq!(code.len(), src.len());
        assert!(!code.contains("assert"), "{code}");
        assert!(!code.contains('}') && !code.contains('{'), "{code}");
        assert_eq!(code.lines().count(), 3);
        // Lifetimes stay.
        assert!(code_only("fn f<'a>(x: &'a str)").contains("'a"));
    }

    #[test]
    fn scan_finds_runtime_asserts_and_track_caller_outside_tests() {
        let src = r#"
const _: () = assert!(1 < 2);
const C: [u8; 1] = {
    assert!(true);
    [0]
};
pub static S: u8 = { assert!(1 > 0); 1 };
pub const fn k<const N: usize>() -> usize {
    const { assert!(N < 4) }
    N
}
pub fn f(x: u8) {
    // PANIC: callers pass a value below 10.
    assert!(x < 10, "x");
    debug_assert!(x < 5);
}
#[track_caller]
pub fn g(&self) -> u8 where T: Clone {
    0
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() { assert_eq!(1, 1); }
}
#[cfg(not(feature = "safety"))]
#[track_caller]
pub fn build(self) { assert!(false) }
"#;
        let found = scan_source(src);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].kind, "assert");
        assert_eq!(found[0].reason.as_deref(), Some("callers pass a value below 10."));
        assert_eq!(found[1].kind, "track_caller");
        assert_eq!(found[1].code, "pub fn g(&self) -> u8");
    }

    #[test]
    fn reason_is_the_comment_block_above_or_trailing() {
        let src = "// unrelated\n// PANIC: no value of `T` exists;\n// a read must diverge.\npanic!(\"x\")\nx.unwrap(); // PANIC: proven above\n";
        assert_eq!(
            reason_above(src, 4).as_deref(),
            Some("no value of `T` exists; a read must diverge.")
        );
        assert_eq!(reason_above(src, 5).as_deref(), Some("proven above"));
        assert_eq!(reason_above("\npanic!()\n", 2), None);
    }

    #[test]
    fn render_lists_explicit_sites_and_counts() {
        let e = |kind: &str, file: &str, reason: Option<&str>| Entry {
            krate: "twine-x".into(),
            kind: kind.into(),
            file: file.into(),
            code: "panic!(\"a|b\")".into(),
            reason: reason.map(Into::into),
            line: 1,
        };
        let doc = render(
            &["twine-x".into()],
            &[
                e("panic", "crates/twine-x/src/a.rs", Some("why")),
                e("indexing_slicing", "crates/twine-x/src/a.rs", None),
                e("indexing_slicing", "crates/twine-x/src/a.rs", None),
            ],
        );
        assert!(
            doc.contains("| twine-x | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 2 | 0 |"),
            "{doc}"
        );
        assert!(
            doc.contains("| `panic!` | `src/a.rs` | `panic!(\"a\\|b\")` | why |"),
            "{doc}"
        );
        assert!(doc.contains("| twine-x | `src/a.rs` | 2 | 0 |"), "{doc}");
    }

    #[test]
    fn scan_proofs_reads_proof_comments_outside_tests() {
        let src = r"
fn f(a: u32, v: &[u8]) -> u8 {
    // SAFE-ARITH: `a < 2^16`, so the sum fits.
    // Second line of the proof.
    // DIV: not part of the proof.
    let b = a.wrapping_add(1) / 2;
    let c = v.get(0).copied().unwrap_or(0); // SAFE-INDEX: `v` is never empty.
    // SAFE-ARITH:
    let d = a.wrapping_sub(1);
    // SAFE-INDEX: dangling.

    c
}
#[cfg(test)]
mod tests {
    // SAFE-ARITH: ignored in tests.
    fn t() {}
}
";
        let found = scan_proofs(src);
        assert_eq!(found.len(), 4, "{found:?}");
        assert_eq!(found[0].kind, "safe_arith");
        assert_eq!(found[0].code, "let b = a.wrapping_add(1) / 2;");
        assert_eq!(
            found[0].reason.as_deref(),
            Some("`a < 2^16`, so the sum fits. Second line of the proof.")
        );
        assert_eq!(found[1].kind, "safe_index");
        assert_eq!(found[1].code, "let c = v.get(0).copied().unwrap_or(0);");
        assert_eq!(found[1].reason.as_deref(), Some("`v` is never empty."));
        assert_eq!(found[2].reason.as_deref(), Some(""), "empty proof");
        assert_eq!(found[3].code, "", "dangling proof");
    }

    #[test]
    fn render_lists_proven_sites() {
        let e = |kind: &str| Entry {
            krate: "twine-x".into(),
            kind: kind.into(),
            file: "crates/twine-x/src/a.rs".into(),
            code: "x.wrapping_add(1)".into(),
            reason: Some("`x < 9`".into()),
            line: 1,
        };
        let doc = render(
            &["twine-x".into()],
            &[e("safe_arith"), e("safe_arith"), e("safe_index")],
        );
        assert!(doc.contains("| twine-x | 1 | 2 |"), "{doc}");
        assert!(
            doc.contains("| `SAFE-ARITH` | `src/a.rs` | `x.wrapping_add(1)` | `x < 9` |"),
            "{doc}"
        );
    }

    #[test]
    fn checked_in_inventory_exists() {
        let doc = std::fs::read_to_string(workspace_root().join(PANICS_DOC)).unwrap();
        assert!(doc.starts_with("# Panic inventory"));
    }
}
