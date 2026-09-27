//! File images (`ImageSource::File`) through the engine's virtual file system (feature `fs`):
//! header probing reads only the header, pixels are decoded once and cached, a missing file
//! draws a placeholder, and dropping the cache entry forces a reload.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Mode, harness, with};
use twine_core::ColorFormat;
use twine_engine::{EngineConfig, InvalidateReason, NodeId};
use twine_fs::{DirEntry, Error, FileHandle, FileSystem, MemoryFs, Metadata, OpenMode, SeekFrom, Vfs};
use twine_image::decoders::tbin::encode_tbin;
use twine_image::{HEADER_PROBE_BYTES, ImageHeader, ImageSource};
use twine_style::Align;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{EngineHarness, capture_logs};
use twine_widgets::image::{self, Image, MISSING_PLACEHOLDER_SIZE};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

static LOGO_QOI: &[u8] = include_bytes!("../../../assets/images/twine_logo.qoi");

/// A 40×30 RGB565 `.tbin` file: horizontal stripes.
fn stripes_tbin() -> &'static [u8] {
    let h = ImageHeader::new(ColorFormat::Rgb565, 40, 30);
    let mut data = Vec::new();
    for y in 0..30u16 {
        let px: u16 = if (y / 5) % 2 == 0 { 0xF800 } else { 0x001F };
        for _ in 0..40 {
            data.extend_from_slice(&px.to_le_bytes());
        }
    }
    Box::leak(encode_tbin(h, None, &data).into_boxed_slice())
}

/// A file system counting the bytes read through it.
struct Counting {
    inner: MemoryFs,
    bytes: Rc<Cell<usize>>,
}

impl FileSystem for Counting {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<FileHandle, Error> {
        self.inner.open(path, mode)
    }
    fn read(&mut self, f: FileHandle, buf: &mut [u8]) -> Result<usize, Error> {
        let n = self.inner.read(f, buf)?;
        self.bytes.set(self.bytes.get() + n);
        Ok(n)
    }
    fn write(&mut self, f: FileHandle, buf: &[u8]) -> Result<usize, Error> {
        self.inner.write(f, buf)
    }
    fn seek(&mut self, f: FileHandle, pos: SeekFrom) -> Result<u64, Error> {
        self.inner.seek(f, pos)
    }
    fn tell(&mut self, f: FileHandle) -> Result<u64, Error> {
        self.inner.tell(f)
    }
    fn close(&mut self, f: FileHandle) -> Result<(), Error> {
        self.inner.close(f)
    }
    fn read_dir(&mut self, path: &str, out: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), Error> {
        self.inner.read_dir(path, out)
    }
    fn metadata(&mut self, path: &str) -> Result<Metadata, Error> {
        self.inner.metadata(path)
    }
}

/// A harness with drive `A:` holding `logo.qoi` and `stripes.tbin`; returns the byte counter.
fn setup(mode: Mode, w: u16, h: u16) -> (EngineHarness, Rc<Cell<usize>>) {
    let files: &'static [(&'static str, &'static [u8])] =
        Box::leak(vec![("img/logo.qoi", LOGO_QOI), ("img/stripes.tbin", stripes_tbin())].into_boxed_slice());
    let bytes = Rc::new(Cell::new(0));
    let mut vfs = Vfs::new();
    vfs.mount(
        'A',
        Box::new(Counting {
            inner: MemoryFs::new(files),
            bytes: bytes.clone(),
        }),
    )
    .unwrap();
    let mut harness = harness(w, h, mode).config(EngineConfig {
        image_cache_bytes: 64 * 1024,
        ..EngineConfig::default()
    });
    harness.engine_mut().set_vfs(vfs);
    (harness, bytes)
}

fn image_of(h: &mut EngineHarness, path: &str) -> NodeId {
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(i, Align::Center, 0, 0);
    with(h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::file(path).unwrap());
    });
    i
}

