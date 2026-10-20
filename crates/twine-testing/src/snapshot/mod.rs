//! Golden-image snapshots: [`assert_rgb_snapshot`], [`SnapshotConfig`], [`Tolerance`], and the
//! committed hash manifest ([`Manifest`], [`Digest`]).
//!
//! A crate's references are its **manifest** `tests/snapshots.toml` (committed): one
//! [`Digest`] per snapshot — size and XXH64 of the decoded 8-bit RGB pixels ([`xxh64`]). A test
//! renders, hashes and compares with the manifest; no image is needed to pass. PNGs exist only
//! for review: `tests/snapshots/<name>.png` is a **local, git-ignored cache** of the approved
//! image (written by update runs and by passing runs outside CI when missing), and
//! `target/twine-snapshots/<crate>/` receives the images of a mismatch or a new snapshot.
//!
//! | Situation | Behaviour |
//! |-----------|-----------|
//! | digest equals the `[approved]` entry | passes |
//! | digest equals a `[pending]` entry | passes, `PENDING SNAPSHOT <name>` on stderr (awaiting review) |
//! | no entry | the image goes to `target/twine-snapshots/<crate>/new/<name>.png`; fails, except in a `propose` or `update` run |
//! | digest differs | `<name>.actual.png` (and, with the cached reference PNG, `<name>.diff.png` plus the count and bounding box of the differing pixels) go to `target/twine-snapshots/<crate>/`; fails — unless the snapshot has a non-exact [`Tolerance`] and the difference from the cached reference is within it |
//! | digest differs, tolerant snapshot, no cached reference | not checked (`UNCHECKED SNAPSHOT`); only exact snapshots are evidence |
//! | `update` run | the image is recorded (and cached); `cargo xtask snapshots --update` makes it the approved entry |
//!
//! Every assertion leaves a [`Record`] in `target/twine-snapshots/<crate>/records/`, so
//! `cargo xtask snapshots` can report the run (matches, pending, new, mismatches, entries no test
//! used) and change the manifests: `--update` approves every recorded image (the user's
//! decision after reviewing the diffs), `--propose` adds new images as pending entries, and
//! `--accept` approves the pending ones. Tests never write a manifest themselves.
//!
//! In the diff image unchanged pixels are shown at 25 % brightness gray and changed pixels in
//! solid magenta.

mod manifest;
mod xxh64;

pub use manifest::{
    Apply, ApplyReport, Approval, Digest, Manifest, Outcome, ParseError, Record, apply_records, read_records,
};
pub use xxh64::xxh64;

use std::path::{Path, PathBuf};

use crate::png_io::{read_rgb_png, write_rgb_png};

/// How a run treats the manifest (environment variable `TWINE_SNAPSHOTS`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SnapshotMode {
    /// Compare; a new or changed image fails (`TWINE_SNAPSHOTS` unset or `check`).
    #[default]
    Check,
    /// Like `Check`, but a snapshot without an entry passes and is recorded for
    /// `cargo xtask snapshots --propose` (`TWINE_SNAPSHOTS=propose`).
    Propose,
    /// Record every image for `cargo xtask snapshots --update` and refresh the local reference
    /// PNGs; nothing fails (`TWINE_SNAPSHOTS=update`).
    Update,
}

impl SnapshotMode {
    /// The mode named by `TWINE_SNAPSHOTS` (`check`, `propose`, `update`; anything else or unset
    /// is `Check`).
    ///
    /// ```
    /// use twine_testing::snapshot::SnapshotMode;
    /// assert_eq!(SnapshotMode::parse("update"), SnapshotMode::Update);
    /// assert_eq!(SnapshotMode::parse(""), SnapshotMode::Check);
    /// ```
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var("TWINE_SNAPSHOTS").map_or(Self::Check, |v| Self::parse(&v))
    }

    /// Parses a mode name (unknown names are `Check`).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "update" => Self::Update,
            "propose" => Self::Propose,
            _ => Self::Check,
        }
    }

    /// The value of `TWINE_SNAPSHOTS` selecting this mode.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Propose => "propose",
            Self::Update => "update",
        }
    }
}

