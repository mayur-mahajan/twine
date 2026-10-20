//! Snapshot comparison against the hash manifest. Each test uses its own temporary manifest
//! and reference cache; artefacts and records go to
//! `target/twine-snapshots/twine-testing-selftest-<tag>/` (one directory per test, since tests
//! run in parallel) and are kept for inspection.
//!
//! req: SAF-023

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use twine_testing::png_io::read_rgb_png;
use twine_testing::snapshot::{
    Apply, Digest, Manifest, Outcome, Record, SnapshotConfig, SnapshotMode, Tolerance, apply_records,
    assert_rgb_snapshot, read_records,
};

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

/// A configuration whose artefact directory is unique to `tag` (cleared first).
fn cfg(dir: &TempDir, tag: &str, ci: bool, mode: SnapshotMode) -> SnapshotConfig {
    SnapshotConfig {
        dir: dir.0.join("snapshots"),
        manifest: dir.0.join("snapshots.toml"),
        crate_name: format!("twine-testing-selftest-{tag}"),
        ci: Some(ci),
        mode: Some(mode),
    }
}

/// Clears the artefacts of `tag` (call once per test, before the first assertion).
fn fresh(c: SnapshotConfig) -> SnapshotConfig {
    let _ = std::fs::remove_dir_all(c.artifacts_dir());
    c
}

/// A 4×3 gradient image.
fn image() -> Vec<u8> {
    (0..4 * 3)
        .flat_map(|i| [(i * 20) as u8, 100, 255 - (i * 20) as u8])
        .collect()
}

/// Approves `img` as `name` (the manifest only; no cached PNG).
fn approve(c: &SnapshotConfig, name: &str, img: &[u8]) {
    let mut m = Manifest::load(&c.manifest).unwrap();
    m.approved.insert(name.into(), Digest::of_rgb(4, 3, img));
    m.save(&c.manifest).unwrap();
}

fn panic_message(e: &Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default()
}

/// The records of `c`'s manifest.
fn records(c: &SnapshotConfig) -> Vec<Record> {
    let all = c
        .artifacts_dir()
        .parent()
        .map(|p| read_records(p, &[]))
        .unwrap_or_default();
    all.into_iter().filter(|r| r.manifest == c.manifest).collect()
}

fn outcomes(c: &SnapshotConfig) -> Vec<(String, Outcome)> {
    records(c).into_iter().map(|r| (r.name, r.outcome)).collect()
}

#[test]
fn matching_digest_passes_and_fills_the_local_cache() {
    let dir = TempDir::new("match");
    let c = fresh(cfg(&dir, "match", false, SnapshotMode::Check));
    approve(&c, "same", &image());
    assert_rgb_snapshot(&c, "same", 4, 3, &image(), Tolerance::EXACT);
    assert_eq!(read_rgb_png(&c.path("same")).unwrap().data, image());
    assert_eq!(outcomes(&c), vec![("same".to_string(), Outcome::Match)]);
}

#[test]
fn ci_passes_from_the_manifest_alone_and_writes_no_cache() {
    let dir = TempDir::new("ci");
    let c = fresh(cfg(&dir, "ci", true, SnapshotMode::Check));
    approve(&c, "same", &image());
    assert_rgb_snapshot(&c, "same", 4, 3, &image(), Tolerance::EXACT);
    assert!(!c.path("same").exists(), "CI does not write reference images");
}

#[test]
fn missing_entry_fails_and_writes_the_new_image() {
    let dir = TempDir::new("new");
    let c = fresh(cfg(&dir, "new", false, SnapshotMode::Check));
    let err = catch_unwind(AssertUnwindSafe(|| {
        assert_rgb_snapshot(&c, "brand_new", 4, 3, &image(), Tolerance::EXACT);
    }))
    .expect_err("an image without an entry must fail");
    assert!(
        panic_message(&err).contains("no entry"),
        "{}",
        panic_message(&err)
    );
    let new = c.artifacts_dir().join("new/brand_new.png");
    assert_eq!(read_rgb_png(&new).unwrap().data, image());
    assert!(!c.manifest.exists(), "a test never writes the manifest");
    assert_eq!(outcomes(&c), vec![("brand_new".to_string(), Outcome::New)]);
}

