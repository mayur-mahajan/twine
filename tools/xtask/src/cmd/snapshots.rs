//! `cargo xtask snapshots [--update]`: runs the snapshot tests (all workspace tests).
//!
//! With `--update`, `TWINE_UPDATE_SNAPSHOTS=1` makes snapshot assertions rewrite the reference
//! PNGs; review the diffs first.

use std::path::{Path, PathBuf};

use crate::util::{R, cargo, run as run_cmd, workspace_root};

fn artefacts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() {
            artefacts(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Artefact directory of `twine-testing`'s own snapshot self-tests (their mismatches are
/// intentional), left out of the listing.
const SELFTEST_DIR: &str = "twine-testing-selftest";

/// Runs the tests; afterwards lists mismatch artefacts under `target/twine-snapshots/`
/// (the directory is cleared first, so only this run's artefacts are shown).
pub fn run(update: bool) -> R {
    let dir = workspace_root().join("target/twine-snapshots");
    let _ = std::fs::remove_dir_all(&dir);
    let mut cmd = cargo();
    cmd.args(["test", "--workspace"]);
    if update {
        cmd.env("TWINE_UPDATE_SNAPSHOTS", "1");
    }
    let result = run_cmd(&mut cmd);
    let mut files = Vec::new();
    artefacts(&dir, &mut files);
    files.retain(|f| !f.starts_with(dir.join(SELFTEST_DIR)));
    files.sort();
    if !files.is_empty() {
        println!("snapshot mismatch artefacts:");
        for f in &files {
            println!("  {}", f.display());
        }
    }
    result
}
