//! Where `ImageSource::File` images are read from: nothing, any [`FileSource`], or (feature
//! `fs`) a `twine-fs` virtual file system.

use alloc::boxed::Box;
#[cfg(feature = "fs")]
use alloc::vec::Vec;

use twine_image::FileSource;

/// The engine's file access for images.
#[derive(Default)]
pub(crate) enum Files {
    /// File images are not found.
    #[default]
    None,
    /// Installed with `Engine::set_file_source`.
    Custom(Box<dyn FileSource>),
    /// Installed with `Engine::set_vfs`.
    #[cfg(feature = "fs")]
    Vfs(VfsFileSource),
}

impl Files {
    /// The file source, if any.
    pub(crate) fn source(&mut self) -> Option<&mut dyn FileSource> {
        match self {
            Files::None => None,
            Files::Custom(f) => Some(f.as_mut()),
            #[cfg(feature = "fs")]
            Files::Vfs(v) => Some(v),
        }
    }

    pub(crate) fn is_some(&self) -> bool {
        !matches!(self, Files::None)
    }
}

/// A [`twine_fs::Vfs`] as an image [`FileSource`] (paths like `"A:/img/logo.qoi"`), feature
/// `fs`. [`Engine::set_vfs`](crate::Engine::set_vfs) installs one; code resolving images
/// without an engine (`twine_image::with_pixels`) can use it directly.
///
/// ```
/// use twine_engine::VfsFileSource;
/// use twine_fs::{MemoryFs, Vfs};
/// use twine_image::FileSource;
///
/// static FILES: &[(&str, &[u8])] = &[("a.bin", b"0123456789")];
/// let mut vfs = Vfs::new();
/// vfs.mount('A', Box::new(MemoryFs::new(FILES))).unwrap();
/// let mut src = VfsFileSource(vfs);
/// let mut out = Vec::new();
/// src.read_prefix("A:/a.bin", 4, &mut out).unwrap();
/// assert_eq!(out, b"0123");
/// ```
#[cfg(feature = "fs")]
#[derive(Debug)]
pub struct VfsFileSource(pub twine_fs::Vfs);

#[cfg(feature = "fs")]
fn map_err(path: &str, e: twine_fs::Error) -> twine_image::Error {
    match e {
        twine_fs::Error::NotFound | twine_fs::Error::UnknownDrive(_) | twine_fs::Error::InvalidPath => {
            twine_image::Error::NotFound
        }
        other => {
            twine_core::warn!(target: "twine::fs", "image file {}: {}", path, other);
            twine_image::Error::Decode("file read error")
        }
    }
}

#[cfg(feature = "fs")]
impl FileSource for VfsFileSource {
    fn read_all(&mut self, path: &str, out: &mut Vec<u8>) -> Result<(), twine_image::Error> {
        out.clear();
        let mut f = self
            .0
            .open(path, twine_fs::OpenMode::Read)
            .map_err(|e| map_err(path, e))?;
        f.read_to_end(out).map_err(|e| map_err(path, e))?;
        twine_core::debug!(target: "twine::fs", "read {} ({} bytes)", path, out.len());
        Ok(())
    }

    fn read_prefix(&mut self, path: &str, max: usize, out: &mut Vec<u8>) -> Result<(), twine_image::Error> {
        out.clear();
        out.resize(max, 0);
        let mut f = self
            .0
            .open(path, twine_fs::OpenMode::Read)
            .map_err(|e| map_err(path, e))?;
        let mut n = 0;
        while n < max {
            match f.read(&mut out[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) => return Err(map_err(path, e)),
            }
        }
        out.truncate(n);
        twine_core::debug!(target: "twine::fs", "probed {} ({} bytes)", path, n);
        Ok(())
    }
}
