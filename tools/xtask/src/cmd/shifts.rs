//! `cargo xtask shifts [--check]`: the shift-amount audit (R5.S09, coding guideline G5 "avoid
//! out-of-range shifts", mandatory) and its inventory [`SHIFTS_DOC`] (`docs/safety/shifts.md`).
//!
//! Every shift in the library code of every crate (`<<`, `>>`, `<<=`, `>>=`, `shl`/`shr`,
//! `wrapping_*` and `overflowing_*` forms; found by the `syn` scan of [`audit`]) whose amount
//! is not a literal must carry a `// SHIFT: …` proof that the amount is below the bit width of
//! the shifted type: on the line of the operator (trailing), or in the comment block directly
//! above the operator's line, the shift expression's first line or the enclosing statement's
//! first line. Constant amounts (named constants) are exempt ([`audit::CONSTANT_OPERANDS_EXEMPT`],
//! user decision 2026-10-20): rustc's deny-by-default `arithmetic_overflow` lint rejects an
//! out-of-range constant amount at compile time; the inventory lists them, marked. The `checked_shl`/`checked_shr` forms are not audited (they report an
//! out-of-range amount).
//!
//! The command writes [`SHIFTS_DOC`] (each proven site with its proof, no line numbers, so the
//! file changes only when sites change); `--check` (CI stage `shifts`) fails when a site has no
//! proof or the file is out of date. Without a `docs/safety/` directory (a checkout without
//! `docs/`) only the proofs are checked.

use std::fmt::Write as _;

use crate::cmd::audit::{self, Op, OpSite, Operand};
use crate::util::{R, workspace_root};

/// The checked-in inventory.
pub const SHIFTS_DOC: &str = "docs/safety/shifts.md";

/// The comment marker of a shift-amount proof.
pub const PROOF_TAG: &str = "SHIFT:";

/// A site with its proof.
#[derive(Debug)]
pub struct Proven {
    /// The site.
    pub site: OpSite,
    /// The proof (`// SHIFT: …`), if any.
    pub proof: Option<String>,
}

/// Every shift site of the library crates with its proof.
pub fn sites() -> Result<(Vec<String>, Vec<Proven>), Box<dyn std::error::Error>> {
    let files = audit::library_files()?;
    let crates = audit::library_crates()?.into_iter().map(|(n, _)| n).collect();
    let mut out = Vec::new();
    for file in &files {
        for site in audit::scan(file, &[Op::Shl, Op::Shr]) {
            let proof = audit::proof_above(&file.src, &site.anchor_lines, PROOF_TAG);
            out.push(Proven { site, proof });
        }
    }
    Ok((crates, out))
}

/// `cargo xtask shifts [--check]`.
pub fn run(check: bool) -> R {
    let (crates, all) = sites()?;
    let mut errors = Vec::new();
    for p in &all {
        if p.site.kind.needs_proof() && p.proof.is_none() {
            errors.push(format!(
                "{}:{}: `{}` shifts by `{}` without a `// {PROOF_TAG} …` proof that the amount is in range",
                p.site.file, p.site.line, p.site.code, p.site.operand
            ));
        }
    }
    let doc = render(&crates, &all);
    let root = workspace_root();
    let path = root.join(SHIFTS_DOC);
    if path.parent().is_some_and(std::path::Path::is_dir) {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != doc {
            if check {
                errors.push(format!(
                    "{SHIFTS_DOC} is out of date (run `cargo xtask shifts` and review the change)"
                ));
            } else {
                std::fs::write(&path, &doc)?;
                println!("shifts: {SHIFTS_DOC} written");
            }
        }
    } else {
        println!("shifts: note: docs/safety/ not found; only the proofs are checked");
    }
    let audited: Vec<&Proven> = all.iter().filter(|p| p.site.kind != Operand::Literal).collect();
    if errors.is_empty() {
        let constant = audited.iter().filter(|p| p.site.kind == Operand::Const).count();
        println!(
            "shifts: ok ({} shifts by a run-time amount, each with a proof, and {constant} by a named constant (exempt), in {} files; {} crates scanned)",
            audited.len() - constant,
            audited
                .iter()
                .map(|p| &p.site.file)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            crates.len()
        );
        Ok(())
    } else {
        for e in &errors {
            println!("shifts: {e}");
        }
        Err(format!("shifts: {} problem(s)", errors.len()).into())
    }
}

