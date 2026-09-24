//! `Vfs`, `File` and `MemoryFs` behaviour.
#![allow(clippy::items_after_statements, clippy::single_match_else)]

use std::cell::Cell;
use std::rc::Rc;

use proptest::prelude::*;
use twine_fs::{
    DirEntry, Error, File, FileHandle, FileSystem, MAX_DRIVES, MemoryFs, Metadata, OpenMode, SeekFrom, Vfs,
    parse_path,
};

static FILES: &[(&str, &[u8])] = &[
    ("/img/logo.qoi", b"qoif-logo-bytes"),
    ("img/icons/ok.png", b"png-ok"),
    ("img/icons/no.png", b"png-no"),
    ("readme.txt", b"0123456789"),
];

fn listing(fs: &mut dyn FileSystem, path: &str) -> Result<Vec<(String, bool, u64)>, Error> {
    let mut v = Vec::new();
    fs.read_dir(path, &mut |e: DirEntry<'_>| {
        v.push((e.name.to_string(), e.is_dir, e.size));
    })?;
    Ok(v)
}

#[test]
fn parse_drive_letter() {
    assert_eq!(parse_path("A:/dir/file.png"), Ok(('A', "/dir/file.png")));
    assert_eq!(parse_path("z:file"), Ok(('z', "file")));
    assert_eq!(parse_path("A:"), Ok(('A', "")));
    for bad in ["", "A", "/dir/file.png", "1:/x", "AB:/x", ":/x", "é:/x"] {
        assert_eq!(parse_path(bad), Err(Error::InvalidPath), "{bad:?}");
    }
}

#[test]
fn missing_drive_error() {
    let mut vfs = Vfs::new();
    vfs.mount('A', Box::new(MemoryFs::new(FILES))).unwrap();
    assert_eq!(
        vfs.open("B:/readme.txt", OpenMode::Read).unwrap_err(),
        Error::UnknownDrive('B')
    );
    assert_eq!(vfs.read_to_vec("readme.txt").unwrap_err(), Error::InvalidPath);
    assert_eq!(vfs.read_to_vec("A:/nope").unwrap_err(), Error::NotFound);
    assert_eq!(vfs.read_to_vec("A:/readme.txt").unwrap(), b"0123456789");
    assert!(vfs.unmount('A').is_some());
    assert_eq!(
        vfs.metadata("A:/readme.txt").unwrap_err(),
        Error::UnknownDrive('A')
    );
}

#[test]
fn mount_limits_and_replacement() {
    let mut vfs = Vfs::new();
    for (i, c) in ('A'..='Z').take(MAX_DRIVES).enumerate() {
        assert!(
            vfs.mount(c, Box::new(MemoryFs::new(&[]))).unwrap().is_none(),
            "{i}"
        );
    }
    assert!(matches!(
        vfs.mount('Z', Box::new(MemoryFs::new(&[]))),
        Err(Error::TooManyDrives)
    ));
    assert!(vfs.mount('A', Box::new(MemoryFs::new(FILES))).unwrap().is_some());
    assert!(matches!(
        vfs.mount('1', Box::new(MemoryFs::new(&[]))),
        Err(Error::InvalidPath)
    ));
    assert_eq!(vfs.letters().count(), MAX_DRIVES);
    assert_eq!(
        vfs.metadata("A:/img").unwrap(),
        Metadata {
            size: 0,
            is_dir: true
        }
    );
}

#[test]
fn memoryfs_read_seek_tell() {
    let mut fs = MemoryFs::new(FILES);
    let mut f = File::open(&mut fs, "readme.txt", OpenMode::Read).unwrap();
    let mut b = [0; 4];
    assert_eq!(f.read(&mut b).unwrap(), 4);
    assert_eq!(&b, b"0123");
    assert_eq!(f.tell().unwrap(), 4);
    assert_eq!(f.seek(SeekFrom::Current(2)).unwrap(), 6);
    assert_eq!(f.read(&mut b).unwrap(), 4);
    assert_eq!(&b, b"6789");
    assert_eq!(f.read(&mut b).unwrap(), 0, "end of file");
    assert_eq!(f.seek(SeekFrom::End(-3)).unwrap(), 7);
    f.read_exact(&mut b[..3]).unwrap();
    assert_eq!(&b[..3], b"789");
    assert_eq!(f.seek(SeekFrom::Current(-11)).unwrap_err(), Error::InvalidSeek);
    assert_eq!(f.seek(SeekFrom::Start(100)).unwrap(), 100);
    assert_eq!(f.read(&mut b).unwrap(), 0, "past the end reads nothing");
    assert_eq!(f.write(b"x").unwrap_err(), Error::PermissionDenied);
    drop(f);
    assert_eq!(fs.open("img", OpenMode::Read).unwrap_err(), Error::IsADirectory);
    assert_eq!(fs.open("/", OpenMode::Read).unwrap_err(), Error::IsADirectory);
    assert_eq!(
        fs.open("readme.txt/x", OpenMode::Read).unwrap_err(),
        Error::NotADirectory
    );
    assert_eq!(
        fs.open("img/../readme.txt", OpenMode::Read).unwrap_err(),
        Error::InvalidPath
    );
    assert_eq!(fs.read(FileHandle(9), &mut b).unwrap_err(), Error::BadHandle);
    assert_eq!(fs.close(FileHandle(0)).unwrap_err(), Error::BadHandle);
}

