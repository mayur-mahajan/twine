//! `cargo xtask snapshots [--update [--prune] | --propose | --accept | --check]`: the golden
//! images (R5.S08).
//!
//! References are the committed hash manifests `crates/<crate>/tests/snapshots.toml`
//! (`twine_testing::snapshot`): an entry is the size and XXH64 of an image's RGB pixels, in an
//! `[approved]` or a `[pending]` block. Reference PNGs are a local, git-ignored cache.
//!
//! | Command | What it does |
//! |---------|--------------|
//! | `snapshots` | runs the workspace tests (`TWINE_SNAPSHOTS=check`) and reports the run: mismatches with their artefacts, images without an entry (`target/twine-snapshots/<crate>/new/`), pending entries, entries no test used |
//! | `--update` | the user-approved way to change references: runs with `TWINE_SNAPSHOTS=update` and makes every recorded image the approved entry (review the diffs first); `--prune` also removes entries no test used |
//! | `--propose` | runs with `TWINE_SNAPSHOTS=propose` and adds images without an entry to `[pending]` (for review; nothing approved changes) |
//! | `--accept` | approves every pending entry (no test run): after the user reviewed the images |
//! | `--check` | no test run: every manifest parses and has no pending entry (CI) |

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use twine_testing::snapshot::{Apply, Manifest, Outcome, Record, SnapshotMode, apply_records, read_records};

use crate::util::{R, cargo, run as run_cmd, warn, workspace_root};

/// What `cargo xtask snapshots` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Run and compare.
    Check,
    /// Run and approve every recorded image; `prune` removes unused entries.
    Update {
        /// Remove the entries no test used.
        prune: bool,
    },
    /// Run and add new images as pending entries.
    Propose,
    /// Approve the pending entries.
    Accept,
    /// Validate the manifests without running tests.
    Verify,
}

/// Artefact directories of `twine-testing`'s own snapshot self-tests (their mismatches are
/// intentional), left out of the report.
const SELFTEST_PREFIX: &str = "twine-testing-selftest";

/// Every crate manifest of the workspace (`crates/*/tests/snapshots.toml`), sorted.
#[must_use]
pub fn manifests() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(workspace_root().join("crates"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path().join("tests/snapshots.toml"))
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn relative(p: &Path) -> String {
    p.strip_prefix(workspace_root())
        .unwrap_or(p)
        .display()
        .to_string()
}

/// Entries of `manifest` that no record of the run used.
fn unused(manifest: &Path, records: &[Record]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let m = Manifest::load(manifest)?;
    let seen: BTreeSet<&str> = records
        .iter()
        .filter(|r| r.manifest == manifest)
        .map(|r| r.name.as_str())
        .collect();
    Ok(m.approved
        .keys()
        .chain(m.pending.keys())
        .filter(|k| !seen.contains(k.as_str()))
        .cloned()
        .collect())
}

/// Whether `f` (under `dir`) belongs to a self-test artefact directory.
fn is_selftest(dir: &Path, f: &Path) -> bool {
    f.strip_prefix(dir)
        .ok()
        .and_then(|r| r.components().next())
        .is_some_and(|c| c.as_os_str().to_string_lossy().starts_with(SELFTEST_PREFIX))
}

/// Manifest entries no test used, per manifest.
type Unused = Vec<(PathBuf, Vec<String>)>;

/// Prints the run's summary; returns the unused entries per manifest.
fn report(dir: &Path, records: &[Record]) -> Result<Unused, Box<dyn std::error::Error>> {
    let count = |o: Outcome| records.iter().filter(|r| r.outcome == o).count();
    println!(
        "\nsnapshots: {} checked — {} match, {} pending, {} new, {} mismatch, {} tolerated, {} unchecked, {} updated",
        records.len(),
        count(Outcome::Match),
        count(Outcome::Pending),
        count(Outcome::New),
        count(Outcome::Mismatch),
        count(Outcome::Tolerated),
        count(Outcome::Unchecked),
        count(Outcome::Updated),
    );
    let mut artefacts = Vec::new();
    files_under(dir, &mut artefacts);
    artefacts.retain(|f| f.extension().is_some_and(|e| e == "png") && !is_selftest(dir, f));
    artefacts.sort();
    if !artefacts.is_empty() {
        println!("review images ({}):", artefacts.len());
        for f in &artefacts {
            println!("  {}", relative(f));
        }
    }
    for o in [Outcome::Pending, Outcome::Unchecked] {
        let names: Vec<&str> = records
            .iter()
            .filter(|r| r.outcome == o)
            .map(|r| r.name.as_str())
            .collect();
        if !names.is_empty() {
            println!("{} ({}): {}", o.as_str(), names.len(), names.join(", "));
        }
    }
    let mut all_unused = Vec::new();
    for m in manifests() {
        let u = unused(&m, records)?;
        if !u.is_empty() {
            println!(
                "unused entries in {} ({}; a feature-gated test may not have run): {}",
                relative(&m),
                u.len(),
                u.join(", ")
            );
            all_unused.push((m, u));
        }
    }
    Ok(all_unused)
}

/// Every manifest parses and has no pending entry.
///
/// # Errors
/// A malformed manifest, or pending entries (listed).
pub fn verify() -> R {
    let mut pending = Vec::new();
    let mut total = 0;
    for path in manifests() {
        let m = Manifest::load(&path)?;
        total += m.approved.len();
        for name in m.pending.keys() {
            pending.push(format!("{}: {name}", relative(&path)));
        }
    }
    println!("snapshot manifests: {total} approved entries");
    if pending.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} golden image(s) await approval (review them with `cargo xtask snapshots`, then \
             `cargo xtask snapshots --accept`):\n  {}",
            pending.len(),
            pending.join("\n  ")
        )
        .into())
    }
}