/// The inventory document.
#[must_use]
pub fn render(crates: &[String], all: &[Proven]) -> String {
    let mut s = String::from(HEADER);
    s.push_str(
        "\n## Summary\n\nShifts by a non-literal amount: run-time amounts with a proof, constant amounts \
         (exempt), and sites without a required proof.\n\n| Crate | run-time amount | constant amount | missing proof |\n|---|---:|---:|---:|\n",
    );
    let (mut tp, mut tm, mut tc) = (0, 0, 0);
    for krate in crates {
        let of = |k: Operand| {
            all.iter()
                .filter(move |p| &p.site.krate == krate && p.site.kind == k)
        };
        let proven = of(Operand::Runtime).filter(|p| p.proof.is_some()).count();
        let constant = of(Operand::Const).count();
        let missing = of(Operand::Runtime)
            .chain(of(Operand::Const))
            .filter(|p| p.site.kind.needs_proof() && p.proof.is_none())
            .count();
        if proven + missing + constant == 0 {
            continue;
        }
        tp += proven;
        tm += missing;
        tc += constant;
        let _ = writeln!(s, "| {krate} | {proven} | {constant} | {missing} |");
    }
    let _ = writeln!(s, "| **all** | {tp} | {tc} | {tm} |");
    s.push_str(
        "\n## Shifts by a non-literal amount\n\nEach with its proof (the `// SHIFT:` comment next to it \
         in the source). *(constant)*: a constant expression, exempt from the proof (rustc's \
         `arithmetic_overflow` lint rejects an out-of-range constant amount); *(macro)*: \
         found in macro input or a `macro_rules!` body.\n",
    );
    for krate in crates {
        let list: Vec<&Proven> = all
            .iter()
            .filter(|p| &p.site.krate == krate && p.site.kind != Operand::Literal)
            .collect();
        if list.is_empty() {
            continue;
        }
        let _ = write!(
            s,
            "\n### {krate}\n\n| File | Code | Amount | Proof |\n|---|---|---|---|\n"
        );
        for p in list {
            let _ = writeln!(
                s,
                "| `{}` | `{}` | `{}`{}{} | {} |",
                p.site
                    .file
                    .strip_prefix(&format!("crates/{krate}/"))
                    .unwrap_or(&p.site.file),
                cell(&p.site.code),
                cell(&p.site.operand),
                if p.site.kind == Operand::Const {
                    " *(constant)*"
                } else {
                    ""
                },
                if p.site.in_macro { " *(macro)*" } else { "" },
                match (&p.proof, p.site.kind.needs_proof()) {
                    (Some(r), _) => r.replace('|', "\\|"),
                    (None, false) => "*constant* (rustc)".to_owned(),
                    (None, true) => "**missing**".to_owned(),
                }
            );
        }
    }
    s
}

/// Text made safe for a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('`', "'")
}

const HEADER: &str = "# Shift-amount inventory

<!-- Generated by `cargo xtask shifts`; checked by CI (stage `shifts`). Do not edit. -->

Every shift by a non-literal amount in the library code of Twine's crates, with the proof that
the amount stays below the bit width of the shifted type (coding guideline G5, mandatory:
`docs/safety/coding-guidelines-deviations.md`; requirement SAF-032). A shift by at least the bit
width panics in debug builds and silently masks the amount in release builds.

**Method.** `cargo xtask shifts` parses every crate's module tree (`syn`; `#[cfg(test)]` code and
compile-time contexts left out) and finds `<<`, `>>`, `<<=`, `>>=`, `shl`/`shr` and the
`wrapping_*`/`overflowing_*` shift methods (`checked_*` report an out-of-range amount and are not
audited). Every run-time amount carries a `// SHIFT: …` proof in the source, copied below.
Constant amounts (named constants such as `FIELD_SHIFT`, outside generic code) are listed but
exempt (user decision 2026-10-20): rustc's deny-by-default `arithmetic_overflow` lint rejects an
out-of-range constant amount at compile time. CI (`cargo xtask shifts --check`) fails on a
run-time amount without a proof and when this file is out of date. Line numbers are left
out, so the file changes only when sites change.

**Decoders** (`twine-image`, `twine-vector`, `twine-lottie`, the fonts of `twine-text` and
`twine-assets`) validate every shift amount derived from their input once, when the input is
parsed, and reject the input otherwise; their hot loops shift by those validated values.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_code_has_a_proof_for_every_run_time_shift() {
        let (_, all) = sites().unwrap();
        let missing: Vec<String> = all
            .iter()
            .filter(|p| p.site.kind.needs_proof() && p.proof.is_none())
            .map(|p| format!("{}:{}", p.site.file, p.site.line))
            .collect();
        assert!(missing.is_empty(), "shifts without a proof: {missing:?}");
    }

    #[test]
    fn render_lists_proofs_and_counts() {
        let site = |kind, proof: Option<&str>| Proven {
            site: OpSite {
                krate: "twine-x".into(),
                file: "crates/twine-x/src/a.rs".into(),
                line: 3,
                op: Op::Shl,
                code: "x << n".into(),
                operand: "n".into(),
                kind,
                in_macro: false,
                anchor_lines: vec![3],
            },
            proof: proof.map(Into::into),
        };
        let doc = render(
            &["twine-x".into()],
            &[
                site(Operand::Runtime, Some("n < 8")),
                site(Operand::Runtime, None),
                site(Operand::Const, None),
            ],
        );
        assert!(doc.contains("| twine-x | 1 | 1 | 1 |"), "{doc}");
        assert!(doc.contains("| *constant* (rustc) |"), "{doc}");
        assert!(doc.contains("| `src/a.rs` | `x << n` | `n` | n < 8 |"), "{doc}");
        assert!(doc.contains("| **missing** |"), "{doc}");
    }
}
