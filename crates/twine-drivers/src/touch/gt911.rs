//! Goodix GT911 capacitive touch controller (I2C, 16-bit register addresses).
//!
//! | | |
//! |-|-|
//! | Datasheet | Goodix GT911 datasheet + programming guide |
//! | Address | `0x5D` or `0x14` (selected by the `INT` level during reset) |
//! | Bus speed | I2C up to 400 kHz |
//! | IRQ | `INT`; the default configuration triggers on the falling edge → active low |
//! | Rotation | reports panel coordinates (resolution from its config): use [`TouchTransform`] |
//!
//! A reading reads the status register `0x814E` (bit 7: buffer ready, bits 3:0: number of
//! points). When ready and touched, the first point is read at `0x8150` (X low, X high, Y low,
//! Y high); the status is then cleared by writing `0` — without that the controller stops
//! reporting. When the buffer is not ready the previous state is kept.
//!
//! [`probe`](Gt911::probe) reads the product ID (`0x8140`, `"911"`) and the configuration
//! version (`0x8047`) and logs them.
//!
//! # Wiring
//!
//! | Controller pin | Driver argument |
//! |----------------|-----------------|
//! | `SDA`, `SCL` | `i2c`, an `embedded_hal::i2c::I2c` (address `0x5D` or `0x14`) |
//! | `INT`/`IRQ` | `irq`: `Some(InputPin)` (`+ Wait` for the async wake-up), or `None` to poll |
//! | `RST` | not driven by the driver: hold it high from your firmware; the address is latched from `INT` during reset |
//!
//! ```
//! use twine_drivers::touch::{Gt911, TouchTransform};
//! use twine_drivers::testkit::Recorder;
//! use twine_drivers::NoPin;
//! use twine_hal::{InputDevice, PollHint};
//!
//! let rec = Recorder::new();
//! let touch = Gt911::new(rec.i2c(), None::<NoPin>, TouchTransform::identity(800, 480));
//! assert_eq!(touch.poll_hint(), PollHint::Periodic);
//! ```

use embedded_hal::digital::InputPin;
use embedded_hal::i2c::I2c;
use twine_core::log::{info, trace, warn};
use twine_hal::{InputData, InputDevice, InputKind, PointerData, PollHint};

use super::{IrqState, TouchTransform, irq_touch_common};

/// Default I2C address.
pub const ADDR: u8 = 0x5D;
/// Alternative I2C address.
pub const ADDR_ALT: u8 = 0x14;
/// Status register (buffer ready, number of points).
pub const REG_STATUS: u16 = 0x814E;
/// First point: X low, X high, Y low, Y high.
pub const REG_POINT1: u16 = 0x8150;
/// Product ID (4 ASCII bytes).
pub const REG_PRODUCT_ID: u16 = 0x8140;
/// Configuration version.
pub const REG_CONFIG_VERSION: u16 = 0x8047;

/// GT911 driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Gt911<I2C, IRQ> {
    i2c: I2C,
    addr: u8,
    pub(super) irq: IrqState<IRQ>,
    transform: TouchTransform,
}

impl<I2C, IRQ> Gt911<I2C, IRQ> {
    /// A driver at address [`ADDR`] (`0x5D`); `irq` is the `INT` pin, if wired.
    #[must_use]
    pub fn new(i2c: I2C, irq: Option<IRQ>, transform: TouchTransform) -> Self {
        Self {
            i2c,
            addr: ADDR,
            irq: IrqState::new(irq),
            transform,
        }
    }

    /// Uses another I2C address (e.g. [`ADDR_ALT`]).
    #[must_use]
    pub fn with_address(mut self, addr: u8) -> Self {
        self.addr = addr;
        self
    }

    /// Treats the IRQ line as active high (configuration with rising-edge trigger).
    #[must_use]
    pub fn with_irq_active_high(mut self, active_high: bool) -> Self {
        self.irq.active_high = active_high;
        self
    }

    /// Returns the bus and the IRQ pin.
    #[must_use]
    pub fn release(self) -> (I2C, Option<IRQ>) {
        (self.i2c, self.irq.irq)
    }
}

impl<I2C: I2c, IRQ> Gt911<I2C, IRQ> {
    fn read_reg(&mut self, reg: u16, buf: &mut [u8]) -> Result<(), I2C::Error> {
        self.i2c.write_read(self.addr, &reg.to_be_bytes(), buf)
    }

    fn write_reg(&mut self, reg: u16, val: u8) -> Result<(), I2C::Error> {
        let [h, l] = reg.to_be_bytes();
        self.i2c.write(self.addr, &[h, l, val])
    }

