//! XPT2046 (ADS7843-compatible) resistive touch controller on SPI.
//!
//! | | |
//! |-|-|
//! | Datasheet | XPTEK XPT2046 (TI ADS7846 compatible) |
//! | Max SPI clock | 2.5 MHz (125 kHz sample rate); use ≤ 2 MHz, SPI mode 0 |
//! | IRQ | `T_IRQ`/`PENIRQ`, **active low** while the panel is touched (pull-up needed if the module has none) |
//! | Quirks | noisy: median of several samples; needs per-module [`Calibration`] |
//!
//! # Sampling
//!
//! Each reading is one 3-byte SPI transfer `[control, 0, 0]`, the 12-bit result is
//! `((rx[1] << 8) | rx[2]) >> 3`. Control bytes (12-bit, differential, `PD = 11` = reference
//! and ADC powered between conversions): `0xB3` Z1, `0xC3` Z2, `0xD3` X, `0x93` Y. A reading
//! ends with `0xD0` (`PD = 00`) to power down and re-enable `PENIRQ`.
//!
//! Pressure is `z = z1 + 4095 − z2`; the panel is pressed when `z > z_threshold` (default 400).
//! Then `samples` (default 5, max 7) X/Y pairs are taken and the median of each axis is
//! calibrated and clamped to the screen.
//!
//! # Power (P1)
//!
//! With an IRQ pin, [`read`](InputDevice::read) returns "released" **without any SPI traffic**
//! while `T_IRQ` is high and the last state was released; [`poll_hint`](InputDevice::poll_hint)
//! is then [`PollHint::Interrupt`], so the engine reads only after a wake-up (call
//! `Ui::notify_input()` from the pin interrupt, or let `twine-embassy` await
//! `AsyncInputWait::wait_for_interrupt` with feature `async`).
//!
//! # Wiring
//!
//! | Module pin | Driver argument |
//! |------------|-----------------|
//! | `T_CLK`, `T_DIN`, `T_DO` | the bus of `spi`, a blocking `embedded_hal::spi::SpiDevice` at ≤ 2 MHz (it may share the display's bus with its own CS) |
//! | `T_CS` | the chip select owned by `spi` |
//! | `T_IRQ` | `irq`: `Some(InputPin)` (with pull-up; `+ Wait` for the async wake-up), or `None` to poll |
//!
//! ```
//! use twine_drivers::touch::xpt2046::Xpt2046;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{Calibration, InputData, InputDevice, PollHint};
//!
//! let rec = Recorder::new();
//! rec.set_level("t_irq", true); // not touched
//! // Pass it to `Ui::builder(..).input(touch)`: it is fitted to the display (clamp size) there.
//! let mut touch = Xpt2046::new(rec.spi(), Some(rec.quiet_pin("t_irq")))
//!     .with_calibration(Calibration::DEFAULT_240X320);
//! assert_eq!(touch.poll_hint(), PollHint::Interrupt);
//! assert!(matches!(touch.read(), InputData::Pointer(p) if !p.pressed));
//! assert!(rec.ops().is_empty()); // no SPI traffic while idle
//! ```

use embedded_hal::digital::InputPin;
use embedded_hal::spi::SpiDevice;
use twine_core::log::{debug, error, trace, warn};
use twine_hal::{
    Calibration, DeviceHealth, DisplayInfo, InputData, InputDevice, InputKind, PointerData, PollHint,
    TouchTransform,
};

/// Control byte: Z1 (12-bit, differential, powered).
pub const CMD_Z1: u8 = 0xB3;
/// Control byte: Z2.
pub const CMD_Z2: u8 = 0xC3;
/// Control byte: X position.
pub const CMD_X: u8 = 0xD3;
/// Control byte: Y position.
pub const CMD_Y: u8 = 0x93;
/// Control byte: X with power-down (re-enables `PENIRQ`); ends every reading.
pub const CMD_POWER_DOWN: u8 = 0xD0;

