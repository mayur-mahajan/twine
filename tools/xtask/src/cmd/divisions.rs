//! `cargo xtask divisions [--check]`: the divisor audit (R5.S10, coding guideline G3 "do not
//! divide by 0", required) and its inventory [`DIVISIONS_DOC`] (`docs/safety/divisions.md`).
//!
//! Every division and remainder in the library code of every crate (`/`, `%`, `/=`, `%=`,
//! `div`/`rem` and the panicking division methods `div_euclid`, `rem_euclid`, `div_ceil`,
//! `wrapping_*`, `overflowing_*`, `saturating_div`; found by the `syn` scan of [`audit`]) whose
//! divisor is not a literal must be shown non-zero, by one of:
//!
//! - its **type**: a `NonZero*` value (a parameter, `let` or struct field declared `NonZero*`,
//!   its `.get()`, `NonZero*::get(..)`, a lossless `u32::from(..)` of it) — Rust's
//!   `u32 / NonZeroU32` cannot panic;
//! - a **guard** in the expression or an immutable `let`: `.max(n)` or `.clamp(n, ..)` with a
//!   literal `n ≥ 1`;
//! - a **proof** comment `// DIV: …` (a checked invariant, a validated header field, a branch
//!   that excludes zero): on the operator's line (trailing), or in the comment block directly
//!   above the operator's line, the division's first line or the enclosing statement's first
//!   line.
//!
//! Floating-point divisions (cannot panic) are counted apart and need nothing. Divisions by a
//! **named constant** are exempt ([`audit::CONSTANT_OPERANDS_EXEMPT`], shared with the shift
//! audit; user decision 2026-10-20): rustc's deny-by-default `unconditional_panic` lint already
//! rejects a division by a constant zero. They are listed, marked *(constant)*.
//! `checked_div`/`checked_rem` are not audited (they return `None` for a zero divisor).
//!
//! The command writes [`DIVISIONS_DOC`] (each audited site with its proof, no line numbers, so
//! the file changes only when sites change); `--check` (CI stage `divisions`) fails when a site
//! has no proof or the file is out of date. Without a `docs/safety/` directory only the proofs
//! are checked.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::cmd::audit::{self, CrateTypes, OpSite, Operand};
use crate::util::{R, workspace_root};

/// The checked-in inventory.
pub const DIVISIONS_DOC: &str = "docs/safety/divisions.md";

/// The comment marker of a divisor proof.
pub const PROOF_TAG: &str = "DIV:";

/// A site with its proof.
#[derive(Debug)]
pub struct Proven {
    /// The site.
    pub site: OpSite,
    /// The proof (`// DIV: …`), if any.
    pub proof: Option<String>,
}

impl Proven {
    /// Whether the site needs a `// DIV:` proof comment.
    #[must_use]
    pub fn needs_proof(&self) -> bool {
        self.site.kind.needs_proof()
    }

    /// Whether the site is a problem: it needs a proof and has none.
    #[must_use]
    pub fn missing(&self) -> bool {
        self.needs_proof() && self.proof.is_none()
    }
}

/// Every division and remainder of the library crates with its proof:
/// `(crate names, sites)`.
pub fn sites() -> Result<(Vec<String>, Vec<Proven>), Box<dyn std::error::Error>> {
    let files = audit::library_files()?;
    let crates: Vec<String> = audit::library_crates()?.into_iter().map(|(n, _)| n).collect();
    let mut out = Vec::new();
    for krate in &crates {
        let of_crate: Vec<&audit::SourceFile> = files.iter().filter(|f| &f.krate == krate).collect();
        let types = CrateTypes::of(of_crate.iter().copied());
        for file in of_crate {
            for site in audit::scan_divisions(file, &types) {
                let proof = audit::proof_above(&file.src, &site.anchor_lines, PROOF_TAG);
                out.push(Proven { site, proof });
            }
        }
    }
    Ok((crates, out))
}

