//! SH1106 monochrome OLED controller (1.3" 128 × 64 modules) over I2C or SPI.
//!
//! | | |
//! |-|-|
//! | Datasheet | Sino Wealth SH1106 V2.3 |
//! | Bus | I2C (`0x3C`/`0x3D`, up to 400 kHz) or 4-wire SPI (up to 4 MHz) |
//! | Format | [`ColorFormat::I1`](twine_core::ColorFormat), `align = 8` |
//! | Quirks | 132-column memory with the 128 visible columns at offset 2; **no horizontal addressing mode**: each page is written separately (`0xB0 + page`, column low/high nibbles) |
//!
//! Init follows the `sh1106` crate (MIT/Apache-2.0, <https://github.com/jamwaffles/sh1106>):
//! display off, clock `0x80`, multiplex 63, offset 0, start line 0, DC-DC on (`0xAD 0x8B`),
//! segment remap / COM scan direction, COM pins `0x12`, contrast `0x80`, pre-charge `0xF1`,
//! VCOM deselect `0x40`, resume from RAM, normal, display on. Rotation handling is as for the
//! [`ssd1306`](crate::ssd1306) driver (hardware `Deg0`/`Deg180`, software `Deg90`/`Deg270`).
//!
//! ```
//! use twine_drivers::interface::I2cInterface;
//! use twine_drivers::sh1106::Sh1106;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let oled = Sh1106::new(I2cInterface::new(rec.i2c(), 0x3C), Rotation::Deg0).unwrap();
//! assert_eq!((oled.info().width, oled.info().height), (128, 64));
//! ```

use heapless::Deque;
use twine_core::Rect;
use twine_core::log::{error, trace, warn};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, Rotation};

use crate::interface::DcsInterface;
use crate::mono::i1_to_page;
use crate::ssd1306::{OLED_DPI, OledError};

/// Visible size.
const SIZE: (u16, u16) = (128, 64);
/// First visible column in the 132-column memory.
pub const COLUMN_OFFSET: u8 = 2;

fn init_bytes(rotation: Rotation) -> [u8; 23] {
    let [seg, com] = match rotation {
        Rotation::Deg180 => [0xA0, 0xC0],
        _ => [0xA1, 0xC8],
    };
    [
        0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0xAD, 0x8B, seg, com, 0xDA, 0x12, 0x81, 0x80, 0xD9,
        0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0xAF,
    ]
}

/// Page address and column address commands for one page write.
const fn page_cmds(page: u8, col: u8) -> [u8; 3] {
    [0xB0 | (page & 0x0F), col & 0x0F, 0x10 | (col >> 4)]
}

fn check(area: Rect, len: usize) -> Option<(usize, core::ops::Range<usize>)> {
    let panel = Rect::new(0, 0, i32::from(SIZE.0), i32::from(SIZE.1));
    if area.is_empty() || !panel.contains_rect(&area) || area.y0 % 8 != 0 || area.y1 % 8 != 0 {
        return None;
    }
    let stride = (area.width() as usize).div_ceil(8);
    (len >= stride * area.height() as usize)
        .then_some((stride, (area.y0 / 8) as usize..(area.y1 / 8) as usize))
}

fn info(rotation: Rotation) -> DisplayInfo {
    let hw = !rotation.swaps_axes();
    let (w, h) = if hw { SIZE } else { (SIZE.1, SIZE.0) };
    DisplayInfo::new(w, h, twine_core::ColorFormat::I1)
        .with_rotation(rotation)
        .with_hw_rotation(hw)
        .with_align(8)
        .with_dpi(OLED_DPI)
}

/// SH1106 driver (blocking) implementing [`DisplayDriver`].
pub struct Sh1106<I> {
    iface: I,
    rotation: Rotation,
    pending: Deque<DrawBufferMem, 2>,
    scratch: [u8; 128],
}

impl<I> core::fmt::Debug for Sh1106<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sh1106")
            .field("rotation", &self.rotation)
            .finish_non_exhaustive()
    }
}

