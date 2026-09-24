//! Focaltech FT6206 / FT6236 / FT6336 capacitive touch controllers (I2C).
//!
//! | | |
//! |-|-|
//! | Datasheet | Focaltech `FT6x36` datasheet + "`FT6x06`/`FT6x36` application note" (register map) |
//! | Address | `0x38` |
//! | Bus speed | I2C up to 400 kHz |
//! | IRQ | `INT`, active low (pulses while touched in the default trigger mode) |
//! | Rotation | reports panel coordinates: use [`TouchTransform`] |
//!
//! A reading is one `write_read` of 5 registers starting at `TD_STATUS` (`0x02`): number of
//! touch points, then `P1_XH` (event flag in bits 7:6, X bits 11:8), `P1_XL`, `P1_YH`, `P1_YL`.
//! Only the first point is used; gestures are ignored. The `FT5x06` family has the same map
//! ([`Ft5x06`](super::Ft5x06)).
//!
//! ```
//! use twine_drivers::touch::{Ft6x36, TouchTransform};
//! use twine_drivers::testkit::Recorder;
//! use twine_drivers::NoPin;
//! use twine_hal::{InputData, InputDevice};
//!
//! let rec = Recorder::new();
//! let mut touch = Ft6x36::new(rec.i2c(), None::<NoPin>, TouchTransform::identity(240, 320));
//! assert!(matches!(touch.read(), InputData::Pointer(p) if !p.pressed));
//! ```

use embedded_hal::digital::InputPin;
use embedded_hal::i2c::I2c;
use twine_core::log::{trace, warn};
use twine_hal::{InputData, InputDevice, InputKind, PointerData, PollHint};

use super::{IrqState, TouchTransform, irq_touch_common};

/// Default I2C address.
pub const ADDR: u8 = 0x38;
/// `TD_STATUS` register (number of touch points).
pub const REG_TD_STATUS: u8 = 0x02;
/// Maximum number of points reported by the family (`FT5x06`: 5).
const MAX_POINTS: u8 = 5;

/// `FT6x36` driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Ft6x36<I2C, IRQ> {
    i2c: I2C,
    addr: u8,
    pub(super) irq: IrqState<IRQ>,
    transform: TouchTransform,
}

impl<I2C, IRQ> Ft6x36<I2C, IRQ> {
    /// A driver at address [`ADDR`]; `irq` is the `INT` pin (active low), if wired.
    #[must_use]
    pub fn new(i2c: I2C, irq: Option<IRQ>, transform: TouchTransform) -> Self {
        Self {
            i2c,
            addr: ADDR,
            irq: IrqState::new(irq),
            transform,
        }
    }

    /// Uses another I2C address.
    #[must_use]
    pub fn with_address(mut self, addr: u8) -> Self {
        self.addr = addr;
        self
    }

    /// Treats the IRQ line as active high (for controllers configured that way).
    #[must_use]
    pub fn with_irq_active_high(mut self, active_high: bool) -> Self {
        self.irq.active_high = active_high;
        self
    }

    /// Replaces the coordinate transform.
    pub fn set_transform(&mut self, t: TouchTransform) {
        self.transform = t;
    }

    /// Returns the bus and the IRQ pin.
    #[must_use]
    pub fn release(self) -> (I2C, Option<IRQ>) {
        (self.i2c, self.irq.irq)
    }
}

impl<I2C: I2c, IRQ: InputPin> Ft6x36<I2C, IRQ> {
    /// Reads the first touch point in raw panel coordinates (`None`: not touched).
    pub fn read_raw(&mut self) -> Result<Option<(u16, u16)>, I2C::Error> {
        let mut b = [0u8; 5];
        self.i2c.write_read(self.addr, &[REG_TD_STATUS], &mut b)?;
        let points = b[0] & 0x0F;
        let event = b[1] >> 6;
        trace!(target: "twine::driver", "ft6x36 regs {:?}", b);
        if points == 0 || points > MAX_POINTS || event == 1 {
            // No touch, invalid count (0x0F after reset) or "lift up" event.
            return Ok(None);
        }
        let x = u16::from(b[1] & 0x0F) << 8 | u16::from(b[2]);
        let y = u16::from(b[3] & 0x0F) << 8 | u16::from(b[4]);
        Ok(Some((x, y)))
    }
}

