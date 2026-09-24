//! [`StdFs`]: a directory of the host file system as a drive (LVGL `lv_fs_stdio`).

use alloc::string::String;
use alloc::vec::Vec;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use crate::types::{DirEntry, Error, FileHandle, FileSystem, Metadata, OpenMode, SeekFrom, components};

/// Maximum number of files a [`StdFs`] keeps open at once.
const MAX_OPEN: usize = 64;

/// The host directory `root` as a file system: `"/x/y"` maps to `root/x/y`.
///
/// Paths are split at `/`; `..` components are rejected with [`Error::InvalidPath`], so no
/// path can escape `root`. Directory listings are sorted by name (deterministic across hosts);
/// entries whose names are not UTF-8 are skipped.
///
/// ```no_run
/// use twine_fs::{StdFs, Vfs};
///
/// let mut vfs = Vfs::new();
/// vfs.mount('A', Box::new(StdFs::new("assets/images"))).unwrap();
/// let logo = vfs.read_to_vec("A:/logo.qoi");
/// ```
#[derive(Debug)]
pub struct StdFs {
    root: PathBuf,
    files: Vec<Option<(std::fs::File, OpenMode)>>,
}

fn map_err(e: &std::io::Error) -> Error {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => Error::NotFound,
        K::PermissionDenied | K::ReadOnlyFilesystem => Error::PermissionDenied,
        K::IsADirectory => Error::IsADirectory,
        K::NotADirectory => Error::NotADirectory,
        K::InvalidInput => Error::InvalidPath,
        K::OutOfMemory => Error::OutOfMemory,
        K::Unsupported => Error::Unsupported,
        _ => Error::Io,
    }
}

impl StdFs {
    /// A file system rooted at `root` (not checked until used).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            files: Vec::new(),
        }
    }

    /// The root directory.
    #[must_use]
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Number of currently open files.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.files.iter().filter(|f| f.is_some()).count()
    }

    /// Maps a driver path to a host path below the root.
    fn host_path(&self, path: &str) -> Result<PathBuf, Error> {
        let mut p = self.root.clone();
        for c in components(path) {
            let c = c?;
            if c.contains('\\') || c.contains(':') {
                return Err(Error::InvalidPath);
            }
            p.push(c);
        }
        Ok(p)
    }

    fn file(&mut self, f: FileHandle) -> Result<&mut (std::fs::File, OpenMode), Error> {
        self.files
            .get_mut(usize::from(f.0))
            .and_then(Option::as_mut)
            .ok_or(Error::BadHandle)
    }
}

impl FileSystem for StdFs {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<FileHandle, Error> {
        let p = self.host_path(path)?;
        if p.is_dir() {
            return Err(Error::IsADirectory);
        }
        let slot = match self.files.iter().position(Option::is_none) {
            Some(i) => i,
            None if self.files.len() < MAX_OPEN => {
                self.files.push(None);
                self.files.len() - 1
            }
            None => return Err(Error::TooManyOpenFiles),
        };
        let mut o = std::fs::OpenOptions::new();
        match mode {
            OpenMode::Read => o.read(true),
            OpenMode::Write => o.write(true).create(true).truncate(true),
            OpenMode::ReadWrite => o.read(true).write(true).create(true).truncate(false),
            OpenMode::Append => o.append(true).create(true),
        };
        let file = o.open(&p).map_err(|e| map_err(&e))?;
        self.files[slot] = Some((file, mode));
        Ok(FileHandle(slot as u16))
    }

    fn read(&mut self, f: FileHandle, buf: &mut [u8]) -> Result<usize, Error> {
        let (file, mode) = self.file(f)?;
        if !mode.can_read() {
            return Err(Error::PermissionDenied);
        }
        file.read(buf).map_err(|e| map_err(&e))
    }

    fn write(&mut self, f: FileHandle, buf: &[u8]) -> Result<usize, Error> {
        let (file, mode) = self.file(f)?;
        if !mode.can_write() {
            return Err(Error::PermissionDenied);
        }
        file.write(buf).map_err(|e| map_err(&e))
    }

    fn seek(&mut self, f: FileHandle, pos: SeekFrom) -> Result<u64, Error> {
        let (file, _) = self.file(f)?;
        let p = match pos {
            SeekFrom::Start(n) => std::io::SeekFrom::Start(n),
            SeekFrom::End(n) => std::io::SeekFrom::End(n),
            SeekFrom::Current(n) => std::io::SeekFrom::Current(n),
        };
        file.seek(p).map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidInput => Error::InvalidSeek,
            _ => map_err(&e),
        })
    }

    fn tell(&mut self, f: FileHandle) -> Result<u64, Error> {
        let (file, _) = self.file(f)?;
        file.stream_position().map_err(|e| map_err(&e))
    }

    fn close(&mut self, f: FileHandle) -> Result<(), Error> {
        let (mut file, _) = self
            .files
            .get_mut(usize::from(f.0))
            .and_then(Option::take)
            .ok_or(Error::BadHandle)?;
        file.flush().map_err(|e| map_err(&e))
    }

    fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
        let p = self.host_path(path)?;
        if p.is_file() {
            return Err(Error::NotADirectory);
        }
        let mut entries: Vec<(String, bool, u64)> = Vec::new();
        for e in std::fs::read_dir(&p).map_err(|e| map_err(&e))? {
            let e = e.map_err(|e| map_err(&e))?;
            let Ok(name) = e.file_name().into_string() else {
                twine_core::warn!(target: "twine::fs", "fs skips a non-UTF-8 name in {}", path);
                continue;
            };
            let md = e.metadata().map_err(|e| map_err(&e))?;
            let size = if md.is_dir() { 0 } else { md.len() };
            entries.push((name, md.is_dir(), size));
        }
        entries.sort();
        for (name, is_dir, size) in &entries {
            out(DirEntry {
                name,
                is_dir: *is_dir,
                size: *size,
            });
        }
        Ok(())
    }

    fn metadata(&mut self, path: &str) -> Result<Metadata, Error> {
        let md = std::fs::metadata(self.host_path(path)?).map_err(|e| map_err(&e))?;
        Ok(Metadata {
            size: if md.is_dir() { 0 } else { md.len() },
            is_dir: md.is_dir(),
        })
    }
}
