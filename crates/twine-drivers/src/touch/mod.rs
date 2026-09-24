//! Touch controllers implementing [`InputDevice`](twine_hal::InputDevice) (`Pointer`).
//!
//! | Module | Controller | Bus | Address | IRQ |
//! |--------|------------|-----|---------|-----|
//! | [`xpt2046`] | XPT2046 / ADS7843 resistive | SPI | — | `T_IRQ`, active low |
//! | [`ft6x36`] | Focaltech FT6206/FT6236/FT6336 capacitive | I2C | `0x38` | `INT`, active low |
//! | [`ft5x06`] | Focaltech FT5206/FT5306/FT5406 capacitive | I2C | `0x38` | `INT`, active low |
//! | [`gt911`] | Goodix GT911 capacitive | I2C | `0x5D` or `0x14` | `INT`, active low (default config) |
//! | [`cst816s`] | Hynitron CST816S capacitive | I2C | `0x15` | `IRQ`, active low pulses |
//! | [`stmpe811`] | ST STMPE811 resistive controller | I2C | `0x41` (or `0x44`) | `INT`, active low |
//!
//! # Power (P1)
//!
//! With an interrupt pin, every driver reports [`PollHint::Interrupt`](twine_hal::PollHint) and
//! `read()` performs **no bus transaction** while the pin is inactive and the last state was
//! released. While pressed it keeps reading until it sees the release. Without an interrupt pin
//! the engine polls periodically.
//!
//! # Coordinates
//!
//! Capacitive controllers report panel coordinates; [`TouchTransform`] maps them to the logical
//! screen (swap / mirror / clamp). Resistive controllers ([`Xpt2046`], [`Stmpe811`]) need a
//! per-module [`Calibration`] instead.

pub mod cst816s;
pub mod ft5x06;
pub mod ft6x36;
pub mod gt911;
pub mod stmpe811;
pub mod xpt2046;

use embedded_hal::digital::InputPin;
use twine_core::Point;
use twine_core::log::debug;
use twine_hal::{PointerData, PollHint, Rotation};

pub use cst816s::Cst816s;
pub use ft5x06::Ft5x06;
pub use ft6x36::Ft6x36;
pub use gt911::Gt911;
pub use stmpe811::Stmpe811;
pub use twine_hal::Calibration;
pub use xpt2046::Xpt2046;

/// Maps raw panel coordinates of a capacitive controller to logical screen coordinates.
///
/// Applied in this order: swap X/Y, mirror X (`width − 1 − x`), mirror Y, clamp to
/// `width × height` (the **logical** screen size).
///
/// ```
/// use twine_core::Point;
/// use twine_drivers::touch::TouchTransform;
///
/// let t = TouchTransform { swap_xy: true, invert_x: true, invert_y: false, width: 320, height: 240 };
/// // Panel point (10, 20) → swap (20, 10) → mirror x (299, 10).
/// assert_eq!(t.apply(10, 20), Point::new(299, 10));
/// assert_eq!(t.apply(-5, 1000), Point::new(0, 0)); // clamped
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TouchTransform {
    /// Swap the raw X and Y axes.
    pub swap_xy: bool,
    /// Mirror X after the swap.
    pub invert_x: bool,
    /// Mirror Y after the swap.
    pub invert_y: bool,
    /// Logical screen width.
    pub width: u16,
    /// Logical screen height.
    pub height: u16,
}

impl TouchTransform {
    /// No swap or mirroring, clamped to `width × height`.
    #[must_use]
    pub const fn identity(width: u16, height: u16) -> Self {
        Self {
            swap_xy: false,
            invert_x: false,
            invert_y: false,
            width,
            height,
        }
    }

