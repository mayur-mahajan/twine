//! The golden-image manifest ([`Manifest`], [`Digest`]) and the per-run records
//! ([`Record`], [`apply_records`]) through which `cargo xtask snapshots` changes it.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use super::xxh64::xxh64;

/// The fingerprint of one image: its size and the XXH64 (seed 0) of its 8-bit RGB pixels,
/// row-major, three bytes per pixel — the decoded pixels, never an encoded file, so a PNG
/// encoder change does not change a digest.
///
/// Written as `<width>x<height> xxh64:<16 hex digits>`; `xxhsum -H64` of the raw RGB bytes
/// gives the same hash.
///
/// ```
/// use twine_testing::snapshot::Digest;
/// let d = Digest::of_rgb(1, 1, &[255, 0, 0]);
/// assert_eq!(d.to_string().parse::<Digest>(), Ok(d));
/// assert!(d.to_string().starts_with("1x1 xxh64:"));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Digest {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// XXH64 of the RGB bytes.
    pub hash: u64,
}

impl Digest {
    /// The digest of a `w × h` RGB image (`rgb` holds `w * h * 3` bytes; not checked here —
    /// [`assert_rgb_snapshot`](super::assert_rgb_snapshot) checks it). Never panics.
    #[must_use]
    pub fn of_rgb(width: u32, height: u32, rgb: &[u8]) -> Self {
        Self {
            width,
            height,
            hash: xxh64(rgb, 0),
        }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{} xxh64:{:016x}", self.width, self.height, self.hash)
    }
}

/// Why a [`Digest`] or a [`Manifest`] could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number (0 for a lone digest).
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Digest {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        let bad = |m: &str| ParseError {
            line: 0,
            message: format!("invalid digest {s:?}: {m}"),
        };
        let (size, hash) = s
            .split_once(' ')
            .ok_or_else(|| bad("expected `<w>x<h> xxh64:<hex>`"))?;
        let (w, h) = size.split_once('x').ok_or_else(|| bad("expected `<w>x<h>`"))?;
        let hex = hash
            .strip_prefix("xxh64:")
            .ok_or_else(|| bad("expected `xxh64:`"))?;
        if hex.len() != 16 {
            return Err(bad("expected 16 hex digits"));
        }
        Ok(Self {
            width: w.parse().map_err(|_| bad("bad width"))?,
            height: h.parse().map_err(|_| bad("bad height"))?,
            hash: u64::from_str_radix(hex, 16).map_err(|_| bad("bad hex"))?,
        })
    }
}

/// Which block of a [`Manifest`] an entry is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Approval {
    /// Reviewed and approved by the user (`[approved]`).
    Approved,
    /// Generated, awaiting the user's review (`[pending]`); compared like an approved entry so
    /// it cannot drift, reported by `cargo xtask snapshots` and refused by its `--check`.
    Pending,
}

/// A crate's golden-image manifest, `tests/snapshots.toml`: snapshot name → [`Digest`], in two
/// blocks, `[approved]` and `[pending]`. Committed; the reference PNGs are not (they are a local
/// cache for diff images).
///
/// The file is a small TOML subset (comments, the two section headers, `name = "digest"`
/// lines), written sorted so that a change is a one-line diff.
///
/// ```
/// use twine_testing::snapshot::{Approval, Digest, Manifest};
/// let mut m = Manifest::default();
/// m.approved.insert("ok_button".into(), Digest::of_rgb(1, 1, &[0, 0, 0]));
/// let text = m.to_string();
/// let back: Manifest = text.parse().unwrap();
/// assert_eq!(back, m);
/// assert_eq!(back.get("ok_button").map(|(_, a)| a), Some(Approval::Approved));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Approved entries.
    pub approved: BTreeMap<String, Digest>,
    /// Entries awaiting approval.
    pub pending: BTreeMap<String, Digest>,
}

