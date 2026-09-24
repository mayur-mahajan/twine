//! The file-system interface: [`FileSystem`], its value types and [`Error`].

/// How a file is opened (LVGL `lv_fs_mode_t`, plus the usual create/append variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OpenMode {
    /// Read an existing file.
    Read,
    /// Create the file or truncate an existing one, then write.
    Write,
    /// Read and write an existing file (created empty when missing); not truncated.
    ReadWrite,
    /// Write at the end of the file (created when missing); every write appends.
    Append,
}

impl OpenMode {
    /// Whether the mode allows reading.
    #[must_use]
    pub const fn can_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    /// Whether the mode allows writing.
    #[must_use]
    pub const fn can_write(self) -> bool {
        !matches!(self, Self::Read)
    }
}

/// A seek target, like [`std::io::SeekFrom`](https://doc.rust-lang.org/std/io/enum.SeekFrom.html).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SeekFrom {
    /// Absolute position.
    Start(u64),
    /// Relative to the end of the file.
    End(i64),
    /// Relative to the current position.
    Current(i64),
}

impl SeekFrom {
    /// The absolute position this target resolves to for a file of `len` bytes whose cursor
    /// is at `pos`; [`Error::InvalidSeek`] before the start or on overflow.
    ///
    /// ```
    /// use twine_fs::{Error, SeekFrom};
    /// assert_eq!(SeekFrom::End(-2).resolve(3, 10), Ok(8));
    /// assert_eq!(SeekFrom::Current(-4).resolve(3, 10), Err(Error::InvalidSeek));
    /// ```
    pub fn resolve(self, pos: u64, len: u64) -> Result<u64, Error> {
        let (base, delta) = match self {
            Self::Start(p) => return Ok(p),
            Self::End(d) => (len, d),
            Self::Current(d) => (pos, d),
        };
        base.checked_add_signed(delta).ok_or(Error::InvalidSeek)
    }
}

/// One entry of a directory listing (see [`FileSystem::read_dir`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DirEntry<'a> {
    /// The entry's name (no path, no trailing `/`).
    pub name: &'a str,
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// Size in bytes (0 for directories).
    pub size: u64,
}

/// File or directory metadata (see [`FileSystem::metadata`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Metadata {
    /// Size in bytes (0 for directories).
    pub size: u64,
    /// Whether the path is a directory.
    pub is_dir: bool,
}

/// An open file of one [`FileSystem`]; only meaningful to the file system that returned it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FileHandle(pub u16);

/// File-system errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The file or directory does not exist.
    #[error("not found")]
    NotFound,
    /// A path component that must be a directory is a file.
    #[error("not a directory")]
    NotADirectory,
    /// A file operation was attempted on a directory.
    #[error("is a directory")]
    IsADirectory,
    /// The file system or the open mode does not allow the operation.
    #[error("permission denied")]
    PermissionDenied,
    /// The path is malformed (no drive letter, `..` escaping the root, unsupported name).
    #[error("invalid path")]
    InvalidPath,
    /// No file system is mounted at this drive letter.
    #[error("unknown drive `{0}:`")]
    UnknownDrive(char),
    /// The file system's open-file limit is reached.
    #[error("too many open files")]
    TooManyOpenFiles,
    /// A device or driver error.
    #[error("I/O error")]
    Io,
    /// The file system does not support the operation.
    #[error("unsupported operation")]
    Unsupported,
    /// Memory could not be allocated.
    #[error("out of memory")]
    OutOfMemory,
    /// Every drive letter slot of the [`Vfs`](crate::Vfs) is taken.
    #[error("too many drives mounted")]
    TooManyDrives,
    /// A seek before the start of the file (or past the addressable size).
    #[error("invalid seek")]
    InvalidSeek,
    /// The handle is not an open file of this file system.
    #[error("bad file handle")]
    BadHandle,
}

/// A file system (LVGL `lv_fs_drv_t`): a driver mounted in a [`Vfs`](crate::Vfs) under a
/// drive letter, or used directly.
///
/// Paths passed to a file system are the part after the drive letter, e.g. `"/img/logo.qoi"`
/// for `"A:/img/logo.qoi"`; a leading `/` is optional and `/` separates directories.
/// Implementations must never panic on bad paths or handles; they return an [`Error`].
pub trait FileSystem {
    /// Opens `path` in `mode`.
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<FileHandle, Error>;
    /// Reads up to `buf.len()` bytes at the cursor; returns the number read (0 at the end).
    fn read(&mut self, f: FileHandle, buf: &mut [u8]) -> Result<usize, Error>;
    /// Writes `buf` at the cursor (at the end for [`OpenMode::Append`]); returns the number of
    /// bytes written.
    fn write(&mut self, f: FileHandle, buf: &[u8]) -> Result<usize, Error>;
    /// Moves the cursor; returns the new absolute position. Seeking past the end is allowed
    /// (a later write fills the gap with zeros where the file system supports it).
    fn seek(&mut self, f: FileHandle, pos: SeekFrom) -> Result<u64, Error>;
    /// The cursor position.
    fn tell(&mut self, f: FileHandle) -> Result<u64, Error>;
    /// Closes the file (flushing written data).
    fn close(&mut self, f: FileHandle) -> Result<(), Error>;
    /// Calls `out` once for every entry of the directory `path` (not for `.` and `..`).
    fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error>;
    /// Metadata of `path`.
    fn metadata(&mut self, path: &str) -> Result<Metadata, Error>;
}

macro_rules! forward_fs {
    ($t:ty) => {
        impl<F: FileSystem + ?Sized> FileSystem for $t {
            fn open(&mut self, path: &str, mode: OpenMode) -> Result<FileHandle, Error> {
                (**self).open(path, mode)
            }
            fn read(&mut self, f: FileHandle, buf: &mut [u8]) -> Result<usize, Error> {
                (**self).read(f, buf)
            }
            fn write(&mut self, f: FileHandle, buf: &[u8]) -> Result<usize, Error> {
                (**self).write(f, buf)
            }
            fn seek(&mut self, f: FileHandle, pos: SeekFrom) -> Result<u64, Error> {
                (**self).seek(f, pos)
            }
            fn tell(&mut self, f: FileHandle) -> Result<u64, Error> {
                (**self).tell(f)
            }
            fn close(&mut self, f: FileHandle) -> Result<(), Error> {
                (**self).close(f)
            }
            fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
                (**self).read_dir(path, out)
            }
            fn metadata(&mut self, path: &str) -> Result<Metadata, Error> {
                (**self).metadata(path)
            }
        }
    };
}

forward_fs!(&mut F);
forward_fs!(alloc::boxed::Box<F>);

/// Splits `path` at `/` into its non-empty components other than `.`; `..` is rejected with
/// [`Error::InvalidPath`] (drivers never walk above their root).
#[cfg(any(feature = "fs-std", feature = "fs-fat"))]
pub(crate) fn components(path: &str) -> impl Iterator<Item = Result<&str, Error>> {
    path.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(|c| if c == ".." { Err(Error::InvalidPath) } else { Ok(c) })
}
