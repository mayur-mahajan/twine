//! ST STMPE811 resistive touch-screen controller with ADC (I2C; STM32F429I-DISC1 and many
//! resistive modules).
//!
//! | | |
//! |-|-|
//! | Datasheet | ST STMPE811 datasheet (`DocID14489`) |
//! | Address | `0x41` (`ADDR0` high, e.g. F429I-DISC1) or `0x44` |
//! | Bus speed | I2C up to 400 kHz |
//! | IRQ | `INT`, open drain, active low (configured by [`init`](Stmpe811::init)) |
//! | Coordinates | raw 12-bit ADC values: needs a per-module [`Calibration`] (default: identity) |
//!
//! # Init
//!
//! Following ST's `STM32Cube` BSP driver `stmpe811.c` (BSD-3-Clause): soft reset, ADC and
//! touch-screen clocks on (`SYS_CTRL2 = 0x0C`), ADC 12-bit / 80 clocks / internal reference
//! (`0x49`), ADC clock 3.25 MHz, touch pins to their touch-screen function (`GPIO_AF = 0`),
//! `TSC_CFG = 0x9A` (4-sample average, 500 µs detect delay and settling), FIFO threshold 1, FIFO
//! reset, Z fraction 1, 50 mA drive, touch screen on in XYZ mode, clear and enable the
//! touch-detect interrupt (global, level, active low).
//!
//! # Reading
//!
//! `TSC_CTRL` bit 7 tells whether the panel is touched. While touched, up to 3 samples are
//! taken from the FIFO (`0xD7`, 4 bytes each: X 12 bit, Y 12 bit, Z 8 bit), filtered with
//! [`Filter`] (default median of 3), calibrated and clamped. The FIFO and the interrupt status
//! are then cleared.
//!
//! # Wiring
//!
//! | Controller pin | Driver argument |
//! |----------------|-----------------|
//! | `SDA`, `SCL` | `i2c`, an `embedded_hal::i2c::I2c` (address `0x41` or `0x44`) |
//! | `INT`/`IRQ` | `irq`: `Some(InputPin)` (`+ Wait` for the async wake-up), or `None` to poll |
//! | `RST` | not driven by the driver: hold it high from your firmware |
//!
//! ```
//! use twine_drivers::touch::Stmpe811;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{Calibration, InputDevice, PollHint};
//!
//! let rec = Recorder::new();
//! let mut touch = Stmpe811::new(rec.i2c(), Some(rec.quiet_pin("int")))
//!     .with_calibration(Calibration::IDENTITY)
//!     .with_screen_size(240, 320);
//! touch.init(&mut rec.delay()).unwrap();
//! assert_eq!(touch.poll_hint(), PollHint::Interrupt);
//! ```

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::InputPin;
use embedded_hal::i2c::I2c;
use twine_core::Point;
use twine_core::log::{trace, warn};
use twine_hal::{Calibration, InputData, InputDevice, InputKind, PointerData, PollHint};

use super::{Filter, IrqState, irq_touch_common, median3};

/// Default I2C address (F429I-DISC1).
pub const ADDR: u8 = 0x41;
/// Alternative I2C address.
pub const ADDR_ALT: u8 = 0x44;

/// Register addresses.
pub mod reg {
    /// System control 1 (soft reset).
    pub const SYS_CTRL1: u8 = 0x03;
    /// System control 2 (clock gating).
    pub const SYS_CTRL2: u8 = 0x04;
    /// Interrupt control.
    pub const INT_CTRL: u8 = 0x09;
    /// Interrupt enable.
    pub const INT_EN: u8 = 0x0A;
    /// Interrupt status.
    pub const INT_STA: u8 = 0x0B;
    /// GPIO alternate function.
    pub const GPIO_AF: u8 = 0x17;
    /// ADC control 1.
    pub const ADC_CTRL1: u8 = 0x20;
    /// ADC control 2.
    pub const ADC_CTRL2: u8 = 0x21;
    /// Touch-screen control (bit 7: touch detected).
    pub const TSC_CTRL: u8 = 0x40;
    /// Touch-screen configuration.
    pub const TSC_CFG: u8 = 0x41;
    /// FIFO threshold.
    pub const FIFO_TH: u8 = 0x4A;
    /// FIFO status / reset.
    pub const FIFO_STA: u8 = 0x4B;
    /// FIFO fill level.
    pub const FIFO_SIZE: u8 = 0x4C;
    /// Z fraction format.
    pub const TSC_FRACTION_Z: u8 = 0x56;
    /// Touch-screen drive current.
    pub const TSC_I_DRIVE: u8 = 0x58;
    /// FIFO data, XYZ, non-incrementing (reads consecutive samples).
    pub const TSC_DATA_XYZ: u8 = 0xD7;
}