const HEADER: &str = "\
# Golden-image manifest (R5.S08): one entry per snapshot test image,
#   <name> = \"<width>x<height> xxh64:<hash>\"
# the XXH64 (seed 0) of the image's 8-bit RGB pixels, row-major (`xxhsum -H64` of the raw RGB
# bytes). Written by `cargo xtask snapshots --update | --propose | --accept`; do not edit by hand.
# Reference PNGs are not committed: `tests/snapshots/*.png` is a local cache used for diffs.
";

const PENDING_NOTE: &str = "\
# Awaiting review: generated, not yet approved. Compared like approved entries. The images
# are in target/twine-snapshots/<crate>/new/; `cargo xtask snapshots --accept` approves them.
";

impl Manifest {
    /// The entry `name` and its block (approved first).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<(Digest, Approval)> {
        self.approved
            .get(name)
            .map(|d| (*d, Approval::Approved))
            .or_else(|| self.pending.get(name).map(|d| (*d, Approval::Pending)))
    }

    /// Reads the manifest at `path`; a missing file is an empty manifest.
    ///
    /// # Errors
    /// I/O errors other than "not found", and parse errors (as `InvalidData`).
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => text.parse().map_err(|e: ParseError| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes the manifest to `path` (creating parent directories).
    ///
    /// # Errors
    /// I/O errors.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_string())
    }

    /// Moves every pending entry to the approved block; returns their names.
    pub fn accept_pending(&mut self) -> Vec<String> {
        let names: Vec<String> = self.pending.keys().cloned().collect();
        self.approved.append(&mut self.pending);
        names
    }
}

impl fmt::Display for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(HEADER)?;
        f.write_str("\n[approved]\n")?;
        for (k, v) in &self.approved {
            writeln!(f, "{k} = \"{v}\"")?;
        }
        if !self.pending.is_empty() {
            f.write_str("\n[pending]\n")?;
            f.write_str(PENDING_NOTE)?;
            for (k, v) in &self.pending {
                writeln!(f, "{k} = \"{v}\"")?;
            }
        }
        Ok(())
    }
}

impl FromStr for Manifest {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        let mut m = Manifest::default();
        let mut section: Option<Approval> = None;
        for (i, raw) in s.lines().enumerate() {
            let line_no = i + 1;
            let err = |message: String| ParseError {
                line: line_no,
                message,
            };
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line {
                "[approved]" => {
                    section = Some(Approval::Approved);
                    continue;
                }
                "[pending]" => {
                    section = Some(Approval::Pending);
                    continue;
                }
                _ => {}
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| err(format!("expected `name = \"digest\"`, got {line:?}")))?;
            let key = key.trim();
            if !super::valid_name(key) {
                return Err(err(format!("invalid snapshot name {key:?}")));
            }
            let value = value
                .trim()
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .ok_or_else(|| err("the digest must be quoted".into()))?;
            let digest: Digest = value.parse().map_err(|e: ParseError| err(e.message))?;
            let block = match section {
                Some(Approval::Approved) => &mut m.approved,
                Some(Approval::Pending) => &mut m.pending,
                None => return Err(err("entry before `[approved]` / `[pending]`".into())),
            };
            if block.insert(key.to_string(), digest).is_some() {
                return Err(err(format!("duplicate entry {key:?}")));
            }
        }
        if let Some(k) = m.approved.keys().find(|k| m.pending.contains_key(*k)) {
            return Err(ParseError {
                line: 0,
                message: format!("{k:?} is both approved and pending"),
            });
        }
        Ok(m)
    }
}

/// What one snapshot assertion found (see [`Record`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Equal to its approved entry.
    Match,
    /// Equal to its pending entry.
    Pending,
    /// Not in the manifest (the image is in `new/` of the artefact directory).
    New,
    /// Differs from its entry (the assertion failed).
    Mismatch,
    /// Differs from its entry but within the snapshot's tolerance (checked against the local
    /// reference PNG).
    Tolerated,
    /// A tolerant snapshot that differs from its entry with no local reference PNG to apply the
    /// tolerance to: not checked.
    Unchecked,
    /// Recorded by an update run.
    Updated,
}