#[test]
fn propose_records_new_images_as_pending_entries() {
    let dir = TempDir::new("propose");
    let c = fresh(cfg(&dir, "propose", false, SnapshotMode::Propose));
    assert_rgb_snapshot(&c, "proposed", 4, 3, &image(), Tolerance::EXACT);
    let report = apply_records(&records(&c), Apply::Propose).unwrap();
    assert_eq!(report.added.len(), 1);
    let m = Manifest::load(&c.manifest).unwrap();
    assert!(m.approved.is_empty());
    assert_eq!(m.pending.get("proposed"), Some(&Digest::of_rgb(4, 3, &image())));
    // A pending entry is compared like an approved one.
    let check = cfg(&dir, "propose", false, SnapshotMode::Check);
    assert_rgb_snapshot(&check, "proposed", 4, 3, &image(), Tolerance::EXACT);
    let mut changed = image();
    changed[0] ^= 1;
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            assert_rgb_snapshot(&check, "proposed", 4, 3, &changed, Tolerance::EXACT);
        }))
        .is_err()
    );
}

#[test]
fn one_pixel_diff_fails_and_writes_artifacts() {
    let dir = TempDir::new("diff");
    let c = fresh(cfg(&dir, "diff", false, SnapshotMode::Check));
    approve(&c, "one_px", &image());
    assert_rgb_snapshot(&c, "one_px", 4, 3, &image(), Tolerance::EXACT); // caches the PNG
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
fn mismatch_without_cache_fails_with_the_actual_image() {
    let dir = TempDir::new("nocache");
    let c = fresh(cfg(&dir, "nocache", true, SnapshotMode::Check));
    approve(&c, "img", &image());
    let other = vec![7u8; 4 * 3 * 3];
    let err = catch_unwind(AssertUnwindSafe(|| {
        assert_rgb_snapshot(&c, "img", 4, 3, &other, Tolerance::EXACT);
    }))
    .expect_err("must fail");
    assert!(
        panic_message(&err).contains("no cached"),
        "{}",
        panic_message(&err)
    );
    assert_eq!(
        read_rgb_png(&c.artifacts_dir().join("img.actual.png"))
            .unwrap()
            .data,
        other
    );
}

#[test]
fn size_mismatch_fails() {
    let dir = TempDir::new("size");
    let c = fresh(cfg(&dir, "size", false, SnapshotMode::Check));
    approve(&c, "sized", &image());
    assert_rgb_snapshot(&c, "sized", 4, 3, &image(), Tolerance::EXACT);
    let err = catch_unwind(AssertUnwindSafe(|| {
        assert_rgb_snapshot(&c, "sized", 3, 4, &image(), Tolerance::EXACT);
    }))
    .expect_err("must fail");
    assert!(panic_message(&err).contains("size mismatch"));
}

#[test]
fn tolerance_is_applied_against_the_cached_reference() {
    let dir = TempDir::new("tolerance");
    let c = fresh(cfg(&dir, "tolerance", false, SnapshotMode::Check));
    approve(&c, "tol", &image());
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
fn tolerant_snapshot_without_cache_is_unchecked() {
    let dir = TempDir::new("unchecked");
    let c = fresh(cfg(&dir, "unchecked", true, SnapshotMode::Check));
    approve(&c, "float", &image());
    let mut changed = image();
    changed[0] ^= 4;
    assert_rgb_snapshot(&c, "float", 4, 3, &changed, Tolerance::new(1, 8));
    assert_eq!(outcomes(&c), vec![("float".to_string(), Outcome::Unchecked)]);
}

#[test]
fn update_records_and_caches_and_apply_approves() {
    let dir = TempDir::new("update");
    let c = fresh(cfg(&dir, "update", false, SnapshotMode::Update));
    approve(&c, "upd", &image());
    let other = vec![7u8; 4 * 3 * 3];
    assert_rgb_snapshot(&c, "upd", 4, 3, &other, Tolerance::EXACT);
    assert_eq!(read_rgb_png(&c.path("upd")).unwrap().data, other);
    // The test itself leaves the manifest alone; applying the records approves the image.
    assert_eq!(
        Manifest::load(&c.manifest).unwrap().approved["upd"],
        Digest::of_rgb(4, 3, &image())
    );
    let report = apply_records(&records(&c), Apply::Update).unwrap();
    assert_eq!(report.changed.len(), 1);
    assert_eq!(
        Manifest::load(&c.manifest).unwrap().approved["upd"],
        Digest::of_rgb(4, 3, &other)
    );
}

#[test]
#[should_panic(expected = "invalid snapshot name")]
fn invalid_name_panics() {
    let dir = TempDir::new("name");
    assert_rgb_snapshot(
        &cfg(&dir, "name", false, SnapshotMode::Check),
        "Bad-Name",
        4,
        3,
        &image(),
        Tolerance::EXACT,
    );
}
