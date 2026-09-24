//! [`FatFs`]: FAT12/16/32 volumes on block devices (SD cards) through `embedded-sdmmc`
//! (LVGL `lv_fs_fatfs`).

use core::fmt::Write as _;

use embedded_sdmmc::{
    BlockDevice, Mode, RawDirectory, RawFile, RawVolume, ShortFileName, TimeSource, Timestamp, VolumeIdx,
    VolumeManager,
};

use crate::types::{DirEntry, Error, FileHandle, FileSystem, Metadata, OpenMode, SeekFrom, components};

/// Directories `embedded-sdmmc` may hold open at once: the root plus a path walk and a listing.
const MAX_DIRS: usize = 4;

/// A [`TimeSource`] that always reports the same timestamp (for devices without a clock).
///
/// ```
/// use twine_fs::FixedTime;
/// let t = FixedTime::default(); // 2000-01-01 00:00:00
/// # let _ = t;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct FixedTime(pub Timestamp);

impl Default for FixedTime {
    fn default() -> Self {
        Self(Timestamp {
            year_since_1970: 30,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        })
    }
}

impl TimeSource for FixedTime {
    fn get_timestamp(&self) -> Timestamp {
        self.0
    }
}

/// The first partition (MBR volume 0, FAT16 or FAT32) of a block device such as an SD card.
///
/// Names are **8.3 short names** (case-insensitive; `embedded-sdmmc` does not create long
/// file names, and listings show the short names in upper case, e.g. `LOGO.QOI`). At most
/// `MAX_FILES` files are open at once ([`Error::TooManyOpenFiles`] beyond). Offsets are
/// 32-bit (FAT's file size limit) and the cursor cannot be placed past the end of a file
/// ([`Error::InvalidSeek`]). Dropping the file system closes open files, the root directory
/// and the volume.
///
/// ```
/// use twine_fs::embedded_sdmmc::BlockDevice;
/// use twine_fs::{Error, FatFs, FixedTime, Vfs};
///
/// fn mount_sd<D: BlockDevice + 'static>(vfs: &mut Vfs, sd: D) -> Result<(), Error> {
///     let fat = FatFs::<_, _, 4>::new(sd, FixedTime::default())?;
///     vfs.mount('S', Box::new(fat))?;
///     Ok(())
/// }
/// ```
pub struct FatFs<D: BlockDevice, T: TimeSource, const MAX_FILES: usize = 4> {
    mgr: VolumeManager<D, T, MAX_DIRS, MAX_FILES, 1>,
    volume: RawVolume,
    root: RawDirectory,
    files: [Option<(RawFile, OpenMode)>; MAX_FILES],
}

impl<D: BlockDevice, T: TimeSource, const MAX_FILES: usize> core::fmt::Debug for FatFs<D, T, MAX_FILES> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FatFs")
            .field("open_files", &self.open_count())
            .finish_non_exhaustive()
    }
}

fn map_err<E: core::fmt::Debug>(e: &embedded_sdmmc::Error<E>) -> Error {
    use embedded_sdmmc::Error as E;
    match e {
        E::NotFound => Error::NotFound,
        E::FilenameError(_) => Error::InvalidPath,
        E::TooManyOpenFiles | E::TooManyOpenDirs | E::TooManyOpenVolumes => Error::TooManyOpenFiles,
        E::BadHandle => Error::BadHandle,
        E::OpenedDirAsFile | E::DeleteDirAsFile => Error::IsADirectory,
        E::OpenedFileAsDir => Error::NotADirectory,
        E::ReadOnly | E::FileAlreadyOpen | E::DirAlreadyOpen => Error::PermissionDenied,
        E::Unsupported => Error::Unsupported,
        E::InvalidOffset => Error::InvalidSeek,
        E::NotEnoughSpace | E::DiskFull | E::AllocationError => Error::OutOfMemory,
        _ => Error::Io,
    }
}

impl<D: BlockDevice, T: TimeSource, const MAX_FILES: usize> FatFs<D, T, MAX_FILES> {
    /// Opens volume 0 of `device` (MBR partition table) and its root directory.
    pub fn new(device: D, time: T) -> Result<Self, Error> {
        let mgr = VolumeManager::new_with_limits(device, time, 0x7000);
        let volume = mgr.open_raw_volume(VolumeIdx(0)).map_err(|e| {
            twine_core::warn!(target: "twine::fs", "fs FAT volume 0 not usable: {:?}", map_err(&e));
            map_err(&e)
        })?;
        let root = mgr.open_root_dir(volume).map_err(|e| map_err(&e))?;
        Ok(Self {
            mgr,
            volume,
            root,
            files: [None; MAX_FILES],
        })
    }

    /// Number of currently open files.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.files.iter().filter(|f| f.is_some()).count()
    }

    fn file(&self, f: FileHandle) -> Result<(RawFile, OpenMode), Error> {
        self.files
            .get(usize::from(f.0))
            .copied()
            .flatten()
            .ok_or(Error::BadHandle)
    }

    /// Opens the directory `dir` (a path relative to the root); the caller closes it unless
    /// it is the root (returned with `false`).
    fn open_dir_path(&self, dir: &str) -> Result<(RawDirectory, bool), Error> {
        let mut cur = self.root;
        let mut owned = false;
        for c in components(dir) {
            let c = match c {
                Ok(c) => c,
                Err(e) => {
                    self.close_dir(cur, owned);
                    return Err(e);
                }
            };
            let next = self.mgr.open_dir(cur, c);
            self.close_dir(cur, owned);
            cur = next.map_err(|e| map_err(&e))?;
            owned = true;
        }
        Ok((cur, owned))
    }

    fn close_dir(&self, d: RawDirectory, owned: bool) {
        if owned {
            let _ = self.mgr.close_dir(d);
        }
    }

    /// Runs `f` with the parent directory of `path` and the last component (`None` for the
    /// root itself).
    fn with_parent<R>(
        &self,
        path: &str,
        f: impl FnOnce(RawDirectory, Option<&str>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let trimmed = path.trim_end_matches('/');
        let (dir, name) = match trimmed.rsplit_once('/') {
            Some((d, n)) => (d, n),
            None => ("", trimmed),
        };
        let name = match name {
            "" | "." => None,
            ".." => return Err(Error::InvalidPath),
            n => Some(n),
        };
        let (d, owned) = self.open_dir_path(dir)?;
        let r = f(d, name);
        self.close_dir(d, owned);
        r
    }
}