/// `cargo xtask divisions [--check]`.
pub fn run(check: bool) -> R {
    let (crates, all) = sites()?;
    let mut errors: Vec<String> = all
        .iter()
        .filter(|p| p.missing())
        .map(|p| {
            format!(
                "{}:{}: `{}` divides by `{}` without a `NonZero` type, a `.max(1)` guard or a `// {PROOF_TAG} …` proof that it is non-zero",
                p.site.file, p.site.line, p.site.code, p.site.operand
            )
        })
        .collect();
    let doc = render(&crates, &all);
    let path = workspace_root().join(DIVISIONS_DOC);
    if path.parent().is_some_and(std::path::Path::is_dir) {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != doc {
            if check {
                errors.push(format!(
                    "{DIVISIONS_DOC} is out of date (run `cargo xtask divisions` and review the change)"
                ));
            } else {
                std::fs::write(&path, &doc)?;
                println!("divisions: {DIVISIONS_DOC} written");
            }
        }
    } else {
        println!("divisions: note: docs/safety/ not found; only the proofs are checked");
    }
    let c = Counts::of(&all, None);
    if errors.is_empty() {
        println!(
            "divisions: ok ({} non-constant divisors: {} by type, {} guarded, {} proven; {} by a named constant; {} float; {} crates scanned)",
            c.nonzero + c.guarded + c.proven,
            c.nonzero,
            c.guarded,
            c.proven,
            c.constant,
            c.float,
            crates.len()
        );
        Ok(())
    } else {
        for e in &errors {
            println!("divisions: {e}");
        }
        Err(format!("divisions: {} problem(s)", errors.len()).into())
    }
}

/// Site counts of one crate (or all).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Divisions by a literal.
    pub literal: usize,
    /// Divisions by a named constant.
    pub constant: usize,
    /// Non-constant divisors proven by a `NonZero*` type.
    pub nonzero: usize,
    /// Non-constant divisors proven by a guard.
    pub guarded: usize,
    /// Non-constant divisors with a `// DIV:` proof.
    pub proven: usize,
    /// Non-constant divisors without a proof.
    pub unproven: usize,
    /// Sites without a required proof (`unproven`, plus constant divisors without a proof unless
    /// [`audit::CONSTANT_OPERANDS_EXEMPT`]).
    pub missing: usize,
    /// Floating-point divisions.
    pub float: usize,
}

impl Counts {
    /// Counts the sites of `krate` (or of all crates).
    #[must_use]
    pub fn of(all: &[Proven], krate: Option<&str>) -> Self {
        let mut c = Counts::default();
        for p in all.iter().filter(|p| krate.is_none_or(|k| p.site.krate == k)) {
            if p.missing() {
                c.missing += 1;
            }
            match p.site.kind {
                Operand::Literal => c.literal += 1,
                Operand::Const => c.constant += 1,
                Operand::NonZero => c.nonzero += 1,
                Operand::Guarded => c.guarded += 1,
                Operand::Float => c.float += 1,
                Operand::Runtime if p.proof.is_some() => c.proven += 1,
                Operand::Runtime => c.unproven += 1,
            }
        }
        c
    }

    /// Integer divisions and remainders (guideline G4).
    #[must_use]
    pub fn integer(&self) -> usize {
        self.literal + self.constant + self.nonzero + self.guarded + self.proven + self.unproven
    }
}

/// The inventory document.
#[must_use]
pub fn render(crates: &[String], all: &[Proven]) -> String {
    let mut s = String::from(HEADER);
    s.push_str(
        "\n## Summary\n\nInteger divisions and remainders by a divisor that is not a literal: non-zero by its \
         `NonZero*` type, by a guard, by a `// DIV:` proof, by a named constant, and sites without a \
         required proof; floating-point divisions apart.\n\n\
         | Crate | `NonZero` type | guard | proof | named constant | missing proof | float |\n\
         |---|---:|---:|---:|---:|---:|---:|\n",
    );
    for krate in crates {
        let c = Counts::of(all, Some(krate));
        if c.nonzero + c.guarded + c.proven + c.constant + c.missing + c.float == 0 {
            continue;
        }
        let _ = writeln!(
            s,
            "| {krate} | {} | {} | {} | {} | {} | {} |",
            c.nonzero, c.guarded, c.proven, c.constant, c.missing, c.float
        );
    }
    let t = Counts::of(all, None);
    let _ = writeln!(
        s,
        "| **all** | {} | {} | {} | {} | {} | {} |",
        t.nonzero, t.guarded, t.proven, t.constant, t.missing, t.float
    );
    let _ = writeln!(
        s,
        "\nDivisions by a literal: {}. Named-constant divisors need a proof: {}.",
        t.literal,
        if audit::CONSTANT_OPERANDS_EXEMPT {
            "no — rustc's deny-by-default `unconditional_panic` lint rejects a division by a constant zero"
        } else {
            "yes"
        }
    );
    s.push_str(
        "\n## Divisors that are not literals\n\nEach with how it is known to be non-zero: *type* (a \
         `NonZero*` value), *guard* (`.max(n)`/`.clamp(n, ..)`, `n ≥ 1`) or the proof (the `// DIV:` \
         comment next to it in the source). *constant*: a named constant (exempt: rustc rejects a constant zero); \
         *(macro)*: found in macro input or a `macro_rules!` body.\n",
    );
    for krate in crates {
        let list: Vec<&Proven> = all
            .iter()
            .filter(|p| &p.site.krate == krate && !matches!(p.site.kind, Operand::Literal | Operand::Float))
            .collect();
        if list.is_empty() {
            continue;
        }
        let _ = write!(
            s,
            "\n### {krate}\n\n| File | Code | Divisor | Non-zero because |\n|---|---|---|---|\n"
        );
        for p in list {
            let why = match (p.site.kind, &p.proof) {
                (_, Some(r)) => r.replace('|', "\\|"),
                (Operand::NonZero, None) => "*type*".to_owned(),
                (Operand::Guarded, None) => "*guard*".to_owned(),
                (Operand::Const, None) if audit::CONSTANT_OPERANDS_EXEMPT => "*constant* (rustc)".to_owned(),
                _ => "**missing**".to_owned(),
            };
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
                why
            );
        }
    }
    let float: BTreeSet<&str> = all
        .iter()
        .filter(|p| p.site.kind == Operand::Float)
        .map(|p| p.site.krate.as_str())
        .collect();
    if !float.is_empty() {
        let _ = writeln!(
            s,
            "\n## Floating-point divisions\n\nNot listed one by one (IEEE division cannot panic); crates: {}.",
            float.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    s
}

/// Text made safe for a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('`', "'")
}

