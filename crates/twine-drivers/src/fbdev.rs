//! Linux framebuffer display (`/dev/fbN`), feature `fbdev` (needs `std`; Unix hosts).
//!
//! | | |
//! |-|-|
//! | Device | `/dev/fbN` (opened for writing; the user needs access, e.g. the `video` group) |
//! | Geometry | sysfs `/sys/class/graphics/fbN/`: `modes` (first mode, else `virtual_size`), `bits_per_pixel`, `stride` |
//! | Formats | 16 bpp → [`ColorFormat::Rgb565`], 24 bpp → [`ColorFormat::Rgb888`], 32 bpp → [`ColorFormat::Xrgb8888`] (the usual little-endian layouts; enable the matching `twine/color-*` feature) |
//! | Rotation | in software ([`with_rotation`](FbDev::with_rotation)) |
//!
//! [`FbDev`] is a [`DisplayDriver`]: the engine renders partial chunks into its draw buffers and
//! each flush writes the chunk's rows into the framebuffer device at their offsets (`pwrite`,
//! one call per row, one per chunk when the chunk spans whole rows of the framebuffer). No
//! `mmap`, no `ioctl`, no `unsafe`: the geometry comes from sysfs. Flushes are synchronous
//! (the buffer is back at the next [`poll_flush`](DisplayDriver::poll_flush)); a write error is
//! a flush error (its OS error number is the fault code).
//!
//! The module only uses portable Unix file I/O, so it builds (and is tested, with plain files
//! standing in for the device and sysfs) on every Unix host; it is meant for Linux.
//!
//! ```
//! # let dir = std::env::temp_dir().join(format!("twine-fbdev-doc-{}", std::process::id()));
//! # std::fs::create_dir_all(&dir).unwrap();
//! # std::fs::write(dir.join("modes"), "U:64x32p-0\n").unwrap();
//! # std::fs::write(dir.join("bits_per_pixel"), "16\n").unwrap();
//! # std::fs::write(dir.join("stride"), "128\n").unwrap();
//! # std::fs::write(dir.join("fb"), vec![0u8; 128 * 32]).unwrap();
//! use twine_core::{ColorFormat, Rect};
//! use twine_drivers::fbdev::FbDev;
//! use twine_hal::{DisplayDriver, DrawBufferMem};
//!
//! // On a device: `FbDev::open(0)` (`/dev/fb0` and `/sys/class/graphics/fb0`).
//! let mut fb = FbDev::open_paths(dir.join("fb"), &dir).unwrap();
//! assert_eq!((fb.info().width, fb.info().height, fb.info().format), (64, 32, ColorFormat::Rgb565));
//! let buf = DrawBufferMem::new(Box::leak(vec![0xAB; 4 * 2 * 2].into_boxed_slice()));
//! fb.begin_flush(Rect::from_xywh(8, 1, 4, 2), buf).unwrap();
//! assert!(fb.poll_flush().is_some());
//! let pixels = std::fs::read(dir.join("fb")).unwrap();
//! assert_eq!(&pixels[128 + 16..128 + 24], &[0xAB; 8]); // row 1, x 8..12
//! # std::fs::remove_dir_all(&dir).unwrap();
//! ```

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::string::String;

use twine_core::log::{info, warn};
use twine_core::{ColorFormat, Rect};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, Rotation};

/// A Linux framebuffer device as a [`DisplayDriver`] (see the [module docs](self)).
#[derive(Debug)]
pub struct FbDev {
    file: File,
    info: DisplayInfo,
    /// Bytes per framebuffer line.
    stride: usize,
    /// Bytes per pixel.
    bytes_pp: usize,
    /// The buffer of the last flush, handed back by the next `poll_flush`.
    done: Option<DrawBufferMem>,
}

impl FbDev {
    /// Opens `/dev/fb<index>` with the geometry of `/sys/class/graphics/fb<index>`. Never
    /// panics (every failure is an error).
    ///
    /// ```no_run
    /// // `no_run`: needs a Linux framebuffer device and write access to it.
    /// use twine_drivers::fbdev::FbDev;
    /// use twine_hal::DisplayDriver;
    ///
    /// let fb = FbDev::open(0)?;
    /// println!("{}x{}", fb.info().width, fb.info().height);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Errors
    /// The device cannot be opened for writing, or the sysfs geometry is missing or not
    /// supported (`InvalidData`: a depth other than 16, 24 or 32 bpp, or a line longer than the
    /// stride).
    pub fn open(index: u8) -> io::Result<Self> {
        Self::open_paths(
            std::format!("/dev/fb{index}"),
            std::format!("/sys/class/graphics/fb{index}"),
        )
    }