/// `(register, value, delay after in ms)` init sequence (see the module docs).
pub const INIT: [(u8, u8, u32); 15] = [
    (reg::SYS_CTRL1, 0x02, 10), // soft reset
    (reg::SYS_CTRL1, 0x00, 2),
    (reg::SYS_CTRL2, 0x0C, 0), // ADC + TSC clocks on, GPIO + temperature sensor off
    (reg::ADC_CTRL1, 0x49, 2), // 80 clocks, 12 bit, internal reference
    (reg::ADC_CTRL2, 0x01, 0), // 3.25 MHz
    (reg::GPIO_AF, 0x00, 0),   // touch pins: touch-screen function
    (reg::TSC_CFG, 0x9A, 0),   // 4 samples, 500 µs detect delay, 500 µs settling
    (reg::FIFO_TH, 0x01, 0),
    (reg::FIFO_STA, 0x01, 0), // FIFO reset
    (reg::FIFO_STA, 0x00, 0),
    (reg::TSC_FRACTION_Z, 0x01, 0),
    (reg::TSC_I_DRIVE, 0x01, 0), // 50 mA
    (reg::TSC_CTRL, 0x01, 0),    // enable, XYZ acquisition
    (reg::INT_STA, 0xFF, 0),     // clear
    (reg::INT_EN, 0x01, 0),      // touch detect
];

/// `INT_CTRL`: global interrupt enable, level, active low (written last by `init`).
const INT_CTRL_VALUE: u8 = 0x01;

/// STMPE811 driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Stmpe811<I2C, IRQ> {
    i2c: I2C,
    addr: u8,
    pub(super) irq: IrqState<IRQ>,
    cal: Calibration,
    filter: Filter,
    width: u16,
    height: u16,
}

impl<I2C, IRQ> Stmpe811<I2C, IRQ> {
    /// A driver at [`ADDR`] with identity calibration, a 240 × 320 screen and the median filter.
    #[must_use]
    pub fn new(i2c: I2C, irq: Option<IRQ>) -> Self {
        Self {
            i2c,
            addr: ADDR,
            irq: IrqState::new(irq),
            cal: Calibration::IDENTITY,
            filter: Filter::Median3,
            width: 240,
            height: 320,
        }
    }

    /// Uses another I2C address (e.g. [`ADDR_ALT`]).
    #[must_use]
    pub fn with_address(mut self, addr: u8) -> Self {
        self.addr = addr;
        self
    }

    /// Sets the calibration (raw → screen).
    #[must_use]
    pub fn with_calibration(mut self, cal: Calibration) -> Self {
        self.cal = cal;
        self
    }

    /// Sets the logical screen size points are clamped to.
    #[must_use]
    pub fn with_screen_size(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the noise filter.
    #[must_use]
    pub fn with_filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// Replaces the calibration.
    pub fn set_calibration(&mut self, cal: Calibration) {
        self.cal = cal;
    }

    /// Returns the bus and the IRQ pin.
    #[must_use]
    pub fn release(self) -> (I2C, Option<IRQ>) {
        (self.i2c, self.irq.irq)
    }
}

impl<I2C: I2c, IRQ> Stmpe811<I2C, IRQ> {
    fn write_reg(&mut self, r: u8, v: u8) -> Result<(), I2C::Error> {
        self.i2c.write(self.addr, &[r, v])
    }