    /// The transform matching the engine's **software** display rotation (`DisplayInfo
    /// .hw_rotation = false`) for a touch panel aligned with a `native_w × native_h` display.
    ///
    /// For panels rotated in hardware (MIPI `MADCTL`) the direction of 90°/270° depends on the
    /// panel's `MADCTL` table; if touches come out mirrored, flip `invert_x`/`invert_y`.
    #[must_use]
    pub const fn for_rotation(rotation: Rotation, native_w: u16, native_h: u16) -> Self {
        match rotation {
            Rotation::Deg0 => Self::identity(native_w, native_h),
            Rotation::Deg90 => Self {
                swap_xy: true,
                invert_x: true,
                invert_y: false,
                width: native_h,
                height: native_w,
            },
            Rotation::Deg180 => Self {
                swap_xy: false,
                invert_x: true,
                invert_y: true,
                width: native_w,
                height: native_h,
            },
            Rotation::Deg270 => Self {
                swap_xy: true,
                invert_x: false,
                invert_y: true,
                width: native_h,
                height: native_w,
            },
        }
    }

    /// Transforms a raw point.
    #[must_use]
    pub fn apply(&self, x: i32, y: i32) -> Point {
        let (mut x, mut y) = if self.swap_xy { (y, x) } else { (x, y) };
        let (w, h) = (i32::from(self.width.max(1)), i32::from(self.height.max(1)));
        if self.invert_x {
            x = w - 1 - x;
        }
        if self.invert_y {
            y = h - 1 - y;
        }
        Point::new(x.clamp(0, w - 1), y.clamp(0, h - 1))
    }
}

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
pub(crate) fn median3(vals: &[u16]) -> u16 {
    match *vals {
        [a] => a,
        [a, b] => a.midpoint(b),
        [a, b, c, ..] => a.max(b).min(a.min(b).max(c)),
        [] => 0,
    }
}

/// Interrupt line + last state shared by the I2C touch drivers.
#[derive(Debug)]
pub(crate) struct IrqState<IRQ> {
    pub irq: Option<IRQ>,
    pub active_high: bool,
    pub last: PointerData,
}

impl<IRQ> IrqState<IRQ> {
    pub const fn new(irq: Option<IRQ>) -> Self {
        Self {
            irq,
            active_high: false,
            last: PointerData {
                point: Point::new(0, 0),
                pressed: false,
            },
        }
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
pub(crate) use irq_touch_common;

#[cfg(test)]
pub(crate) mod test_util {
    use alloc::collections::BTreeMap;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use crate::mock::Recorder;

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
    fn transform_swap_invert() {
        let id = TouchTransform::identity(100, 50);
        assert_eq!(id.apply(10, 20), Point::new(10, 20));
        assert_eq!(id.apply(200, 60), Point::new(99, 49));
        let t = TouchTransform {
            swap_xy: false,
            invert_x: true,
            invert_y: true,
            width: 100,
            height: 50,
        };
        assert_eq!(t.apply(0, 0), Point::new(99, 49));
        let t = TouchTransform {
            swap_xy: true,
            invert_x: false,
            invert_y: true,
            width: 50,
            height: 100,
        };
        assert_eq!(t.apply(10, 20), Point::new(20, 89));
    }

    #[test]
    fn for_rotation_matches_software_rotation() {
        // The engine maps logical (x, y) to physical (y, w − 1 − x) for 90° (w = logical width).
        let (nw, nh) = (240u16, 320u16);
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            let t = TouchTransform::for_rotation(rot, nw, nh);
            let (lw, lh) = (i32::from(t.width), i32::from(t.height));
            for (x, y) in [(0, 0), (5, 7), (lw - 1, lh - 1), (lw / 2, 3)] {
                let phys = twine_core::Rect::from_xywh(x, y, 1, 1).rotate_in(rot, lw, lh);
                assert_eq!(t.apply(phys.x0, phys.y0), Point::new(x, y), "{rot:?} ({x}, {y})");
            }
        }
    }

    #[test]
    fn median3_values() {
        assert_eq!(median3(&[5]), 5);
        assert_eq!(median3(&[4, 6]), 5);
        assert_eq!(median3(&[9, 1, 5]), 5);
        assert_eq!(median3(&[1, 4000, 3]), 3);
        assert_eq!(median3(&[]), 0);
    }
}
