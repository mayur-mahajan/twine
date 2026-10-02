//! Touch coordinates: [`TouchTransform`] maps raw panel coordinates of a touch controller to the
//! logical (rotated) screen, [`TouchMount`] describes how the touch panel sits on the display.
//!
//! A touch controller reports points in its own axes. Two things turn them into logical screen
//! coordinates:
//!
//! 1. the **mount** ([`TouchMount`]): how the raw axes relate to the display's *native* (unrotated)
//!    axes — a property of the module (some panels are glued with an axis mirrored or swapped);
//! 2. the **display rotation** ([`DisplayInfo::rotation`]): how the logical screen is turned
//!    relative to the native panel — a property of the display configuration.
//!
//! [`TouchTransform::for_display`] derives (2) from the display's [`DisplayInfo`], and
//! [`TouchTransform::with_mount`] composes (1) in front of it. Pointer drivers do this in
//! [`InputDevice::fit_to_display`](crate::InputDevice::fit_to_display), which the engine calls
//! when the device is registered for a display, so applications never derive native sizes or
//! rotation tables by hand.
//!
//! ```
//! use twine_core::{ColorFormat, Point, Rotation};
//! use twine_hal::{DisplayInfo, TouchMount, TouchTransform};
//!
//! // A 240 × 320 panel turned to landscape: the logical screen is 320 × 240.
//! let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg90);
//! let t = TouchTransform::for_display(&info);
//! assert_eq!((t.width, t.height), (320, 240));
//! assert_eq!(t.apply(10, 20), Point::new(299, 10)); // native (10, 20) → logical
//!
//! // The same panel with the touch film's raw X axis mirrored.
//! let mount = TouchMount { mirror_x: true, ..TouchMount::ALIGNED };
//! let mirrored = TouchTransform::for_display(&info).with_mount(mount);
//! assert_eq!(mirrored.apply(239 - 10, 20), t.apply(10, 20));
//! ```

use twine_core::Point;

use crate::{DisplayInfo, Rotation};

/// How a touch panel's raw axes relate to the display's native (unrotated) axes.
///
/// Applied to a raw point in this order: swap X/Y, mirror X (`native_width − 1 − x`), mirror Y
/// (`native_height − 1 − y`). [`ALIGNED`](Self::ALIGNED) (the default) is a touch panel whose raw
/// axes run like the display's native columns and rows, which is how most modules are built.
///
/// ```
/// use twine_hal::TouchMount;
///
/// assert_eq!(TouchMount::default(), TouchMount::ALIGNED);
/// let mirrored = TouchMount { mirror_x: true, ..TouchMount::ALIGNED };
/// assert!(!mirrored.swap_xy);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TouchMount {
    /// The raw X axis runs along the native rows (raw X and Y swapped).
    pub swap_xy: bool,
    /// The (swapped) raw X axis runs opposite to the native columns.
    pub mirror_x: bool,
    /// The (swapped) raw Y axis runs opposite to the native rows.
    pub mirror_y: bool,
}

impl TouchMount {
    /// Raw axes aligned with the native display axes (no swap, no mirror).
    pub const ALIGNED: Self = Self {
        swap_xy: false,
        mirror_x: false,
        mirror_y: false,
    };
}

