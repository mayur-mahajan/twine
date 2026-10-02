//! Touch controllers implementing [`InputDevice`](twine_hal::InputDevice) (`Pointer`).
//!
//! | Module | Controller | Bus | Address | IRQ |
//! |--------|------------|-----|---------|-----|
//! | [`xpt2046`] | XPT2046 / ADS7843 resistive | SPI | — | `T_IRQ`, active low |
//! | [`ft6x36`] | Focaltech FT6206/FT6236/FT6336, FT3168 (model flag) capacitive | I2C | `0x38` | `INT`, active low |
//! | [`ft5x06`] | Focaltech FT5206/FT5306/FT5406 capacitive | I2C | `0x38` | `INT`, active low |
//! | [`gt911`] | Goodix GT911 capacitive | I2C | `0x5D` or `0x14` | `INT`, active low (default config) |
//! | [`cst816s`] | Hynitron CST816S capacitive | I2C | `0x15` | `IRQ`, active low pulses |
//! | [`axs5106l`] | AXS5106L capacitive | I2C | `0x63` | `INT`, active low |
//! | [`stmpe811`] | ST STMPE811 resistive controller | I2C | `0x41` (or `0x44`) | `INT`, active low |
//!
//! # Power (P1)
//!
//! With an interrupt pin, every driver reports [`PollHint::Interrupt`](twine_hal::PollHint) and
//! `read()` performs **no bus transaction** while the pin is inactive and the last state was
//! released. While pressed it keeps reading until it sees the release. Without an interrupt pin
//! the engine polls periodically.
//!
//! # Failures
//!
//! Every driver counts **consecutive** failed bus transactions and reports them through
//! [`InputDevice::health`](twine_hal::InputDevice::health): `Degraded { errors }` after the
//! first, `Failed` after `with_fail_after(n)` of them (default
//! [`DeviceHealth::DEFAULT_FAIL_AFTER`](twine_hal::DeviceHealth::DEFAULT_FAIL_AFTER) = 3),
//! `Ok` after the next successful read. While degraded a driver repeats its last good sample
//! (a single glitch in the middle of a press neither releases — which would click — nor moves
//! the pointer); once failed it reports released, and the engine processes a failed device as
//! released anyway (ending a held press with `PressLost`, never a click), raising
//! `FaultKind::InputDevice`. The [`Cst816s`] sleeps and does not answer between touches:
//! only errors while pressed count for it.
//!
//! # Coordinates
//!
//! Capacitive controllers report panel coordinates; a [`TouchTransform`] maps them to the
//! logical screen (swap / mirror / clamp). Every driver derives it **itself** from the display
//! it is registered for ([`InputDevice::fit_to_display`](twine_hal::InputDevice::fit_to_display),
//! called by the engine — `Ui::builder(..).input(touch)` — before the first read): the display's
//! rotation and native size select the axes, so firmware never derives native sizes or rotation
//! tables by hand. A touch panel glued with its axes swapped or mirrored relative to the display
//! is described once with [`with_mount`](Ft6x36::with_mount) ([`TouchMount`]); it is composed
//! with the display's transform at every fit. Drivers used without the engine call
//! [`for_display`](Ft6x36::for_display) themselves; until fitted, a driver reports raw panel
//! coordinates ([`TouchTransform::PASS_THROUGH`]).
//!
//! ```
//! use twine_core::{ColorFormat, Rotation};
//! use twine_drivers::NoPin;
//! use twine_drivers::testkit::Recorder;
//! use twine_drivers::touch::{Ft6x36, TouchMount, TouchTransform};
//! use twine_hal::DisplayInfo;
//!
//! let rec = Recorder::new();
//! let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg90);
//! let touch = Ft6x36::new(rec.i2c(), None::<NoPin>)
//!     .with_mount(TouchMount { mirror_x: true, ..TouchMount::ALIGNED })
//!     .with_fail_after(5)
//!     .for_display(&info); // what the engine does when the touch is added
//! assert_eq!(touch.transform(), TouchTransform::for_display(&info).with_mount(TouchMount { mirror_x: true, ..TouchMount::ALIGNED }));
//! ```
//!
//! Resistive controllers ([`Xpt2046`], [`Stmpe811`]) need a per-module [`Calibration`] (raw
//! readings to logical screen coordinates) instead; fitting them to the display sets the screen
//! size their points are clamped to.