/// Maximum number of X/Y samples per reading (size of the median buffer).
pub const MAX_SAMPLES: u8 = 7;

/// XPT2046 touch driver implementing [`InputDevice`] (`Pointer`).
#[derive(Debug)]
pub struct Xpt2046<SPI, IRQ> {
    spi: SPI,
    irq: Option<IRQ>,
    cal: Calibration,
    last: PointerData,
    z_threshold: u16,
    samples: u8,
    /// Calibrated native points → logical screen (from the fit; pass-through until then).
    transform: TouchTransform,
    health: DeviceHealth,
    fail_after: u16,
}

impl<SPI, IRQ> Xpt2046<SPI, IRQ> {
    /// A driver with the defaults: [`Calibration::DEFAULT_240X320`], pressure threshold
    /// 400, 5 samples, not clamped until fitted to a display ([`for_display`](Self::for_display)).
    /// `irq` is the `T_IRQ` pin (active low), if wired.
    #[must_use]
    pub fn new(spi: SPI, irq: Option<IRQ>) -> Self {
        Self {
            spi,
            irq,
            cal: Calibration::DEFAULT_240X320,
            last: PointerData::default(),
            z_threshold: 400,
            samples: 5,
            transform: TouchTransform::PASS_THROUGH,
            health: DeviceHealth::Ok,
            fail_after: DeviceHealth::DEFAULT_FAIL_AFTER,
        }
    }

    /// Reports the device [`Failed`](DeviceHealth::Failed) after `n` consecutive SPI errors
    /// (default [`DeviceHealth::DEFAULT_FAIL_AFTER`]; see [`health`](InputDevice::health)).
    /// While degraded the last good sample is repeated; once failed, released.
    #[must_use]
    pub fn with_fail_after(mut self, n: u16) -> Self {
        self.fail_after = n;
        self
    }

    /// Sets the calibration (raw → native panel pixels, see [`Calibration`]).
    #[must_use]
    pub fn with_calibration(mut self, cal: Calibration) -> Self {
        self.cal = cal;
        self
    }

    /// Fits the driver to the display described by `info`: calibrated (native) points are
    /// rotated to its logical screen and clamped to its size. The engine does this when the driver is added (`Ui::builder(..).input(touch)`,
    /// `Engine::add_input`) through
    /// [`InputDevice::fit_to_display`]; call it yourself only when you read the driver without
    /// the engine. Until fitted, points are not clamped (raw readings with
    /// [`Calibration::IDENTITY`], e.g. for a calibration screen). The [`Calibration`] maps raw
    /// readings to the display's **native** (unrotated) pixels; the fit adds the rotation, so
    /// one calibration holds in every rotation (also when the display is rotated at run time,
    /// which fits the driver again). Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Rotation};
    /// use twine_drivers::touch::Xpt2046;
    /// use twine_drivers::testkit::Recorder;
    /// use twine_hal::DisplayInfo;
    ///
    /// let rec = Recorder::new();
    /// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg90);
    /// let touch = Xpt2046::new(rec.spi(), Some(rec.quiet_pin("irq"))).for_display(&info);
    /// assert_eq!(touch.screen_size(), (320, 240));
    /// ```
    #[must_use]
    pub fn for_display(mut self, info: &DisplayInfo) -> Self {
        self.fit(info);
        self
    }

    /// The logical screen size points are clamped to (`(u16::MAX, u16::MAX)` until fitted).
    ///
    /// ```
    /// use twine_drivers::touch::Xpt2046;
    /// use twine_drivers::testkit::Recorder;
    ///
    /// let rec = Recorder::new();
    /// let touch = Xpt2046::new(rec.spi(), Some(rec.quiet_pin("irq")));
    /// assert_eq!(touch.screen_size(), (u16::MAX, u16::MAX));
    /// ```
    #[must_use]
    pub fn screen_size(&self) -> (u16, u16) {
        (self.transform.width, self.transform.height)
    }