impl Outcome {
    const NAMES: [(Outcome, &'static str); 7] = [
        (Outcome::Match, "match"),
        (Outcome::Pending, "pending"),
        (Outcome::New, "new"),
        (Outcome::Mismatch, "mismatch"),
        (Outcome::Tolerated, "tolerated"),
        (Outcome::Unchecked, "unchecked"),
        (Outcome::Updated, "updated"),
    ];

    /// The outcome's word in a record file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(o, _)| *o == self)
            .map_or("match", |(_, s)| s)
    }

    fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().find(|(_, n)| *n == s).map(|(o, _)| *o)
    }
}

/// One snapshot assertion of a test run, written to
/// `<target>/twine-snapshots/<crate>/records/<name>.rec` — one file per snapshot, so tests
/// running in parallel never write the same file. `cargo xtask snapshots` reads them to report
/// the run and, with `--update` / `--propose`, to change the manifests ([`apply_records`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Snapshot name.
    pub name: String,
    /// What the assertion found.
    pub outcome: Outcome,
    /// The rendered image's digest.
    pub digest: Digest,
    /// The manifest the snapshot belongs to.
    pub manifest: PathBuf,
}

impl Record {
    pub(super) fn write(&self, records_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(records_dir)?;
        std::fs::write(
            records_dir.join(format!("{}.rec", self.name)),
            format!(
                "{}\t{}\t{}\n",
                self.outcome.as_str(),
                self.digest,
                self.manifest.display()
            ),
        )
    }

    fn read(path: &Path) -> Option<Self> {
        let name = path.file_stem()?.to_str()?.to_string();
        let text = std::fs::read_to_string(path).ok()?;
        let mut parts = text.trim_end().splitn(3, '\t');
        let outcome = Outcome::parse(parts.next()?)?;
        let digest = parts.next()?.parse().ok()?;
        let manifest = PathBuf::from(parts.next()?);
        Some(Self {
            name,
            outcome,
            digest,
            manifest,
        })
    }
}

/// Every record under `snapshots_dir` (`<target>/twine-snapshots`), skipping the artefact
/// directories whose name starts with one of `skip` (e.g. the snapshot module's own
/// self-tests); sorted by manifest and name. Unreadable files are skipped.
#[must_use]
pub fn read_records(snapshots_dir: &Path, skip: &[&str]) -> Vec<Record> {
    let mut out = Vec::new();
    let Ok(crates) = std::fs::read_dir(snapshots_dir) else {
        return out;
    };
    for c in crates.filter_map(Result::ok) {
        let name = c.file_name();
        if skip
            .iter()
            .any(|s| name.to_str().is_some_and(|n| n.starts_with(s)))
        {
            continue;
        }
        let Ok(files) = std::fs::read_dir(c.path().join("records")) else {
            continue;
        };
        out.extend(
            files
                .filter_map(Result::ok)
                .filter_map(|f| Record::read(&f.path())),
        );
    }
    out.sort_by(|a, b| (&a.manifest, &a.name).cmp(&(&b.manifest, &b.name)));
    out
}

/// How [`apply_records`] changes the manifests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Apply {
    /// `--update` (the user's approval): every recorded image becomes the approved entry.
    Update,
    /// `--propose`: images without an entry become pending entries; nothing else changes.
    Propose,
}

/// What [`apply_records`] changed, per manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Entries added (`(manifest, name)`).
    pub added: Vec<(PathBuf, String)>,
    /// Entries whose digest changed.
    pub changed: Vec<(PathBuf, String)>,
    /// Manifests written.
    pub written: Vec<PathBuf>,
}