/// Maps raw panel coordinates of a touch controller to logical screen coordinates.
///
/// Applied in this order: swap X/Y, mirror X (`width − 1 − x`), mirror Y, clamp to
/// `width × height` (the **logical** screen size). Integer only; [`apply`](Self::apply) is a few
/// compares and subtractions per point.
///
/// Build it from the display ([`for_display`](Self::for_display)), optionally composed with the
/// panel's [`TouchMount`] ([`with_mount`](Self::with_mount)); the fields are public for
/// transforms of unusual hardware.
///
/// ```
/// use twine_core::Point;
/// use twine_hal::TouchTransform;
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
    /// Raw coordinates passed through unchanged (only clamped to `0..u16::MAX`): what a pointer
    /// driver reports before it is fitted to a display.
    pub const PASS_THROUGH: Self = Self::identity(u16::MAX, u16::MAX);

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

    /// The transform of an aligned touch panel ([`TouchMount::ALIGNED`]) on a display rotated by
    /// `rotation` whose native (unrotated) size is `native_w × native_h`. Software rotation and
    /// the `MADCTL` tables of `twine-drivers` turn the picture the same way, so this holds for
    /// both. Prefer [`for_display`](Self::for_display), which reads both from the display.
    ///
    /// ```
    /// use twine_core::{Point, Rotation};
    /// use twine_hal::TouchTransform;
    ///
    /// let t = TouchTransform::for_rotation(Rotation::Deg180, 240, 320);
    /// assert_eq!(t.apply(0, 0), Point::new(239, 319));
    /// ```
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

    /// The transform of an aligned touch panel on the display described by `info`: its
    /// rotation and native size ([`DisplayInfo::native_size`]) select the axes, its logical
    /// size is the clamp. Compose a mounted panel with [`with_mount`](Self::with_mount).
    /// Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Point, Rotation};
    /// use twine_hal::{DisplayInfo, TouchTransform};
    ///
    /// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg270);
    /// let t = TouchTransform::for_display(&info);
    /// assert_eq!(t, TouchTransform::for_rotation(Rotation::Deg270, 240, 320));
    /// assert_eq!(t.apply(0, 0), Point::new(0, 239));
    /// ```
    #[must_use]
    pub const fn for_display(info: &DisplayInfo) -> Self {
        let (native_w, native_h) = info.native_size();
        Self::for_rotation(info.rotation, native_w, native_h)
    }

    /// This transform with a touch panel mounted as `mount` in front of it: raw points are first
    /// mapped to the display's native axes by `mount`, then by `self` (a transform of an
    /// aligned panel, e.g. from [`for_display`](Self::for_display)). The result is still one
    /// swap and two mirrors, so [`apply`](Self::apply) costs the same.
    ///
    /// ```
    /// use twine_core::{Point, Rotation};
    /// use twine_hal::{TouchMount, TouchTransform};
    ///
    /// // A 172 × 320 panel whose touch film has its raw X axis mirrored.
    /// let mount = TouchMount { mirror_x: true, ..TouchMount::ALIGNED };
    /// let t = TouchTransform::for_rotation(Rotation::Deg0, 172, 320).with_mount(mount);
    /// assert_eq!(t.apply(0, 5), Point::new(171, 5));
    /// let t = TouchTransform::for_rotation(Rotation::Deg270, 172, 320).with_mount(mount);
    /// assert_eq!(t.apply(10, 20), Point::new(20, 10)); // swap only
    /// ```
    #[must_use]
    pub const fn with_mount(self, mount: TouchMount) -> Self {
        // Both maps are "swap, then mirror". After this transform's swap, the mount's X mirror
        // lands on the logical Y axis (and its Y mirror on X); mirrors of the same axis cancel.
        let (mx, my) = if self.swap_xy {
            (mount.mirror_y, mount.mirror_x)
        } else {
            (mount.mirror_x, mount.mirror_y)
        };
        Self {
            swap_xy: self.swap_xy != mount.swap_xy,
            invert_x: self.invert_x != mx,
            invert_y: self.invert_y != my,
            width: self.width,
            height: self.height,
        }
    }

    /// The inverse transform: maps logical screen coordinates back to the raw (native, for an
    /// aligned panel) coordinates this transform maps from, clamped to the native size. Used
    /// where a known logical point must be expressed in panel coordinates, e.g. the targets of
    /// a touch calibration ([`Calibration`](crate::Calibration) maps to native coordinates, so
    /// it survives a rotation). Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Point, Rotation};
    /// use twine_hal::{DisplayInfo, TouchTransform};
    ///
    /// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg90);
    /// let t = TouchTransform::for_display(&info);
    /// let native = t.inverse();
    /// assert_eq!((native.width, native.height), (240, 320));
    /// let p = native.apply(299, 10);
    /// assert_eq!(p, Point::new(10, 20));
    /// assert_eq!(t.apply(p.x, p.y), Point::new(299, 10));
    /// ```
    #[must_use]
    pub const fn inverse(self) -> Self {
        // Forward: swap, then mirror against the logical size. Inverse: un-mirror, then
        // un-swap — written as "swap, then mirror" the mirrors exchange axes with the swap.
        if self.swap_xy {
            Self {
                swap_xy: true,
                invert_x: self.invert_y,
                invert_y: self.invert_x,
                width: self.height,
                height: self.width,
            }
        } else {
            self
        }
    }

    /// Transforms a raw point. Never panics (a zero size counts as 1).
    #[must_use]
    #[inline]
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

#[cfg(test)]
mod tests {
    use super::*;
    use twine_core::ColorFormat;

    const ROTATIONS: [Rotation; 4] = [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ];

    /// The derivation every firmware main used to write by hand before `for_display`.
    fn manual(info: &DisplayInfo) -> TouchTransform {
        let native = if info.rotation.swaps_axes() {
            (info.height, info.width)
        } else {
            (info.width, info.height)
        };
        TouchTransform::for_rotation(info.rotation, native.0, native.1)
    }