#[cfg(feature = "axs5106l")]
#[cfg_attr(docsrs, doc(cfg(feature = "axs5106l")))]
pub mod axs5106l;
#[cfg(feature = "cst816s")]
#[cfg_attr(docsrs, doc(cfg(feature = "cst816s")))]
pub mod cst816s;
#[cfg(feature = "ft6x36")]
#[cfg_attr(docsrs, doc(cfg(feature = "ft6x36")))]
pub mod ft5x06;
#[cfg(feature = "ft6x36")]
#[cfg_attr(docsrs, doc(cfg(feature = "ft6x36")))]
pub mod ft6x36;
#[cfg(feature = "gt911")]
#[cfg_attr(docsrs, doc(cfg(feature = "gt911")))]
pub mod gt911;
#[cfg(feature = "stmpe811")]
#[cfg_attr(docsrs, doc(cfg(feature = "stmpe811")))]
pub mod stmpe811;
#[cfg(feature = "xpt2046")]
#[cfg_attr(docsrs, doc(cfg(feature = "xpt2046")))]
pub mod xpt2046;

#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "stmpe811",
    feature = "cst816s",
    feature = "axs5106l"
))]
use twine_core::Point;
#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "cst816s",
    feature = "axs5106l"
))]
use twine_hal::DisplayInfo;
#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "stmpe811",
    feature = "cst816s",
    feature = "axs5106l"
))]
use {
    embedded_hal::digital::InputPin,
    twine_core::log::{debug, error},
    twine_hal::{DeviceHealth, PointerData, PollHint},
};

#[cfg(feature = "axs5106l")]
pub use axs5106l::Axs5106l;
#[cfg(feature = "cst816s")]
pub use cst816s::Cst816s;
#[cfg(feature = "ft6x36")]
pub use ft5x06::Ft5x06;
#[cfg(feature = "ft6x36")]
pub use ft6x36::{Ft6x36, FtModel};
#[cfg(feature = "gt911")]
pub use gt911::Gt911;
#[cfg(feature = "stmpe811")]
pub use stmpe811::Stmpe811;
pub use twine_hal::{Calibration, TouchMount, TouchTransform};
#[cfg(feature = "xpt2046")]
pub use xpt2046::Xpt2046;

/// The coordinate mapping of a capacitive touch driver: the display's transform (set when the
/// driver is fitted) composed with the panel's [`TouchMount`].
#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "cst816s",
    feature = "axs5106l"
))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct PanelMap {
    /// The aligned panel's transform of the display last fitted to (`None`: not fitted yet).
    display: Option<TouchTransform>,
    mount: TouchMount,
    /// `display` composed with `mount`; the only field read per sample.
    transform: TouchTransform,
}

#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "cst816s",
    feature = "axs5106l"
))]
impl PanelMap {
    pub const fn new() -> Self {
        Self {
            display: None,
            mount: TouchMount::ALIGNED,
            transform: TouchTransform::PASS_THROUGH,
        }
    }

    /// Fits the mapping to `info` (keeps the mount).
    pub fn fit(&mut self, info: &DisplayInfo) {
        self.display = Some(TouchTransform::for_display(info));
        self.update();
    }

    /// Sets the mount (applied now if already fitted, else at the fit).
    pub fn set_mount(&mut self, mount: TouchMount) {
        self.mount = mount;
        self.update();
    }

    fn update(&mut self) {
        self.transform = match self.display {
            Some(d) => d.with_mount(self.mount),
            None => TouchTransform::PASS_THROUGH,
        };
    }

    pub const fn transform(&self) -> TouchTransform {
        self.transform
    }

    #[inline]
    pub fn apply(&self, x: u16, y: u16) -> Point {
        self.transform.apply(i32::from(x), i32::from(y))
    }
}