/// Approves every pending entry.
fn accept() -> R {
    for path in manifests() {
        let mut m = Manifest::load(&path)?;
        let names = m.accept_pending();
        if !names.is_empty() {
            m.save(&path)?;
            println!("{}: approved {} entr(y/ies)", relative(&path), names.len());
        }
    }
    Ok(())
}

/// Runs `action` (see the module docs).
pub fn run(action: Action) -> R {
    let mode = match action {
        Action::Accept => return accept(),
        Action::Verify => return verify(),
        Action::Update { .. } => SnapshotMode::Update,
        Action::Propose => SnapshotMode::Propose,
        Action::Check => SnapshotMode::Check,
    };
    let dir = workspace_root().join("target/twine-snapshots");
    let _ = std::fs::remove_dir_all(&dir);
    let mut cmd = cargo();
    // Every crate's tests run even when an earlier one has a mismatch: the report lists all.
    cmd.args(["test", "--workspace", "--no-fail-fast"])
        .env("TWINE_SNAPSHOTS", mode.as_str());
    let result = run_cmd(&mut cmd);
    let records = read_records(&dir, &[SELFTEST_PREFIX]);
    let unused = report(&dir, &records)?;
    match action {
        Action::Update { prune } => {
            let r = apply_records(&records, Apply::Update)?;
            println!(
                "approved: {} added, {} changed, {} manifest(s) written",
                r.added.len(),
                r.changed.len(),
                r.written.len()
            );
            for (m, name) in &r.changed {
                println!("  changed {}: {name}", relative(m));
            }
            if prune {
                for (path, names) in unused {
                    let mut m = Manifest::load(&path)?;
                    for n in &names {
                        m.approved.remove(n);
                        m.pending.remove(n);
                    }
                    m.save(&path)?;
                    println!("pruned {}: {}", relative(&path), names.join(", "));
                }
            }
            if result.is_err() {
                warn("some tests failed; the recorded images were approved anyway (review the failures)");
            }
        }
        Action::Propose => {
            let r = apply_records(&records, Apply::Propose)?;
            println!("proposed {} pending entr(y/ies):", r.added.len());
            for (m, name) in &r.added {
                println!("  {}: {name}", relative(m));
            }
        }
        Action::Check | Action::Accept | Action::Verify => {}
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workspace_manifest_parses() {
        let all = manifests();
        assert!(!all.is_empty(), "the workspace has golden-image manifests");
        for m in all {
            Manifest::load(&m).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn selftest_artefacts_are_recognised() {
        let dir = Path::new("/t/twine-snapshots");
        assert!(is_selftest(dir, &dir.join("twine-testing-selftest-diff/a.png")));
        assert!(!is_selftest(dir, &dir.join("twine-widgets/new/a.png")));
    }
}
