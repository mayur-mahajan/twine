//! [`QspiInterface`]: quad-SPI transport of AMOLED controllers (CO5300, SH8601, RM67162, …).
//!
//! QSPI panels have no DC pin. Every transaction starts with an 8-bit instruction and a 24-bit
//! address, both on one data line; the address carries the DCS command in its middle byte:
//!
//! | Purpose | Instruction | Address | Data |
//! |---------|-------------|---------|------|
//! | register write (command + parameters) | `0x02` | `0x00, cmd, 0x00` | parameters, 1 line |
//! | pixel write (`RAMWR`) | `0x32` | `0x00, 0x2C, 0x00` | pixels, 4 lines |
//! | pixel write, continued (`RAMWRC`) | `0x32` | `0x00, 0x3C, 0x00` | pixels, 4 lines |
//!
//! This is the framing of the vendors' reference code (Espressif `esp_lcd` QSPI panel IO,
//! Waveshare and `LilyGO` board support packages). The MCU side is a [`QspiBus`] (blocking) or
//! `AsyncQspiBus` (feature `async`) implemented by the firmware over its HAL's half-duplex
//! SPI (on ESP32-S3: `esp_hal::spi::master::SpiDmaBus::half_duplex_write` with a single-line
//! command/address phase and `DataMode::Quad` data).
//!
//! ```
//! use twine_drivers::interface::{DcsInterface, QspiInterface};
//! use twine_drivers::testkit::{BusOp, Recorder};
//!
//! let rec = Recorder::new();
//! let mut iface = QspiInterface::new(rec.qspi());
//! iface.command(0x51, &[0xFF]).unwrap(); // brightness
//! iface.command(0x2C, &[]).unwrap(); // RAMWR: sent with the pixels
//! iface.write_pixels(&[0; 8]).unwrap();
//! assert_eq!(
//!     rec.ops(),
//!     [
//!         BusOp::QspiCmd { instr: 0x02, addr: 0x00_51_00, data: vec![0xFF] },
//!         BusOp::QspiPixels { instr: 0x32, addr: 0x00_2C_00, len: 8 },
//!     ]
//! );
//! ```

use twine_core::log::trace;

use super::DcsInterface;

/// QSPI instructions and DCS commands of the panel framing.
pub mod opcode {
    /// Register write: command in the address, parameters on one line.
    pub const WRITE_CMD: u8 = 0x02;
    /// Pixel write: `RAMWR`/`RAMWRC` in the address, pixels on four lines.
    pub const WRITE_PIXELS: u8 = 0x32;
    /// DCS memory write (start at the window origin).
    pub const RAMWR: u8 = 0x2C;
    /// DCS memory write continue.
    pub const RAMWRC: u8 = 0x3C;
}

/// Splits a QSPI write for a bus whose transactions carry at most `max` data bytes: returns
/// `(address, bytes)` per transaction. Register writes are never split; a pixel write
/// ([`opcode::WRITE_PIXELS`]) is cut into pieces of `max` rounded down to an even count (no
/// RGB565 pixel is cut), and every piece after the first uses `RAMWRC` so the panel continues
/// where the previous one stopped.
///
/// ```
/// use twine_drivers::interface::qspi::{opcode, split_write};
///
/// let data = [0u8; 10];
/// let parts: Vec<_> = split_write(opcode::WRITE_PIXELS, 0x2C00, &data, 5).map(|(a, d)| (a, d.len())).collect();
/// assert_eq!(parts, [(0x2C00, 4), (0x3C00, 4), (0x3C00, 2)]);
/// ```
pub fn split_write(instr: u8, addr: u32, data: &[u8], max: usize) -> impl Iterator<Item = (u32, &[u8])> {
    let step = if instr == opcode::WRITE_PIXELS {
        (max & !1).max(2)
    } else {
        usize::MAX
    };
    let first = core::iter::once((addr, &data[..data.len().min(step)]));
    let rest = data
        .get(step..)
        .unwrap_or(&[])
        .chunks(step.min(data.len()).max(1))
        .map(|c| (u32::from(opcode::RAMWRC) << 8, c));
    first.chain(rest)
}

