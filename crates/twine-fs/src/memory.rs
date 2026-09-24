//! [`MemoryFs`]: files compiled into flash plus optional writable files in RAM (LVGL
//! `lv_fs_memfs`, extended with directories and writing).

use alloc::string::String;
use alloc::vec::Vec;

use crate::types::{DirEntry, Error, FileHandle, FileSystem, Metadata, OpenMode, SeekFrom};

/// Maximum number of files a [`MemoryFs`] keeps open at once.
pub const MEMORY_FS_MAX_OPEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Static(usize),
    Ram(usize),
}

#[derive(Clone, Copy, Debug)]
struct Open {
    node: Node,
    pos: u64,
    mode: OpenMode,
}

#[derive(Debug)]
struct RamFile {
    name: String,
    data: Vec<u8>,
}

/// An in-memory file system.
///
/// Read-only files come from a `static` table of `(path, bytes)` pairs (paths use `/`, a
/// leading `/` is optional); directories are implied by path prefixes. With
/// [`writable`](Self::writable), files opened for writing are created in RAM (`Vec<u8>`), and
/// RAM files shadow read-only ones of the same path. Writing to a read-only file is
/// [`Error::PermissionDenied`].
///
/// ```
/// use twine_fs::{FileSystem, MemoryFs};
///
/// static FILES: &[(&str, &[u8])] = &[("fonts/a.ttf", b"...."), ("fonts/b.ttf", b".."), ("readme", b"x")];
/// let mut fs = MemoryFs::new(FILES);
/// let mut names = Vec::new();
/// fs.read_dir("/", &mut |e| names.push((e.name.to_string(), e.is_dir))).unwrap();
/// assert_eq!(names, [("fonts".to_string(), true), ("readme".to_string(), false)]);
/// assert_eq!(fs.metadata("/fonts/a.ttf").unwrap().size, 4);
/// ```
#[derive(Debug)]
pub struct MemoryFs {
    statics: &'static [(&'static str, &'static [u8])],
    ram: Vec<RamFile>,
    writable: bool,
    open: Vec<Option<Open>>,
}

/// Strips leading and trailing `/`; empty, `.` and `..` components inside are
/// [`Error::InvalidPath`].
fn normalize(path: &str) -> Result<&str, Error> {
    let p = path.trim_matches('/');
    if !p.is_empty() && p.split('/').any(|c| matches!(c, "" | "." | "..")) {
        return Err(Error::InvalidPath);
    }
    Ok(p)
}

/// If `name` lies inside directory `dir` (`""` = root), the part of `name` below `dir`.
fn below<'n>(name: &'n str, dir: &str) -> Option<&'n str> {
    let name = name.trim_start_matches('/');
    if dir.is_empty() {
        return Some(name);
    }
    name.strip_prefix(dir)?.strip_prefix('/')
}

impl MemoryFs {
    /// A read-only file system over `files`.
    #[must_use]
    pub const fn new(files: &'static [(&'static str, &'static [u8])]) -> Self {
        Self {
            statics: files,
            ram: Vec::new(),
            writable: false,
            open: Vec::new(),
        }
    }

    /// Allows creating and writing files in RAM.
    #[must_use]
    pub fn writable(mut self) -> Self {
        self.writable = true;
        self
    }

    /// Number of currently open files.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.open.iter().filter(|o| o.is_some()).count()
    }

    /// Every file path (RAM files first), leading `/` stripped.
    fn names(&self) -> impl Iterator<Item = (&str, u64)> {
        self.ram
            .iter()
            .map(|f| (f.name.as_str(), f.data.len() as u64))
            .chain(
                self.statics
                    .iter()
                    .map(|(n, d)| (n.trim_start_matches('/'), d.len() as u64)),
            )
    }

    fn find(&self, p: &str) -> Option<Node> {
        if let Some(i) = self.ram.iter().position(|f| f.name == p) {
            return Some(Node::Ram(i));
        }
        self.statics
            .iter()
            .position(|(n, _)| n.trim_start_matches('/') == p)
            .map(Node::Static)
    }

    fn is_dir(&self, p: &str) -> bool {
        p.is_empty()
            || self
                .names()
                .any(|(n, _)| below(n, p).is_some_and(|r| !r.is_empty()))
    }

    fn data(&self, node: Node) -> &[u8] {
        match node {
            Node::Static(i) => self.statics[i].1,
            Node::Ram(i) => &self.ram[i].data,
        }
    }

    fn slot(&mut self, f: FileHandle) -> Result<&mut Open, Error> {
        self.open
            .get_mut(usize::from(f.0))
            .and_then(Option::as_mut)
            .ok_or(Error::BadHandle)
    }

    fn open_node(&mut self, p: &str, mode: OpenMode) -> Result<Node, Error> {
        if self.is_dir(p) {
            return Err(Error::IsADirectory);
        }
        // A parent that is a file makes the path invalid.
        if let Some((parent, _)) = p.rsplit_once('/') {
            if self.find(parent).is_some() {
                return Err(Error::NotADirectory);
            }
        }
        match (self.find(p), mode) {
            (Some(node), OpenMode::Read) => Ok(node),
            (None, OpenMode::Read) => Err(Error::NotFound),
            (Some(Node::Static(_)), _) => Err(Error::PermissionDenied),
            (_, _) if !self.writable => Err(Error::PermissionDenied),
            (Some(Node::Ram(i)), m) => {
                if m == OpenMode::Write {
                    self.ram[i].data.clear();
                }
                Ok(Node::Ram(i))
            }
            (None, _) => {
                self.ram.push(RamFile {
                    name: String::from(p),
                    data: Vec::new(),
                });
                Ok(Node::Ram(self.ram.len() - 1))
            }
        }
    }
}