    fn read_regs(&mut self, r: u8, buf: &mut [u8]) -> Result<(), I2C::Error> {
        self.i2c.write_read(self.addr, &[r], buf)
    }

    /// Runs the init sequence (see the module docs). Call once before reading.
    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), I2C::Error> {
        for (r, v, ms) in INIT {
            self.write_reg(r, v)?;
            if ms > 0 {
                delay.delay_ms(ms);
            }
        }
        self.write_reg(reg::INT_CTRL, INT_CTRL_VALUE)
    }

    fn clear(&mut self) -> Result<(), I2C::Error> {
        self.write_reg(reg::FIFO_STA, 0x01)?;
        self.write_reg(reg::FIFO_STA, 0x00)?;
        self.write_reg(reg::INT_STA, 0xFF)
    }

    /// Reads a filtered raw sample: `Ok(None)` if not touched, `Ok(Some(None))` if touched
    /// but no sample is ready yet, else the raw 12-bit `(x, y)`.
    pub fn read_raw(&mut self) -> Result<Option<Option<(u16, u16)>>, I2C::Error> {
        let mut b = [0u8; 1];
        self.read_regs(reg::TSC_CTRL, &mut b)?;
        if b[0] & 0x80 == 0 {
            self.clear()?;
            return Ok(None);
        }
        self.read_regs(reg::FIFO_SIZE, &mut b)?;
        let n = usize::from(b[0].min(3));
        let (mut xs, mut ys) = ([0u16; 3], [0u16; 3]);
        for i in 0..n {
            let mut d = [0u8; 4];
            self.read_regs(reg::TSC_DATA_XYZ, &mut d)?;
            xs[i] = u16::from(d[0]) << 4 | u16::from(d[1] >> 4);
            ys[i] = u16::from(d[1] & 0x0F) << 8 | u16::from(d[2]);
        }
        self.clear()?;
        if n == 0 {
            return Ok(Some(None));
        }
        let (x, y) = match self.filter {
            Filter::None => (xs[n - 1], ys[n - 1]),
            Filter::Median3 => (median3(&xs[..n]), median3(&ys[..n])),
        };
        trace!(target: "twine::driver", "stmpe811 raw x {} y {} ({} samples)", x, y, n);
        Ok(Some(Some((x, y))))
    }
}

impl<I2C: I2c, IRQ: InputPin> InputDevice for Stmpe811<I2C, IRQ> {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        if !self.irq.should_read() {
            return InputData::Pointer(self.irq.last);
        }
        let data = match self.read_raw() {
            Ok(Some(Some((x, y)))) => {
                let (sx, sy) = self.cal.apply(i32::from(x), i32::from(y));
                let point = Point::new(
                    sx.clamp(0, i32::from(self.width.max(1)) - 1),
                    sy.clamp(0, i32::from(self.height.max(1)) - 1),
                );
                PointerData { point, pressed: true }
            }
            Ok(Some(None)) => self.irq.last,
            Ok(None) => self.irq.released(),
            Err(_) => {
                warn!(target: "twine::driver", "stmpe811: I2C error");
                self.irq.released()
            }
        };
        InputData::Pointer(self.irq.update("stmpe811", data))
    }

    fn poll_hint(&self) -> PollHint {
        self.irq.poll_hint()
    }
}

