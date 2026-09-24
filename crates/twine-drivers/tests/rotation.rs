//! Hardware rotation (`MADCTL`) must turn the picture exactly like the engine's software
//! rotation (`twine_render::rotate_buffer`), for every panel model.
//!
//! A simulated controller decodes `MADCTL`, `CASET`, `RASET` and `RAMWR` and writes the
//! pixels into its frame memory the way MIPI DCS controllers do: the column counter runs
//! fastest; with `MV` the column counter walks memory rows and the page counter memory
//! columns; `MX` mirrors the memory column, `MY` the memory row.
//!
//! For each rotation `r` a logical image `L` (unique pixel values) is flushed at `r`. The
//! reference is `rotate_buffer(L, r)` flushed at `Deg0`: both frame memories must be equal.

use std::boxed::Box;
use std::vec;
use std::vec::Vec;

use twine_core::Rect;
use twine_drivers::NoPin;
use twine_drivers::interface::DcsInterface;
use twine_drivers::mipi_dcs::{Madctl, MipiDcs, PanelSpec, cmd};
use twine_drivers::testkit::Recorder;
use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};

/// A MIPI DCS controller's frame memory.
struct PanelSim {
    ram_w: usize,
    ram_h: usize,
    bpp: usize,
    mem: Vec<u32>,
    madctl: Madctl,
    cols: (usize, usize),
    pages: (usize, usize),
    last_cmd: u8,
}

impl PanelSim {
    fn new(spec: &PanelSpec) -> Self {
        let (ram_w, ram_h) = (usize::from(spec.ram_w), usize::from(spec.ram_h));
        Self {
            ram_w,
            ram_h,
            bpp: spec.bytes_per_pixel(),
            mem: vec![0; ram_w * ram_h],
            madctl: Madctl::EMPTY,
            cols: (0, 0),
            pages: (0, 0),
            last_cmd: 0,
        }
    }

    /// Memory cell of address counter position `(c, p)`.
    fn cell(&self, c: usize, p: usize) -> usize {
        let m = self.madctl;
        let (mut x, mut y) = if m.contains(Madctl::MV) { (p, c) } else { (c, p) };
        if m.contains(Madctl::MX) {
            x = self.ram_w - 1 - x;
        }
        if m.contains(Madctl::MY) {
            y = self.ram_h - 1 - y;
        }
        y * self.ram_w + x
    }
}

fn be_range(p: &[u8]) -> (usize, usize) {
    (
        usize::from(u16::from_be_bytes([p[0], p[1]])),
        usize::from(u16::from_be_bytes([p[2], p[3]])),
    )
}

impl DcsInterface for PanelSim {
    type Error = core::convert::Infallible;

    fn command(&mut self, c: u8, params: &[u8]) -> Result<(), Self::Error> {
        self.last_cmd = c;
        match c {
            cmd::MADCTL => self.madctl = Madctl::from_bits(params[0]),
            cmd::CASET => self.cols = be_range(params),
            cmd::RASET => self.pages = be_range(params),
            _ => {}
        }
        Ok(())
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        assert_eq!(self.last_cmd, cmd::RAMWR);
        let w = self.cols.1 - self.cols.0 + 1;
        for (i, px) in data.chunks_exact(self.bpp).enumerate() {
            let c = self.cols.0 + i % w;
            let p = self.pages.0 + i / w;
            assert!(p <= self.pages.1, "pixel data past the window");
            let v = px.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b));
            let cell = self.cell(c, p);
            self.mem[cell] = v;
        }
        Ok(())
    }
}

/// Flushes the `w × h` image `px` (full screen) at `rot` and returns the frame memory.
fn flush(spec: &'static PanelSpec, rot: Rotation, px: &[u8]) -> Vec<u32> {
    let rec = Recorder::new();
    let mut d = MipiDcs::new(PanelSim::new(spec), None::<NoPin>, spec, rot, &mut rec.delay()).unwrap();
    let info = d.info();
    let buf: &'static mut [u8] = Box::leak(px.to_vec().into_boxed_slice());
    let area = Rect::from_xywh(0, 0, i32::from(info.width), i32::from(info.height));
    d.begin_flush(area, DrawBufferMem::new(buf)).unwrap();
    assert!(d.poll_flush().is_some());
    d.release().0.mem
}

/// A logical `w × h` image with a distinct value per pixel (`bpp` bytes, big-endian).
fn image(w: usize, h: usize, bpp: usize) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let v = (i as u32 + 1).to_be_bytes();
            v[4 - bpp..].to_vec()
        })
        .collect()
}

fn check(spec: &'static PanelSpec) {
    let bpp = spec.bytes_per_pixel();
    let (nw, nh) = (usize::from(spec.native_w), usize::from(spec.native_h));
    for rot in [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ] {
        let (w, h) = spec.logical_size(rot);
        let (w, h) = (usize::from(w), usize::from(h));
        assert_eq!((w, h), if rot.swaps_axes() { (nh, nw) } else { (nw, nh) });
        let logical = image(w, h, bpp);
        let mut upright = vec![0u8; logical.len()];
        twine_render::rotate_buffer(&logical, w * bpp, &mut upright, nw * bpp, w, h, rot, bpp).unwrap();
        let hw = flush(spec, rot, &logical);
        let sw = flush(spec, Rotation::Deg0, &upright);
        assert!(
            hw == sw,
            "{}: hardware {rot:?} (MADCTL {:#04x}) differs from software {rot:?}",
            spec.name,
            spec.madctl_for(rot).bits()
        );
    }
}

#[test]
fn hardware_rotation_matches_software_rotation() {
    for spec in [
        &twine_drivers::ili9341::ILI9341,
        &twine_drivers::ili9342::ILI9342C,
        &twine_drivers::ili9488::ILI9488,
        &twine_drivers::st7789::ST7789,
        &twine_drivers::st7789::ST7789_240X240,
        &twine_drivers::st7789::ST7789_135X240,
        &twine_drivers::st7735::ST7735R_REDTAB,
        &twine_drivers::st7735::ST7735R_BLACKTAB,
        &twine_drivers::st7735::ST7735R_GREENTAB,
        &twine_drivers::st7735::ST7735_80X160,
        &twine_drivers::st7796::ST7796,
        &twine_drivers::gc9a01::GC9A01,
        &twine_drivers::jd9853::JD9853_172X320,
        &twine_drivers::rm67162::RM67162_240X536,
    ] {
        check(spec);
    }
}

/// A mirrored table (the pre-fix ILI9341 `Deg90 = MV`) is caught.
#[test]
fn opposite_direction_is_detected() {
    static WRONG: PanelSpec = twine_drivers::ili9341::ILI9341.with_madctl([
        Madctl::MX,
        Madctl::MV,
        Madctl::MY,
        Madctl::MX.union(Madctl::MY).union(Madctl::MV),
    ]);
    let r = std::panic::catch_unwind(|| check(&WRONG));
    assert!(r.is_err());
}
