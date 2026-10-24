//! Goodix GT911 capacitive touch controller (I2C, 16-bit register addresses).
//!
//! | | |
//! |-|-|
//! | Datasheet | Goodix GT911 datasheet + programming guide |
//! | Address | `0x5D` or `0x14` (selected by the `INT` level during reset) |
//! | Bus speed | I2C up to 400 kHz |
//! | IRQ | `INT`; the default configuration triggers on the falling edge → active low |
//! | Coordinates | reported in the resolution of its configuration (`0x8048`), scaled to the display's native size and mapped by [`TouchTransform`](super::TouchTransform) (fitted automatically, see [module docs](super#coordinates)) |
//!
//! A reading reads the status register `0x814E` (bit 7: buffer ready, bits 3:0: number of
//! points). When ready and touched, the first point is read at `0x8150` (X low, X high, Y low,
//! Y high); the status is then cleared by writing `0` — without that the controller stops
//! reporting. When the buffer is not ready the previous state is kept.
//!
//! The controller reports points in the X/Y resolution of its configuration (`0x8048`), which
//! module makers do not always set to the panel's: the CYD 8048S043 ships configured for
//! 480 × 272 on an 800 × 480 display. The driver reads it once (in [`probe`](Gt911::probe), or
//! before the first sample) and scales points from it to the display's native size; a zero
//! resolution is taken as native pixels.
//!
//! [`probe`](Gt911::probe) reads the product ID (`0x8140`, `"911"`), the configuration
//! version (`0x8047`) and the resolution, and logs them.
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
//! use twine_drivers::touch::Gt911;
//! use twine_drivers::testkit::Recorder;
//! use twine_drivers::NoPin;
//! use twine_hal::{InputDevice, PollHint};
//!
//! let rec = Recorder::new();
//! let touch = Gt911::new(rec.i2c(), None::<NoPin>);
//! assert_eq!(touch.poll_hint(), PollHint::Periodic);
//! ```

use embedded_hal::digital::InputPin;
use embedded_hal::i2c::I2c;
use twine_core::log::{info, trace, warn};
use twine_hal::{DeviceHealth, DisplayInfo, InputData, InputDevice, InputKind, PointerData, PollHint};

use super::{IrqState, PanelMap, irq_touch_common, panel_map_methods};

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
/// Configured resolution: X max low, X max high, Y max low, Y max high.
pub const REG_RESOLUTION: u16 = 0x8048;

/// GT911 driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Gt911<I2C, IRQ> {
    i2c: I2C,
    addr: u8,
    pub(super) irq: IrqState<IRQ>,
    map: PanelMap,
    /// The configured resolution has been read (it is in `map`).
    resolution_read: bool,
}

impl<I2C, IRQ> Gt911<I2C, IRQ> {
    /// A driver at address [`ADDR`] (`0x5D`); `irq` is the `INT` pin, if wired. Its
    /// coordinates are fitted to the display when it is added to the engine (see
    /// [`for_display`](Self::for_display)). Touches no bus; never panics.
    ///
    /// ```
    /// use twine_drivers::testkit::Recorder;
    /// use twine_drivers::touch::Gt911;
    /// use twine_hal::{InputDevice, PollHint};
    ///
    /// // On hardware: the HAL's `I2c` and, if wired, the interrupt input pin.
    /// let rec = Recorder::new();
    /// let touch = Gt911::new(rec.i2c(), Some(rec.quiet_pin("int")));
    /// assert_eq!(touch.poll_hint(), PollHint::Interrupt);
    /// ```
    #[must_use]
    pub fn new(i2c: I2C, irq: Option<IRQ>) -> Self {
        Self {
            i2c,
            addr: ADDR,
            irq: IrqState::new(irq),
            map: PanelMap::new(),
            resolution_read: false,
        }
    }

    panel_map_methods!(Gt911);