    #[test]
    fn inverse_round_trips_every_rotation_and_mount() {
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            for bits in 0..8u8 {
                let mount = TouchMount {
                    swap_xy: bits & 1 != 0,
                    mirror_x: bits & 2 != 0,
                    mirror_y: bits & 4 != 0,
                };
                let t = TouchTransform::for_rotation(rot, 24, 32).with_mount(mount);
                let inv = t.inverse();
                for (x, y) in [
                    (0, 0),
                    (5, 7),
                    (i32::from(t.width) - 1, i32::from(t.height) - 1),
                    (3, 20),
                ] {
                    let raw = inv.apply(x, y);
                    assert_eq!(t.apply(raw.x, raw.y), Point::new(x, y), "{rot:?} {mount:?}");
                }
                assert_eq!(inv.inverse(), t);
            }
        }
    }

    #[test]
    fn for_display_matches_the_manual_derivation() {
        // Table: native panel size, rotation → logical size and the image of three points.
        type Row = (u16, u16, Rotation, (u16, u16), [(i32, i32, Point); 3]);
        let table: [Row; 4] = [
            (
                240,
                320,
                Rotation::Deg0,
                (240, 320),
                [
                    (0, 0, Point::new(0, 0)),
                    (10, 20, Point::new(10, 20)),
                    (239, 319, Point::new(239, 319)),
                ],
            ),
            (
                240,
                320,
                Rotation::Deg90,
                (320, 240),
                [
                    (0, 0, Point::new(319, 0)),
                    (10, 20, Point::new(299, 10)),
                    (239, 319, Point::new(0, 239)),
                ],
            ),
            (
                240,
                320,
                Rotation::Deg180,
                (240, 320),
                [
                    (0, 0, Point::new(239, 319)),
                    (10, 20, Point::new(229, 299)),
                    (239, 319, Point::new(0, 0)),
                ],
            ),
            (
                240,
                320,
                Rotation::Deg270,
                (320, 240),
                [
                    (0, 0, Point::new(0, 239)),
                    (10, 20, Point::new(20, 229)),
                    (239, 319, Point::new(319, 0)),
                ],
            ),
        ];
        for (nw, nh, rot, logical, points) in table {
            let (lw, lh) = if rot.swaps_axes() { (nh, nw) } else { (nw, nh) };
            let info = DisplayInfo::new(lw, lh, ColorFormat::Rgb565).with_rotation(rot);
            assert_eq!(info.native_size(), (nw, nh), "{rot:?}");
            let t = TouchTransform::for_display(&info);
            assert_eq!(t, manual(&info), "{rot:?}");
            assert_eq!((t.width, t.height), logical, "{rot:?}");
            for (x, y, p) in points {
                assert_eq!(t.apply(x, y), p, "{rot:?} ({x}, {y})");
            }
        }
    }

    #[test]
    fn for_rotation_matches_software_rotation() {
        // The engine maps logical (x, y) to physical with `Rect::rotate_in`.
        let (nw, nh) = (240u16, 320u16);
        for rot in ROTATIONS {
            let t = TouchTransform::for_rotation(rot, nw, nh);
            let (lw, lh) = (i32::from(t.width), i32::from(t.height));
            for (x, y) in [(0, 0), (5, 7), (lw - 1, lh - 1), (lw / 2, 3)] {
                let phys = twine_core::Rect::from_xywh(x, y, 1, 1).rotate_in(rot, lw, lh);
                assert_eq!(t.apply(phys.x0, phys.y0), Point::new(x, y), "{rot:?} ({x}, {y})");
            }
        }
    }

    #[test]
    fn with_mount_equals_mount_then_rotation() {
        let (nw, nh) = (172u16, 320u16);
        for rot in ROTATIONS {
            for bits in 0..8u8 {
                let mount = TouchMount {
                    swap_xy: bits & 1 != 0,
                    mirror_x: bits & 2 != 0,
                    mirror_y: bits & 4 != 0,
                };
                // The mount as a map from raw to native coordinates.
                let to_native = TouchTransform {
                    swap_xy: mount.swap_xy,
                    invert_x: mount.mirror_x,
                    invert_y: mount.mirror_y,
                    width: nw,
                    height: nh,
                };
                let rotate = TouchTransform::for_rotation(rot, nw, nh);
                let composed = rotate.with_mount(mount);
                let (rw, rh) = if mount.swap_xy { (nh, nw) } else { (nw, nh) };
                for (x, y) in [(0, 0), (5, 7), (i32::from(rw) - 1, i32::from(rh) - 1), (86, 3)] {
                    let native = to_native.apply(x, y);
                    assert_eq!(
                        composed.apply(x, y),
                        rotate.apply(native.x, native.y),
                        "{rot:?} {mount:?} ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn identity_clamps_and_pass_through_keeps_raw_values() {
        let id = TouchTransform::identity(100, 50);
        assert_eq!(id.apply(10, 20), Point::new(10, 20));
        assert_eq!(id.apply(200, 60), Point::new(99, 49));
        assert_eq!(
            TouchTransform::PASS_THROUGH.apply(4000, 123),
            Point::new(4000, 123)
        );
        assert_eq!(TouchTransform::identity(0, 0).apply(5, 5), Point::new(0, 0));
    }
}