/// Width of a transaction's data phase (instruction and address always use one line).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum QspiLines {
    /// Data on `D0` only (register writes).
    Single,
    /// Data on `D0`–`D3` (pixel writes).
    Quad,
}

/// A blocking quad-SPI bus with chip select: one call is one transaction.
pub trait QspiBus {
    /// The bus error type.
    type Error: core::fmt::Debug;

    /// Writes `instr` (8 bits) and `addr` (low 24 bits, most significant byte first) on one
    /// line, then `data` on `lines`, with chip select asserted for the whole transaction.
    fn write(&mut self, instr: u8, addr: u32, data: &[u8], lines: QspiLines) -> Result<(), Self::Error>;
}

/// An async quad-SPI bus (feature `async`); same semantics as [`QspiBus`]. The future starts
/// the transfer (DMA) on its first poll.
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub trait AsyncQspiBus {
    /// The bus error type.
    type Error: core::fmt::Debug;

    /// See [`QspiBus::write`].
    async fn write(&mut self, instr: u8, addr: u32, data: &[u8], lines: QspiLines)
    -> Result<(), Self::Error>;
}

impl<T: QspiBus + ?Sized> QspiBus for &mut T {
    type Error = T::Error;
    fn write(&mut self, instr: u8, addr: u32, data: &[u8], lines: QspiLines) -> Result<(), Self::Error> {
        T::write(self, instr, addr, data, lines)
    }
}

/// The DCS transport over a [`QspiBus`] (see the module docs for the framing).
///
/// `RAMWR` is not sent on its own: [`command`](DcsInterface::command)`(RAMWR, &[])` arms it
/// and the next [`write_pixels`](DcsInterface::write_pixels) carries it in its address. A
/// pixel write that does not follow a `RAMWR` continues the previous one (`RAMWRC`).
#[derive(Debug)]
pub struct QspiInterface<B> {
    bus: B,
    ramwr_pending: bool,
}

impl<B> QspiInterface<B> {
    /// Wraps a QSPI bus.
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            ramwr_pending: false,
        }
    }

    /// Returns the bus.
    #[must_use]
    pub fn release(self) -> B {
        self.bus
    }

    /// The `(instruction, address, lines)` of a command, or `None` for an armed `RAMWR`.
    fn frame_cmd(&mut self, cmd: u8, params: &[u8]) -> Option<(u8, u32)> {
        trace!(target: "twine::driver", "qspi cmd {:x} len {}", cmd, params.len());
        if cmd == opcode::RAMWR && params.is_empty() {
            self.ramwr_pending = true;
            return None;
        }
        self.ramwr_pending = false;
        Some((opcode::WRITE_CMD, u32::from(cmd) << 8))
    }

    fn frame_pixels(&mut self) -> (u8, u32) {
        let c = if core::mem::take(&mut self.ramwr_pending) {
            opcode::RAMWR
        } else {
            opcode::RAMWRC
        };
        (opcode::WRITE_PIXELS, u32::from(c) << 8)
    }
}

impl<B: QspiBus> DcsInterface for QspiInterface<B> {
    type Error = B::Error;

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        match self.frame_cmd(cmd, params) {
            Some((instr, addr)) => self.bus.write(instr, addr, params, QspiLines::Single),
            None => Ok(()),
        }
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let (instr, addr) = self.frame_pixels();
        self.bus.write(instr, addr, data, QspiLines::Quad)
    }
}

#[cfg(feature = "async")]
impl<B: AsyncQspiBus> super::AsyncDcsInterface for QspiInterface<B> {
    type Error = B::Error;