    /// Reports the device [`Failed`](DeviceHealth::Failed) after `n` consecutive bus errors
    /// (default [`DeviceHealth::DEFAULT_FAIL_AFTER`]; see [`health`](InputDevice::health)).
    #[must_use]
    pub fn with_fail_after(mut self, n: u16) -> Self {
        self.irq.fail_after = n;
        self
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

    /// Reads and logs the product ID and configuration version, and reads the configured
    /// resolution (see [`read_resolution`](Self::read_resolution)); returns `(id, version)`.
    ///
    /// # Errors
    /// The I²C bus error.
    pub fn probe(&mut self) -> Result<([u8; 4], u8), I2C::Error> {
        let mut id = [0u8; 4];
        self.read_reg(REG_PRODUCT_ID, &mut id)?;
        let mut ver = [0u8; 1];
        self.read_reg(REG_CONFIG_VERSION, &mut ver)?;
        info!(target: "twine::driver", "gt911: product id {:?}, config version {}", id, ver[0]);
        self.read_resolution()?;
        Ok((id, ver[0]))
    }

    /// Reads and logs the resolution of the configuration (`(0, 0)` if not set) and scales
    /// points from it to the display from now on. The driver calls it before its first sample
    /// unless [`probe`](Self::probe) did.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Point};
    /// use twine_drivers::NoPin;
    /// use twine_drivers::testkit::Recorder;
    /// use twine_drivers::touch::Gt911;
    /// use twine_hal::DisplayInfo;
    ///
    /// let rec = Recorder::new();
    /// rec.set_i2c_responder(|_, _, rx| rx.copy_from_slice(&[0xE0, 0x01, 0x10, 0x01])); // 480 × 272
    /// let mut touch = Gt911::new(rec.i2c(), None::<NoPin>)
    ///     .for_display(&DisplayInfo::new(800, 480, ColorFormat::Rgb565));
    /// assert_eq!(touch.read_resolution(), Ok((480, 272)));
    /// ```
    ///
    /// # Errors
    /// The I²C bus error (the resolution is read again before the next sample).
    pub fn read_resolution(&mut self) -> Result<(u16, u16), I2C::Error> {
        let mut r = [0u8; 4];
        self.read_reg(REG_RESOLUTION, &mut r)?;
        let size = (u16::from_le_bytes([r[0], r[1]]), u16::from_le_bytes([r[2], r[3]]));
        info!(target: "twine::driver", "gt911: configured for {}x{}", size.0, size.1);
        self.map.set_panel_size(Some(size));
        self.resolution_read = true;
        Ok(size)
    }

    /// Reads one report: `Ok(None)` if the buffer is not ready, `Ok(Some(None))` if ready and
    /// not touched, `Ok(Some(Some((x, y))))` for the first point. Clears the status when ready.
    ///
    /// # Errors
    /// The I²C bus error.
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
        let raw = if self.resolution_read {
            self.read_raw()
        } else {
            self.read_resolution().and_then(|_| self.read_raw())
        };
        let data = match raw {
            Ok(Some(Some((x, y)))) => PointerData {
                point: self.map.apply(x, y),
                pressed: true,
            },
            Ok(Some(None)) => self.irq.released(),
            Ok(None) => self.irq.last,
            Err(_) => {
                warn!(target: "twine::driver", "gt911: I2C error");
                return InputData::Pointer(self.irq.error("gt911"));
            }
        };
        InputData::Pointer(self.irq.ok("gt911", data))
    }

    fn poll_hint(&self) -> PollHint {
        self.irq.poll_hint()
    }

    fn health(&self) -> DeviceHealth {
        self.irq.health
    }

    fn fit_to_display(&mut self, info: &DisplayInfo) {
        self.map.fit(info);
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
        let mut t: Gt911<_, crate::NoPin> =
            Gt911::new(rec.i2c(), None).for_display(&crate::touch::test_util::display(800, 480));
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
                // The configured resolution, once (unset here: native pixels).
                BusOp::I2cWrite {
                    addr: 0x5D,
                    data: vec![0x80, 0x48]
                },
                BusOp::I2cRead { addr: 0x5D, len: 4 },
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

    /// A GT911 configured for 480 × 272 on an 800 × 480 panel (the CYD 8048S043) reported
    /// its raw coordinates as pixels: touches landed in the top-left 60 % of the screen.
    #[test]
    fn regression_gt911_scales_configured_resolution_to_display() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(REG_RESOLUTION, &[0xE0, 0x01, 0x10, 0x01]); // 480 × 272
        regs.set(0x814E, &[0x81]);
        regs.set(0x8150, &[0xF0, 0x00, 0x88, 0x00]); // x 240, y 136: the centre
        let mut t: Gt911<_, crate::NoPin> =
            Gt911::new(rec.i2c(), None).for_display(&crate::touch::test_util::display(800, 480));
        assert!(matches!(t.read(), InputData::Pointer(p) if p.point == Point::new(400, 240)));
        regs.set(0x814E, &[0x81]);
        regs.set(0x8150, &[0xDF, 0x01, 0x0F, 0x01]); // x 479, y 271: the last raw point
        assert!(matches!(t.read(), InputData::Pointer(p) if p.point == Point::new(798, 478)));
        // Read once: the next sample reads only the status and the point.
        let _ = rec.take_ops();
        regs.set(0x814E, &[0x81]);
        let _ = t.read();
        assert_eq!(rec.ops().len(), 5);
    }

