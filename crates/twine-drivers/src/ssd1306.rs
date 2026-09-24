//! SSD1306 monochrome OLED controller (0.49"–1.3" 128 × 64, 128 × 32, 72 × 40 modules) over I2C
//! or SPI.
//!
//! | | |
//! |-|-|
//! | Datasheet | Solomon Systech SSD1306 Rev 1.1 |
//! | Bus | I2C ([`I2cInterface`](crate::interface::I2cInterface), address `0x3C`/`0x3D`, up to 400 kHz) or 4-wire SPI ([`SpiInterface`](crate::interface::SpiInterface), up to 10 MHz) |
//! | Format | [`ColorFormat::I1`] (1 = lit), `align = 8` (whole 8-row pages) |
//! | Quirks | 72 × 40 modules (SSD1306B) need the internal current reference and a 28-column offset |
//!
//! # Init
//!
//! The command sequence of the `ssd1306` crate (MIT/Apache-2.0,
//! <https://github.com/rust-embedded-community/ssd1306>) with its default brightness, sent as one
//! command stream: display off, clock `0x80`, multiplex `h − 1`, offset 0, start line 0, charge
//! pump on, horizontal addressing, COM pins (`0x12` for 64/40 rows, `0x02` for 32 rows),
//! segment remap / COM scan direction for the rotation, pre-charge `0x21`, contrast `0x5F`,
//! VCOMH auto (`0x40`), resume from RAM, normal (not inverted), scrolling off, display on.
//!
//! # Rotation
//!
//! `Deg0` and `Deg180` are done by the controller (segment remap + COM scan direction,
//! `hw_rotation = true`). `Deg90`/`Deg270` are not supported by the hardware: the controller
//! stays at `Deg0`, [`DisplayInfo::hw_rotation`] is `false` and the engine rotates each chunk
//! in software before flushing.
//!
//! # Flush
//!
//! The engine renders `I1` in whole pages (`align = 8`). `begin_flush` sets the column and page
//! address window and converts the chunk page by page ([`mono::i1_to_page`](crate::mono::i1_to_page))
//! through a 128-byte scratch buffer — no allocation.
//!
//! # Wiring
//!
//! | Panel pin | Driver argument |
//! |-----------|-----------------|
//! | `SDA`, `SCL` (I2C modules) | `I2cInterface::new(i2c, 0x3C)` with an `embedded_hal(_async)::i2c::I2c` |
//! | `SCK`, `MOSI`, `CS`, `DC` (SPI modules) | `SpiInterface::new(spi_device, dc)` |
//! | `RES` | not driven by the driver: hold it high (or pulse it low once) from your firmware |
//!
//! ```
//! use twine_core::{ColorFormat, Rect};
//! use twine_drivers::interface::I2cInterface;
//! use twine_drivers::ssd1306::{Ssd1306, Ssd1306Size};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};
//!
//! let rec = Recorder::new();
//! let mut oled = Ssd1306::new(I2cInterface::new(rec.i2c(), 0x3C), Ssd1306Size::Size128x64, Rotation::Deg0).unwrap();
//! let info = oled.info();
//! assert_eq!((info.width, info.height, info.format, info.align), (128, 64, ColorFormat::I1, 8));
//! let buf = DrawBufferMem::new(Box::leak(Box::new([0u8; 16 * 8])));
//! oled.begin_flush(Rect::from_xywh(0, 8, 128, 8), buf).unwrap();
//! assert!(oled.poll_flush().is_some());
//! ```
//!
//! Pair it with the monochrome theme (`MonoTheme`) of the `twine` crate, e.g.
//! `Ui::builder(oled).theme(MonoTheme::new())`, and enable `I1` rendering (feature `color-i1`).

use heapless::{Deque, Vec};
use twine_core::log::{error, trace, warn};
use twine_core::{ColorFormat, Rect};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, Rotation};

use crate::interface::DcsInterface;
use crate::mono::i1_to_page;
pub use crate::mono::{OLED_DPI, OledError};

// NOTE(P21.S03): the engine-side test `engine_rounds_area_to_8_rows` (EngineHarness with an
// `align = 8` MemoryDisplay) and the `mono_oled` simulator example belong to the engine and
// examples crates and are added there.

/// Size of an SSD1306 module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Ssd1306Size {
    /// 128 × 64 (0.96", 1.3").
    Size128x64,
    /// 128 × 32 (0.91").
    Size128x32,
    /// 72 × 40 (0.42", SSD1306B), columns 28…99 of the controller memory.
    Size72x40,
}

