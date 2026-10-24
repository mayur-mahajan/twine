//! The linking fixtures of `cargo xtask nostd` (R7.S01; design `docs/plan/r7-theming-design.md`
//! §2.9.7): pay for what you use, checked on linked images.
//!
//! `tools/link-fixtures` is a standalone crate of minimal `no_std` images (one per `src/bin/`
//! file), each building a small UI the way an application does. They are linked for
//! [`FIXTURE_TARGET`] in the release profile (fat LTO, `--gc-sections`), and each image's
//! symbol table is checked against the rules of `tools/link-fixtures/fixtures.toml`: an item
//! (a theme, a mode table, a recipe, a font, an icon) that the source does not reference must
//! not be linked. A rule marked `until = "<step>"` is a known failure of today's code (the R7
//! baseline): it is reported, not failed, until that step removes the item — and once it
//! passes, the mark must be removed (the check fails), so the baseline only shrinks.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cmd::firmware::{ElfSymbol, elf_symbols};
use crate::util::{R, cargo_ci, run as run_cmd, workspace_root};

/// The target the fixtures are linked for (Cortex-M4F: CAS atomics, no `portable-atomic`
/// configuration needed).
pub const FIXTURE_TARGET: &str = "thumbv7em-none-eabihf";

/// The fixture crate, relative to the workspace root.
const FIXTURE_DIR: &str = "tools/link-fixtures";

/// The rule file (`fixtures.toml`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixtures {
    /// One entry per image.
    pub fixture: Vec<Fixture>,
}

/// One image and what it must not link.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// The binary (`src/bin/<bin>.rs`).
    pub bin: String,
    /// The rules.
    #[serde(default)]
    pub rule: Vec<Rule>,
}

/// An item the image must not link.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// What the item is (for the report).
    pub item: String,
    /// Substrings of demangled symbol names: a symbol containing one is the item.
    pub forbid: Vec<String>,
    /// Substrings exempting a symbol that matches `forbid`.
    #[serde(default)]
    pub allow: Vec<String>,
    /// A known failure: the step expected to remove the item.
    pub until: Option<String>,
}

/// The result of one rule on one image.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing of the item is linked.
    Pass,
    /// The item is linked: the offending symbols (name, size), largest first, and their total.
    Linked {
        /// `(demangled name, bytes)`.
        symbols: Vec<(String, u64)>,
        /// Sum of the sizes.
        bytes: u64,
    },
}

/// Applies `rule` to the symbols of an image.
#[must_use]
pub fn check_rule(rule: &Rule, symbols: &[ElfSymbol]) -> Outcome {
    let mut hits: Vec<(String, u64)> = symbols
        .iter()
        .filter(|s| rule.forbid.iter().any(|p| s.name.contains(p.as_str())))
        .filter(|s| !rule.allow.iter().any(|p| s.name.contains(p.as_str())))
        .map(|s| (s.name.clone(), s.size))
        .collect();
    if hits.is_empty() {
        return Outcome::Pass;
    }
    hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let bytes = hits.iter().map(|h| h.1).sum();
    Outcome::Linked { symbols: hits, bytes }
}

/// The verdicts of one image.
#[derive(Debug, Default)]
pub struct Verdict {
    /// One line per rule (and the largest offending symbols).
    pub report: String,
    /// Regressions and stale baseline marks.
    pub failures: Vec<String>,
    /// Known failures (rules with `until` whose item is linked).
    pub known: usize,
}

/// Judges one image's symbols against its fixture's rules.
#[must_use]
pub fn judge(fixture: &Fixture, symbols: &[ElfSymbol]) -> Verdict {
    let mut v = Verdict::default();
    let (report, failures) = (&mut v.report, &mut v.failures);
    for rule in &fixture.rule {
        let outcome = check_rule(rule, symbols);
        match (&outcome, rule.until.as_deref()) {
            (Outcome::Pass, None) => {
                let _ = writeln!(report, "  ok        {}", rule.item);
            }
            (Outcome::Pass, Some(step)) => {
                let _ = writeln!(report, "  FIXED     {} (baseline until {step})", rule.item);
                failures.push(format!(
                    "{}: `{}` is no longer linked: remove its `until = \"{step}\"` mark from \
                     {FIXTURE_DIR}/fixtures.toml",
                    fixture.bin, rule.item
                ));
            }
            (Outcome::Linked { symbols, bytes }, until) => {
                let tag = if until.is_some() { "baseline" } else { "LINKED" };
                let when = until.map(|s| format!(" (until {s})")).unwrap_or_default();
                let _ = writeln!(report, "  {tag:<9} {}{when}: {bytes} B", rule.item);
                for (name, size) in symbols.iter().take(5) {
                    let _ = writeln!(report, "              {size:>6} B  {name}");
                }
                if symbols.len() > 5 {
                    let _ = writeln!(report, "              … {} more symbols", symbols.len() - 5);
                }
                if until.is_some() {
                    v.known += 1;
                } else {
                    failures.push(format!(
                        "{}: links {} ({bytes} B) although its source does not reference it",
                        fixture.bin, rule.item
                    ));
                }
            }
        }
    }
    v
}

