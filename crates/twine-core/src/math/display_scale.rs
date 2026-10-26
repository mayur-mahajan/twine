//! [`DisplayScale`]: device pixels per logical pixel.

use core::fmt;

/// Device pixels per logical pixel of a display, stored with 256 = 1× (CSS
/// `devicePixelRatio`, the scale factor of macOS, Windows and Wayland).
///
/// Twine has three length units: **logical pixels** (`Length::Px` and bare integers, the unit
/// application code writes), **density-independent pixels** (`Length::Dp`, logical pixels
/// scaled by the display's DPI) and **device pixels** (`Length::Dev`, the pixels of the panel).
/// A display's scale converts logical to device pixels once, when styles are resolved; layout,
/// rendering, hit-testing and every geometry the engine reports are in device pixels.
///
/// Embedded panels are 1× (the default): a logical pixel *is* a device pixel. Desktop screens
/// are often 2× (Retina) or fractional (1.25×, 1.5×), which the 256-based representation
/// keeps exact for the common factors. The range is [`MIN`](Self::MIN) (½×) to
/// [`MAX`](Self::MAX) (8×); constructors clamp into it, so a scale is never zero.
///
/// The field is private: build scales with [`DisplayScale::pct`], [`DisplayScale::from_ratio`],
/// [`DisplayScale::from_raw_256`] or the constants; the newtype is exactly a `u16`.
///
/// ```
/// use twine_core::DisplayScale;
///
/// let retina = DisplayScale::X2;
/// assert_eq!(retina.to_device(12), 24);
/// assert_eq!(retina.to_logical(24), 12);
///
/// let windows = DisplayScale::pct(125); // 1.25×
/// assert_eq!(windows.to_device(10), 13); // 12.5 rounds half away from zero
/// assert_eq!(windows.to_device(-10), -13);
/// assert_eq!(windows.to_device(1), 1);
/// assert_eq!(DisplayScale::from_ratio(3, 2), DisplayScale::pct(150));
/// assert!(DisplayScale::ONE.is_one());
/// assert_eq!(DisplayScale::pct(150).to_string(), "1.5×");
/// ```
///
/// A bare number is not a scale:
///
/// ```compile_fail
/// let _ = twine_core::DisplayScale(512); // private field: write DisplayScale::X2
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DisplayScale(u16);

impl DisplayScale {
    /// 1×: a logical pixel is a device pixel (every embedded panel).
    pub const ONE: DisplayScale = DisplayScale(256);
    /// 2× (Retina and most hi-DPI desktop screens).
    pub const X2: DisplayScale = DisplayScale(512);
    /// 3× (phones and some 5K/6K screens).
    pub const X3: DisplayScale = DisplayScale(768);
    /// The smallest scale, ½×.
    pub const MIN: DisplayScale = DisplayScale(128);
    /// The largest scale, 8×.
    pub const MAX: DisplayScale = DisplayScale(2048);

    /// The scale whose 256-based representation is `raw` (`256` = 1×, `384` = 1.5×), clamped
    /// to [`MIN`](Self::MIN)..=[`MAX`](Self::MAX).
    ///
    /// ```
    /// use twine_core::DisplayScale;
    /// assert_eq!(DisplayScale::from_raw_256(384).raw_256(), 384);
    /// assert_eq!(DisplayScale::from_raw_256(0), DisplayScale::MIN);
    /// assert_eq!(DisplayScale::from_raw_256(u16::MAX), DisplayScale::MAX);
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_raw_256(raw: u16) -> DisplayScale {
        DisplayScale(if raw < Self::MIN.0 {
            Self::MIN.0
        } else if raw > Self::MAX.0 {
            Self::MAX.0
        } else {
            raw
        })
    }

    /// `p` percent (`125` → 1.25×), rounded to the nearest 1/256 and clamped like
    /// [`from_raw_256`](Self::from_raw_256).
    ///
    /// ```
    /// use twine_core::DisplayScale;
    /// assert_eq!(DisplayScale::pct(200), DisplayScale::X2);
    /// assert_eq!(DisplayScale::pct(175).raw_256(), 448);
    /// ```
    #[inline]
    #[must_use]
    pub const fn pct(p: u16) -> DisplayScale {
        // SAFE-ARITH: `p < 2^16`, so `p · 256 + 50 < 2^25`.
        let raw = (p as u32).wrapping_mul(256).wrapping_add(50) / 100;
        Self::from_raw_256(if raw > u16::MAX as u32 {
            u16::MAX
        } else {
            raw as u16
        })
    }

    /// `num / den` (`from_ratio(3, 2)` = 1.5×), rounded to the nearest 1/256 and clamped like
    /// [`from_raw_256`](Self::from_raw_256). A zero `den` gives [`MAX`](Self::MAX).
    ///
    /// ```
    /// use twine_core::DisplayScale;
    /// assert_eq!(DisplayScale::from_ratio(5, 4), DisplayScale::pct(125));
    /// assert_eq!(DisplayScale::from_ratio(1, 0), DisplayScale::MAX);
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_ratio(num: u16, den: u16) -> DisplayScale {
        if den == 0 {
            return Self::MAX;
        }
        // SAFE-ARITH: `num < 2^16`, so `num · 256 + den / 2 < 2^25`.
        let n = (num as u32).wrapping_mul(256).wrapping_add((den as u32) >> 1);
        // `den ≠ 0` (checked above): the fallback is never taken.
        let raw = match n.checked_div(den as u32) {
            Some(q) => q,
            None => u32::MAX,
        };
        Self::from_raw_256(if raw > u16::MAX as u32 {
            u16::MAX
        } else {
            raw as u16
        })
    }

