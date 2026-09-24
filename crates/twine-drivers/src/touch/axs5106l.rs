//! AXS5106L capacitive touch controller (I2C; 1.47" 172 × 320 IPS modules such as the Waveshare
//! ESP32-C6-Touch-LCD-1.47).
//!
//! | | |
//! |-|-|
//! | Reference | Waveshare ESP32-C6-Touch-LCD-1.47 demo: ESP-IDF component `esp_lcd_touch_axs5106` (Apache-2.0) and Arduino library `esp_lcd_touch_axs5106l` |
//! | Address | `0x63` |
//! | Bus speed | I2C up to 400 kHz (the vendor demo's clock) |
//! | IRQ | `INT`, active low (the demo triggers on the falling edge) |
//! | Reset | optional `RST` (active low) |
//! | Rotation | reports panel coordinates (12-bit): use [`TouchTransform`] |
//!
//! A reading is a register write of `0x01` followed by a separate 14-byte read (with a STOP in
//! between, as the vendor code does): byte 1 holds the number of points (bits 3:0), then per
//! point 6 bytes starting at byte 2: `XH` (X bits 11:8 in 3:0), `XL`, `YH`, `YL`, two more.
//! Only the first point is used. [`read_id`](Axs5106l::read_id) reads the 3 ID bytes at `0x08`.
//!
//! On the Waveshare 1.47" board the touch X axis runs opposite to the display's columns: use
//! [`TouchTransform::with_raw_mirror_x`] (the vendor demo mirrors X at rotation 0).
//!
//! # Wiring
//!
//! | Controller pin | Driver argument |
//! |----------------|-----------------|
//! | `SDA`, `SCL` | `i2c`, an `embedded_hal::i2c::I2c` (address `0x63`) |
//! | `INT` | `irq`: `Some(InputPin)` (`+ Wait` for the async wake-up), or `None` to poll |
//! | `RST` | optional: [`with_reset_pin`](Axs5106l::with_reset_pin) + [`reset`](Axs5106l::reset), or hold it high from your firmware |
//!
//! ```
//! use twine_drivers::touch::{Axs5106l, TouchTransform};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{InputDevice, PollHint};
//!
//! let rec = Recorder::new();
//! let transform = TouchTransform::identity(172, 320).with_raw_mirror_x();
//! let touch = Axs5106l::new(rec.i2c(), Some(rec.quiet_pin("int")), transform);
//! assert_eq!(touch.poll_hint(), PollHint::Interrupt);
//! ```

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::i2c::I2c;
use twine_core::log::{trace, warn};
use twine_hal::{InputData, InputDevice, InputKind, PointerData, PollHint};

use super::{IrqState, TouchTransform};
use crate::NoPin;

/// I2C address.
pub const ADDR: u8 = 0x63;
/// First touch report register (gesture byte, then the point count).
pub const REG_TOUCH_DATA: u8 = 0x01;
/// ID register (3 bytes).
pub const REG_ID: u8 = 0x08;
/// Bytes of one touch report as read by the vendor driver (up to 2 points).
const REPORT_LEN: usize = 14;
/// Largest point count the vendor library accepts; larger values are treated as noise.
const MAX_POINTS: u8 = 5;

/// AXS5106L driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Axs5106l<I2C, IRQ, RST = NoPin> {
    i2c: I2C,
    irq: IrqState<IRQ>,
    rst: Option<RST>,
    transform: TouchTransform,
}

impl<I2C, IRQ> Axs5106l<I2C, IRQ, NoPin> {
    /// A driver without reset pin; `irq` is the `INT` pin (active low), if wired.
    #[must_use]
    pub fn new(i2c: I2C, irq: Option<IRQ>, transform: TouchTransform) -> Self {
        Self {
            i2c,
            irq: IrqState::new(irq),
            rst: None,
            transform,
        }
    }

    /// Adds the reset pin (see [`reset`](Axs5106l::reset)).
    #[must_use]
    pub fn with_reset_pin<RST: OutputPin>(self, rst: RST) -> Axs5106l<I2C, IRQ, RST> {
        Axs5106l {
            i2c: self.i2c,
            irq: self.irq,
            rst: Some(rst),
            transform: self.transform,
        }
    }
}

impl<I2C, IRQ, RST> Axs5106l<I2C, IRQ, RST> {
    /// Replaces the coordinate transform.
    pub fn set_transform(&mut self, t: TouchTransform) {
        self.transform = t;
    }