#[test]
fn file_image_header_probe_reads_only_header() {
    let (mut h, bytes) = setup(Mode::Light, 160, 120);
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::file("A:/img/logo.qoi").unwrap());
    });
    assert!(
        bytes.get() <= HEADER_PROBE_BYTES,
        "{} bytes read to probe",
        bytes.get()
    );
    // The header came from the probe: the size is known before any draw.
    h.run_until_idle();
    assert_eq!(h.engine().coords(i).width(), 64);
}

#[test]
fn file_image_decoded_once_then_cached() {
    let (mut h, bytes) = setup(Mode::Light, 160, 120);
    let i = image_of(&mut h, "A:/img/logo.qoi");
    h.run_until_idle();
    assert_eq!(h.engine().coords(i).width(), 64);
    let after_first = bytes.get();
    assert!(after_first >= LOGO_QOI.len(), "decoded from the file");
    for _ in 0..3 {
        h.engine_mut()
            .invalidate(i, InvalidateReason::WidgetSetter("test"));
        h.run_until_idle();
    }
    assert_eq!(bytes.get(), after_first, "redraws are served from the cache");
    assert!(h.engine_mut().vfs_mut().is_some());
}

#[test]
fn cached_file_redraw_reads_nothing_and_allocates_nothing() {
    let (mut h, bytes) = setup(Mode::Light, 160, 120);
    let i = image_of(&mut h, "A:/img/stripes.tbin");
    h.run_until_idle();
    h.engine_mut()
        .invalidate(i, InvalidateReason::WidgetSetter("test"));
    h.run_until_idle(); // warm-up of any lazily grown buffer
    let before = bytes.get();
    let ((), stats) = count_allocs(|| {
        h.engine_mut()
            .invalidate(i, InvalidateReason::WidgetSetter("test"));
        h.run_until_idle();
    });
    assert_eq!(bytes.get(), before, "no file system reads");
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn cache_drop_forces_reload() {
    let (mut h, bytes) = setup(Mode::Light, 160, 120);
    let i = image_of(&mut h, "A:/img/logo.qoi");
    h.run_until_idle();
    let first = bytes.get();
    let src = ImageSource::file("A:/img/logo.qoi").unwrap();
    h.engine_mut().image_cache_invalidate(&src);
    h.engine_mut()
        .invalidate(i, InvalidateReason::WidgetSetter("test"));
    h.run_until_idle();
    assert!(bytes.get() >= first + LOGO_QOI.len(), "read and decoded again");
}

#[test]
fn tbin_file_draws_like_static_pixels() {
    let (mut h, _) = setup(Mode::Light, 80, 60);
    let i = image_of(&mut h, "A:/img/stripes.tbin");
    h.run_until_idle();
    let c = h.engine().coords(i);
    assert_eq!((c.width(), c.height()), (40, 30));
    let red = h.pixel((c.x0 + 5) as u32, (c.y0 + 2) as u32);
    let blue = h.pixel((c.x0 + 5) as u32, (c.y0 + 7) as u32);
    assert!(red.r > 0xF0 && red.b < 0x10, "{red:?}");
    assert!(blue.b > 0xF0 && blue.r < 0x10, "{blue:?}");
}

#[test]
fn missing_file_warns_once_per_path() {
    let (mut h, _) = setup(Mode::Light, 80, 60);
    let ((), logs) = capture_logs(|| {
        let a = image_of(&mut h, "A:/img/missing.qoi");
        let b = image_of(&mut h, "A:/img/missing.qoi");
        h.run_until_idle();
        for i in [a, b] {
            let c = h.engine().coords(i);
            assert_eq!(
                (c.width(), c.height()),
                (MISSING_PLACEHOLDER_SIZE, MISSING_PLACEHOLDER_SIZE)
            );
        }
    });
    let warns = logs
        .iter()
        .filter(|l| l.level == log::Level::Warn && l.message.contains("missing.qoi"))
        .count();
    assert_eq!(warns, 1, "{logs:?}");
}

#[test]
fn missing_file_placeholder_snapshot() {
    for m in Mode::ALL {
        let (mut h, _) = setup(m, 80, 60);
        image_of(&mut h, "A:/img/missing.qoi");
        h.run_until_idle();
        h.assert_snapshot(&format!("image_missing_file_{}", m.suffix()));
    }
}
