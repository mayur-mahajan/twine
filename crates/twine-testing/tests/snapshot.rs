//! Snapshot comparison. Each test uses its own temporary snapshot directory; mismatch
//! artefacts go to `target/twine-snapshots/twine-testing-selftest/` and are kept for inspection.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use twine_testing::png_io::read_rgb_png;
use twine_testing::snapshot::{SnapshotConfig, Tolerance, assert_rgb_snapshot};

/// A unique temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("twine-snapshot-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfg(dir: &TempDir, ci: bool, update: bool) -> SnapshotConfig {
    SnapshotConfig {
        dir: dir.0.join("snapshots"),
        crate_name: "twine-testing-selftest".into(),
        ci: Some(ci),
        update: Some(update),
    }
}

/// A 4×3 gradient image.
fn image() -> Vec<u8> {
    (0..4 * 3)
        .flat_map(|i| [(i * 20) as u8, 100, 255 - (i * 20) as u8])
        .collect()
}

fn panic_message(e: &Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default()
}

#[test]
fn writes_missing_snapshot_locally() {
    let dir = TempDir::new("missing");
    let c = cfg(&dir, false, false);
    assert_rgb_snapshot(&c, "new_one", 4, 3, &image(), Tolerance::EXACT);
    let written = read_rgb_png(&c.path("new_one")).unwrap();
    assert_eq!((written.width, written.height), (4, 3));
    assert_eq!(written.data, image());
}

#[test]
fn skips_missing_in_ci_without_writing() {
    let dir = TempDir::new("ci");
    let cfg = cfg(&dir, true, false);
    assert_rgb_snapshot(&cfg, "absent", 4, 3, &image(), Tolerance::EXACT);
    assert!(
        !cfg.path("absent").exists(),
        "CI must not create reference images"
    );
}

#[test]
fn identical_passes() {
    let dir = TempDir::new("identical");
    let c = cfg(&dir, false, false);
    assert_rgb_snapshot(&c, "same", 4, 3, &image(), Tolerance::EXACT);
    let c = cfg(&dir, true, false); // reference now exists, CI does not matter
    assert_rgb_snapshot(&c, "same", 4, 3, &image(), Tolerance::EXACT);
}

#[test]
fn one_pixel_diff_fails_and_writes_artifacts() {
    let dir = TempDir::new("diff");
    let c = cfg(&dir, false, false);
    assert_rgb_snapshot(&c, "one_px", 4, 3, &image(), Tolerance::EXACT);
    let mut changed = image();
    let px = (4 + 2) * 3; // pixel (2, 1)
    changed[px] = changed[px].wrapping_add(1);
    let err = catch_unwind(AssertUnwindSafe(|| {
        assert_rgb_snapshot(&c, "one_px", 4, 3, &changed, Tolerance::EXACT);
    }))
    .expect_err("must fail");
    let msg = panic_message(&err);
    assert!(msg.contains("1 pixel(s) differ"), "{msg}");
    assert!(msg.contains("[2,1 .. 3,2)"), "{msg}");
    let actual = c.artifacts_dir().join("one_px.actual.png");
    let diff = c.artifacts_dir().join("one_px.diff.png");
    assert!(msg.contains(&actual.display().to_string()), "{msg}");
    assert_eq!(read_rgb_png(&actual).unwrap().data, changed);
    let d = read_rgb_png(&diff).unwrap();
    assert_eq!(&d.data[px..px + 3], &[255, 0, 255]);
    assert_ne!(&d.data[0..3], &[255, 0, 255]);
}

#[test]
fn size_mismatch_fails() {
    let dir = TempDir::new("size");
    let c = cfg(&dir, false, false);
    assert_rgb_snapshot(&c, "sized", 4, 3, &image(), Tolerance::EXACT);
    let err = catch_unwind(AssertUnwindSafe(|| {
        assert_rgb_snapshot(&c, "sized", 3, 4, &image(), Tolerance::EXACT);
    }))
    .expect_err("must fail");
    assert!(panic_message(&err).contains("size mismatch"));
}

#[test]
fn tolerance_allows_small_diff() {
    let dir = TempDir::new("tolerance");
    let c = cfg(&dir, false, false);
    assert_rgb_snapshot(&c, "tol", 4, 3, &image(), Tolerance::EXACT);
    let mut changed = image();
    changed[0] = changed[0].wrapping_add(2); // small delta on pixel 0
    changed[3 * 5 + 1] = 0; // big delta on pixel 5
    assert_rgb_snapshot(&c, "tol", 4, 3, &changed, Tolerance::new(1, 2));
    let err = catch_unwind(AssertUnwindSafe(|| {
        assert_rgb_snapshot(&c, "tol", 4, 3, &changed, Tolerance::new(1, 1));
    }));
    assert!(err.is_err(), "two pixels exceed a channel delta of 1");
}

#[test]
fn update_env_overwrites() {
    let dir = TempDir::new("update");
    assert_rgb_snapshot(&cfg(&dir, false, false), "upd", 4, 3, &image(), Tolerance::EXACT);
    let other = vec![7u8; 4 * 3 * 3];
    let c = cfg(&dir, true, true);
    assert_rgb_snapshot(&c, "upd", 4, 3, &other, Tolerance::EXACT);
    assert_eq!(read_rgb_png(&c.path("upd")).unwrap().data, other);
}

#[test]
#[should_panic(expected = "invalid snapshot name")]
fn invalid_name_panics() {
    let dir = TempDir::new("name");
    assert_rgb_snapshot(
        &cfg(&dir, false, false),
        "Bad-Name",
        4,
        3,
        &image(),
        Tolerance::EXACT,
    );
}