irq_touch_common!(Stmpe811, "stmpe811");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, RecordingI2c, RecordingPin};
    use crate::touch::test_util::Regs;
    use alloc::vec;
    use alloc::vec::Vec;

    fn sample(x: u16, y: u16) -> [u8; 4] {
        [
            (x >> 4) as u8,
            ((x & 0x0F) << 4) as u8 | (y >> 8) as u8,
            y as u8,
            0x20,
        ]
    }

    fn dut(rec: &Recorder) -> Stmpe811<RecordingI2c, RecordingPin> {
        Stmpe811::new(rec.i2c(), Some(rec.quiet_pin("int"))).with_screen_size(240, 320)
    }

    #[test]
    fn stmpe811_init_sequence() {
        let rec = Recorder::new();
        let mut t = dut(&rec);
        t.init(&mut rec.delay()).unwrap();
        let w = |r: u8, v: u8| BusOp::I2cWrite {
            addr: 0x41,
            data: vec![r, v],
        };
        let expected: Vec<BusOp> = vec![
            w(0x03, 0x02),
            BusOp::DelayUs(10_000),
            w(0x03, 0x00),
            BusOp::DelayUs(2_000),
            w(0x04, 0x0C),
            w(0x20, 0x49),
            BusOp::DelayUs(2_000),
            w(0x21, 0x01),
            w(0x17, 0x00),
            w(0x41, 0x9A),
            w(0x4A, 0x01),
            w(0x4B, 0x01),
            w(0x4B, 0x00),
            w(0x56, 0x01),
            w(0x58, 0x01),
            w(0x40, 0x01),
            w(0x0B, 0xFF),
            w(0x0A, 0x01),
            w(0x09, 0x01),
        ];
        assert_eq!(rec.ops(), expected);
    }

    #[test]
    fn stmpe811_calibration_maps_corners() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        rec.set_level("int", false);
        // Raw corners (300, 3800) … (3800, 300) map to the screen corners (mirrored Y).
        let cal = Calibration::from_points(
            [(300, 3800), (3800, 3800), (300, 300)],
            [(0, 0), (239, 0), (0, 319)],
        )
        .unwrap();
        let mut t = dut(&rec).with_calibration(cal);
        regs.set(reg::TSC_CTRL.into(), &[0x81]);
        regs.set(reg::FIFO_SIZE.into(), &[1]);
        for (raw, screen) in [
            ((300, 3800), (0, 0)),
            ((3800, 300), (239, 319)),
            ((3800, 3800), (239, 0)),
        ] {
            regs.set(0xD7, &sample(raw.0, raw.1));
            assert_eq!(
                t.read(),
                InputData::Pointer(PointerData {
                    point: Point::new(screen.0, screen.1),
                    pressed: true
                })
            );
        }
        // Beyond the calibrated range: clamped.
        regs.set(0xD7, &sample(4095, 0));
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(239, 319),
                pressed: true
            })
        );
        // Released.
        regs.set(reg::TSC_CTRL.into(), &[0x01]);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
    }

    #[test]
    fn stmpe811_median_filter_and_clear() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        let mut t: Stmpe811<_, crate::NoPin> = Stmpe811::new(rec.i2c(), None);
        regs.set(reg::TSC_CTRL.into(), &[0x80]);
        regs.set(reg::FIFO_SIZE.into(), &[5]);
        // The mock returns the same bytes for each FIFO read: the median of equal samples.
        regs.set(0xD7, &sample(100, 200));
        assert_eq!(t.read_raw().unwrap(), Some(Some((100, 200))));
        let ops = rec.take_ops();
        // 3 FIFO reads, then FIFO reset and interrupt clear.
        assert_eq!(
            ops.iter()
                .filter(|o| **o
                    == BusOp::I2cWrite {
                        addr: 0x41,
                        data: vec![0xD7]
                    })
                .count(),
            3
        );
        assert!(ops.ends_with(&[
            BusOp::I2cWrite {
                addr: 0x41,
                data: vec![0x4B, 0x01]
            },
            BusOp::I2cWrite {
                addr: 0x41,
                data: vec![0x4B, 0x00]
            },
            BusOp::I2cWrite {
                addr: 0x41,
                data: vec![0x0B, 0xFF]
            },
        ]));
        regs.set(reg::FIFO_SIZE.into(), &[0]);
        assert_eq!(t.read_raw().unwrap(), Some(None));
        let mut t = t.with_filter(Filter::None).with_address(ADDR_ALT);
        t.set_calibration(Calibration::IDENTITY);
        assert!(t.read_raw().is_ok());
        assert_eq!(regs.get(0xD7), 100 >> 4);
    }

    #[test]
    fn stmpe811_irq_idle_no_traffic() {
        let rec = Recorder::new();
        rec.set_level("int", true);
        let mut t = dut(&rec);
        let _ = t.read();
        assert_eq!(rec.i2c_transactions(), 0);
        assert_eq!(t.kind(), InputKind::Pointer);
        let _ = t.release();
    }
}