impl Ssd1306Size {
    /// `(width, height)` in pixels.
    #[must_use]
    pub const fn dimensions(self) -> (u16, u16) {
        match self {
            Ssd1306Size::Size128x64 => (128, 64),
            Ssd1306Size::Size128x32 => (128, 32),
            Ssd1306Size::Size72x40 => (72, 40),
        }
    }

    /// First visible column in the 128-column memory.
    #[must_use]
    pub const fn column_offset(self) -> u8 {
        match self {
            Ssd1306Size::Size72x40 => 28,
            _ => 0,
        }
    }

    /// `COM` pins hardware configuration (`0xDA` parameter).
    #[must_use]
    pub const fn com_pins(self) -> u8 {
        match self {
            Ssd1306Size::Size128x32 => 0x02,
            _ => 0x12,
        }
    }
}

/// `(segment remap, COM scan)` commands for the rotation the controller performs.
const fn orientation(rotation: Rotation) -> [u8; 2] {
    match rotation {
        Rotation::Deg180 => [0xA0, 0xC0],
        _ => [0xA1, 0xC8],
    }
}

/// The init command stream (see the module docs).
fn init_bytes(size: Ssd1306Size, rotation: Rotation) -> Vec<u8, 32> {
    let (_, h) = size.dimensions();
    let [seg, com] = orientation(rotation);
    let mut v: Vec<u8, 32> = Vec::new();
    let mut push = |bytes: &[u8]| {
        // Capacity 32 holds the longest sequence (28 bytes).
        let _ = v.extend_from_slice(bytes);
    };
    push(&[
        0xAE,
        0xD5,
        0x80,
        0xA8,
        (h - 1) as u8,
        0xD3,
        0x00,
        0x40,
        0x8D,
        0x14,
        0x20,
        0x00,
    ]);
    push(&[0xDA, size.com_pins()]);
    if size == Ssd1306Size::Size72x40 {
        push(&[0xAD, 0x30]); // internal IREF, 30 µA (SSD1306B)
    }
    push(&[
        seg, com, 0xD9, 0x21, 0x81, 0x5F, 0xDB, 0x40, 0xA4, 0xA6, 0x2E, 0xAF,
    ]);
    v
}

/// Shared checks of a flush; returns `(stride, pages)`.
fn check_area(native: (u16, u16), area: Rect, len: usize) -> Option<(usize, core::ops::Range<usize>)> {
    let panel = Rect::new(0, 0, i32::from(native.0), i32::from(native.1));
    if area.is_empty() || !panel.contains_rect(&area) || area.y0 % 8 != 0 || area.y1 % 8 != 0 {
        return None;
    }
    let stride = ColorFormat::I1.stride(area.width() as u32) as usize;
    if len < stride * area.height() as usize {
        return None;
    }
    Some((stride, (area.y0 / 8) as usize..(area.y1 / 8) as usize))
}

fn oled_info(native: (u16, u16), rotation: Rotation, dpi: u16) -> DisplayInfo {
    let hw = !rotation.swaps_axes();
    let (w, h) = if hw { native } else { (native.1, native.0) };
    DisplayInfo::new(w, h, ColorFormat::I1)
        .with_rotation(rotation)
        .with_hw_rotation(hw)
        .with_align(8)
        .with_dpi(dpi)
}

/// SSD1306 driver (blocking) implementing [`DisplayDriver`].
pub struct Ssd1306<I> {
    iface: I,
    size: Ssd1306Size,
    rotation: Rotation,
    pending: Deque<DrawBufferMem, 2>,
    scratch: [u8; 128],
}

impl<I> core::fmt::Debug for Ssd1306<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ssd1306")
            .field("size", &self.size)
            .field("rotation", &self.rotation)
            .finish_non_exhaustive()
    }
}

impl<I: DcsInterface> Ssd1306<I> {
    /// Initializes the controller (see the module docs).
    pub fn new(iface: I, size: Ssd1306Size, rotation: Rotation) -> Result<Self, OledError<I::Error>> {
        let mut this = Self {
            iface,
            size,
            rotation,
            pending: Deque::new(),
            scratch: [0; 128],
        };
        this.commands(&init_bytes(size, rotation))?;
        twine_core::log::info!(target: "twine::driver", "SSD1306 {}x{} initialized", size.dimensions().0, size.dimensions().1);
        Ok(this)
    }