    /// Opens the framebuffer `device` with the geometry read from the sysfs directory `sysfs`
    /// (`modes` or `virtual_size`, `bits_per_pixel`, `stride`). See the [module docs](self) for
    /// an example. Never panics (every failure is an error).
    ///
    /// # Errors
    /// As [`open`](Self::open).
    pub fn open_paths(device: impl AsRef<Path>, sysfs: impl AsRef<Path>) -> io::Result<Self> {
        let sysfs = sysfs.as_ref();
        let bits = parse_number(&read(sysfs, "bits_per_pixel")?)?;
        let stride = parse_number(&read(sysfs, "stride")?)?;
        let (width, height) = match read(sysfs, "modes").ok().as_deref().and_then(parse_mode) {
            Some(size) => size,
            None => parse_pair(&read(sysfs, "virtual_size")?)?,
        };
        let format = match bits {
            16 => ColorFormat::Rgb565,
            24 => ColorFormat::Rgb888,
            32 => ColorFormat::Xrgb8888,
            _ => return Err(invalid("fbdev: only 16, 24 and 32 bits per pixel are supported")),
        };
        let bytes_pp = bits / 8;
        let (width, height) = (
            u16::try_from(width).map_err(|_| invalid("fbdev: width"))?,
            u16::try_from(height).map_err(|_| invalid("fbdev: height"))?,
        );
        if usize::from(width) * bytes_pp > stride || width == 0 || height == 0 {
            return Err(invalid("fbdev: the line is longer than the stride"));
        }
        let file = OpenOptions::new().write(true).open(device.as_ref())?;
        info!(target: "twine::driver", "fbdev: {}x{} {:?}, stride {}", width, height, format, stride);
        Ok(Self {
            file,
            info: DisplayInfo::new(width, height, format),
            stride,
            bytes_pp,
            done: None,
        })
    }

    /// Turns the logical screen by `rotation` (the engine rotates each chunk in software; the
    /// logical size swaps for 90° and 270°). Never panics.
    ///
    /// ```
    /// # let dir = std::env::temp_dir().join(format!("twine-fbdev-rot-{}", std::process::id()));
    /// # std::fs::create_dir_all(&dir).unwrap();
    /// # std::fs::write(dir.join("virtual_size"), "64,32\n").unwrap();
    /// # std::fs::write(dir.join("bits_per_pixel"), "32\n").unwrap();
    /// # std::fs::write(dir.join("stride"), "256\n").unwrap();
    /// # std::fs::write(dir.join("fb"), vec![0u8; 256 * 32]).unwrap();
    /// use twine_drivers::fbdev::FbDev;
    /// use twine_hal::{DisplayDriver, Rotation};
    ///
    /// let fb = FbDev::open_paths(dir.join("fb"), &dir).unwrap().with_rotation(Rotation::Deg90);
    /// assert_eq!((fb.info().width, fb.info().height), (32, 64));
    /// # std::fs::remove_dir_all(&dir).unwrap();
    /// ```
    #[must_use]
    pub fn with_rotation(mut self, rotation: Rotation) -> Self {
        let (w, h) = self.info.native_size();
        let (w, h) = if rotation.swaps_axes() { (h, w) } else { (w, h) };
        self.info = DisplayInfo {
            width: w,
            height: h,
            ..self.info.with_rotation(rotation).with_hw_rotation(false)
        };
        self
    }

    fn write(&self, area: Rect, pixels: &[u8]) -> io::Result<()> {
        let row = area.width() as usize * self.bytes_pp;
        let rows = area.height() as usize;
        if pixels.len() < row * rows {
            return Err(invalid("fbdev: buffer shorter than the area"));
        }
        let start = area.y0 as usize * self.stride + area.x0 as usize * self.bytes_pp;
        if row == self.stride {
            // Whole framebuffer lines: one write.
            return self.file.write_all_at(&pixels[..row * rows], start as u64);
        }
        for (i, line) in pixels.chunks_exact(row).take(rows).enumerate() {
            self.file.write_all_at(line, (start + i * self.stride) as u64)?;
        }
        Ok(())
    }
}

impl DisplayDriver for FbDev {
    type Error = io::Error;

    fn info(&self) -> DisplayInfo {
        self.info
    }