impl<D: BlockDevice, T: TimeSource, const MAX_FILES: usize> FileSystem for FatFs<D, T, MAX_FILES> {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<FileHandle, Error> {
        let slot = self
            .files
            .iter()
            .position(Option::is_none)
            .ok_or(Error::TooManyOpenFiles)?;
        let m = match mode {
            OpenMode::Read => Mode::ReadOnly,
            OpenMode::Write => Mode::ReadWriteCreateOrTruncate,
            // Opened in append mode (cursor at the end), then rewound below.
            OpenMode::ReadWrite | OpenMode::Append => Mode::ReadWriteCreateOrAppend,
        };
        let raw = self.with_parent(path, |dir, name| {
            let name = name.ok_or(Error::IsADirectory)?;
            self.mgr.open_file_in_dir(dir, name, m).map_err(|e| map_err(&e))
        })?;
        if mode == OpenMode::ReadWrite {
            if let Err(e) = self.mgr.file_seek_from_start(raw, 0) {
                let _ = self.mgr.close_file(raw);
                return Err(map_err(&e));
            }
        }
        self.files[slot] = Some((raw, mode));
        Ok(FileHandle(slot as u16))
    }

    fn read(&mut self, f: FileHandle, buf: &mut [u8]) -> Result<usize, Error> {
        let (raw, mode) = self.file(f)?;
        if !mode.can_read() {
            return Err(Error::PermissionDenied);
        }
        self.mgr.read(raw, buf).map_err(|e| map_err(&e))
    }

    fn write(&mut self, f: FileHandle, buf: &[u8]) -> Result<usize, Error> {
        let (raw, mode) = self.file(f)?;
        if !mode.can_write() {
            return Err(Error::PermissionDenied);
        }
        if mode == OpenMode::Append {
            self.mgr.file_seek_from_end(raw, 0).map_err(|e| map_err(&e))?;
        }
        self.mgr.write(raw, buf).map_err(|e| map_err(&e))?;
        Ok(buf.len())
    }

    fn seek(&mut self, f: FileHandle, pos: SeekFrom) -> Result<u64, Error> {
        let (raw, _) = self.file(f)?;
        let len = u64::from(self.mgr.file_length(raw).map_err(|e| map_err(&e))?);
        let cur = u64::from(self.mgr.file_offset(raw).map_err(|e| map_err(&e))?);
        let target = pos.resolve(cur, len)?;
        // embedded-sdmmc cannot place the cursor past the end of the file.
        if target > len {
            return Err(Error::InvalidSeek);
        }
        let target32 = u32::try_from(target).map_err(|_| Error::InvalidSeek)?;
        self.mgr
            .file_seek_from_start(raw, target32)
            .map_err(|e| map_err(&e))?;
        Ok(target)
    }

    fn tell(&mut self, f: FileHandle) -> Result<u64, Error> {
        let (raw, _) = self.file(f)?;
        Ok(u64::from(self.mgr.file_offset(raw).map_err(|e| map_err(&e))?))
    }

    fn close(&mut self, f: FileHandle) -> Result<(), Error> {
        let (raw, _) = self.file(f)?;
        self.files[usize::from(f.0)] = None;
        self.mgr.close_file(raw).map_err(|e| map_err(&e))
    }

    fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
        let (dir, owned) = self.open_dir_path(path)?;
        let r = self.mgr.iterate_dir(dir, |e| {
            if e.attributes.is_volume()
                || e.attributes.is_lfn()
                || e.name == ShortFileName::this_dir()
                || e.name == ShortFileName::parent_dir()
            {
                return;
            }
            let mut name: heapless::String<12> = heapless::String::new();
            // 8.3 names are at most 12 characters: this never fails.
            let _ = write!(name, "{}", e.name);
            let is_dir = e.attributes.is_directory();
            out(DirEntry {
                name: &name,
                is_dir,
                size: if is_dir { 0 } else { u64::from(e.size) },
            });
        });
        self.close_dir(dir, owned);
        r.map_err(|e| map_err(&e))
    }

    fn metadata(&mut self, path: &str) -> Result<Metadata, Error> {
        self.with_parent(path, |dir, name| {
            let Some(name) = name else {
                return Ok(Metadata {
                    size: 0,
                    is_dir: true,
                });
            };
            let e = self
                .mgr
                .find_directory_entry(dir, name)
                .map_err(|e| map_err(&e))?;
            let is_dir = e.attributes.is_directory();
            Ok(Metadata {
                size: if is_dir { 0 } else { u64::from(e.size) },
                is_dir,
            })
        })
    }
}

impl<D: BlockDevice, T: TimeSource, const MAX_FILES: usize> Drop for FatFs<D, T, MAX_FILES> {
    fn drop(&mut self) {
        for (raw, _) in self.files.iter_mut().filter_map(Option::take) {
            let _ = self.mgr.close_file(raw);
        }
        let _ = self.mgr.close_dir(self.root);
        let _ = self.mgr.close_volume(self.volume);
    }
}