    /// The 256-based representation (`DisplayScale::ONE.raw_256() == 256`).
    #[inline]
    #[must_use]
    pub const fn raw_256(self) -> u16 {
        self.0
    }

    /// Whether this is 1× (logical and device pixels coincide).
    #[inline]
    #[must_use]
    pub const fn is_one(self) -> bool {
        self.0 == Self::ONE.0
    }

    /// `logical` pixels in device pixels: `v · scale`, rounded half away from zero,
    /// saturating; a non-zero length stays non-zero (a 1 px hairline is at least one device
    /// pixel at every scale).
    ///
    /// ```
    /// use twine_core::DisplayScale;
    /// let s = DisplayScale::pct(150);
    /// assert_eq!(s.to_device(0), 0);
    /// assert_eq!(s.to_device(3), 5); // 4.5 rounds up
    /// assert_eq!(DisplayScale::MIN.to_device(1), 1); // never collapses to 0
    /// assert_eq!(DisplayScale::MAX.to_device(i32::MAX), i32::MAX); // saturates
    /// ```
    #[inline]
    #[must_use]
    pub const fn to_device(self, logical: i32) -> i32 {
        if self.0 == Self::ONE.0 || logical == 0 {
            return logical;
        }
        let mag = (logical as i64).abs();
        // SAFE-ARITH: `mag ≤ 2^31` and `self.0 ≤ 2^11`, so the product is below 2^43.
        let v = mag.wrapping_mul(self.0 as i64).wrapping_add(128) >> 8;
        let v = if v < 1 {
            1
        } else if v > i32::MAX as i64 {
            i32::MAX as i64
        } else {
            v
        };
        // SAFE-ARITH: `1 ≤ v ≤ i32::MAX`, whose negation fits.
        if logical < 0 {
            (v as i32).wrapping_neg()
        } else {
            v as i32
        }
    }

    /// `device` pixels in logical pixels: `v / scale`, rounded half away from zero (the
    /// inverse of [`to_device`](Self::to_device) for exact multiples).
    ///
    /// ```
    /// use twine_core::DisplayScale;
    /// assert_eq!(DisplayScale::X2.to_logical(25), 13); // 12.5 rounds up
    /// assert_eq!(DisplayScale::pct(150).to_logical(-15), -10);
    /// ```
    #[inline]
    #[must_use]
    pub const fn to_logical(self, device: i32) -> i32 {
        if self.0 == Self::ONE.0 {
            return device;
        }
        let s = self.0 as i64;
        let mag = (device as i64).abs();
        // SAFE-ARITH: `mag ≤ 2^31`, so `mag · 256 < 2^40`.
        // `s ≥ MIN = 128` (every constructor clamps): the fallback is never taken.
        let v = match mag.wrapping_mul(256).wrapping_add(s >> 1).checked_div(s) {
            Some(q) => q,
            None => mag,
        };
        // SAFE-ARITH: `v ≤ mag · 2 ≤ 2^32`; clamped to `i32` before the negation.
        let v = if v > i32::MAX as i64 { i32::MAX } else { v as i32 };
        if device < 0 { v.wrapping_neg() } else { v }
    }
}

impl Default for DisplayScale {
    /// [`DisplayScale::ONE`].
    fn default() -> Self {
        DisplayScale::ONE
    }
}

impl fmt::Display for DisplayScale {
    /// `1×`, `1.5×`, `1.25×`: up to two decimals, trailing zeros removed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let int = self.0 >> 8;
        // SAFE-ARITH: the fraction is below 256, so `· 100 + 128 < 2^15`.
        let dec = u32::from(self.0 & 0xFF).wrapping_mul(100).wrapping_add(128) >> 8;
        if dec == 0 {
            return write!(f, "{int}×");
        }
        // `dec < 100`: one decimal when the second is 0.
        let (tens, ones) = (dec.wrapping_div(10), dec.wrapping_rem(10));
        if ones == 0 {
            return write!(f, "{int}.{tens}×");
        }
        write!(f, "{int}.{dec:02}×")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use proptest::prelude::*;

    #[test]
    fn display_formats_common_factors() {
        assert_eq!(DisplayScale::ONE.to_string(), "1×");
        assert_eq!(DisplayScale::X2.to_string(), "2×");
        assert_eq!(DisplayScale::pct(125).to_string(), "1.25×");
        assert_eq!(DisplayScale::pct(150).to_string(), "1.5×");
        assert_eq!(DisplayScale::MIN.to_string(), "0.5×");
    }

    proptest! {
        #[test]
        fn integer_scales_are_exact(v in -1_000_000i32..1_000_000, k in 1u16..=8) {
            let s = DisplayScale::from_raw_256(k * 256);
            prop_assert_eq!(s.to_device(v), v * i32::from(k));
            prop_assert_eq!(s.to_logical(s.to_device(v)), v);
        }

        #[test]
        fn to_device_is_monotonic_and_symmetric(a in -100_000i32..100_000, raw in 128u16..=2048) {
            let s = DisplayScale::from_raw_256(raw);
            prop_assert!(s.to_device(a) <= s.to_device(a + 1));
            prop_assert_eq!(s.to_device(-a), -s.to_device(a));
        }
    }
}