/// The builder methods of the coordinate mapping, for a capacitive driver with a
/// `map: PanelMap` field (`$ty`: the driver's name, for the examples).
#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "cst816s",
    feature = "axs5106l"
))]
macro_rules! panel_map_methods {
    ($ty:ident) => {
        /// Describes how the touch panel's raw axes sit on the display's native axes (default
        /// [`TouchMount::ALIGNED`](twine_hal::TouchMount::ALIGNED)). Composed with the display's
        /// rotation whenever the driver is fitted, so set it once, in any order with
        /// [`for_display`](Self::for_display). Never panics.
        ///
        #[doc = concat!("```\nuse twine_core::{ColorFormat, Point};\nuse twine_drivers::NoPin;\nuse twine_drivers::testkit::Recorder;\nuse twine_drivers::touch::{", stringify!($ty), ", TouchMount};\nuse twine_hal::DisplayInfo;\n\nlet rec = Recorder::new();\nlet info = DisplayInfo::new(172, 320, ColorFormat::Rgb565Swapped);\nlet touch = ", stringify!($ty), "::new(rec.i2c(), None::<NoPin>)\n    .with_mount(TouchMount { mirror_x: true, ..TouchMount::ALIGNED })\n    .for_display(&info);\nassert_eq!(touch.transform().apply(0, 5), Point::new(171, 5)); // raw X runs right to left\n```")]
        #[must_use]
        pub fn with_mount(mut self, mount: ::twine_hal::TouchMount) -> Self {
            self.map.set_mount(mount);
            self
        }

        /// Fits the coordinate mapping to the display described by `info` (its rotation, native
        /// and logical size), composed with the [`with_mount`](Self::with_mount) mount. The
        /// engine does this when the driver is added (`Ui::builder(..).input(touch)`,
        /// `Engine::add_input`) through
        /// [`InputDevice::fit_to_display`](twine_hal::InputDevice::fit_to_display); call it
        /// yourself only when you read the driver without the engine. Never panics.
        ///
        #[doc = concat!("```\nuse twine_core::{ColorFormat, Point, Rotation};\nuse twine_drivers::NoPin;\nuse twine_drivers::testkit::Recorder;\nuse twine_drivers::touch::", stringify!($ty), ";\nuse twine_hal::DisplayInfo;\n\nlet rec = Recorder::new();\n// A 240 × 320 panel turned to landscape.\nlet info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg90);\nlet touch = ", stringify!($ty), "::new(rec.i2c(), None::<NoPin>).for_display(&info);\nassert_eq!(touch.transform().apply(10, 20), Point::new(299, 10));\n```")]
        #[must_use]
        pub fn for_display(mut self, info: &::twine_hal::DisplayInfo) -> Self {
            self.map.fit(info);
            self
        }

        /// The current mapping of raw panel coordinates to the logical screen
        /// ([`TouchTransform::PASS_THROUGH`](twine_hal::TouchTransform::PASS_THROUGH) until the
        /// driver is fitted to a display).
        ///
        #[doc = concat!("```\nuse twine_drivers::NoPin;\nuse twine_drivers::testkit::Recorder;\nuse twine_drivers::touch::{", stringify!($ty), ", TouchTransform};\n\nlet rec = Recorder::new();\nlet touch = ", stringify!($ty), "::new(rec.i2c(), None::<NoPin>);\nassert_eq!(touch.transform(), TouchTransform::PASS_THROUGH);\n```")]
        #[must_use]
        pub fn transform(&self) -> ::twine_hal::TouchTransform {
            self.map.transform()
        }
    };
}
#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "cst816s",
    feature = "axs5106l"
))]
pub(crate) use panel_map_methods;

/// Noise filter of resistive controllers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Filter {
    /// Use the last sample.
    None,
    /// Median of (up to) the last 3 samples of each axis.
    #[default]
    Median3,
}

/// Median of up to 3 values (`vals` non-empty).
#[cfg(feature = "stmpe811")]
pub(crate) fn median3(vals: &[u16]) -> u16 {
    match *vals {
        [a] => a,
        [a, b] => a.midpoint(b),
        [a, b, c, ..] => a.max(b).min(a.min(b).max(c)),
        [] => 0,
    }
}