/// Where a crate's manifest, reference cache and artefacts are, and how the run behaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotConfig {
    /// Directory of the local reference PNG cache (usually `<manifest>/tests/snapshots`).
    pub dir: PathBuf,
    /// The committed manifest (usually `<manifest>/tests/snapshots.toml`).
    pub manifest: PathBuf,
    /// Crate name, used for the artefact directory `target/twine-snapshots/<crate_name>/`.
    pub crate_name: String,
    /// Override of the `CI` environment variable (`None` = read the environment).
    pub ci: Option<bool>,
    /// Override of `TWINE_SNAPSHOTS` (`None` = read the environment).
    pub mode: Option<SnapshotMode>,
}

impl SnapshotConfig {
    /// The configuration of the crate at `manifest_dir`: cache `<manifest_dir>/tests/snapshots`,
    /// manifest `<manifest_dir>/tests/snapshots.toml`, switches read from the environment.
    /// Usually created with [`snapshot_config!`](crate::snapshot_config).
    #[must_use]
    pub fn for_crate(manifest_dir: &str, crate_name: &str) -> Self {
        let tests = Path::new(manifest_dir).join("tests");
        Self {
            dir: tests.join("snapshots"),
            manifest: tests.join("snapshots.toml"),
            crate_name: crate_name.to_string(),
            ci: None,
            mode: None,
        }
    }

    /// Whether the run is in CI (override, else `CI` is set to anything but `""`, `0`, `false`).
    #[must_use]
    pub fn is_ci(&self) -> bool {
        self.ci.unwrap_or_else(|| env_flag("CI"))
    }

    /// The run's mode (override, else [`SnapshotMode::from_env`]).
    #[must_use]
    pub fn mode(&self) -> SnapshotMode {
        self.mode.unwrap_or_else(SnapshotMode::from_env)
    }

    /// The cached reference PNG of snapshot `name`.
    #[must_use]
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.png"))
    }

    /// The directory mismatch artefacts are written to: `<target>/twine-snapshots/<crate_name>`.
    ///
    /// `<target>` is `CARGO_TARGET_DIR`, else `target/` next to the `Cargo.lock` found by walking
    /// up from [`dir`](Self::dir) (or, if there is none, from this crate's manifest directory).
    #[must_use]
    pub fn artifacts_dir(&self) -> PathBuf {
        target_dir(&self.dir)
            .join("twine-snapshots")
            .join(&self.crate_name)
    }
}

/// The [`SnapshotConfig`] of the calling crate (`CARGO_MANIFEST_DIR`, `CARGO_PKG_NAME`).
///
/// ```
/// let cfg = twine_testing::snapshot_config!();
/// assert!(cfg.dir.ends_with("tests/snapshots"));
/// assert!(cfg.manifest.ends_with("tests/snapshots.toml"));
/// ```
#[macro_export]
macro_rules! snapshot_config {
    () => {
        $crate::snapshot::SnapshotConfig::for_crate(env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"))
    };
}

/// Allowed deviation of an image from its reference.
///
/// Only [`EXACT`](Self::EXACT) snapshots are evidence: a tolerance is applied against the
/// locally cached reference PNG, so a tolerant snapshot that differs from its manifest entry on
/// a machine without that cache is not checked (see the [module docs](self)). Tolerances exist
/// for the floating-point parts excluded from the safety profile (TrueType, Lottie).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tolerance {
    /// Maximum number of differing pixels.
    pub max_diff_px: u32,
    /// A pixel differs when some channel differs by more than this.
    pub max_channel_delta: u8,
}

impl Tolerance {
    /// Pixel-exact comparison.
    pub const EXACT: Tolerance = Tolerance {
        max_diff_px: 0,
        max_channel_delta: 0,
    };

    /// A tolerance of `max_diff_px` pixels differing by more than `max_channel_delta`.
    #[must_use]
    pub const fn new(max_diff_px: u32, max_channel_delta: u8) -> Self {
        Self {
            max_diff_px,
            max_channel_delta,
        }
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !matches!(v.trim(), "" | "0" | "false" | "FALSE" | "False"))
}

fn find_lock_dir(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join("Cargo.lock").is_file())
        .map(Path::to_path_buf)
}

fn target_dir(from: &Path) -> PathBuf {
    if let Some(t) = std::env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(t);
    }
    find_lock_dir(from)
        .or_else(|| find_lock_dir(Path::new(env!("CARGO_MANIFEST_DIR"))))
        .map_or_else(|| PathBuf::from("target"), |d| d.join("target"))
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Result of comparing two equally sized RGB images.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSummary {
    /// Number of differing pixels.
    pub count: u32,
    /// Bounding box of the differing pixels `(x0, y0, x1, y1)`, half-open.
    pub bbox: Option<(u32, u32, u32, u32)>,
    /// The diff image (25 % gray for equal pixels, magenta for differing ones).
    pub image: Vec<u8>,
}