    /// Returns the bus and the pins.
    #[must_use]
    pub fn release(self) -> (I2C, Option<IRQ>, Option<RST>) {
        (self.i2c, self.irq.irq, self.rst)
    }
}

impl<I2C, IRQ, RST: OutputPin> Axs5106l<I2C, IRQ, RST> {
    /// Pulses `RST` low for 10 ms and waits 300 ms for the controller to boot (the vendor
    /// Arduino library's wait); no-op without a reset pin.
    pub fn reset(&mut self, delay: &mut impl DelayNs) -> Result<(), RST::Error> {
        if let Some(rst) = self.rst.as_mut() {
            rst.set_low()?;
            delay.delay_ms(10);
            rst.set_high()?;
            delay.delay_ms(300);
        }
        Ok(())
    }
}

impl<I2C: I2c, IRQ, RST> Axs5106l<I2C, IRQ, RST> {
    /// Reads `buf.len()` bytes from register `reg` (register write, STOP, read).
    fn read_reg(&mut self, reg: u8, buf: &mut [u8]) -> Result<(), I2C::Error> {
        self.i2c.write(ADDR, &[reg])?;
        self.i2c.read(ADDR, buf)
    }

    /// Reads the 3 ID bytes (register `0x08`).
    pub fn read_id(&mut self) -> Result<[u8; 3], I2C::Error> {
        let mut id = [0u8; 3];
        self.read_reg(REG_ID, &mut id)?;
        Ok(id)
    }

    /// Reads the first touch point in raw panel coordinates (`None`: no finger).
    pub fn read_raw(&mut self) -> Result<Option<(u16, u16)>, I2C::Error> {
        let mut b = [0u8; REPORT_LEN];
        self.read_reg(REG_TOUCH_DATA, &mut b)?;
        trace!(target: "twine::driver", "axs5106l regs {:?}", b);
        let points = b[1] & 0x0F;
        if points == 0 || points > MAX_POINTS {
            return Ok(None);
        }
        let x = u16::from(b[2] & 0x0F) << 8 | u16::from(b[3]);
        let y = u16::from(b[4] & 0x0F) << 8 | u16::from(b[5]);
        Ok(Some((x, y)))
    }
}

impl<I2C: I2c, IRQ: InputPin, RST> InputDevice for Axs5106l<I2C, IRQ, RST> {
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
                warn!(target: "twine::driver", "axs5106l: I2C error");
                self.irq.released()
            }
        };
        InputData::Pointer(self.irq.update("axs5106l", data))
    }

    fn poll_hint(&self) -> PollHint {
        self.irq.poll_hint()
    }
}

#[cfg(feature = "async")]
impl<I2C, IRQ: embedded_hal_async::digital::Wait, RST> twine_hal::AsyncInputWait for Axs5106l<I2C, IRQ, RST> {
    /// Waits until the `INT` line is active; never completes without an IRQ pin.
    async fn wait_for_interrupt(&mut self) {
        self.irq.wait("axs5106l").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, RecordingI2c, RecordingPin, block_on};
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cell::RefCell;
    use twine_core::Point;

    /// Serves `report` for 14-byte reads and `id` for 3-byte reads (the register pointer is
    /// written in a separate transaction, which the ops log checks).
    fn install(rec: &Recorder, report: &Rc<RefCell<Vec<u8>>>) {
        let r = Rc::clone(report);
        rec.set_i2c_responder(move |_addr, _written, rx| {
            if rx.len() == 3 {
                rx.copy_from_slice(&[0x05, 0x10, 0x6A]);
            } else {
                let src = r.borrow();
                for (d, s) in rx.iter_mut().zip(src.iter()) {
                    *d = *s;
                }
            }
        });
    }

    fn dut(rec: &Recorder) -> Axs5106l<RecordingI2c, RecordingPin> {
        Axs5106l::new(
            rec.i2c(),
            Some(rec.quiet_pin("int")),
            TouchTransform::identity(4096, 4096),
        )
    }

    fn report(points: u8, p: &[(u16, u16)]) -> Vec<u8> {
        let mut v = vec![0u8; REPORT_LEN];
        v[1] = points;
        for (i, (x, y)) in p.iter().enumerate() {
            let o = 2 + i * 6;
            // Upper nibbles of XH/YH carry event bits the driver must mask.
            v[o] = 0x80 | (x >> 8) as u8;
            v[o + 1] = *x as u8;
            v[o + 2] = 0x40 | (y >> 8) as u8;
            v[o + 3] = *y as u8;
        }
        v
    }