#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "stmpe811",
    feature = "cst816s",
    feature = "axs5106l"
))]
/// Interrupt line, last state and health shared by the I2C touch drivers.
#[derive(Debug)]
pub(crate) struct IrqState<IRQ> {
    pub irq: Option<IRQ>,
    pub active_high: bool,
    pub last: PointerData,
    pub health: DeviceHealth,
    /// Consecutive errors after which the device is `Failed`.
    pub fail_after: u16,
}

#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "stmpe811",
    feature = "cst816s",
    feature = "axs5106l"
))]
impl<IRQ> IrqState<IRQ> {
    pub const fn new(irq: Option<IRQ>) -> Self {
        Self {
            irq,
            active_high: false,
            last: PointerData {
                point: Point::new(0, 0),
                pressed: false,
            },
            health: DeviceHealth::Ok,
            fail_after: DeviceHealth::DEFAULT_FAIL_AFTER,
        }
    }

    /// A successful read of `data`: the device is healthy again.
    pub fn ok(&mut self, name: &str, data: PointerData) -> PointerData {
        if !self.health.is_ok() {
            debug!(target: "twine::driver", "{} reads again ({:?} before)", name, self.health);
        }
        self.health = DeviceHealth::Ok;
        self.update(name, data)
    }

    /// A failed read: counts the error and returns the sample to report, the last good one
    /// while degraded, released once failed.
    pub fn error(&mut self, name: &str) -> PointerData {
        let before = self.health;
        self.health = before.after_error(self.fail_after);
        if !self.health.is_failed() {
            return self.last;
        }
        if !before.is_failed() {
            error!(target: "twine::driver", "{}: failed ({} consecutive bus errors)", name, self.fail_after.max(1));
        }
        let released = self.released();
        self.update(name, released)
    }

    pub fn poll_hint(&self) -> PollHint {
        if self.irq.is_some() {
            PollHint::Interrupt
        } else {
            PollHint::Periodic
        }
    }

    /// Stores a new sample (logging press/release transitions) and returns it.
    pub fn update(&mut self, name: &str, data: PointerData) -> PointerData {
        if data.pressed != self.last.pressed {
            debug!(
                target: "twine::driver",
                "{} {} at {}",
                name,
                if data.pressed { "press" } else { "release" },
                data.point
            );
        }
        self.last = data;
        data
    }

    /// The last state as released.
    pub fn released(&self) -> PointerData {
        PointerData {
            point: self.last.point,
            pressed: false,
        }
    }
}

#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "stmpe811",
    feature = "cst816s",
    feature = "axs5106l"
))]
impl<IRQ: InputPin> IrqState<IRQ> {
    /// Whether the bus must be read: always without IRQ pin, while pressed, or when the
    /// interrupt line is active (errors reading the pin count as active).
    pub fn should_read(&mut self) -> bool {
        if self.last.pressed {
            return true;
        }
        match self.irq.as_mut() {
            None => true,
            Some(p) => p.is_high().map_or(true, |high| high == self.active_high),
        }
    }
}

#[cfg(feature = "async")]
#[cfg(any(
    feature = "ft6x36",
    feature = "gt911",
    feature = "stmpe811",
    feature = "cst816s",
    feature = "axs5106l"
))]
impl<IRQ: embedded_hal_async::digital::Wait> IrqState<IRQ> {
    /// Waits for the interrupt line to become active; never completes without an IRQ pin.
    pub async fn wait(&mut self, name: &str) {
        let active_high = self.active_high;
        match self.irq.as_mut() {
            Some(p) => {
                let r = if active_high {
                    p.wait_for_high().await
                } else {
                    p.wait_for_low().await
                };
                if r.is_err() {
                    twine_core::log::warn!(target: "twine::driver", "{}: IRQ wait failed", name);
                }
            }
            None => core::future::pending::<()>().await,
        }
    }
}

