//! Hynitron CST816S capacitive touch controller (I2C; round 1.28" GC9A01 boards and small
//! watches).
//!
//! | | |
//! |-|-|
//! | Datasheet | Hynitron CST816S datasheet / register description |
//! | Address | `0x15` |
//! | IRQ | `IRQ`, active-low pulses per report; **required**: the chip sleeps when idle and NACKs I2C until touched |
//! | Reset | optional `RST` (active low) to wake / restart it |
//! | Rotation | reports panel coordinates (12-bit): use [`TouchTransform`] |
//!
//! A reading reads 6 registers from `0x01`: gesture, finger count, `XH` (bits 11:8 in 3:0),
//! `XL`, `YH`, `YL`. Because the controller does not answer while asleep, a failed read is
//! reported as "released" and logged at debug level only.
//!
//! ```
//! use twine_drivers::touch::{Cst816s, TouchTransform};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{InputDevice, PollHint};
//!
//! let rec = Recorder::new();
//! let touch = Cst816s::new(rec.i2c(), Some(rec.quiet_pin("irq")), TouchTransform::identity(240, 240));
//! assert_eq!(touch.poll_hint(), PollHint::Interrupt);
//! ```

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::i2c::I2c;
use twine_core::log::{debug, trace};
use twine_hal::{InputData, InputDevice, InputKind, PointerData, PollHint};

use super::{IrqState, TouchTransform};
use crate::NoPin;

/// I2C address.
pub const ADDR: u8 = 0x15;
/// First data register (gesture ID).
pub const REG_DATA: u8 = 0x01;

/// CST816S driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Cst816s<I2C, IRQ, RST = NoPin> {
    i2c: I2C,
    irq: IrqState<IRQ>,
    rst: Option<RST>,
    transform: TouchTransform,
}

impl<I2C, IRQ> Cst816s<I2C, IRQ, NoPin> {
    /// A driver without reset pin; `irq` is the `IRQ` pin (active low).
    #[must_use]
    pub fn new(i2c: I2C, irq: Option<IRQ>, transform: TouchTransform) -> Self {
        Self {
            i2c,
            irq: IrqState::new(irq),
            rst: None,
            transform,
        }
    }

    /// Adds the reset pin (see [`reset`](Cst816s::reset)).
    #[must_use]
    pub fn with_reset_pin<RST: OutputPin>(self, rst: RST) -> Cst816s<I2C, IRQ, RST> {
        Cst816s {
            i2c: self.i2c,
            irq: self.irq,
            rst: Some(rst),
            transform: self.transform,
        }
    }
}

impl<I2C, IRQ, RST: OutputPin> Cst816s<I2C, IRQ, RST> {
    /// Pulses `RST` low for 10 ms and waits 50 ms for the controller to boot (no-op without a
    /// reset pin).
    pub fn reset(&mut self, delay: &mut impl DelayNs) -> Result<(), RST::Error> {
        if let Some(rst) = self.rst.as_mut() {
            rst.set_low()?;
            delay.delay_ms(10);
            rst.set_high()?;
            delay.delay_ms(50);
        }
        Ok(())
    }

    /// Returns the bus and the pins.
    #[must_use]
    pub fn release(self) -> (I2C, Option<IRQ>, Option<RST>) {
        (self.i2c, self.irq.irq, self.rst)
    }
}

impl<I2C: I2c, IRQ, RST> Cst816s<I2C, IRQ, RST> {
    /// Reads the first touch point in raw panel coordinates (`None`: no finger).
    pub fn read_raw(&mut self) -> Result<Option<(u16, u16)>, I2C::Error> {
        let mut b = [0u8; 6];
        self.i2c.write_read(ADDR, &[REG_DATA], &mut b)?;
        trace!(target: "twine::driver", "cst816s regs {:?}", b);
        if b[1] == 0 {
            return Ok(None);
        }
        let x = u16::from(b[2] & 0x0F) << 8 | u16::from(b[3]);
        let y = u16::from(b[4] & 0x0F) << 8 | u16::from(b[5]);
        Ok(Some((x, y)))
    }
}

impl<I2C: I2c, IRQ: InputPin, RST> InputDevice for Cst816s<I2C, IRQ, RST> {
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
                debug!(target: "twine::driver", "cst816s: no answer (asleep)");
                self.irq.released()
            }
        };
        InputData::Pointer(self.irq.update("cst816s", data))
    }

    fn poll_hint(&self) -> PollHint {
        self.irq.poll_hint()
    }
}

#[cfg(feature = "async")]
impl<I2C, IRQ: embedded_hal_async::digital::Wait, RST> twine_hal::AsyncInputWait for Cst816s<I2C, IRQ, RST> {
    /// Waits until the IRQ line is active; never completes without an IRQ pin.
    async fn wait_for_interrupt(&mut self) {
        self.irq.wait("cst816s").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::touch::test_util::Regs;
    use twine_core::Point;

    #[test]
    fn cst816s_coordinates_12bit() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        rec.set_level("irq", false);
        // 1 finger, x = 0x3E8 = 1000 (upper nibble of XH carries event bits), y = 0x0F0 = 240.
        regs.set(0x01, &[0x00, 0x01, 0x83, 0xE8, 0x00, 0xF0]);
        let mut t = Cst816s::new(
            rec.i2c(),
            Some(rec.quiet_pin("irq")),
            TouchTransform::identity(4096, 4096),
        );
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(1000, 240),
                pressed: true
            })
        );
        regs.set(0x02, &[0x00]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        rec.fail_next();
        rec.set_level("irq", false);
        let _ = t.read();
        assert_eq!(t.kind(), InputKind::Pointer);
    }

    #[test]
    fn cst816s_reset_pulse_and_wait() {
        let rec = Recorder::new();
        let mut t = Cst816s::new(rec.i2c(), None::<NoPin>, TouchTransform::identity(1, 1))
            .with_reset_pin(rec.pin("rst"));
        t.reset(&mut rec.delay()).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::Pin("rst", false),
                BusOp::DelayUs(10_000),
                BusOp::Pin("rst", true),
                BusOp::DelayUs(50_000)
            ]
        );
        let (_, irq, rst) = t.release();
        assert!(irq.is_none() && rst.is_some());
    }

    #[test]
    fn cst816s_async_wait() {
        use twine_hal::AsyncInputWait;
        let rec = Recorder::new();
        let mut t = Cst816s::new(
            rec.i2c(),
            Some(rec.quiet_pin("irq")),
            TouchTransform::identity(1, 1),
        );
        block_on(t.wait_for_interrupt());
        assert_eq!(rec.ops(), [BusOp::Wait("irq", false)]);
    }
}