    /// Reads and logs the product ID and configuration version; returns `(id, version)`.
    pub fn probe(&mut self) -> Result<([u8; 4], u8), I2C::Error> {
        let mut id = [0u8; 4];
        self.read_reg(REG_PRODUCT_ID, &mut id)?;
        let mut ver = [0u8; 1];
        self.read_reg(REG_CONFIG_VERSION, &mut ver)?;
        info!(target: "twine::driver", "gt911: product id {:?}, config version {}", id, ver[0]);
        Ok((id, ver[0]))
    }

    /// Reads one report: `Ok(None)` if the buffer is not ready, `Ok(Some(None))` if ready and
    /// not touched, `Ok(Some(Some((x, y))))` for the first point. Clears the status when ready.
    pub fn read_raw(&mut self) -> Result<Option<Option<(u16, u16)>>, I2C::Error> {
        let mut status = [0u8; 1];
        self.read_reg(REG_STATUS, &mut status)?;
        trace!(target: "twine::driver", "gt911 status {:x}", status[0]);
        if status[0] & 0x80 == 0 {
            return Ok(None);
        }
        let point = if status[0] & 0x0F > 0 {
            let mut p = [0u8; 4];
            self.read_reg(REG_POINT1, &mut p)?;
            Some((u16::from_le_bytes([p[0], p[1]]), u16::from_le_bytes([p[2], p[3]])))
        } else {
            None
        };
        self.write_reg(REG_STATUS, 0)?;
        Ok(Some(point))
    }
}

impl<I2C: I2c, IRQ: InputPin> InputDevice for Gt911<I2C, IRQ> {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        if !self.irq.should_read() {
            return InputData::Pointer(self.irq.last);
        }
        let data = match self.read_raw() {
            Ok(Some(Some((x, y)))) => PointerData {
                point: self.transform.apply(i32::from(x), i32::from(y)),
                pressed: true,
            },
            Ok(Some(None)) => self.irq.released(),
            Ok(None) => self.irq.last,
            Err(_) => {
                warn!(target: "twine::driver", "gt911: I2C error");
                self.irq.released()
            }
        };
        InputData::Pointer(self.irq.update("gt911", data))
    }

    fn poll_hint(&self) -> PollHint {
        self.irq.poll_hint()
    }
}

irq_touch_common!(Gt911, "gt911");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder};
    use crate::touch::test_util::Regs;
    use alloc::vec;
    use twine_core::Point;

    #[test]
    fn gt911_clears_status_after_read() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(0x814E, &[0x81]);
        regs.set(0x8150, &[0x2C, 0x01, 0xC8, 0x00]); // x 300, y 200
        let mut t: Gt911<_, crate::NoPin> = Gt911::new(rec.i2c(), None, TouchTransform::identity(800, 480));
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(300, 200),
                pressed: true
            })
        );
        assert_eq!(
            rec.ops(),
            [
                BusOp::I2cWrite {
                    addr: 0x5D,
                    data: vec![0x81, 0x4E]
                },
                BusOp::I2cRead { addr: 0x5D, len: 1 },
                BusOp::I2cWrite {
                    addr: 0x5D,
                    data: vec![0x81, 0x50]
                },
                BusOp::I2cRead { addr: 0x5D, len: 4 },
                BusOp::I2cWrite {
                    addr: 0x5D,
                    data: vec![0x81, 0x4E, 0x00]
                },
            ]
        );
        // Not ready: the state is kept and nothing is cleared.
        regs.set(0x814E, &[0x00]);
        let _ = rec.take_ops();
        assert!(matches!(t.read(), InputData::Pointer(p) if p.pressed));
        assert_eq!(rec.ops().len(), 2);
        // Ready with 0 points: released.
        regs.set(0x814E, &[0x80]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed && p.point == Point::new(300, 200)));
    }

    #[test]
    fn gt911_probe_logs_version() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(0x8140, b"911\0");
        regs.set(0x8047, &[0x41]);
        let mut t: Gt911<_, crate::NoPin> =
            Gt911::new(rec.i2c(), None, TouchTransform::identity(1, 1)).with_address(ADDR_ALT);
        assert_eq!(t.probe().unwrap(), (*b"911\0", 0x41));
        assert_eq!(
            rec.ops()[0],
            BusOp::I2cWrite {
                addr: 0x14,
                data: vec![0x81, 0x40]
            }
        );
        let _ = t.with_irq_active_high(true).release();
    }

    #[test]
    fn gt911_irq_idle_no_traffic() {
        let rec = Recorder::new();
        let mut t = Gt911::new(
            rec.i2c(),
            Some(rec.quiet_pin("int")),
            TouchTransform::identity(1, 1),
        );
        rec.set_level("int", true);
        let _ = t.read();
        assert!(rec.ops().is_empty());
        assert_eq!(t.poll_hint(), PollHint::Interrupt);
        assert_eq!(t.kind(), InputKind::Pointer);
    }
}