    #[test]
    fn axs5106l_parses_point_and_bus_ops() {
        let rec = Recorder::new();
        let r = Rc::new(RefCell::new(report(1, &[(0x0AB, 0x13F)])));
        install(&rec, &r);
        rec.set_level("int", false);
        let mut t = dut(&rec);
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(0xAB, 0x13F),
                pressed: true
            })
        );
        assert_eq!(
            rec.ops(),
            [
                BusOp::I2cWrite {
                    addr: 0x63,
                    data: vec![0x01]
                },
                BusOp::I2cRead { addr: 0x63, len: 14 }
            ]
        );
        assert_eq!(rec.i2c_transactions(), 2);
        assert_eq!(t.kind(), InputKind::Pointer);
    }

    #[test]
    fn axs5106l_no_touch_and_noise() {
        let rec = Recorder::new();
        let r = Rc::new(RefCell::new(report(0, &[(10, 20)])));
        install(&rec, &r);
        rec.set_level("int", false);
        let mut t = dut(&rec);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        // Count in the low nibble only; 0x0F is noise (e.g. just after reset).
        *r.borrow_mut() = report(0x0F, &[(10, 20)]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        *r.borrow_mut() = report(0x31, &[(10, 20)]);
        assert!(matches!(t.read(), InputData::Pointer(p) if p.pressed && p.point == Point::new(10, 20)));
        // I2C errors report a release.
        rec.fail_next();
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
    }

    #[test]
    fn axs5106l_multi_touch_uses_first_point() {
        let rec = Recorder::new();
        let r = Rc::new(RefCell::new(report(2, &[(100, 200), (150, 300)])));
        install(&rec, &r);
        rec.set_level("int", false);
        let mut t = dut(&rec);
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(100, 200),
                pressed: true
            })
        );
    }

    #[test]
    fn axs5106l_irq_idle_does_no_i2c_traffic() {
        let rec = Recorder::new();
        let r = Rc::new(RefCell::new(report(1, &[(5, 6)])));
        install(&rec, &r);
        rec.set_level("int", true); // inactive
        let mut t = dut(&rec);
        for _ in 0..5 {
            assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        }
        assert_eq!(rec.i2c_transactions(), 0);
        assert!(rec.ops().is_empty());
        // Active → reads; keeps reading while pressed even after the line goes idle.
        rec.set_level("int", false);
        assert!(matches!(t.read(), InputData::Pointer(p) if p.pressed));
        rec.set_level("int", true);
        *r.borrow_mut() = report(0, &[]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        let n = rec.i2c_transactions();
        let _ = t.read();
        assert_eq!(rec.i2c_transactions(), n);
    }

    #[test]
    fn axs5106l_transform_and_mirror() {
        let rec = Recorder::new();
        let r = Rc::new(RefCell::new(report(1, &[(10, 20)])));
        install(&rec, &r);
        let mut t = Axs5106l::new(
            rec.i2c(),
            None::<NoPin>,
            TouchTransform::identity(172, 320).with_raw_mirror_x(),
        );
        assert_eq!(t.poll_hint(), PollHint::Periodic);
        assert!(matches!(t.read(), InputData::Pointer(p) if p.point == Point::new(161, 20)));
    }

    #[test]
    fn axs5106l_reset_and_id() {
        let rec = Recorder::new();
        let r = Rc::new(RefCell::new(Vec::new()));
        install(&rec, &r);
        let mut t = Axs5106l::new(rec.i2c(), None::<NoPin>, TouchTransform::identity(1, 1))
            .with_reset_pin(rec.pin("rst"));
        t.reset(&mut rec.delay()).unwrap();
        assert_eq!(t.read_id().unwrap(), [0x05, 0x10, 0x6A]);
        assert_eq!(
            rec.ops(),
            [
                BusOp::Pin("rst", false),
                BusOp::DelayUs(10_000),
                BusOp::Pin("rst", true),
                BusOp::DelayUs(300_000),
                BusOp::I2cWrite {
                    addr: 0x63,
                    data: vec![0x08]
                },
                BusOp::I2cRead { addr: 0x63, len: 3 }
            ]
        );
        let (_, irq, rst) = t.release();
        assert!(irq.is_none() && rst.is_some());
    }

    #[test]
    fn axs5106l_async_wait() {
        use twine_hal::AsyncInputWait;
        let rec = Recorder::new();
        let mut t = dut(&rec);
        block_on(t.wait_for_interrupt());
        assert_eq!(rec.ops(), [BusOp::Wait("int", false)]);
    }
}