    fn fit(&mut self, info: &DisplayInfo) {
        self.transform = TouchTransform::for_display(info);
    }

    /// Sets the pressure threshold (`z = z1 + 4095 − z2` must exceed it).
    #[must_use]
    pub fn with_z_threshold(mut self, z: u16) -> Self {
        self.z_threshold = z;
        self
    }

    /// Sets the number of X/Y samples per reading (clamped to `1..=7`).
    #[must_use]
    pub fn with_samples(mut self, n: u8) -> Self {
        self.samples = n.clamp(1, MAX_SAMPLES);
        self
    }

    /// Replaces the calibration.
    pub fn set_calibration(&mut self, c: Calibration) {
        self.cal = c;
    }

    /// The current calibration.
    #[must_use]
    pub fn calibration(&self) -> Calibration {
        self.cal
    }

    /// Returns the SPI device and the IRQ pin.
    #[must_use]
    pub fn release(self) -> (SPI, Option<IRQ>) {
        (self.spi, self.irq)
    }
}

/// Median of `v` (sorts it in place); `v` must not be empty.
fn median(v: &mut [u16]) -> u16 {
    // Insertion sort: at most 7 elements.
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && v[j - 1] > v[j] {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
    v[v.len() / 2]
}

impl<SPI: SpiDevice, IRQ> Xpt2046<SPI, IRQ> {
    fn convert(&mut self, cmd: u8) -> Result<u16, SPI::Error> {
        let mut buf = [cmd, 0, 0];
        self.spi.transfer_in_place(&mut buf)?;
        Ok(((u16::from(buf[1]) << 8) | u16::from(buf[2])) >> 3 & 0x0FFF)
    }

    /// Takes one filtered reading: `Some((x, y, z))` in raw 12-bit units if the pressure
    /// exceeds the threshold, `None` if not touched. Always ends with a power-down conversion.
    pub fn read_raw(&mut self) -> Result<Option<(u16, u16, u16)>, SPI::Error> {
        let z1 = self.convert(CMD_Z1)?;
        let z2 = self.convert(CMD_Z2)?;
        let z = (z1 + 4095).saturating_sub(z2);
        let result = if z > self.z_threshold {
            let n = usize::from(self.samples.clamp(1, MAX_SAMPLES));
            let mut xs = [0u16; MAX_SAMPLES as usize];
            let mut ys = [0u16; MAX_SAMPLES as usize];
            for i in 0..n {
                xs[i] = self.convert(CMD_X)?;
                ys[i] = self.convert(CMD_Y)?;
            }
            let (x, y) = (median(&mut xs[..n]), median(&mut ys[..n]));
            trace!(target: "twine::driver", "xpt2046 raw x {} y {} z {}", x, y, z);
            Some((x, y, z))
        } else {
            None
        };
        self.convert(CMD_POWER_DOWN)?;
        Ok(result)
    }
}

impl<SPI: SpiDevice, IRQ: InputPin> Xpt2046<SPI, IRQ> {
    /// `true` if an IRQ pin is wired and reports "not touched" (high).
    fn irq_idle(&mut self) -> bool {
        self.irq.as_mut().is_some_and(|p| p.is_high().unwrap_or(false))
    }

    fn sample(&mut self) -> PointerData {
        let result = self.read_raw();
        if result.is_ok() {
            if !self.health.is_ok() {
                debug!(target: "twine::driver", "xpt2046 reads again ({:?} before)", self.health);
            }
            self.health = DeviceHealth::Ok;
        }
        match result {
            Ok(Some((x, y, _))) => {
                let (sx, sy) = self.cal.apply(i32::from(x), i32::from(y));
                // Native panel pixels, rotated to the logical screen and clamped.
                let point = self.transform.apply(sx, sy);
                PointerData { point, pressed: true }
            }
            Ok(None) => PointerData {
                point: self.last.point,
                pressed: false,
            },
            Err(_) => {
                warn!(target: "twine::driver", "xpt2046: SPI error");
                let before = self.health;
                self.health = before.after_error(self.fail_after);
                if !self.health.is_failed() {
                    // Degraded: repeat the last good sample.
                    return self.last;
                }
                if !before.is_failed() {
                    error!(target: "twine::driver", "xpt2046: failed ({} consecutive SPI errors)", self.fail_after.max(1));
                }
                PointerData {
                    point: self.last.point,
                    pressed: false,
                }
            }
        }
    }
}

impl<SPI: SpiDevice, IRQ: InputPin> InputDevice for Xpt2046<SPI, IRQ> {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        let data = if !self.last.pressed && self.irq_idle() {
            // Not touched and nothing in progress: no bus traffic (P1).
            self.last
        } else {
            self.sample()
        };
        if data.pressed != self.last.pressed {
            debug!(
                target: "twine::driver",
                "xpt2046 {} at {}",
                if data.pressed { "press" } else { "release" },
                data.point
            );
        }
        self.last = data;
        InputData::Pointer(data)
    }

    fn poll_hint(&self) -> PollHint {
        if self.irq.is_some() {
            PollHint::Interrupt
        } else {
            PollHint::Periodic
        }
    }

    fn health(&self) -> DeviceHealth {
        self.health
    }

    fn fit_to_display(&mut self, info: &DisplayInfo) {
        self.fit(info);
    }
}

