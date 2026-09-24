//! `StdFs` over a temporary host directory.

use std::path::PathBuf;

use twine_fs::{Error, FileSystem, OpenMode, StdFs, Vfs};

fn temp_root(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("twine-fs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("hello.txt"), b"hello std").unwrap();
    std::fs::write(d.join("sub/b.bin"), [1, 2, 3]).unwrap();
    d
}

#[test]
fn stdfs_reads_file() {
    let root = temp_root("read");
    let mut vfs = Vfs::new();
    vfs.mount('A', Box::new(StdFs::new(&root))).unwrap();
    assert_eq!(vfs.read_to_vec("A:/hello.txt").unwrap(), b"hello std");
    assert_eq!(vfs.read_to_vec("A:sub/./b.bin").unwrap(), [1, 2, 3]);
    assert_eq!(vfs.read_to_vec("A:/missing").unwrap_err(), Error::NotFound);
    assert_eq!(
        vfs.open("A:/sub", OpenMode::Read).unwrap_err(),
        Error::IsADirectory
    );
    let mut names = Vec::new();
    vfs.read_dir("A:/", &mut |e| names.push((e.name.to_string(), e.is_dir, e.size)))
        .unwrap();
    assert_eq!(names, [("hello.txt".into(), false, 9), ("sub".into(), true, 0)]);
    assert!(vfs.metadata("A:/sub").unwrap().is_dir);
    {
        let mut f = vfs.open("A:/new/../x", OpenMode::Write);
        assert_eq!(f.as_mut().unwrap_err(), &mut Error::InvalidPath);
    }
    {
        let mut f = vfs.open("A:/sub/w.txt", OpenMode::Write).unwrap();
        f.write_all(b"abc").unwrap();
    }
    {
        let mut f = vfs.open("A:/sub/w.txt", OpenMode::Append).unwrap();
        f.write_all(b"def").unwrap();
        assert_eq!(f.read(&mut [0; 2]).unwrap_err(), Error::PermissionDenied);
    }
    assert_eq!(std::fs::read(root.join("sub/w.txt")).unwrap(), b"abcdef");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stdfs_blocks_parent_escape() {
    let root = temp_root("escape");
    let mut fs = StdFs::new(root.join("sub"));
    for p in [
        "../hello.txt",
        "/../hello.txt",
        "a/../../hello.txt",
        "..",
        "x\\..\\y",
    ] {
        assert_eq!(fs.open(p, OpenMode::Read).unwrap_err(), Error::InvalidPath, "{p}");
    }
    assert_eq!(fs.metadata("../hello.txt").unwrap_err(), Error::InvalidPath);
    assert_eq!(fs.read_dir("..", &mut |_| {}).unwrap_err(), Error::InvalidPath);
    assert_eq!(fs.metadata("b.bin").unwrap().size, 3);
    assert_eq!(fs.open_count(), 0);
    let _ = std::fs::remove_dir_all(&root);
}
