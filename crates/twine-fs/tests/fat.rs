//! `FatFs` over an in-memory FAT16 image (MBR + one partition) built with the `fatfs` crate.

use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::rc::Rc;

use twine_fs::embedded_sdmmc::{Block, BlockCount, BlockDevice, BlockIdx};
use twine_fs::{Error, FatFs, FileSystem, FixedTime, OpenMode, SeekFrom, Vfs};

const SECTOR: usize = 512;
/// 16 MiB partition (FAT16 with 2 KiB clusters), starting at sector 2048.
const PART_START: usize = 2048;
const PART_SECTORS: usize = 16 * 1024 * 1024 / SECTOR;

#[derive(Clone)]
struct RamDisk(Rc<RefCell<Vec<u8>>>);

impl BlockDevice for RamDisk {
    type Error = ();
    fn read(&self, blocks: &mut [Block], start: BlockIdx) -> Result<(), ()> {
        let d = self.0.borrow();
        for (i, b) in blocks.iter_mut().enumerate() {
            let o = (start.0 as usize + i) * SECTOR;
            b.contents.copy_from_slice(d.get(o..o + SECTOR).ok_or(())?);
        }
        Ok(())
    }
    fn write(&self, blocks: &[Block], start: BlockIdx) -> Result<(), ()> {
        let mut d = self.0.borrow_mut();
        for (i, b) in blocks.iter().enumerate() {
            let o = (start.0 as usize + i) * SECTOR;
            d.get_mut(o..o + SECTOR).ok_or(())?.copy_from_slice(&b.contents);
        }
        Ok(())
    }
    fn num_blocks(&self) -> Result<BlockCount, ()> {
        Ok(BlockCount((self.0.borrow().len() / SECTOR) as u32))
    }
}

/// Formats a FAT16 partition, writes a few files and wraps it in an MBR.
fn image() -> RamDisk {
    let mut part = Cursor::new(vec![0u8; PART_SECTORS * SECTOR]);
    fatfs::format_volume(
        &mut part,
        fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat16),
    )
    .unwrap();
    {
        let fs = fatfs::FileSystem::new(&mut part, fatfs::FsOptions::new()).unwrap();
        let root = fs.root_dir();
        root.create_file("HELLO.TXT")
            .unwrap()
            .write_all(b"Hello FAT")
            .unwrap();
        let img = root.create_dir("IMG").unwrap();
        let mut big = img.create_file("LOGO.QOI").unwrap();
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        big.write_all(&data).unwrap();
        img.create_file("A.BIN").unwrap().write_all(&[7; 3]).unwrap();
    }
    let mut disk = vec![0u8; (PART_START + PART_SECTORS) * SECTOR];
    let e = 446;
    disk[e] = 0x00; // not bootable
    disk[e + 4] = 0x06; // FAT16
    disk[e + 8..e + 12].copy_from_slice(&(PART_START as u32).to_le_bytes());
    disk[e + 12..e + 16].copy_from_slice(&(PART_SECTORS as u32).to_le_bytes());
    disk[510] = 0x55;
    disk[511] = 0xAA;
    disk[PART_START * SECTOR..].copy_from_slice(part.get_ref());
    RamDisk(Rc::new(RefCell::new(disk)))
}

#[test]
fn fat_reads_file_from_image() {
    let fat = FatFs::<_, _, 4>::new(image(), FixedTime::default()).unwrap();
    let mut vfs = Vfs::new();
    vfs.mount('S', Box::new(fat)).unwrap();
    assert_eq!(vfs.read_to_vec("S:/HELLO.TXT").unwrap(), b"Hello FAT");
    assert_eq!(
        vfs.read_to_vec("S:/hello.txt").unwrap(),
        b"Hello FAT",
        "8.3 names are case-insensitive"
    );
    let logo = vfs.read_to_vec("S:/img/logo.qoi").unwrap();
    assert_eq!(logo.len(), 5000);
    assert!(logo.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8));
    let mut f = vfs.open("S:/IMG/LOGO.QOI", OpenMode::Read).unwrap();
    assert_eq!(f.seek(SeekFrom::End(-10)).unwrap(), 4990);
    let mut b = [0; 4];
    f.read_exact(&mut b).unwrap();
    assert_eq!(b[0], (4990 % 251) as u8);
    assert_eq!(f.tell().unwrap(), 4994);
    assert_eq!(f.seek(SeekFrom::Start(6000)).unwrap_err(), Error::InvalidSeek);
    drop(f);
    assert_eq!(vfs.read_to_vec("S:/nope.txt").unwrap_err(), Error::NotFound);
    assert_eq!(
        vfs.read_to_vec("S:/a_very_long_name.text").unwrap_err(),
        Error::InvalidPath
    );
    assert_eq!(
        vfs.open("S:/IMG", OpenMode::Read).unwrap_err(),
        Error::IsADirectory
    );
    assert_eq!(vfs.metadata("S:/IMG/LOGO.QOI").unwrap().size, 5000);
    assert!(vfs.metadata("S:/IMG").unwrap().is_dir);
    assert!(vfs.metadata("S:/").unwrap().is_dir);
}