    /// Writes the rows of `area` (physical panel coordinates) to the device; the buffer comes
    /// back at the next `poll_flush`, also after an error.
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), io::Error> {
        let (w, h) = self.info.native_size();
        let r = if area.is_empty() || !Rect::new(0, 0, i32::from(w), i32::from(h)).contains_rect(&area) {
            Err(invalid("fbdev: area outside the framebuffer"))
        } else {
            self.write(area, buf.as_slice())
        };
        if let Err(e) = &r {
            warn!(target: "twine::driver", "fbdev: flush of {} failed (os error {:?})", area, e.raw_os_error());
        }
        self.done = Some(buf);
        r
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.done.take()
    }

    /// The OS error number (`errno`), `0` for errors without one.
    fn error_code(&self, error: &io::Error) -> u32 {
        error.raw_os_error().map_or(0, i32::unsigned_abs)
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn read(dir: &Path, name: &str) -> io::Result<String> {
    std::fs::read_to_string(dir.join(name))
}

fn parse_number(s: &str) -> io::Result<usize> {
    s.trim()
        .parse()
        .map_err(|_| invalid("fbdev: a sysfs value is not a number"))
}

/// `"W,H"` (`virtual_size`).
fn parse_pair(s: &str) -> io::Result<(usize, usize)> {
    let (w, h) = s
        .trim()
        .split_once(',')
        .ok_or_else(|| invalid("fbdev: virtual_size"))?;
    Ok((parse_number(w)?, parse_number(h)?))
}

/// The size of the first line of `modes`, e.g. `"U:1024x600p-0"`.
fn parse_mode(modes: &str) -> Option<(usize, usize)> {
    let mode = modes.lines().next()?.split_once(':').map_or(modes, |(_, m)| m);
    let (w, rest) = mode.split_once('x')?;
    let h: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((w.trim().parse().ok()?, h.parse().ok()?))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::boxed::Box;
    use std::path::PathBuf;
    use std::vec;

    /// A fake sysfs directory and device file.
    fn fake(name: &str, files: &[(&str, &str)], device_bytes: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(std::format!("twine-fbdev-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (f, v) in files {
            std::fs::write(dir.join(f), v).unwrap();
        }
        std::fs::write(dir.join("fb"), vec![0u8; device_bytes]).unwrap();
        dir
    }

    fn buf(bytes: Vec<u8>) -> DrawBufferMem {
        DrawBufferMem::new(Box::leak(bytes.into_boxed_slice()))
    }

    use std::vec::Vec;

    #[test]
    fn parses_modes_and_virtual_size() {
        assert_eq!(parse_mode("U:1024x600p-0\nU:800x480p-0\n"), Some((1024, 600)));
        assert_eq!(parse_mode("S:640x480i-60"), Some((640, 480)));
        assert_eq!(parse_mode(""), None);
        assert_eq!(parse_pair("800,960\n").unwrap(), (800, 960));
        assert!(parse_pair("800").is_err());
    }

    #[test]
    fn writes_rows_at_their_offsets_with_a_padded_stride() {
        // 8 × 4 at 32 bpp with 40-byte lines (8 bytes of padding); virtual height 8 (two pages).
        let dir = fake(
            "rows",
            &[
                ("virtual_size", "8,8"),
                ("bits_per_pixel", "32"),
                ("stride", "40"),
            ],
            40 * 8,
        );
        let mut fb = FbDev::open_paths(dir.join("fb"), &dir).unwrap();
        assert_eq!((fb.info().width, fb.info().height), (8, 8));
        assert_eq!(fb.info().format, ColorFormat::Xrgb8888);
        let pixels: Vec<u8> = (0..2 * 2 * 4).map(|i| i as u8 + 1).collect();
        fb.begin_flush(Rect::from_xywh(3, 1, 2, 2), buf(pixels.clone()))
            .unwrap();
        assert_eq!(fb.poll_flush().map(|b| b.len()), Some(16));
        assert!(fb.poll_flush().is_none());
        let mem = std::fs::read(dir.join("fb")).unwrap();
        assert_eq!(&mem[40 + 12..40 + 20], &pixels[..8]);
        assert_eq!(&mem[80 + 12..80 + 20], &pixels[8..]);
        assert_eq!(
            mem.iter().filter(|b| **b != 0).count(),
            16,
            "nothing else written"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn whole_lines_are_one_write_and_errors_return_the_buffer() {
        let dir = fake(
            "lines",
            &[("modes", "U:4x2p-0\n"), ("bits_per_pixel", "16"), ("stride", "8")],
            16,
        );
        let mut fb = FbDev::open_paths(dir.join("fb"), &dir).unwrap();
        fb.begin_flush(Rect::from_xywh(0, 0, 4, 2), buf(vec![7; 16]))
            .unwrap();
        let _ = fb.poll_flush();
        assert_eq!(std::fs::read(dir.join("fb")).unwrap(), vec![7; 16]);
        // Outside the framebuffer: an error, and the buffer still comes back.
        let e = fb
            .begin_flush(Rect::from_xywh(2, 0, 4, 1), buf(vec![0; 8]))
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fb.error_code(&e), 0);
        assert!(fb.poll_flush().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unsupported_geometry() {
        let dir = fake(
            "bad",
            &[("virtual_size", "8,8"), ("bits_per_pixel", "8"), ("stride", "8")],
            64,
        );
        assert_eq!(
            FbDev::open_paths(dir.join("fb"), &dir).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::write(dir.join("bits_per_pixel"), "16").unwrap();
        // 8 px × 2 bytes do not fit 8-byte lines.
        assert!(FbDev::open_paths(dir.join("fb"), &dir).is_err());
        assert!(FbDev::open_paths(dir.join("missing"), dir.join("nowhere")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn software_rotation_swaps_the_logical_size() {
        let dir = fake(
            "rot",
            &[
                ("virtual_size", "8,4"),
                ("bits_per_pixel", "16"),
                ("stride", "16"),
            ],
            64,
        );
        let fb = FbDev::open_paths(dir.join("fb"), &dir)
            .unwrap()
            .with_rotation(Rotation::Deg270);
        let info = fb.info();
        assert_eq!(
            (info.width, info.height, info.rotation, info.hw_rotation),
            (4, 8, Rotation::Deg270, false)
        );
        assert_eq!(info.native_size(), (8, 4));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