/// Compares two `w × h` RGB images pixel by pixel.
///
/// ```
/// use twine_testing::snapshot::diff_rgb;
/// let a = [0u8; 2 * 3];
/// let mut b = a;
/// b[3] = 9; // pixel 1, red channel
/// let d = diff_rgb(2, 1, &a, &b, 8);
/// assert_eq!(d.count, 1);
/// assert_eq!(d.bbox, Some((1, 0, 2, 1)));
/// ```
#[must_use]
pub fn diff_rgb(w: u32, h: u32, expected: &[u8], actual: &[u8], max_channel_delta: u8) -> DiffSummary {
    let mut count = 0;
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    let mut image = Vec::with_capacity(expected.len());
    for (i, (e, a)) in expected.chunks_exact(3).zip(actual.chunks_exact(3)).enumerate() {
        let delta = e.iter().zip(a).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        if delta > max_channel_delta {
            count += 1;
            let (x, y) = ((i as u32) % w.max(1), (i as u32) / w.max(1));
            bbox = Some(match bbox {
                None => (x, y, x + 1, y + 1),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)),
            });
            image.extend_from_slice(&[255, 0, 255]);
        } else {
            let g = e[0] / 4 / 3 + e[1] / 4 / 3 + e[2] / 4 / 3;
            image.extend_from_slice(&[g, g, g]);
        }
    }
    debug_assert_eq!(image.len(), w as usize * h as usize * 3);
    DiffSummary { count, bbox, image }
}