#[test]
fn memoryfs_read_dir_lists_children_once() {
    let mut fs = MemoryFs::new(FILES);
    assert_eq!(
        listing(&mut fs, "/").unwrap(),
        [("img".into(), true, 0), ("readme.txt".into(), false, 10)]
    );
    assert_eq!(
        listing(&mut fs, "/img/").unwrap(),
        [("logo.qoi".into(), false, 15), ("icons".into(), true, 0)]
    );
    assert_eq!(listing(&mut fs, "img/icons").unwrap().len(), 2);
    assert_eq!(listing(&mut fs, "readme.txt").unwrap_err(), Error::NotADirectory);
    assert_eq!(listing(&mut fs, "nope").unwrap_err(), Error::NotFound);
    assert_eq!(
        fs.metadata("img/icons/ok.png").unwrap(),
        Metadata {
            size: 6,
            is_dir: false
        }
    );
    assert_eq!(
        fs.metadata("").unwrap(),
        Metadata {
            size: 0,
            is_dir: true
        }
    );
    assert_eq!(fs.metadata("img/ic").unwrap_err(), Error::NotFound);
}

#[test]
fn write_append_readback() {
    let mut fs = MemoryFs::new(FILES).writable();
    {
        let mut f = File::open(&mut fs, "/logs/boot.txt", OpenMode::Write).unwrap();
        f.write_all(b"hello").unwrap();
        f.seek(SeekFrom::Start(7)).unwrap();
        f.write_all(b"!").unwrap(); // gap filled with zeros
        assert_eq!(f.read(&mut [0; 1]).unwrap_err(), Error::PermissionDenied);
    }
    {
        let mut f = File::open(&mut fs, "logs/boot.txt", OpenMode::Append).unwrap();
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(b"++").unwrap(); // appends regardless of the cursor
    }
    let mut v = Vec::new();
    File::open(&mut fs, "logs/boot.txt", OpenMode::Read)
        .unwrap()
        .read_to_end(&mut v)
        .unwrap();
    assert_eq!(v, b"hello\0\0!++");
    {
        let mut f = File::open(&mut fs, "logs/boot.txt", OpenMode::ReadWrite).unwrap();
        f.write_all(b"J").unwrap();
        let mut b = [0; 4];
        f.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"ello");
    }
    // Write truncates.
    File::open(&mut fs, "logs/boot.txt", OpenMode::Write).unwrap();
    assert_eq!(fs.metadata("logs/boot.txt").unwrap().size, 0);
    assert_eq!(listing(&mut fs, "/").unwrap().len(), 3, "img, logs, readme.txt");
    // Read-only files and file systems refuse writes.
    assert_eq!(
        fs.open("readme.txt", OpenMode::Append).unwrap_err(),
        Error::PermissionDenied
    );
    let mut ro = MemoryFs::new(FILES);
    assert_eq!(
        ro.open("new.txt", OpenMode::Write).unwrap_err(),
        Error::PermissionDenied
    );
}