/// Reads `fixtures.toml`.
///
/// # Errors
/// The file is missing or malformed.
pub fn read_fixtures(dir: &Path) -> Result<Fixtures, String> {
    let path = dir.join("fixtures.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Where the fixtures are built (inside the workspace's `target/`, so `cargo clean` removes it).
fn target_dir() -> PathBuf {
    workspace_root().join("target").join("link-fixtures")
}

/// Links every fixture and checks it; fails on a regression or a stale baseline mark.
pub fn run() -> R {
    let dir = workspace_root().join(FIXTURE_DIR);
    let fixtures = read_fixtures(&dir)?;
    let mut cmd = cargo_ci();
    cmd.current_dir(&dir).env("CARGO_TARGET_DIR", target_dir()).args([
        "build",
        "--release",
        "--bins",
        "--target",
        FIXTURE_TARGET,
    ]);
    // The fixture crate is its own workspace: its manifest and lock file, not the library's.
    for k in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        cmd.env_remove(k);
    }
    run_cmd(&mut cmd)?;
    let bins = target_dir().join(FIXTURE_TARGET).join("release");
    let mut failures = Vec::new();
    let mut baseline = 0usize;
    for f in &fixtures.fixture {
        let path = bins.join(&f.bin);
        let elf = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let symbols = elf_symbols(&elf)?;
        let mut v = judge(f, &symbols);
        baseline += v.known;
        println!("link-check {}:\n{}", f.bin, v.report);
        failures.append(&mut v.failures);
    }
    if failures.is_empty() {
        println!(
            "link-check: {} fixtures as expected ({baseline} known baseline failures)",
            fixtures.fixture.len()
        );
        Ok(())
    } else {
        Err(format!("link-check:\n  {}", failures.join("\n  ")).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, size: u64) -> ElfSymbol {
        ElfSymbol {
            name: name.into(),
            raw: name.into(),
            size,
            flash: true,
        }
    }

    fn rule(forbid: &[&str], allow: &[&str], until: Option<&str>) -> Rule {
        Rule {
            item: "item".into(),
            forbid: forbid.iter().map(ToString::to_string).collect(),
            allow: allow.iter().map(ToString::to_string).collect(),
            until: until.map(str::to_string),
        }
    }

    const SYMS: &[(&str, u64)] = &[
        ("twine_assets::fonts::montserrat_14::MONTSERRAT_14_BITMAP", 8811),
        ("twine_assets::fonts::montserrat_20::MONTSERRAT_20_BITMAP", 11105),
        ("<twine_theme::default::styles::Styles>::new", 6042),
    ];

    fn syms() -> Vec<ElfSymbol> {
        SYMS.iter().map(|(n, s)| sym(n, *s)).collect()
    }

    #[test]
    fn forbid_allow_and_totals() {
        let r = rule(&["twine_assets::fonts::"], &["montserrat_14::"], None);
        assert_eq!(
            check_rule(&r, &syms()),
            Outcome::Linked {
                symbols: vec![(
                    "twine_assets::fonts::montserrat_20::MONTSERRAT_20_BITMAP".into(),
                    11105
                )],
                bytes: 11105
            }
        );
        let r = rule(&["twine_theme::mono::"], &[], None);
        assert_eq!(check_rule(&r, &syms()), Outcome::Pass);
    }

    #[test]
    fn regressions_and_stale_baselines_fail_known_failures_do_not() {
        let fixture = Fixture {
            bin: "f".into(),
            rule: vec![
                rule(&["Styles>::new"], &[], Some("R7.S04")), // known failure: reported only
                rule(&["twine_theme::mono::"], &[], None),    // passes
            ],
        };
        let v = judge(&fixture, &syms());
        assert!(v.failures.is_empty(), "{:?}", v.failures);
        assert_eq!(v.known, 1);
        assert!(
            v.report.contains("baseline") && v.report.contains("6042 B"),
            "{}",
            v.report
        );

        let fixed = Fixture {
            bin: "f".into(),
            rule: vec![rule(&["twine_theme::mono::"], &[], Some("R7.S04"))],
        };
        let failures = judge(&fixed, &syms()).failures;
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("remove its `until"), "{failures:?}");

        let regressed = Fixture {
            bin: "f".into(),
            rule: vec![rule(&["montserrat_20"], &[], None)],
        };
        let failures = judge(&regressed, &syms()).failures;
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("11105 B"), "{failures:?}");
    }

    #[test]
    fn fixtures_file_parses_and_names_existing_binaries() {
        let dir = workspace_root().join(FIXTURE_DIR);
        let f = read_fixtures(&dir).unwrap();
        assert!(!f.fixture.is_empty());
        for fx in &f.fixture {
            assert!(
                dir.join("src/bin").join(format!("{}.rs", fx.bin)).is_file(),
                "{}: no src/bin/{}.rs",
                fx.bin,
                fx.bin
            );
            assert!(!fx.rule.is_empty(), "{}: no rules", fx.bin);
            for r in &fx.rule {
                assert!(!r.forbid.is_empty(), "{}: `{}` forbids nothing", fx.bin, r.item);
            }
        }
    }
}