    fn commands(&mut self, bytes: &[u8]) -> Result<(), OledError<I::Error>> {
        self.iface.command_bytes(bytes).map_err(|e| {
            warn!(target: "twine::driver", "ssd1306: command failed");
            OledError::Interface(e)
        })
    }

    /// Sets the contrast (`0x81`), 0…255.
    pub fn set_contrast(&mut self, contrast: u8) -> Result<(), OledError<I::Error>> {
        self.commands(&[0x81, contrast])
    }

    /// Turns the panel on or off (`0xAF` / `0xAE`, RAM kept; off saves most of the power).
    pub fn display_on(&mut self, on: bool) -> Result<(), OledError<I::Error>> {
        self.commands(&[if on { 0xAF } else { 0xAE }])
    }

    /// Inverts all pixels in hardware (`0xA7` / `0xA6`).
    pub fn invert(&mut self, on: bool) -> Result<(), OledError<I::Error>> {
        self.commands(&[if on { 0xA7 } else { 0xA6 }])
    }

    fn flush_now(&mut self, area: Rect, buf: &[u8]) -> Result<(), OledError<I::Error>> {
        let Some((stride, pages)) = check_area(self.size.dimensions(), area, buf.len()) else {
            error!(target: "twine::driver", "ssd1306: bad flush area {:?} ({} bytes)", area, buf.len());
            return Err(OledError::BadArea);
        };
        trace!(target: "twine::driver", "ssd1306: flush {:?}", area);
        let off = self.size.column_offset();
        let (x0, x1) = (area.x0 as u8 + off, (area.x1 - 1) as u8 + off);
        self.commands(&[0x21, x0, x1, 0x22, pages.start as u8, (pages.end - 1) as u8])?;
        let w = area.width() as usize;
        let first = pages.start;
        for p in pages {
            i1_to_page(buf, stride, w, p - first, &mut self.scratch);
            let data = &self.scratch[..w];
            self.iface.write_pixels(data).map_err(|e| {
                warn!(target: "twine::driver", "ssd1306: data write failed");
                OledError::Interface(e)
            })?;
        }
        Ok(())
    }

    /// Returns the interface.
    #[must_use]
    pub fn release(self) -> I {
        self.iface
    }
}

impl<I: DcsInterface> DisplayDriver for Ssd1306<I> {
    type Error = OledError<I::Error>;

    fn info(&self) -> DisplayInfo {
        oled_info(self.size.dimensions(), self.rotation, OLED_DPI)
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        let r = self.flush_now(area, buf.as_slice());
        if self.pending.push_back(buf).is_err() {
            error!(target: "twine::driver", "ssd1306: more than 2 buffers in flight; buffer dropped");
        }
        r
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.pending.pop_front()
    }
}

#[cfg(feature = "async")]
pub use self::asynch::AsyncSsd1306;

#[cfg(feature = "async")]
mod asynch {
    use twine_core::Rect;
    use twine_core::log::{error, warn};
    use twine_hal::{AsyncDisplayDriver, DisplayInfo, Rotation};

    use super::{OLED_DPI, OledError, Ssd1306Size, check_area, init_bytes, oled_info};
    use crate::interface::AsyncDcsInterface;
    use crate::mono::i1_to_page;

    /// Async SSD1306 driver (feature `async`); sends the same bytes as
    /// [`Ssd1306`](super::Ssd1306).
    pub struct AsyncSsd1306<I> {
        iface: I,
        size: Ssd1306Size,
        rotation: Rotation,
        scratch: [u8; 128],
    }