#[test]
fn fat_read_dir_83_names() {
    let mut fat = FatFs::<_, _, 4>::new(image(), FixedTime::default()).unwrap();
    let mut names = Vec::new();
    fat.read_dir("/", &mut |e| names.push((e.name.to_string(), e.is_dir, e.size)))
        .unwrap();
    assert_eq!(names, [("HELLO.TXT".into(), false, 9), ("IMG".into(), true, 0)]);
    names.clear();
    fat.read_dir("/img", &mut |e| {
        names.push((e.name.to_string(), e.is_dir, e.size));
    })
    .unwrap();
    assert_eq!(
        names,
        [("LOGO.QOI".into(), false, 5000), ("A.BIN".into(), false, 3)]
    );
    assert_eq!(
        fat.read_dir("/HELLO.TXT", &mut |_| {}).unwrap_err(),
        Error::NotADirectory
    );
}

#[test]
fn fat_write_then_read() {
    let disk = image();
    {
        let mut fat = FatFs::<_, _, 2>::new(disk.clone(), FixedTime::default()).unwrap();
        let h = fat.open("/IMG/NEW.DAT", OpenMode::Write).unwrap();
        assert_eq!(fat.write(h, &[42; 3000]).unwrap(), 3000);
        assert_eq!(fat.read(h, &mut [0; 1]).unwrap_err(), Error::PermissionDenied);
        fat.close(h).unwrap();
        let h = fat.open("/IMG/NEW.DAT", OpenMode::Append).unwrap();
        fat.seek(h, SeekFrom::Start(0)).unwrap();
        fat.write(h, b"tail").unwrap();
        let h2 = fat.open("/HELLO.TXT", OpenMode::ReadWrite).unwrap();
        assert_eq!(fat.tell(h2).unwrap(), 0, "read-write starts at the beginning");
        fat.write(h2, b"J").unwrap();
        assert_eq!(
            fat.open("/IMG/A.BIN", OpenMode::Read).unwrap_err(),
            Error::TooManyOpenFiles
        );
        fat.close(h2).unwrap();
        // `h` stays open: dropping the file system closes (and flushes) it.
    }
    let mut fat = FatFs::<_, _, 2>::new(disk.clone(), FixedTime::default()).unwrap();
    let h = fat.open("/IMG/NEW.DAT", OpenMode::Read).unwrap();
    let mut data = vec![0; 4000];
    let mut n = 0;
    loop {
        let r = fat.read(h, &mut data[n..]).unwrap();
        if r == 0 {
            break;
        }
        n += r;
    }
    assert_eq!(n, 3004);
    assert!(data[..3000].iter().all(|&b| b == 42));
    assert_eq!(&data[3000..3004], b"tail");
    fat.close(h).unwrap();
    let mut vfs = Vfs::new();
    vfs.mount('S', Box::new(fat)).unwrap();
    assert_eq!(vfs.read_to_vec("S:/HELLO.TXT").unwrap(), b"Jello FAT");
    // The image stays readable by another FAT implementation.
    drop(vfs);
    let bytes = disk.0.borrow()[PART_START * SECTOR..].to_vec();
    let fs = fatfs::FileSystem::new(Cursor::new(bytes), fatfs::FsOptions::new()).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut fs.root_dir().open_file("HELLO.TXT").unwrap(), &mut s).unwrap();
    assert_eq!(s, "Jello FAT");
}