/// Implements the shared builder methods and `AsyncInputWait` for an I2C touch driver with an
/// `irq: IrqState<IRQ>` field.
#[cfg(any(feature = "ft6x36", feature = "gt911", feature = "stmpe811"))]
macro_rules! irq_touch_common {
    ($ty:ident, $name:literal) => {
        #[cfg(feature = "async")]
        impl<I2C, IRQ: ::embedded_hal_async::digital::Wait> ::twine_hal::AsyncInputWait for $ty<I2C, IRQ> {
            /// Waits until the interrupt line is active; never completes without an IRQ pin.
            async fn wait_for_interrupt(&mut self) {
                self.irq.wait($name).await;
            }
        }
    };
}
#[cfg(any(feature = "ft6x36", feature = "gt911", feature = "stmpe811"))]
pub(crate) use irq_touch_common;

#[cfg(test)]
pub(crate) mod test_util {
    use alloc::collections::BTreeMap;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use crate::mock::Recorder;
    use twine_core::ColorFormat;
    use twine_hal::{DeviceHealth, DisplayInfo, InputData, InputDevice, PointerData};

    /// An unrotated `w × h` display to fit a driver to (identity transform clamped to it).
    pub fn display(w: u16, h: u16) -> DisplayInfo {
        DisplayInfo::new(w, h, ColorFormat::Rgb565Swapped)
    }

    /// Takes a touch driver that reads pressed from `rec` through a dead bus and back:
    /// the last good sample is held while degraded, released once failed (and while it stays
    /// failed), and the next successful read makes it healthy again.
    pub fn assert_bus_failure_sequence(rec: &Recorder, dev: &mut impl InputDevice) {
        let pressed = dev.read();
        let InputData::Pointer(PointerData { point, pressed: true }) = pressed else {
            panic!("the driver must read pressed first, got {pressed:?}");
        };
        assert_eq!(dev.health(), DeviceHealth::Ok);
        rec.set_bus_down(true);
        for errors in 1..DeviceHealth::DEFAULT_FAIL_AFTER {
            assert_eq!(dev.read(), pressed, "degraded: the last good sample is held");
            assert_eq!(dev.health(), DeviceHealth::Degraded { errors });
        }
        let released = InputData::Pointer(PointerData {
            point,
            pressed: false,
        });
        assert_eq!(dev.read(), released);
        assert_eq!(dev.health(), DeviceHealth::Failed);
        assert_eq!(dev.read(), released);
        assert_eq!(dev.health(), DeviceHealth::Failed);
        rec.set_bus_down(false);
        let _ = dev.read();
        assert_eq!(dev.health(), DeviceHealth::Ok);
    }

    /// A register file answering I2C reads: the register address is the bytes written in the
    /// same transaction (1 or 2 bytes, big-endian); reads return consecutive bytes from there.
    #[derive(Clone, Default)]
    pub struct Regs(pub Rc<RefCell<BTreeMap<u16, u8>>>);

    impl Regs {
        pub fn install(rec: &Recorder) -> Self {
            let regs = Self::default();
            let r = regs.clone();
            rec.set_i2c_responder(move |_addr, written, rx| {
                let base = match written {
                    [a] => u16::from(*a),
                    [h, l, ..] => u16::from_be_bytes([*h, *l]),
                    [] => 0,
                };
                let m = r.0.borrow();
                for (i, b) in rx.iter_mut().enumerate() {
                    *b = m.get(&(base + i as u16)).copied().unwrap_or(0);
                }
            });
            regs
        }

        pub fn set(&self, reg: u16, bytes: &[u8]) {
            let mut m = self.0.borrow_mut();
            for (i, b) in bytes.iter().enumerate() {
                m.insert(reg + i as u16, *b);
            }
        }

        pub fn get(&self, reg: u16) -> u8 {
            self.0.borrow().get(&reg).copied().unwrap_or(0)
        }

        #[allow(dead_code)]
        pub fn dump(&self) -> Vec<(u16, u8)> {
            self.0.borrow().iter().map(|(k, v)| (*k, *v)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median3_values() {
        assert_eq!(median3(&[5]), 5);
        assert_eq!(median3(&[4, 6]), 5);
        assert_eq!(median3(&[9, 1, 5]), 5);
        assert_eq!(median3(&[1, 4000, 3]), 3);
        assert_eq!(median3(&[]), 0);
    }
}