    impl<I> core::fmt::Debug for AsyncSsd1306<I> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("AsyncSsd1306")
                .field("size", &self.size)
                .field("rotation", &self.rotation)
                .finish_non_exhaustive()
        }
    }

    impl<I: AsyncDcsInterface> AsyncSsd1306<I> {
        /// Initializes the controller.
        pub async fn new(
            iface: I,
            size: Ssd1306Size,
            rotation: Rotation,
        ) -> Result<Self, OledError<I::Error>> {
            let mut this = Self {
                iface,
                size,
                rotation,
                scratch: [0; 128],
            };
            this.commands(&init_bytes(size, rotation)).await?;
            Ok(this)
        }

        async fn commands(&mut self, bytes: &[u8]) -> Result<(), OledError<I::Error>> {
            match self.iface.command_bytes(bytes).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    warn!(target: "twine::driver", "ssd1306: command failed");
                    Err(OledError::Interface(e))
                }
            }
        }

        /// Sets the contrast (`0x81`).
        pub async fn set_contrast(&mut self, contrast: u8) -> Result<(), OledError<I::Error>> {
            self.commands(&[0x81, contrast]).await
        }

        /// Turns the panel on or off.
        pub async fn display_on(&mut self, on: bool) -> Result<(), OledError<I::Error>> {
            self.commands(&[if on { 0xAF } else { 0xAE }]).await
        }

        /// Returns the interface.
        #[must_use]
        pub fn release(self) -> I {
            self.iface
        }
    }

    impl<I: AsyncDcsInterface> AsyncDisplayDriver for AsyncSsd1306<I> {
        type Error = OledError<I::Error>;

        fn info(&self) -> DisplayInfo {
            oled_info(self.size.dimensions(), self.rotation, OLED_DPI)
        }

        async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), Self::Error> {
            let Some((stride, pages)) = check_area(self.size.dimensions(), area, buf.len()) else {
                error!(target: "twine::driver", "ssd1306: bad flush area {:?}", area);
                return Err(OledError::BadArea);
            };
            let off = self.size.column_offset();
            let (x0, x1) = (area.x0 as u8 + off, (area.x1 - 1) as u8 + off);
            self.commands(&[0x21, x0, x1, 0x22, pages.start as u8, (pages.end - 1) as u8])
                .await?;
            let w = area.width() as usize;
            let first = pages.start;
            for p in pages {
                i1_to_page(buf, stride, w, p - first, &mut self.scratch);
                if let Err(e) = self.iface.write_pixels(&self.scratch[..w]).await {
                    warn!(target: "twine::driver", "ssd1306: data write failed");
                    return Err(OledError::Interface(e));
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{I2cInterface, SpiInterface};
    use crate::mock::{BusOp, Recorder, RecordingI2c, block_on};
    use alloc::boxed::Box;
    use alloc::vec;
    use alloc::vec::Vec as AVec;

    fn i2c_dut(size: Ssd1306Size, rot: Rotation) -> (Recorder, Ssd1306<I2cInterface<RecordingI2c>>) {
        let rec = Recorder::new();
        let d = Ssd1306::new(I2cInterface::new(rec.i2c(), 0x3C), size, rot).unwrap();
        (rec, d)
    }

    fn buf(bytes: AVec<u8>) -> DrawBufferMem {
        DrawBufferMem::new(Box::leak(bytes.into_boxed_slice()))
    }

    #[test]
    fn ssd1306_init_128x64_bytes() {
        let (rec, _) = i2c_dut(Ssd1306Size::Size128x64, Rotation::Deg0);
        assert_eq!(
            rec.ops(),
            [BusOp::I2cWrite {
                addr: 0x3C,
                data: vec![
                    0x00, // control: command stream
                    0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xDA, 0x12, 0xA1,
                    0xC8, 0xD9, 0x21, 0x81, 0x5F, 0xDB, 0x40, 0xA4, 0xA6, 0x2E, 0xAF,
                ],
            }]
        );
    }

    #[test]
    fn ssd1306_init_128x32_com_pins_0x02() {
        let (rec, d) = i2c_dut(Ssd1306Size::Size128x32, Rotation::Deg180);
        let BusOp::I2cWrite { data, .. } = &rec.ops()[0] else {
            panic!()
        };
        assert_eq!(&data[4..6], &[0xA8, 0x1F]);
        assert_eq!(&data[13..17], &[0xDA, 0x02, 0xA0, 0xC0]);
        let info = d.info();
        assert_eq!((info.width, info.height, info.hw_rotation), (128, 32, true));
    }

    #[test]
    fn ssd1306_72x40_iref_and_offset() {
        let (rec, mut d) = i2c_dut(Ssd1306Size::Size72x40, Rotation::Deg0);
        let BusOp::I2cWrite { data, .. } = &rec.ops()[0] else {
            panic!()
        };
        assert_eq!(&data[13..17], &[0xDA, 0x12, 0xAD, 0x30]);
        let _ = rec.take_ops();
        d.begin_flush(Rect::from_xywh(0, 0, 72, 8), buf(vec![0; 9 * 8]))
            .unwrap();
        assert_eq!(
            rec.ops()[0],
            BusOp::I2cWrite {
                addr: 0x3C,
                data: vec![0x00, 0x21, 28, 99, 0x22, 0, 0]
            }
        );
    }

    #[test]
    fn flush_converts_pages() {
        let (rec, mut d) = i2c_dut(Ssd1306Size::Size128x64, Rotation::Deg0);
        let _ = rec.take_ops();
        // 16 × 16 chunk at (8, 16) with pixel (3, 10) set → page 3, column 11, bit 2.
        let mut px = vec![0u8; 2 * 16];
        px[10 * 2] = 0b0001_0000;
        d.begin_flush(Rect::from_xywh(8, 16, 16, 16), buf(px)).unwrap();
        let mut page1 = vec![0x40];
        page1.extend([0u8; 16]);
        page1[1 + 3] = 1 << 2;
        let mut page0 = vec![0x40];
        page0.extend([0u8; 16]);
        assert_eq!(
            rec.ops(),
            [
                BusOp::I2cWrite {
                    addr: 0x3C,
                    data: vec![0x00, 0x21, 8, 23, 0x22, 2, 3]
                },
                BusOp::I2cWrite {
                    addr: 0x3C,
                    data: page0
                },
                BusOp::I2cWrite {
                    addr: 0x3C,
                    data: page1
                },
            ]
        );
        assert!(d.poll_flush().is_some());
    }

    #[test]
    fn bad_areas() {
        let (_, mut d) = i2c_dut(Ssd1306Size::Size128x64, Rotation::Deg0);
        for area in [
            Rect::from_xywh(0, 4, 8, 8),
            Rect::from_xywh(0, 0, 8, 12),
            Rect::from_xywh(120, 0, 16, 8),
        ] {
            assert_eq!(d.begin_flush(area, buf(vec![0; 64])), Err(OledError::BadArea));
            assert!(d.poll_flush().is_some());
        }
        assert_eq!(
            d.begin_flush(Rect::from_xywh(0, 0, 16, 16), buf(vec![0; 31])),
            Err(OledError::BadArea)
        );
    }

    #[test]
    fn software_rotation_for_90_and_270() {
        let (rec, d) = i2c_dut(Ssd1306Size::Size128x64, Rotation::Deg90);
        let info = d.info();
        assert_eq!(
            (info.width, info.height, info.hw_rotation, info.rotation),
            (64, 128, false, Rotation::Deg90)
        );
        // The controller itself stays in its Deg0 orientation.
        let BusOp::I2cWrite { data, .. } = &rec.ops()[0] else {
            panic!()
        };
        assert_eq!(&data[15..17], &[0xA1, 0xC8]);
    }

    #[test]
    fn spi_variant_and_misc() {
        let rec = Recorder::new();
        let mut d = Ssd1306::new(
            SpiInterface::new(rec.spi(), rec.quiet_pin("dc")),
            Ssd1306Size::Size128x32,
            Rotation::Deg0,
        )
        .unwrap();
        assert_eq!(rec.ops()[0], BusOp::Cmd(0xAE));
        assert_eq!(rec.spi_transactions(), 1);
        let _ = rec.take_ops();
        d.set_contrast(0x10).unwrap();
        d.display_on(false).unwrap();
        d.invert(true).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::Cmd(0x81),
                BusOp::Cmd(0x10),
                BusOp::Cmd(0xAE),
                BusOp::Cmd(0xA7)
            ]
        );
        let _ = d.release();
    }

    #[test]
    fn async_same_bytes() {
        use twine_hal::AsyncDisplayDriver;
        let (a, mut d) = i2c_dut(Ssd1306Size::Size128x64, Rotation::Deg180);
        let px = vec![0x5Au8; 16 * 16];
        d.begin_flush(Rect::from_xywh(0, 0, 128, 16), buf(px.clone()))
            .unwrap();
        let b = Recorder::new();
        block_on(async {
            let mut ad = AsyncSsd1306::new(
                I2cInterface::new(b.i2c(), 0x3C),
                Ssd1306Size::Size128x64,
                Rotation::Deg180,
            )
            .await
            .unwrap();
            assert_eq!(ad.info(), d.info());
            ad.flush(Rect::from_xywh(0, 0, 128, 16), &px).await.unwrap();
            assert_eq!(
                ad.flush(Rect::from_xywh(0, 1, 128, 16), &px).await,
                Err(OledError::BadArea)
            );
        });
        assert_eq!(a.ops(), b.ops());
    }
}