#[test]
fn file_closes_on_drop() {
    let mut fs = MemoryFs::new(FILES);
    {
        let _a = File::open(&mut fs, "readme.txt", OpenMode::Read).unwrap();
    }
    let a = fs.open("readme.txt", OpenMode::Read).unwrap();
    let b = fs.open("img/logo.qoi", OpenMode::Read).unwrap();
    assert_eq!(fs.open_count(), 2);
    fs.close(a).unwrap();
    fs.close(b).unwrap();
    {
        let f = File::open(&mut fs, "readme.txt", OpenMode::Read).unwrap();
        f.close().unwrap();
        let _g = File::open(&mut fs, "readme.txt", OpenMode::Read).unwrap();
    }
    assert_eq!(fs.open_count(), 0);
    // Also through the Vfs (a driver that counts its open files).
    struct Counting(MemoryFs, Rc<Cell<usize>>);
    impl FileSystem for Counting {
        fn open(&mut self, p: &str, m: OpenMode) -> Result<FileHandle, Error> {
            let h = self.0.open(p, m)?;
            self.1.set(self.0.open_count());
            Ok(h)
        }
        fn read(&mut self, f: FileHandle, b: &mut [u8]) -> Result<usize, Error> {
            self.0.read(f, b)
        }
        fn write(&mut self, f: FileHandle, b: &[u8]) -> Result<usize, Error> {
            self.0.write(f, b)
        }
        fn seek(&mut self, f: FileHandle, p: SeekFrom) -> Result<u64, Error> {
            self.0.seek(f, p)
        }
        fn tell(&mut self, f: FileHandle) -> Result<u64, Error> {
            self.0.tell(f)
        }
        fn close(&mut self, f: FileHandle) -> Result<(), Error> {
            self.0.close(f)?;
            self.1.set(self.0.open_count());
            Ok(())
        }
        fn read_dir(&mut self, p: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
            self.0.read_dir(p, out)
        }
        fn metadata(&mut self, p: &str) -> Result<Metadata, Error> {
            self.0.metadata(p)
        }
    }
    let count = Rc::new(Cell::new(0));
    let mut vfs = Vfs::new();
    vfs.mount('M', Box::new(Counting(MemoryFs::new(FILES), count.clone())))
        .unwrap();
    {
        let _f = vfs.open("M:/readme.txt", OpenMode::Read).unwrap();
        assert_eq!(count.get(), 1);
    }
    assert_eq!(count.get(), 0);
    vfs.read_to_vec("M:/img/logo.qoi").unwrap();
    assert_eq!(count.get(), 0);
}

#[test]
fn too_many_open_files() {
    let mut fs = MemoryFs::new(FILES);
    let handles: Vec<_> = (0..twine_fs::MEMORY_FS_MAX_OPEN)
        .map(|_| fs.open("readme.txt", OpenMode::Read).unwrap())
        .collect();
    assert_eq!(
        fs.open("readme.txt", OpenMode::Read).unwrap_err(),
        Error::TooManyOpenFiles
    );
    fs.close(handles[3]).unwrap();
    assert_eq!(
        fs.open("readme.txt", OpenMode::Read).unwrap(),
        handles[3],
        "slot reused"
    );
}

static BIG: [u8; 4096] = {
    let mut a = [0u8; 4096];
    let mut i = 0;
    while i < a.len() {
        a[i] = (i * 7 + i / 13) as u8;
        i += 1;
    }
    a
};
static BIG_FILES: &[(&str, &[u8])] = &[("big.bin", &BIG)];

proptest! {
    #[test]
    fn random_seek_read_matches_slice(ops in prop::collection::vec((0u8..3, -5000i64..5000, 0usize..600), 1..40)) {
        let mut fs = MemoryFs::new(BIG_FILES);
        let mut f = File::open(&mut fs, "big.bin", OpenMode::Read).unwrap();
        let mut pos: u64 = 0;
        let len = BIG.len() as u64;
        for (kind, off, n) in ops {
            let target = match kind {
                0 => SeekFrom::Start(off.unsigned_abs()),
                1 => SeekFrom::End(off),
                _ => SeekFrom::Current(off),
            };
            let expect = match target {
                SeekFrom::Start(p) => Some(p),
                SeekFrom::End(d) => len.checked_add_signed(d),
                SeekFrom::Current(d) => pos.checked_add_signed(d),
            };
            match expect {
                Some(p) => { prop_assert_eq!(f.seek(target).unwrap(), p); pos = p; }
                None => prop_assert_eq!(f.seek(target).unwrap_err(), Error::InvalidSeek),
            }
            let mut buf = vec![0u8; n];
            let got = f.read(&mut buf).unwrap();
            let start = (pos as usize).min(BIG.len());
            let want = &BIG[start..(start + n).min(BIG.len())];
            prop_assert_eq!(&buf[..got], want);
            pos += got as u64;
            prop_assert_eq!(f.tell().unwrap(), pos);
        }
    }
}
