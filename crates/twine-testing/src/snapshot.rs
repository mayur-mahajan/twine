//! Pixel-exact PNG snapshots: [`assert_rgb_snapshot`], [`SnapshotConfig`], [`Tolerance`].
//!
//! Reference images live in `<crate>/tests/snapshots/<name>.png` (8-bit RGB).
//!
//! | Situation | Behaviour |
//! |-----------|-----------|
//! | `TWINE_UPDATE_SNAPSHOTS=1` (`cargo xtask snapshots --update`) | the reference is (re)written, the assertion passes |
//! | reference missing, `CI` set | skipped: nothing is written, `SKIPPED SNAPSHOT <path> (no reference in CI)` is printed to stderr, the assertion passes (reference images are kept out of version control, so CI has none to compare against) |
//! | reference missing locally | the reference is written, `NEW SNAPSHOT <path>` is printed to stderr, the assertion passes |
//! | size differs | panic |
//! | more than `max_diff_px` pixels differ by more than `max_channel_delta` in some channel | `<name>.actual.png` and `<name>.diff.png` are written to `target/twine-snapshots/<crate>/` and the assertion panics with both paths, the count and the bounding box of the differences |
//!
//! In the diff image unchanged pixels are shown at 25 % brightness gray and changed pixels in
//! solid magenta.

use std::path::{Path, PathBuf};

use crate::png_io::{read_rgb_png, write_rgb_png};

/// Where a crate's snapshots live and how the update/CI switches are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotConfig {
    /// Directory of the reference PNGs (usually `<manifest>/tests/snapshots`).
    pub dir: PathBuf,
    /// Crate name, used for the artefact directory `target/twine-snapshots/<crate_name>/`.
    pub crate_name: String,
    /// Override of the `CI` environment variable (`None` = read the environment).
    pub ci: Option<bool>,
    /// Override of `TWINE_UPDATE_SNAPSHOTS` (`None` = read the environment).
    pub update: Option<bool>,
}

impl SnapshotConfig {
    /// The configuration of the crate at `manifest_dir`: `dir = <manifest_dir>/tests/snapshots`,
    /// switches read from the environment. Usually created with [`snapshot_config!`](crate::snapshot_config).
    #[must_use]
    pub fn for_crate(manifest_dir: &str, crate_name: &str) -> Self {
        Self {
            dir: Path::new(manifest_dir).join("tests").join("snapshots"),
            crate_name: crate_name.to_string(),
            ci: None,
            update: None,
        }
    }

    /// Whether the run is in CI (override, else `CI` is set to anything but `""`, `0`, `false`).
    #[must_use]
    pub fn is_ci(&self) -> bool {
        self.ci.unwrap_or_else(|| env_flag("CI"))
    }

    /// Whether references should be rewritten (override, else `TWINE_UPDATE_SNAPSHOTS`).
    #[must_use]
    pub fn is_update(&self) -> bool {
        self.update.unwrap_or_else(|| env_flag("TWINE_UPDATE_SNAPSHOTS"))
    }

    /// The reference path of snapshot `name`.
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
/// ```
#[macro_export]
macro_rules! snapshot_config {
    () => {
        $crate::snapshot::SnapshotConfig::for_crate(env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"))
    };
}

/// Allowed deviation of an image from its reference.
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

fn valid_name(name: &str) -> bool {
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

/// Compares the `w × h` 8-bit RGB image `rgb` with the reference snapshot `name` of `cfg`
/// (see the [module docs](self) for the full behaviour).
///
/// ```
/// use twine_testing::snapshot::{assert_rgb_snapshot, SnapshotConfig, Tolerance};
///
/// let dir = std::env::temp_dir().join(format!("twine-snapshot-doc-{}", std::process::id()));
/// let cfg = SnapshotConfig { dir: dir.clone(), crate_name: "doc".into(), ci: Some(false), update: Some(false) };
/// let img = [255u8, 0, 0, 0, 255, 0];
/// assert_rgb_snapshot(&cfg, "two_px", 2, 1, &img, Tolerance::EXACT); // written (new)
/// assert_rgb_snapshot(&cfg, "two_px", 2, 1, &img, Tolerance::EXACT); // compared
/// std::fs::remove_dir_all(dir).unwrap();
/// ```
///
/// # Panics
/// On an invalid name (must match `^[a-z0-9_]+$`), a size mismatch,
/// too many differing pixels, or I/O errors.
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
    let path = cfg.path(name);
    if cfg.is_update() {
        write_rgb_png(&path, w, h, rgb)
            .unwrap_or_else(|e| panic!("cannot write snapshot {}: {e}", path.display()));
        eprintln!("UPDATED SNAPSHOT {}", path.display());
        return;
    }
    if !path.exists() {
        if cfg.is_ci() {
            eprintln!("SKIPPED SNAPSHOT {} (no reference in CI)", path.display());
            return;
        }
        write_rgb_png(&path, w, h, rgb)
            .unwrap_or_else(|e| panic!("cannot write snapshot {}: {e}", path.display()));
        eprintln!("NEW SNAPSHOT {}", path.display());
        return;
    }
    let reference =
        read_rgb_png(&path).unwrap_or_else(|e| panic!("cannot read snapshot {}: {e}", path.display()));
    let artifacts = cfg.artifacts_dir();
    let actual_path = artifacts.join(format!("{name}.actual.png"));
    let diff_path = artifacts.join(format!("{name}.diff.png"));
    if (reference.width, reference.height) != (w, h) {
        let _ = write_rgb_png(&actual_path, w, h, rgb);
        panic!(
            "snapshot {name}: size mismatch: reference {}x{}, actual {w}x{h}\n  reference: {}\n  actual:    {}",
            reference.width,
            reference.height,
            path.display(),
            actual_path.display()
        );
    }
    let diff = diff_rgb(w, h, &reference.data, rgb, tol.max_channel_delta);
    if diff.count > tol.max_diff_px {
        write_rgb_png(&actual_path, w, h, rgb)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", actual_path.display()));
        write_rgb_png(&diff_path, w, h, &diff.image)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", diff_path.display()));
        let (x0, y0, x1, y1) = diff.bbox.unwrap_or_default();
        panic!(
            "snapshot {name}: {} pixel(s) differ (allowed {}, channel delta > {}), bounding box \
             [{x0},{y0} .. {x1},{y1})\n  reference: {}\n  actual:    {}\n  diff:      {}",
            diff.count,
            tol.max_diff_px,
            tol.max_channel_delta,
            path.display(),
            actual_path.display(),
            diff_path.display()
        );
    }
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