impl FileSystem for MemoryFs {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<FileHandle, Error> {
        let p = normalize(path)?;
        let free = match self.open.iter().position(Option::is_none) {
            Some(i) => i,
            None if self.open.len() < MEMORY_FS_MAX_OPEN => {
                self.open.push(None);
                self.open.len() - 1
            }
            None => return Err(Error::TooManyOpenFiles),
        };
        let node = self.open_node(p, mode)?;
        self.open[free] = Some(Open { node, pos: 0, mode });
        Ok(FileHandle(free as u16))
    }

    fn read(&mut self, f: FileHandle, buf: &mut [u8]) -> Result<usize, Error> {
        let o = *self.slot(f)?;
        if !o.mode.can_read() {
            return Err(Error::PermissionDenied);
        }
        let data = self.data(o.node);
        let start = usize::try_from(o.pos).unwrap_or(usize::MAX).min(data.len());
        let n = buf.len().min(data.len() - start);
        buf[..n].copy_from_slice(&data[start..start + n]);
        self.slot(f)?.pos += n as u64;
        Ok(n)
    }

    fn write(&mut self, f: FileHandle, buf: &[u8]) -> Result<usize, Error> {
        let o = *self.slot(f)?;
        let Node::Ram(i) = o.node else {
            return Err(Error::PermissionDenied);
        };
        if !o.mode.can_write() {
            return Err(Error::PermissionDenied);
        }
        let data = &mut self.ram[i].data;
        let start = if o.mode == OpenMode::Append {
            data.len()
        } else {
            usize::try_from(o.pos).map_err(|_| Error::OutOfMemory)?
        };
        let end = start.checked_add(buf.len()).ok_or(Error::OutOfMemory)?;
        if end > data.len() {
            data.try_reserve(end - data.len())
                .map_err(|_| Error::OutOfMemory)?;
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(buf);
        self.slot(f)?.pos = end as u64;
        Ok(buf.len())
    }

    fn seek(&mut self, f: FileHandle, pos: SeekFrom) -> Result<u64, Error> {
        let o = *self.slot(f)?;
        let len = self.data(o.node).len() as u64;
        let new = pos.resolve(o.pos, len)?;
        self.slot(f)?.pos = new;
        Ok(new)
    }

    fn tell(&mut self, f: FileHandle) -> Result<u64, Error> {
        Ok(self.slot(f)?.pos)
    }

    fn close(&mut self, f: FileHandle) -> Result<(), Error> {
        self.slot(f)?;
        self.open[usize::from(f.0)] = None;
        Ok(())
    }

    fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
        let d = normalize(path)?;
        if !self.is_dir(d) {
            return Err(if self.find(d).is_some() {
                Error::NotADirectory
            } else {
                Error::NotFound
            });
        }
        let mut seen: Vec<&str> = Vec::new();
        for (name, size) in self.names() {
            let Some(rest) = below(name, d) else { continue };
            let (child, is_dir) = match rest.split_once('/') {
                Some((c, _)) => (c, true),
                None => (rest, false),
            };
            if child.is_empty() || seen.contains(&child) {
                continue;
            }
            seen.push(child);
            out(DirEntry {
                name: child,
                is_dir,
                size: if is_dir { 0 } else { size },
            });
        }
        Ok(())
    }

    fn metadata(&mut self, path: &str) -> Result<Metadata, Error> {
        let p = normalize(path)?;
        if let Some(node) = self.find(p) {
            return Ok(Metadata {
                size: self.data(node).len() as u64,
                is_dir: false,
            });
        }
        if self.is_dir(p) {
            return Ok(Metadata {
                size: 0,
                is_dir: true,
            });
        }
        Err(Error::NotFound)
    }
}