/// Applies a run's records to their manifests (see [`Apply`]); manifests without a change are
/// not rewritten.
///
/// ```
/// use twine_testing::snapshot::{Apply, Digest, Manifest, Outcome, Record, apply_records};
/// let dir = std::env::temp_dir().join(format!("twine-apply-doc-{}", std::process::id()));
/// let manifest = dir.join("snapshots.toml");
/// let rec = Record { name: "a".into(), outcome: Outcome::New,
///                    digest: Digest::of_rgb(1, 1, &[1, 2, 3]), manifest: manifest.clone() };
/// let report = apply_records(&[rec], Apply::Propose).unwrap();
/// assert_eq!(report.added.len(), 1);
/// assert!(Manifest::load(&manifest).unwrap().pending.contains_key("a"));
/// std::fs::remove_dir_all(dir).unwrap();
/// ```
///
/// # Errors
/// Reading or writing a manifest.
pub fn apply_records(records: &[Record], how: Apply) -> std::io::Result<ApplyReport> {
    let mut by_manifest: BTreeMap<&Path, Vec<&Record>> = BTreeMap::new();
    for r in records {
        by_manifest.entry(r.manifest.as_path()).or_default().push(r);
    }
    let mut report = ApplyReport::default();
    for (path, recs) in by_manifest {
        let mut m = Manifest::load(path)?;
        let before = m.clone();
        for r in recs {
            let old = m.get(&r.name).map(|(d, _)| d);
            match how {
                Apply::Update => {
                    m.pending.remove(&r.name);
                    m.approved.insert(r.name.clone(), r.digest);
                }
                Apply::Propose => {
                    if old.is_none() {
                        m.pending.insert(r.name.clone(), r.digest);
                    }
                }
            }
            match old {
                None if m.get(&r.name).is_some() => report.added.push((path.to_path_buf(), r.name.clone())),
                Some(d) if d != r.digest && how == Apply::Update => {
                    report.changed.push((path.to_path_buf(), r.name.clone()));
                }
                _ => {}
            }
        }
        if m != before {
            m.save(path)?;
            report.written.push(path.to_path_buf());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_text_round_trip_and_errors() {
        let d = Digest {
            width: 12,
            height: 3,
            hash: 0xab,
        };
        assert_eq!(d.to_string(), "12x3 xxh64:00000000000000ab");
        assert_eq!("12x3 xxh64:00000000000000ab".parse(), Ok(d));
        for bad in [
            "",
            "12x3",
            "12x3 crc:00",
            "12x3 xxh64:ab",
            "ax3 xxh64:00000000000000ab",
        ] {
            assert!(bad.parse::<Digest>().is_err(), "{bad}");
        }
    }

    #[test]
    fn manifest_parse_errors_name_the_line() {
        let e = "[approved]\nx = \"1x1 xxh64:0000000000000000\"\nx = \"1x1 xxh64:0000000000000001\"\n"
            .parse::<Manifest>()
            .unwrap_err();
        assert_eq!(e.line, 3);
        assert!("y = \"1x1 xxh64:0000000000000000\"".parse::<Manifest>().is_err());
        assert!(
            "[approved]\nBad = \"1x1 xxh64:0000000000000000\""
                .parse::<Manifest>()
                .is_err()
        );
        assert!(
            "[approved]\na = \"1x1 xxh64:0000000000000000\"\n[pending]\na = \"1x1 xxh64:0000000000000000\""
                .parse::<Manifest>()
                .is_err()
        );
    }

    #[test]
    fn manifest_is_written_sorted_with_pending_last() {
        let d = Digest::of_rgb(1, 1, &[0, 0, 0]);
        let mut m = Manifest::default();
        m.approved.insert("b".into(), d);
        m.approved.insert("a".into(), d);
        m.pending.insert("c".into(), d);
        let text = m.to_string();
        let a = text.find("\na = ").unwrap();
        let b = text.find("\nb = ").unwrap();
        let p = text.find("[pending]").unwrap();
        let c = text.find("\nc = ").unwrap();
        assert!(a < b && b < p && p < c);
        assert_eq!(m.accept_pending(), vec!["c".to_string()]);
        assert!(!m.to_string().contains("[pending]"));
    }
}