impl<I: DcsInterface> Sh1106<I> {
    /// Initializes the controller.
    pub fn new(iface: I, rotation: Rotation) -> Result<Self, OledError<I::Error>> {
        let mut this = Self {
            iface,
            rotation,
            pending: Deque::new(),
            scratch: [0; 128],
        };
        this.commands(&init_bytes(rotation))?;
        Ok(this)
    }

    fn commands(&mut self, bytes: &[u8]) -> Result<(), OledError<I::Error>> {
        self.iface.command_bytes(bytes).map_err(|e| {
            warn!(target: "twine::driver", "sh1106: command failed");
            OledError::Interface(e)
        })
    }

    /// Sets the contrast (`0x81`).
    pub fn set_contrast(&mut self, contrast: u8) -> Result<(), OledError<I::Error>> {
        self.commands(&[0x81, contrast])
    }

    /// Turns the panel on or off.
    pub fn display_on(&mut self, on: bool) -> Result<(), OledError<I::Error>> {
        self.commands(&[if on { 0xAF } else { 0xAE }])
    }

    fn flush_now(&mut self, area: Rect, buf: &[u8]) -> Result<(), OledError<I::Error>> {
        let Some((stride, pages)) = check(area, buf.len()) else {
            error!(target: "twine::driver", "sh1106: bad flush area {:?}", area);
            return Err(OledError::BadArea);
        };
        trace!(target: "twine::driver", "sh1106: flush {:?}", area);
        let w = area.width() as usize;
        let col = area.x0 as u8 + COLUMN_OFFSET;
        let first = pages.start;
        for p in pages {
            self.commands(&page_cmds(p as u8, col))?;
            i1_to_page(buf, stride, w, p - first, &mut self.scratch);
            self.iface.write_pixels(&self.scratch[..w]).map_err(|e| {
                warn!(target: "twine::driver", "sh1106: data write failed");
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

impl<I: DcsInterface> DisplayDriver for Sh1106<I> {
    type Error = OledError<I::Error>;

    fn info(&self) -> DisplayInfo {
        info(self.rotation)
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        let r = self.flush_now(area, buf.as_slice());
        if self.pending.push_back(buf).is_err() {
            error!(target: "twine::driver", "sh1106: more than 2 buffers in flight; buffer dropped");
        }
        r
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.pending.pop_front()
    }
}

#[cfg(feature = "async")]
pub use self::asynch::AsyncSh1106;

#[cfg(feature = "async")]
mod asynch {
    use twine_core::Rect;
    use twine_core::log::{error, warn};
    use twine_hal::{AsyncDisplayDriver, DisplayInfo, Rotation};

    use super::{COLUMN_OFFSET, check, info, init_bytes, page_cmds};
    use crate::interface::AsyncDcsInterface;
    use crate::mono::i1_to_page;
    use crate::ssd1306::OledError;

    /// Async SH1106 driver (feature `async`); same bytes as [`Sh1106`](super::Sh1106).
    pub struct AsyncSh1106<I> {
        iface: I,
        rotation: Rotation,
        scratch: [u8; 128],
    }

    impl<I> core::fmt::Debug for AsyncSh1106<I> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("AsyncSh1106")
                .field("rotation", &self.rotation)
                .finish_non_exhaustive()
        }
    }

    impl<I: AsyncDcsInterface> AsyncSh1106<I> {
        /// Initializes the controller.
        pub async fn new(iface: I, rotation: Rotation) -> Result<Self, OledError<I::Error>> {
            let mut this = Self {
                iface,
                rotation,
                scratch: [0; 128],
            };
            this.commands(&init_bytes(rotation)).await?;
            Ok(this)
        }

        async fn commands(&mut self, bytes: &[u8]) -> Result<(), OledError<I::Error>> {
            match self.iface.command_bytes(bytes).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    warn!(target: "twine::driver", "sh1106: command failed");
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

    impl<I: AsyncDcsInterface> AsyncDisplayDriver for AsyncSh1106<I> {
        type Error = OledError<I::Error>;

        fn info(&self) -> DisplayInfo {
            info(self.rotation)
        }

        async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), Self::Error> {
            let Some((stride, pages)) = check(area, buf.len()) else {
                error!(target: "twine::driver", "sh1106: bad flush area {:?}", area);
                return Err(OledError::BadArea);
            };
            let w = area.width() as usize;
            let col = area.x0 as u8 + COLUMN_OFFSET;
            let first = pages.start;
            for p in pages {
                self.commands(&page_cmds(p as u8, col)).await?;
                i1_to_page(buf, stride, w, p - first, &mut self.scratch);
                if let Err(e) = self.iface.write_pixels(&self.scratch[..w]).await {
                    warn!(target: "twine::driver", "sh1106: data write failed");
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
    use crate::interface::I2cInterface;
    use crate::mock::{BusOp, Recorder, block_on};
    use alloc::boxed::Box;
    use alloc::vec;

    #[test]
    fn sh1106_init_bytes() {
        let rec = Recorder::new();
        let _ = Sh1106::new(I2cInterface::new(rec.i2c(), 0x3C), Rotation::Deg0).unwrap();
        let mut expected = vec![0x00];
        expected.extend_from_slice(&[
            0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0xAD, 0x8B, 0xA1, 0xC8, 0xDA, 0x12, 0x81, 0x80,
            0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0xAF,
        ]);
        assert_eq!(
            rec.ops(),
            [BusOp::I2cWrite {
                addr: 0x3C,
                data: expected
            }]
        );
    }

    #[test]
    fn sh1106_column_offset_2() {
        let rec = Recorder::new();
        let mut d = Sh1106::new(I2cInterface::new(rec.i2c(), 0x3C), Rotation::Deg0).unwrap();
        let _ = rec.take_ops();
        let buf = DrawBufferMem::new(Box::leak(vec![0xFFu8; 16 * 16].into_boxed_slice()));
        d.begin_flush(Rect::from_xywh(16, 8, 128 - 16, 16), buf).unwrap();
        let ops = rec.ops();
        // Column 16 + 2 = 18 = 0x12: low nibble 0x02, high nibble 0x11.
        assert_eq!(
            ops[0],
            BusOp::I2cWrite {
                addr: 0x3C,
                data: vec![0x00, 0xB1, 0x02, 0x11]
            }
        );
        assert_eq!(
            ops[2],
            BusOp::I2cWrite {
                addr: 0x3C,
                data: vec![0x00, 0xB2, 0x02, 0x11]
            }
        );
        let BusOp::I2cWrite { data, .. } = &ops[1] else {
            panic!()
        };
        assert_eq!(data.len(), 1 + 112);
        assert!(data[1..].iter().all(|b| *b == 0xFF));
        assert_eq!(ops.len(), 4);
        assert!(d.poll_flush().is_some());
        let buf = DrawBufferMem::new(Box::leak(vec![0u8; 4].into_boxed_slice()));
        assert_eq!(
            d.begin_flush(Rect::from_xywh(0, 0, 8, 8), buf),
            Err(OledError::BadArea)
        );
    }

    #[test]
    fn sh1106_async_same_bytes() {
        use twine_hal::AsyncDisplayDriver;
        let a = Recorder::new();
        let mut d = Sh1106::new(I2cInterface::new(a.i2c(), 0x3C), Rotation::Deg270).unwrap();
        assert_eq!((d.info().width, d.info().hw_rotation), (64, false));
        d.set_contrast(1).unwrap();
        d.display_on(true).unwrap();
        let px = vec![0x81u8; 8 * 8];
        d.begin_flush(
            Rect::from_xywh(0, 56, 64, 8),
            DrawBufferMem::new(Box::leak(px.clone().into_boxed_slice())),
        )
        .unwrap();
        let b = Recorder::new();
        block_on(async {
            let mut ad = AsyncSh1106::new(I2cInterface::new(b.i2c(), 0x3C), Rotation::Deg270)
                .await
                .unwrap();
            ad.set_contrast(1).await.unwrap();
            ad.display_on(true).await.unwrap();
            ad.flush(Rect::from_xywh(0, 56, 64, 8), &px).await.unwrap();
            assert_eq!(ad.info(), info(Rotation::Deg270));
        });
        assert_eq!(a.ops(), b.ops());
    }
}
