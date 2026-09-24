//! [`Vfs`]: drive-letter registry (LVGL `lv_fs_drv_register` / `lv_fs_open`) and the RAII
//! [`File`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::types::{DirEntry, Error, FileHandle, FileSystem, Metadata, OpenMode, SeekFrom};

/// Maximum number of mounted drives.
pub const MAX_DRIVES: usize = 8;

/// Splits `"A:/dir/file.png"` into the drive letter and the driver path (`"/dir/file.png"`).
///
/// The letter must be an ASCII letter followed by `:`; anything else is
/// [`Error::InvalidPath`]. Letters are case-sensitive (like LVGL).
///
/// ```
/// use twine_fs::{Error, parse_path};
/// assert_eq!(parse_path("A:/img/logo.qoi"), Ok(('A', "/img/logo.qoi")));
/// assert_eq!(parse_path("S:"), Ok(('S', "")));
/// assert_eq!(parse_path("/img/logo.qoi"), Err(Error::InvalidPath));
/// ```
pub fn parse_path(path: &str) -> Result<(char, &str), Error> {
    let b = path.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return Err(Error::InvalidPath);
    }
    Ok((char::from(b[0]), &path[2..]))
}

/// A set of file systems mounted under drive letters.
///
/// ```
/// use twine_fs::{MemoryFs, OpenMode, Vfs};
///
/// static FILES: &[(&str, &[u8])] = &[("img/logo.qoi", b"qoif..."), ("hello.txt", b"hi")];
/// let mut vfs = Vfs::new();
/// vfs.mount('A', Box::new(MemoryFs::new(FILES))).unwrap();
/// assert_eq!(vfs.read_to_vec("A:/hello.txt").unwrap(), b"hi");
///
/// let mut f = vfs.open("A:/img/logo.qoi", OpenMode::Read).unwrap();
/// let mut magic = [0; 4];
/// f.read(&mut magic).unwrap();
/// assert_eq!(&magic, b"qoif");
/// // `f` closes the file when dropped.
/// ```
#[derive(Default)]
pub struct Vfs {
    drives: heapless::Vec<(char, Box<dyn FileSystem>), MAX_DRIVES>,
}

impl core::fmt::Debug for Vfs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.drives.iter().map(|(c, _)| c))
            .finish()
    }
}

impl Vfs {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts `fs` as drive `letter` (an ASCII letter), replacing (and returning) a file system
    /// already mounted there. [`Error::TooManyDrives`] when [`MAX_DRIVES`] are mounted.
    pub fn mount(
        &mut self,
        letter: char,
        fs: Box<dyn FileSystem>,
    ) -> Result<Option<Box<dyn FileSystem>>, Error> {
        if !letter.is_ascii_alphabetic() {
            return Err(Error::InvalidPath);
        }
        if let Some(slot) = self.drives.iter_mut().find(|(c, _)| *c == letter) {
            twine_core::debug!(target: "twine::fs", "fs remount {}:", letter);
            return Ok(Some(core::mem::replace(&mut slot.1, fs)));
        }
        self.drives.push((letter, fs)).map_err(|_| Error::TooManyDrives)?;
        twine_core::debug!(target: "twine::fs", "fs mount {}:", letter);
        Ok(None)
    }

    /// Unmounts drive `letter`, returning its file system.
    pub fn unmount(&mut self, letter: char) -> Option<Box<dyn FileSystem>> {
        let i = self.drives.iter().position(|(c, _)| *c == letter)?;
        twine_core::debug!(target: "twine::fs", "fs unmount {}:", letter);
        Some(self.drives.swap_remove(i).1)
    }