const HEADER: &str = "# Divisor inventory

<!-- Generated by `cargo xtask divisions`; checked by CI (stage `divisions`). Do not edit. -->

Every integer division and remainder by a divisor that is not a literal in the library code of
Twine's crates, with the reason the divisor cannot be zero (coding guideline G3, required:
`docs/safety/coding-guidelines-deviations.md`; requirement SAF-033). An integer division by
zero panics in every build profile.

**Method.** `cargo xtask divisions` parses every crate's module tree (`syn`; `#[cfg(test)]` code
and compile-time contexts left out) and finds `/`, `%`, `/=`, `%=`, `div`/`rem` and the panicking
division methods (`div_euclid`, `rem_euclid`, `div_ceil`, `wrapping_*`, `overflowing_*`,
`saturating_div`; `checked_*` return `None` for zero and are not audited). A divisor is non-zero
by its **type** (a `NonZero*` value: Rust's `u32 / NonZeroU32` cannot panic), by a **guard**
(`.max(n)`/`.clamp(n, ..)` with a literal `n ≥ 1`, in the expression or an immutable `let`), or by
a **proof** — a `// DIV: …` comment in the source, copied below. CI (`cargo xtask divisions
--check`) fails on a site without one and when this file is out of date. Line numbers are left
out, so the file changes only when sites change.

**API boundary.** Divisor-like values the application supplies (ranges, row and column sizes,
durations and periods, scale factors, font metrics, image sizes) are `NonZero*` types or are
clamped with a `warn!` where they enter Twine, so zero never reaches the arithmetic; decoders
reject a zero size in the header. Hot loops divide only by values proven non-zero at set-up.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::audit::Op;

    #[test]
    fn library_code_has_a_proof_for_every_divisor() {
        let (_, all) = sites().unwrap();
        let missing: Vec<String> = all
            .iter()
            .filter(|p| p.missing())
            .map(|p| format!("{}:{}", p.site.file, p.site.line))
            .collect();
        assert!(missing.is_empty(), "divisions without a proof: {missing:?}");
    }

    #[test]
    fn render_lists_proofs_and_counts() {
        let site = |kind, proof: Option<&str>| Proven {
            site: OpSite {
                krate: "twine-x".into(),
                file: "crates/twine-x/src/a.rs".into(),
                line: 3,
                op: Op::Div,
                code: "x / n".into(),
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
                site(Operand::Runtime, Some("n > 0 (checked by `new`)")),
                site(Operand::Runtime, None),
                site(Operand::NonZero, None),
                site(Operand::Guarded, None),
                site(Operand::Float, None),
                site(Operand::Literal, None),
            ],
        );
        assert!(doc.contains("| twine-x | 1 | 1 | 1 | 0 | 1 | 1 |"), "{doc}");
        assert!(
            doc.contains("| `src/a.rs` | `x / n` | `n` | n > 0 (checked by `new`) |"),
            "{doc}"
        );
        assert!(doc.contains("| **missing** |"), "{doc}");
        assert!(doc.contains("| *type* |") && doc.contains("| *guard* |"), "{doc}");
    }
}