/// Compares the `w × h` 8-bit RGB image `rgb` with the manifest entry `name` of `cfg` (see the
/// [module docs](self) for the full behaviour).
///
/// ```
/// use twine_testing::snapshot::{assert_rgb_snapshot, Manifest, SnapshotConfig, SnapshotMode, Tolerance};
///
/// let dir = std::env::temp_dir().join(format!("twine-snapshot-doc-{}", std::process::id()));
/// let cfg = SnapshotConfig {
///     dir: dir.join("snapshots"),
///     manifest: dir.join("snapshots.toml"),
///     crate_name: "doc".into(),
///     ci: Some(false),
///     mode: Some(SnapshotMode::Check),
/// };
/// let img = [255u8, 0, 0, 0, 255, 0];
/// let mut m = Manifest::default();
/// m.approved.insert("two_px".into(), twine_testing::snapshot::Digest::of_rgb(2, 1, &img));
/// m.save(&cfg.manifest).unwrap();
/// assert_rgb_snapshot(&cfg, "two_px", 2, 1, &img, Tolerance::EXACT); // equal to its entry
/// std::fs::remove_dir_all(dir).unwrap();
/// ```
///
/// # Panics
/// On an invalid name (must match `^[a-z0-9_]+$`), an `rgb` of the wrong length, an unreadable
/// manifest, a missing entry (`Check` mode), a mismatch, or I/O errors writing artefacts.
#[track_caller]
pub fn assert_rgb_snapshot(cfg: &SnapshotConfig, name: &str, w: u32, h: u32, rgb: &[u8], tol: Tolerance) {
    assert!(
        valid_name(name),
        "invalid snapshot name {name:?}: must match ^[a-z0-9_]+$"
    );
    assert_eq!(
        rgb.len(),
        w as usize * h as usize * 3,
        "snapshot {name}: {w}x{h} RGB image needs {} bytes, got {}",
        w as usize * h as usize * 3,
        rgb.len()
    );
    let digest = Digest::of_rgb(w, h, rgb);
    let artifacts = cfg.artifacts_dir();
    let record = |outcome| {
        let r = Record {
            name: name.to_string(),
            outcome,
            digest,
            manifest: cfg.manifest.clone(),
        };
        r.write(&artifacts.join("records"))
            .unwrap_or_else(|e| panic!("cannot write the record of snapshot {name}: {e}"));
    };
    let write = |path: &Path, data: &[u8]| {
        write_rgb_png(path, w, h, data).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    };
    let mode = cfg.mode();
    if mode == SnapshotMode::Update {
        write(&cfg.path(name), rgb);
        record(Outcome::Updated);
        eprintln!("UPDATED SNAPSHOT {name} {digest}");
        return;
    }
    let manifest =
        Manifest::load(&cfg.manifest).unwrap_or_else(|e| panic!("cannot read the snapshot manifest: {e}"));
    let Some((expected, approval)) = manifest.get(name) else {
        let new = artifacts.join("new").join(format!("{name}.png"));
        write(&new, rgb);
        record(Outcome::New);
        if mode == SnapshotMode::Propose {
            eprintln!(
                "NEW SNAPSHOT {name} {digest} (proposed; review {})",
                new.display()
            );
            return;
        }
        panic!(
            "snapshot {name}: no entry in {}\n  image: {}\n  review it, then run `cargo xtask snapshots --propose` \
             (pending approval) or `cargo xtask snapshots --update` (approve)",
            cfg.manifest.display(),
            new.display()
        );
    };
    if expected == digest {
        if approval == Approval::Pending {
            eprintln!("PENDING SNAPSHOT {name} (awaiting approval)");
            record(Outcome::Pending);
        } else {
            record(Outcome::Match);
        }
        if !cfg.is_ci() && !cfg.path(name).exists() {
            write(&cfg.path(name), rgb);
        }
        return;
    }
    // Mismatch: compare with the cached reference when it is the image of the entry.
    let reference = read_rgb_png(&cfg.path(name))
        .ok()
        .filter(|r| Digest::of_rgb(r.width, r.height, &r.data) == expected);
    let actual_path = artifacts.join(format!("{name}.actual.png"));
    if tol != Tolerance::EXACT && reference.is_none() {
        record(Outcome::Unchecked);
        eprintln!(
            "UNCHECKED SNAPSHOT {name}: differs from its entry; no cached reference to apply the tolerance to"
        );
        return;
    }
    let Some(reference) = reference else {
        write(&actual_path, rgb);
        record(Outcome::Mismatch);
        panic!(
            "snapshot {name}: {digest} differs from the {} entry {expected} in {} (no cached \
             reference PNG for a diff)\n  actual: {}",
            if approval == Approval::Pending {
                "pending"
            } else {
                "approved"
            },
            cfg.manifest.display(),
            actual_path.display()
        );
    };
    if (reference.width, reference.height) != (w, h) {
        write(&actual_path, rgb);
        record(Outcome::Mismatch);
        panic!(
            "snapshot {name}: size mismatch: reference {}x{}, actual {w}x{h}\n  reference: {}\n  actual:    {}",
            reference.width,
            reference.height,
            cfg.path(name).display(),
            actual_path.display()
        );
    }
    let diff = diff_rgb(w, h, &reference.data, rgb, tol.max_channel_delta);
    if diff.count <= tol.max_diff_px {
        record(Outcome::Tolerated);
        return;
    }
    let diff_path = artifacts.join(format!("{name}.diff.png"));
    write(&actual_path, rgb);
    write(&diff_path, &diff.image);
    record(Outcome::Mismatch);
    let (x0, y0, x1, y1) = diff.bbox.unwrap_or_default();
    panic!(
        "snapshot {name}: {} pixel(s) differ (allowed {}, channel delta > {}), bounding box \
         [{x0},{y0} .. {x1},{y1})\n  expected:  {expected}\n  actual:    {digest}\n  reference: {}\n  \
         actual:    {}\n  diff:      {}",
        diff.count,
        tol.max_diff_px,
        tol.max_channel_delta,
        cfg.path(name).display(),
        actual_path.display(),
        diff_path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation() {
        assert!(valid_name("fill_page_2"));
        assert!(!valid_name("Fill"));
        assert!(!valid_name("a-b"));
        assert!(!valid_name(""));
        assert!(!valid_name("../x"));
    }

    #[test]
    fn diff_image_colors() {
        let e = [200u8, 200, 200, 0, 0, 0];
        let a = [200u8, 200, 200, 1, 0, 0];
        let d = diff_rgb(2, 1, &e, &a, 0);
        assert_eq!(d.count, 1);
        assert_eq!(&d.image[..3], &[48, 48, 48]);
        assert_eq!(&d.image[3..], &[255, 0, 255]);
        assert_eq!(diff_rgb(2, 1, &e, &a, 1).count, 0);
    }
}