    #[test]
    fn gt911_scales_along_swapped_mount_axes() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(0x8140, b"911\0");
        regs.set(REG_RESOLUTION, &[0x10, 0x01, 0xE0, 0x01]); // 272 × 480: X along the rows
        let mut t: Gt911<_, crate::NoPin> = Gt911::new(rec.i2c(), None)
            .with_mount(twine_hal::TouchMount {
                swap_xy: true,
                ..twine_hal::TouchMount::ALIGNED
            })
            .for_display(&crate::touch::test_util::display(800, 480));
        t.probe().unwrap();
        regs.set(0x814E, &[0x81]);
        regs.set(0x8150, &[0x44, 0x00, 0x78, 0x00]); // x 68 of 272, y 120 of 480
        // Scaled to 120 of 480 rows and 200 of 800 columns, then swapped.
        assert!(matches!(t.read(), InputData::Pointer(p) if p.point == Point::new(200, 120)));
        // `probe` read the resolution: no second read before the sample.
        let resolution_reads = BusOp::I2cWrite {
            addr: 0x5D,
            data: vec![0x80, 0x48],
        };
        assert_eq!(rec.ops().iter().filter(|op| **op == resolution_reads).count(), 1);
    }

    #[test]
    fn gt911_probe_logs_version() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(0x8140, b"911\0");
        regs.set(0x8047, &[0x41]);
        let mut t: Gt911<_, crate::NoPin> = Gt911::new(rec.i2c(), None)
            .for_display(&crate::touch::test_util::display(1, 1))
            .with_address(ADDR_ALT);
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
        let mut t = Gt911::new(rec.i2c(), Some(rec.quiet_pin("int")))
            .for_display(&crate::touch::test_util::display(1, 1));
        rec.set_level("int", true);
        let _ = t.read();
        assert_eq!(rec.ops(), []);
        assert_eq!(t.poll_hint(), PollHint::Interrupt);
        assert_eq!(t.kind(), InputKind::Pointer);
    }

    #[test]
    fn gt911_bus_failure_degrades_then_fails() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(0x814E, &[0x81]);
        regs.set(0x8150, &[0x2C, 0x01, 0xC8, 0x00]);
        let mut t: Gt911<_, crate::NoPin> =
            Gt911::new(rec.i2c(), None).for_display(&crate::touch::test_util::display(800, 480));
        // The status is cleared by each read: keep it "ready, 1 point" for the recovery read.
        let r = regs.clone();
        let mut t = Probe(&mut t, move || r.set(0x814E, &[0x81]));
        crate::touch::test_util::assert_bus_failure_sequence(&rec, &mut t);
        let _ = Gt911::<_, crate::NoPin>::new(rec.i2c(), None)
            .for_display(&crate::touch::test_util::display(1, 1))
            .with_fail_after(5);
    }

    /// Runs `before` ahead of every read of the wrapped driver.
    struct Probe<'a, D, F>(&'a mut D, F);
    impl<D: InputDevice, F: FnMut()> InputDevice for Probe<'_, D, F> {
        fn kind(&self) -> InputKind {
            self.0.kind()
        }
        fn read(&mut self) -> InputData {
            (self.1)();
            self.0.read()
        }
        fn health(&self) -> twine_hal::DeviceHealth {
            self.0.health()
        }
    }
}