    async fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        match self.frame_cmd(cmd, params) {
            Some((instr, addr)) => self.bus.write(instr, addr, params, QspiLines::Single).await,
            None => Ok(()),
        }
    }

    async fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let (instr, addr) = self.frame_pixels();
        self.bus.write(instr, addr, data, QspiLines::Quad).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use alloc::vec;

    #[test]
    fn qspi_register_write_framing() {
        let rec = Recorder::new();
        let mut i = QspiInterface::new(rec.qspi());
        i.command(0x2A, &[0x00, 0x16, 0x01, 0xAF]).unwrap();
        i.command(0x11, &[]).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::QspiCmd {
                    instr: 0x02,
                    addr: 0x00_2A00,
                    data: vec![0x00, 0x16, 0x01, 0xAF]
                },
                BusOp::QspiCmd {
                    instr: 0x02,
                    addr: 0x00_1100,
                    data: vec![]
                },
            ]
        );
    }

    #[test]
    fn qspi_pixels_ramwr_then_ramwrc() {
        let rec = Recorder::new();
        let mut i = QspiInterface::new(rec.qspi());
        i.command(0x2C, &[]).unwrap();
        assert!(rec.ops().is_empty(), "RAMWR travels with the pixels");
        i.write_pixels(&[1, 2, 3, 4]).unwrap();
        i.write_pixels(&[5, 6]).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::QspiPixels {
                    instr: 0x32,
                    addr: 0x00_2C00,
                    len: 4
                },
                BusOp::QspiPixels {
                    instr: 0x32,
                    addr: 0x00_3C00,
                    len: 2
                },
            ]
        );
        assert_eq!(rec.pixel_bytes(), [1, 2, 3, 4, 5, 6]);
        // Another command disarms a pending RAMWR.
        let _ = rec.take_ops();
        i.command(0x2C, &[]).unwrap();
        i.command(0x51, &[0x80]).unwrap();
        i.write_pixels(&[0, 0]).unwrap();
        assert_eq!(
            rec.ops()[1],
            BusOp::QspiPixels {
                instr: 0x32,
                addr: 0x00_3C00,
                len: 2
            }
        );
    }

    #[test]
    fn qspi_async_same_frames() {
        let a = Recorder::new();
        let b = Recorder::new();
        let mut i = QspiInterface::new(a.qspi());
        let mut j = QspiInterface::new(b.qspi());
        DcsInterface::command(&mut i, 0x36, &[0x00]).unwrap();
        DcsInterface::command(&mut i, 0x2C, &[]).unwrap();
        DcsInterface::write_pixels(&mut i, &[9; 6]).unwrap();
        b.pending_once();
        block_on(async {
            use crate::interface::AsyncDcsInterface as A;
            A::command(&mut j, 0x36, &[0x00]).await.unwrap();
            A::command(&mut j, 0x2C, &[]).await.unwrap();
            A::write_pixels(&mut j, &[9; 6]).await.unwrap();
        });
        assert_eq!(a.ops(), b.ops());
        assert_eq!(a.pixel_bytes(), b.pixel_bytes());
    }

    #[test]
    fn split_write_pieces() {
        let data = [7u8; 9];
        let p: alloc::vec::Vec<_> = split_write(opcode::WRITE_CMD, 0x2A00, &data, 4)
            .map(|(a, d)| (a, d.len()))
            .collect();
        assert_eq!(p, [(0x2A00, 9)], "register writes are never split");
        let p: alloc::vec::Vec<_> = split_write(opcode::WRITE_PIXELS, 0x2C00, &data, 4)
            .map(|(a, d)| (a, d.len()))
            .collect();
        assert_eq!(p, [(0x2C00, 4), (0x3C00, 4), (0x3C00, 1)]);
        let p: alloc::vec::Vec<_> = split_write(opcode::WRITE_PIXELS, 0x2C00, &[], 4).collect();
        assert_eq!(p, [(0x2C00, &[][..])]);
    }

    #[test]
    fn qspi_error_propagates() {
        let rec = Recorder::new();
        let mut i = QspiInterface::new(rec.qspi());
        rec.fail_next();
        assert!(i.command(0x29, &[]).is_err());
    }
}