    /// The mounted drive letters.
    pub fn letters(&self) -> impl Iterator<Item = char> + '_ {
        self.drives.iter().map(|(c, _)| *c)
    }

    /// The file system mounted at `letter`.
    pub fn drive_mut(&mut self, letter: char) -> Option<&mut (dyn FileSystem + 'static)> {
        self.drives
            .iter_mut()
            .find(|(c, _)| *c == letter)
            .map(|(_, fs)| &mut **fs)
    }

    /// Resolves a full path to its file system and driver path.
    fn resolve<'p>(&mut self, path: &'p str) -> Result<(&mut (dyn FileSystem + 'static), &'p str), Error> {
        let (letter, rest) = parse_path(path)?;
        let fs = self.drive_mut(letter).ok_or(Error::UnknownDrive(letter))?;
        Ok((fs, rest))
    }

    /// Opens `path` (`"A:/dir/file"`); the returned [`File`] closes itself when dropped.
    pub fn open(&mut self, path: &str, mode: OpenMode) -> Result<File<'_>, Error> {
        let res = self
            .resolve(path)
            .and_then(|(fs, rest)| File::open(fs, rest, mode));
        twine_core::debug!(
            target: "twine::fs",
            "fs open {} -> {:?}",
            path,
            res.as_ref().map(File::handle)
        );
        res
    }

    /// Lists the directory `path` (see [`FileSystem::read_dir`]).
    pub fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
        let (fs, rest) = self.resolve(path)?;
        fs.read_dir(rest, out)
    }

    /// Metadata of `path`.
    pub fn metadata(&mut self, path: &str) -> Result<Metadata, Error> {
        let (fs, rest) = self.resolve(path)?;
        fs.metadata(rest)
    }

    /// Reads the whole file `path` into a new vector.
    pub fn read_to_vec(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        self.open(path, OpenMode::Read)?.read_to_end(&mut out)?;
        Ok(out)
    }
}

/// An open file that closes itself when dropped (errors on drop are logged; call
/// [`close`](Self::close) to observe them).
///
/// ```
/// use twine_fs::{File, FileSystem, MemoryFs, OpenMode, SeekFrom};
///
/// let mut fs = MemoryFs::new(&[]).writable();
/// {
///     let mut f = File::open(&mut fs, "/log.txt", OpenMode::Append).unwrap();
///     f.write_all(b"boot\n").unwrap();
/// }
/// let mut f = File::open(&mut fs, "/log.txt", OpenMode::Read).unwrap();
/// assert_eq!(f.seek(SeekFrom::End(0)).unwrap(), 5);
/// ```
pub struct File<'a> {
    fs: &'a mut dyn FileSystem,
    handle: FileHandle,
    open: bool,
}

impl core::fmt::Debug for File<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

impl<'a> File<'a> {
    /// Opens `path` (a driver path, without drive letter) on `fs`.
    pub fn open(fs: &'a mut dyn FileSystem, path: &str, mode: OpenMode) -> Result<Self, Error> {
        let handle = fs.open(path, mode)?;
        Ok(Self {
            fs,
            handle,
            open: true,
        })
    }

    /// The file system's handle of this file.
    #[must_use]
    pub fn handle(&self) -> FileHandle {
        self.handle
    }

    /// Reads up to `buf.len()` bytes; returns the number read (0 at the end).
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        self.fs.read(self.handle, buf)
    }

    /// Reads exactly `buf.len()` bytes; [`Error::Io`] if the file ends first.
    pub fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            let n = self.read(buf)?;
            if n == 0 {
                return Err(Error::Io);
            }
            buf = &mut buf[n..];
        }
        Ok(())
    }

    /// Appends the rest of the file to `out`; returns the number of bytes read.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, Error> {
        let start = out.len();
        let pos = self.tell()?;
        let end = self.seek(SeekFrom::End(0))?;
        self.seek(SeekFrom::Start(pos))?;
        let hint = usize::try_from(end.saturating_sub(pos)).map_err(|_| Error::OutOfMemory)?;
        out.try_reserve(hint).map_err(|_| Error::OutOfMemory)?;
        let mut chunk = [0u8; 256];
        loop {
            let n = self.read(&mut chunk)?;
            if n == 0 {
                return Ok(out.len() - start);
            }
            out.try_reserve(n).map_err(|_| Error::OutOfMemory)?;
            out.extend_from_slice(&chunk[..n]);
        }
    }

    /// Writes `buf`; returns the number of bytes written.
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        self.fs.write(self.handle, buf)
    }

    /// Writes all of `buf`; [`Error::Io`] if the file system stops accepting data.
    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                return Err(Error::Io);
            }
            buf = &buf[n..];
        }
        Ok(())
    }

    /// Moves the cursor; returns the new position.
    pub fn seek(&mut self, pos: SeekFrom) -> Result<u64, Error> {
        self.fs.seek(self.handle, pos)
    }

    /// The cursor position.
    pub fn tell(&mut self) -> Result<u64, Error> {
        self.fs.tell(self.handle)
    }

    /// Closes the file, reporting errors (dropping closes too, but only logs them).
    pub fn close(mut self) -> Result<(), Error> {
        self.open = false;
        self.fs.close(self.handle)
    }
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        if self.open {
            if let Err(e) = self.fs.close(self.handle) {
                twine_core::warn!(target: "twine::fs", "fs close {:?} failed: {:?}", self.handle, e);
            }
        }
    }
}