impl<I2C: I2c, IRQ: InputPin> InputDevice for Ft6x36<I2C, IRQ> {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        if !self.irq.should_read() {
            return InputData::Pointer(self.irq.last);
        }
        let data = match self.read_raw() {
            Ok(Some((x, y))) => PointerData {
                point: self.transform.apply(i32::from(x), i32::from(y)),
                pressed: true,
            },
            Ok(None) => self.irq.released(),
            Err(_) => {
                warn!(target: "twine::driver", "ft6x36: I2C error");
                self.irq.released()
            }
        };
        InputData::Pointer(self.irq.update("ft6x36", data))
    }

    fn poll_hint(&self) -> PollHint {
        self.irq.poll_hint()
    }
}

irq_touch_common!(Ft6x36, "ft6x36");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, RecordingI2c, RecordingPin, block_on};
    use crate::touch::test_util::Regs;
    use alloc::vec;
    use twine_core::Point;

    fn dut(rec: &Recorder) -> Ft6x36<RecordingI2c, RecordingPin> {
        Ft6x36::new(
            rec.i2c(),
            Some(rec.quiet_pin("int")),
            TouchTransform::identity(240, 320),
        )
    }

    #[test]
    fn ft6x36_parses_single_touch() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        rec.set_level("int", false);
        // 1 point, event "contact" (2), x = 0x0AB = 171, y = 0x123 = 291.
        regs.set(0x02, &[0x01, 0x80, 0xAB, 0x01, 0x23]);
        let mut t = dut(&rec);
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(171, 291),
                pressed: true
            })
        );
        assert_eq!(
            rec.ops(),
            [
                BusOp::I2cWrite {
                    addr: 0x38,
                    data: vec![0x02]
                },
                BusOp::I2cRead { addr: 0x38, len: 5 }
            ]
        );
        assert_eq!(t.poll_hint(), PollHint::Interrupt);
    }

    #[test]
    fn ft6x36_no_touch_when_td_status_zero() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        rec.set_level("int", false);
        regs.set(0x02, &[0x00, 0x80, 0xAB, 0x01, 0x23]);
        let mut t = dut(&rec);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        regs.set(0x02, &[0x0F]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        // Lift-up event with a point count still set.
        regs.set(0x02, &[0x01, 0x40]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
    }

    #[test]
    fn irq_idle_does_no_i2c_traffic() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        rec.set_level("int", true); // inactive
        regs.set(0x02, &[0x01, 0x80, 0x10, 0x00, 0x20]);
        let mut t = dut(&rec);
        for _ in 0..5 {
            assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        }
        assert_eq!(rec.i2c_transactions(), 0);
        // Active → reads; then keeps reading while pressed even if the line goes idle.
        rec.set_level("int", false);
        assert!(matches!(t.read(), InputData::Pointer(p) if p.pressed));
        rec.set_level("int", true);
        regs.set(0x02, &[0x00]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        assert_eq!(rec.i2c_transactions(), 2);
        let _ = t.read();
        assert_eq!(rec.i2c_transactions(), 2);
    }

    #[test]
    fn active_high_and_errors() {
        let rec = Recorder::new();
        let mut t = dut(&rec).with_irq_active_high(true).with_address(0x39);
        rec.set_level("int", true);
        rec.fail_next();
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        assert_eq!(t.kind(), InputKind::Pointer);
        t.set_transform(TouchTransform::identity(1, 1));
        let _ = t.release();
    }

    #[test]
    fn async_wait() {
        use twine_hal::AsyncInputWait;
        let rec = Recorder::new();
        let mut t = dut(&rec);
        block_on(t.wait_for_interrupt());
        assert_eq!(rec.ops(), [BusOp::Wait("int", false)]);
    }
}