#[cfg(feature = "async")]
impl<SPI, IRQ: embedded_hal_async::digital::Wait> twine_hal::AsyncInputWait for Xpt2046<SPI, IRQ> {
    /// Waits until `T_IRQ` goes low (touched). Without an IRQ pin this never completes
    /// (the device is polled periodically instead).
    async fn wait_for_interrupt(&mut self) {
        match self.irq.as_mut() {
            Some(irq) => {
                if irq.wait_for_low().await.is_err() {
                    warn!(target: "twine::driver", "xpt2046: IRQ wait failed");
                }
            }
            None => core::future::pending::<()>().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, RecordingPin, RecordingSpi, block_on};
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cell::Cell;
    use proptest::prelude::*;
    use twine_core::Point;

    fn encode(v: u16) -> [u8; 2] {
        let s = v << 3;
        [(s >> 8) as u8, s as u8]
    }

    /// Responds with fixed z1/z2 and a sequence of X/Y values.
    fn touch(rec: &Recorder, z1: u16, z2: u16, xs: Vec<u16>, ys: Vec<u16>) {
        let (ix, iy) = (Rc::new(Cell::new(0usize)), Rc::new(Cell::new(0usize)));
        rec.set_spi_responder(move |tx, rx| {
            let v = match tx[0] {
                CMD_Z1 => z1,
                CMD_Z2 => z2,
                CMD_X => {
                    let i = ix.get();
                    ix.set(i + 1);
                    xs[i % xs.len()]
                }
                CMD_Y => {
                    let i = iy.get();
                    iy.set(i + 1);
                    ys[i % ys.len()]
                }
                _ => 0,
            };
            rx[1..3].copy_from_slice(&encode(v));
        });
    }

    fn dut(rec: &Recorder) -> Xpt2046<RecordingSpi, RecordingPin> {
        Xpt2046::new(rec.spi(), Some(rec.quiet_pin("irq"))).with_calibration(Calibration::IDENTITY)
    }

    #[test]
    fn calibration_from_points_roundtrip_fixed() {
        let raw = [(300, 300), (3800, 2000), (2000, 3800)];
        let screen = [(10, 12), (300, 120), (150, 230)];
        let c = Calibration::from_points(raw, screen).unwrap();
        for (r, s) in raw.iter().zip(screen) {
            assert_eq!(c.apply(r.0, r.1), s);
        }
    }

    proptest! {
        #[test]
        fn calibration_from_points_roundtrip(
            a in -200i64..200, b in -200i64..200, d in -200i64..200, e in -200i64..200,
            c in -100_000i64..100_000, f in -100_000i64..100_000,
            pts in proptest::array::uniform3((0i32..4096, 0i32..4096)),
        ) {
            // A random affine transform with scale 1/256: screen = (A·raw + C) / 256.
            let div = 256i64;
            let det = i64::from(pts[0].0 - pts[2].0) * i64::from(pts[1].1 - pts[2].1)
                - i64::from(pts[1].0 - pts[2].0) * i64::from(pts[0].1 - pts[2].1);
            prop_assume!(det.abs() > 1000);
            prop_assume!(a * e - b * d != 0);
            let truth = Calibration { a, b, c, d, e, f, div };
            let screen = pts.map(|(x, y)| truth.apply(x, y));
            let cal = Calibration::from_points(pts, screen).unwrap();
            for ((x, y), s) in pts.iter().zip(screen) {
                let got = cal.apply(*x, *y);
                prop_assert!((got.0 - s.0).abs() <= 1 && (got.1 - s.1).abs() <= 1, "{:?} vs {:?}", got, s);
            }
        }
    }

    #[test]
    fn degenerate_points_none() {
        let screen = [(0, 0), (100, 0), (0, 100)];
        assert!(Calibration::from_points([(10, 10), (20, 20), (30, 30)], screen).is_none());
        assert!(Calibration::from_points([(5, 5), (5, 5), (100, 7)], screen).is_none());
    }

    #[test]
    fn median_rejects_outlier() {
        assert_eq!(median(&mut [1000, 1002, 4095, 999, 1001]), 1001);
        assert_eq!(median(&mut [7]), 7);
        let rec = Recorder::new();
        rec.set_level("irq", false);
        touch(
            &rec,
            2000,
            2000,
            vec![1000, 1002, 4095, 999, 1001],
            vec![500, 0, 501, 499, 502],
        );
        let mut t = dut(&rec);
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(1001, 500),
                pressed: true
            })
        );
    }

    #[test]
    fn read_without_irq_no_spi_traffic() {
        let rec = Recorder::new();
        rec.set_level("irq", true);
        let mut t = dut(&rec);
        for _ in 0..10 {
            assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        }
        assert!(rec.ops().is_empty());
        assert_eq!(rec.spi_transactions(), 0);
    }

    #[test]
    fn keeps_sampling_until_release_seen() {
        let rec = Recorder::new();
        rec.set_level("irq", false);
        touch(&rec, 2000, 2000, vec![100], vec![200]);
        let mut t = dut(&rec);
        assert!(matches!(t.read(), InputData::Pointer(p) if p.pressed));
        // IRQ already high, but the last state was pressed → one more sample sees the release.
        rec.set_level("irq", true);
        touch(&rec, 0, 4095, vec![100], vec![200]);
        let _ = rec.take_ops();
        let r = t.read();
        assert_eq!(
            r,
            InputData::Pointer(PointerData {
                point: Point::new(100, 200),
                pressed: false
            })
        );
        assert!(!rec.ops().is_empty());
        let _ = rec.take_ops();
        let _ = t.read();
        assert!(rec.ops().is_empty());
    }

    #[test]
    fn pressure_threshold() {
        let rec = Recorder::new();
        rec.set_level("irq", false);
        // z = 400 + 4095 − 4095 = 400: not above the threshold.
        touch(&rec, 400, 4095, vec![1], vec![1]);
        let mut t = dut(&rec);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
        touch(&rec, 401, 4095, vec![1], vec![1]);
        assert!(matches!(t.read(), InputData::Pointer(p) if p.pressed));
        let mut t = dut(&rec).with_z_threshold(500);
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
    }

    #[test]
    fn sample_command_bytes_exact() {
        let rec = Recorder::new();
        rec.set_level("irq", false);
        touch(&rec, 2000, 2000, vec![10], vec![20]);
        let mut t = dut(&rec).with_samples(2);
        let _ = t.read();
        let d = |c: u8| BusOp::Data(vec![c, 0, 0]);
        assert_eq!(
            rec.ops(),
            [d(0xB3), d(0xC3), d(0xD3), d(0x93), d(0xD3), d(0x93), d(0xD0)]
        );
        // Not pressed: Z1, Z2, power-down only.
        touch(&rec, 0, 4095, vec![10], vec![20]);
        let _ = rec.take_ops();
        let _ = t.read();
        assert_eq!(rec.ops(), [d(0xB3), d(0xC3), d(0xD0)]);
    }

    #[test]
    fn calibration_and_clamp() {
        let rec = Recorder::new();
        touch(&rec, 2000, 2000, vec![200], vec![3900]);
        let mut t: Xpt2046<_, RecordingPin> = Xpt2046::new(rec.spi(), None);
        assert_eq!(t.poll_hint(), PollHint::Periodic);
        assert_eq!(t.calibration(), Calibration::DEFAULT_240X320);
        // Not fitted yet: native panel pixels, not clamped.
        assert!(matches!(t.read(), InputData::Pointer(p) if p.point == Point::new(240, 320)));
        // What the engine does when the driver is added for a 320 × 240 landscape display.
        let landscape = crate::touch::test_util::display(320, 240).with_rotation(twine_core::Rotation::Deg90);
        t.fit_to_display(&landscape);
        assert_eq!(t.screen_size(), (320, 240));
        // Native (240, 320) → logical (−1, 240), clamped to (0, 239).
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(0, 239),
                pressed: true
            })
        );
        t.set_calibration(Calibration::IDENTITY);
        assert_eq!(t.kind(), InputKind::Pointer);
        let (_spi, irq) = t.release();
        assert!(irq.is_none());
    }

    #[test]
    fn calibration_survives_rotation() {
        // One calibration (native pixels); the same raw tap lands on the same physical spot of
        // the panel in every rotation: logical = TouchTransform::for_display(info)(native).
        let rec = Recorder::new();
        touch(&rec, 2000, 2000, vec![1950], vec![1000]);
        let mut t: Xpt2046<_, RecordingPin> = Xpt2046::new(rec.spi(), None);
        let native = Calibration::DEFAULT_240X320.apply(1950, 1000);
        for rot in [
            twine_core::Rotation::Deg0,
            twine_core::Rotation::Deg90,
            twine_core::Rotation::Deg180,
            twine_core::Rotation::Deg270,
        ] {
            let (w, h) = if rot.swaps_axes() { (320, 240) } else { (240, 320) };
            let info = crate::touch::test_util::display(w, h).with_rotation(rot);
            t.fit_to_display(&info);
            let expect = TouchTransform::for_display(&info).apply(native.0, native.1);
            assert!(
                matches!(t.read(), InputData::Pointer(p) if p.point == expect),
                "{rot:?}"
            );
        }
    }

    #[test]
    fn spi_error_reads_released() {
        let rec = Recorder::new();
        let mut t: Xpt2046<_, RecordingPin> = Xpt2046::new(rec.spi(), None);
        rec.fail_next();
        assert!(matches!(t.read(), InputData::Pointer(p) if !p.pressed));
    }

    #[test]
    fn async_wait_for_low() {
        use twine_hal::AsyncInputWait;
        let rec = Recorder::new();
        let mut t = dut(&rec);
        block_on(t.wait_for_interrupt());
        assert_eq!(rec.ops(), [BusOp::Wait("irq", false)]);
    }

    #[test]
    fn xpt2046_spi_failure_degrades_then_fails() {
        let rec = Recorder::new();
        touch(&rec, 1000, 500, vec![1000], vec![2000]);
        rec.set_level("irq", false);
        let mut t = dut(&rec).with_fail_after(DeviceHealth::DEFAULT_FAIL_AFTER);
        crate::touch::test_util::assert_bus_failure_sequence(&rec, &mut t);
    }
}
