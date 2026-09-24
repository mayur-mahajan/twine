//! # twine-fs
//!
//! File system abstraction of the Twine GUI library, the equivalent of LVGL's `lv_fs`: images,
//! fonts and other assets can be loaded from storage through drive-letter paths such as
//! `"A:/img/logo.qoi"`.
//!
//! - [`FileSystem`] is the driver interface (open / read / write / seek / tell / close /
//!   directory listing / metadata), addressed by [`FileHandle`]s.
//! - [`Vfs`] mounts up to [`MAX_DRIVES`] file systems under drive letters and resolves full
//!   paths; [`File`] is an RAII wrapper that closes itself when dropped.
//! - Drivers: [`MemoryFs`] (files compiled into flash plus optional RAM files, always
//!   available), `StdFs` (a host directory, feature `fs-std`) and `FatFs` (FAT volumes on SD
//!   cards through `embedded-sdmmc`, feature `fs-fat`).
//!
//! The crate is `no_std` + `alloc`. Opening files is logged at debug level with target
//! `"twine::fs"`.
//!
//! ```
//! use twine_fs::{MemoryFs, Vfs};
//!
//! static ASSETS: &[(&str, &[u8])] = &[("greeting.txt", b"hello")];
//! let mut vfs = Vfs::new();
//! vfs.mount('A', Box::new(MemoryFs::new(ASSETS))).unwrap();
//! assert_eq!(vfs.read_to_vec("A:/greeting.txt").unwrap(), b"hello");
//! assert!(vfs.read_to_vec("B:/greeting.txt").is_err());
//! ```
//!
//! ## Features
//!
//! `std` (`std` conveniences), `fs-std` (`StdFs`), `fs-fat` (`FatFs`), `log` / `defmt`
//! (logging backend).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "fs-fat")]
mod fat;
mod memory;
#[cfg(feature = "fs-std")]
mod std_fs;
mod types;
mod vfs;

/// The `embedded-sdmmc` crate `FatFs` builds on (its `BlockDevice`, `SdCard`, `TimeSource`).
#[cfg(feature = "fs-fat")]
pub use embedded_sdmmc;
#[cfg(feature = "fs-fat")]
pub use fat::{FatFs, FixedTime};
pub use memory::{MEMORY_FS_MAX_OPEN, MemoryFs};
#[cfg(feature = "fs-std")]
pub use std_fs::StdFs;
pub use types::{DirEntry, Error, FileHandle, FileSystem, Metadata, OpenMode, SeekFrom};
pub use vfs::{File, MAX_DRIVES, Vfs, parse_path};
