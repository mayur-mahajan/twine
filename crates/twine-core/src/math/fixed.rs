//! 16.16 fixed point ([`Fx`]) and scale factors ([`Scale`]).

use core::fmt;
use core::num::{NonZeroI64, NonZeroU64};
use core::ops::{Add, Div, Mul, Neg, Sub};

use crate::geometry::sat_i32;

/// Divides `num` by `den`, rounding half away from zero (`den` is non-zero by its type, so
/// the division cannot panic).
#[inline]
pub(crate) fn div_round(num: i64, den: NonZeroI64) -> i64 {
    let n = num.unsigned_abs();
    let d: NonZeroU64 = den.unsigned_abs();
    // SAFE-ARITH: `n ≤ 2^63` and `d / 2 < 2^63`, so the sum is below 2^64.
    let q = (n.wrapping_add(d.get() / 2) / d) as i64;
    // SAFE-ARITH: `q` is `i64::MIN` (as `2^63` cast) only for `num = i64::MIN`, `|den| = 1`;
    // its wrapping negation is then `i64::MIN`, the exact quotient for `den = 1`.
    if (num < 0) == (den.get() < 0) {
        q
    } else {
        q.wrapping_neg()
    }
}

/// Shifts `v` right by 16, rounding half away from zero. `|v| ≤ 2^62` (an `i32` or a product
/// of two `i32`s: the only callers).
#[inline]
const fn shr16_round(v: i64) -> i64 {
    // SAFE-ARITH: `|v| ≤ 2^62`, so `±v + 0x8000` and the negations stay inside `i64`.
    if v >= 0 {
        v.wrapping_add(0x8000) >> 16
    } else {
        (v.wrapping_neg().wrapping_add(0x8000) >> 16).wrapping_neg()
    }
}

/// 16.16 signed fixed-point number (`Fx::from_raw(1 << 16)` is 1.0).
///
/// The raw representation is private; build values with [`Fx::from_int`], [`Fx::from_ratio`]
/// or [`Fx::from_raw`] and read it back with [`Fx::raw`] (both `const`, zero cost).
///
/// Arithmetic saturates at [`Fx::MIN`]/[`Fx::MAX`] and rounds half away from zero, so results
/// are deterministic on every platform.
///
/// ```
/// use twine_core::Fx;
/// let a = Fx::from_int(3);
/// let b = Fx::from_ratio(1, 2);
/// assert_eq!(a * b, Fx::from_ratio(3, 2));
/// assert_eq!((a / b).to_int_round(), 6);
/// assert_eq!(Fx::from_ratio(-5, 2).to_int_round(), -3); // half away from zero
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Fx(pub(crate) i32);

#[allow(clippy::should_implement_trait)] // `mul`/`div` are the documented saturating forms; the traits delegate to them
impl Fx {
    /// Number of fractional bits.
    pub const SHIFT: u32 = 16;
    /// 1.0
    pub const ONE: Fx = Fx(1 << 16);
    /// 0.5
    pub const HALF: Fx = Fx(1 << 15);
    /// 0.0
    pub const ZERO: Fx = Fx(0);
    /// Largest value (≈ 32767.99998).
    pub const MAX: Fx = Fx(i32::MAX);
    /// Smallest value (−32768.0).
    pub const MIN: Fx = Fx(i32::MIN);

    /// The number whose 16.16 representation is `raw` (`from_raw(1 << 16)` is 1.0).
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: i32) -> Fx {
        Fx(raw)
    }

    /// The 16.16 representation (`Fx::ONE.raw() == 1 << 16`).
    #[inline]
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Converts an integer (saturating outside −32768..=32767).
    #[must_use]
    pub const fn from_int(v: i32) -> Fx {
        Fx(sat_i32((v as i64) << 16))
    }

    /// `num / den`, rounded. `den == 0` returns [`Fx::ZERO`] and logs a warning.
    #[must_use]
    pub fn from_ratio(num: i32, den: i32) -> Fx {
        let Some(den) = NonZeroI64::new(i64::from(den)) else {
            crate::warn!(target: "twine::core", "Fx::from_ratio: division by zero ({}/0)", num);
            return Fx::ZERO;
        };
        Fx(sat_i32(div_round(i64::from(num) << 16, den)))
    }

    /// Largest integer `<= self`.
    #[must_use]
    pub const fn to_int_floor(self) -> i32 {
        self.0 >> 16
    }

    /// Nearest integer, halves rounded away from zero.
    #[must_use]
    pub const fn to_int_round(self) -> i32 {
        shr16_round(self.0 as i64) as i32
    }

    /// Smallest integer `>= self`.
    #[must_use]
    pub const fn to_int_ceil(self) -> i32 {
        // SAFE-ARITH: an `i32` plus `0xFFFF` fits in `i64`.
        ((self.0 as i64).wrapping_add(0xFFFF) >> 16) as i32
    }

    /// Fractional part above the floor, in 1/65536 (always non-negative: `-0.25` → `0.75`).
    #[must_use]
    pub const fn frac(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    /// Absolute value (saturating: `abs(MIN) == MAX`).
    #[must_use]
    pub const fn abs(self) -> Fx {
        Fx(self.0.saturating_abs())
    }

    /// Product, rounded half away from zero, saturating.
    #[must_use]
    pub const fn mul(self, o: Fx) -> Fx {
        // SAFE-ARITH: a product of two `i32`s is at most 2^62 in magnitude.
        Fx(sat_i32(shr16_round((self.0 as i64).wrapping_mul(o.0 as i64))))
    }

    /// Quotient, rounded half away from zero, saturating. Division by zero saturates to
    /// [`Fx::MAX`] / [`Fx::MIN`] by the sign of `self` (zero stays zero) and logs a warning.
    #[must_use]
    pub fn div(self, o: Fx) -> Fx {
        let Some(den) = NonZeroI64::new(i64::from(o.0)) else {
            crate::warn!(target: "twine::core", "Fx::div: division by zero");
            return match self.0 {
                0 => Fx::ZERO,
                v if v > 0 => Fx::MAX,
                _ => Fx::MIN,
            };
        };
        Fx(sat_i32(div_round(i64::from(self.0) << 16, den)))
    }

    /// Saturating addition.
    #[must_use]
    pub const fn saturating_add(self, o: Fx) -> Fx {
        Fx(self.0.saturating_add(o.0))
    }

    /// Saturating subtraction.
    #[must_use]
    pub const fn saturating_sub(self, o: Fx) -> Fx {
        Fx(self.0.saturating_sub(o.0))
    }

    /// Linear interpolation from `a` to `b` with `t` in `0..=1024`.
    #[must_use]
    pub const fn lerp(a: Fx, b: Fx, t: u16) -> Fx {
        Fx(crate::math::lerp_i32(a.0, b.0, t))
    }
}

impl Add for Fx {
    type Output = Fx;
    #[inline]
    fn add(self, o: Fx) -> Fx {
        self.saturating_add(o)
    }
}

impl Sub for Fx {
    type Output = Fx;
    #[inline]
    fn sub(self, o: Fx) -> Fx {
        self.saturating_sub(o)
    }
}

impl Neg for Fx {
    type Output = Fx;
    #[inline]
    fn neg(self) -> Fx {
        Fx(self.0.saturating_neg())
    }
}

impl Mul for Fx {
    type Output = Fx;
    #[inline]
    fn mul(self, o: Fx) -> Fx {
        Fx::mul(self, o)
    }
}

impl Div for Fx {
    type Output = Fx;
    #[inline]
    fn div(self, o: Fx) -> Fx {
        Fx::div(self, o)
    }
}

impl fmt::Display for Fx {
    /// Decimal with 4 fractional digits, e.g. `-1.2500`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = i64::from(self.0);
        let a = v.unsigned_abs();
        let mut int = a >> 16;
        // SAFE-ARITH: `a & 0xFFFF < 2^16`, so the sum is below 2^30.
        let mut dec = (a & 0xFFFF).wrapping_mul(10_000).wrapping_add(0x8000) >> 16;
        if dec >= 10_000 {
            // SAFE-ARITH: `int = |v| >> 16 < 2^16`, and `dec ≥ 10_000`.
            int = int.wrapping_add(1);
            dec = dec.wrapping_sub(10_000);
        }
        let sign = if v < 0 { "-" } else { "" };
        write!(f, "{sign}{int}.{dec:04}")
    }
}

/// Scale factor, stored with 256 = 1.0 (LVGL zoom convention).
///
/// The field is private: build scales with [`Scale::pct`], [`Scale::ONE`] or, for the raw
/// 256-based value, [`Scale::from_raw_256`]; read it with [`Scale::raw_256`]. The newtype is
/// exactly a `u16` (no runtime cost).
///
/// ```
/// use twine_core::Scale;
/// assert_eq!(Scale::pct(50).apply(101), 51); // 50.5 rounds half up
/// assert_eq!(Scale::pct(50).apply(-101), -51); // symmetric
/// assert_eq!(Scale::from_raw_256(512).inverse_apply(100), 50);
/// assert_eq!(Scale::pct(100), Scale::ONE);
/// assert_eq!(Scale::ONE.raw_256(), 256);
/// ```
///
/// A bare number is not a scale:
///
/// ```compile_fail
/// let _ = twine_core::Scale(256); // private field: write Scale::ONE or Scale::pct(100)
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Scale(pub(crate) u16);

impl Scale {
    /// 1.0 (no scaling).
    pub const ONE: Scale = Scale(256);
    /// 0 (everything collapses to a point).
    pub const ZERO: Scale = Scale(0);

    /// `p` percent (`100` → 256), truncated; saturates at `u16::MAX` (≈ 256 ×).
    #[inline]
    #[must_use]
    pub const fn pct(p: u16) -> Scale {
        // SAFE-ARITH: `p < 2^16`, so `p · 256 < 2^24`.
        let v = (p as u32).wrapping_mul(256) / 100;
        Scale(if v > u16::MAX as u32 { u16::MAX } else { v as u16 })
    }

    /// The scale whose 256-based representation is `raw` (`256` = 1.0, `128` = 0.5).
    #[inline]
    #[must_use]
    pub const fn from_raw_256(raw: u16) -> Scale {
        Scale(raw)
    }

    /// The 256-based representation (`Scale::ONE.raw_256() == 256`).
    #[inline]
    #[must_use]
    pub const fn raw_256(self) -> u16 {
        self.0
    }

    /// `v · s / 256`, rounded half away from zero, saturating.
    #[must_use]
    pub const fn apply(self, v: i32) -> i32 {
        let s = self.0 as i64;
        let v = v as i64;
        // SAFE-ARITH: `|v| ≤ 2^31` and `s < 2^16`, so every step is below 2^48 in magnitude.
        sat_i32(if v >= 0 {
            v.wrapping_mul(s).wrapping_add(128) >> 8
        } else {
            (v.wrapping_neg().wrapping_mul(s).wrapping_add(128) >> 8).wrapping_neg()
        })
    }

    /// `v · 256 / s` (truncated towards zero). A zero scale returns `v` and logs a warning.
    #[must_use]
    pub fn inverse_apply(self, v: i32) -> i32 {
        let Some(s) = NonZeroI64::new(i64::from(self.0)) else {
            crate::warn!(target: "twine::core", "Scale::inverse_apply: zero scale");
            return v;
        };
        // SAFE-ARITH: `|v · 256| < 2^39`.
        let num = i64::from(v).wrapping_mul(256);
        // `s > 0` (a non-zero `u16`): the quotient exists; the fallback is never taken.
        sat_i32(num.checked_div(s.get()).unwrap_or(num))
    }

    /// The scale as a fixed-point number (`ONE` → `Fx::ONE`).
    #[must_use]
    pub const fn to_fx(self) -> Fx {
        Fx((self.0 as i32) << 8)
    }
}

impl Default for Scale {
    /// [`Scale::ONE`].
    fn default() -> Self {
        Scale::ONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn fx_mul_div_roundtrip() {
        let vals = [-1000, -3, -1, 1, 2, 7, 100, 3000];
        for &a in &vals {
            for &b in &vals {
                let fa = Fx::from_ratio(a, 7);
                let fb = Fx::from_ratio(b, 3);
                if (i64::from(a) * i64::from(b)).abs() / 21 > 30_000 {
                    continue; // product out of range (saturates)
                }
                let back = (fa * fb) / fb;
                assert!(
                    (back.0 - fa.0).abs() <= 2 + fa.0.abs() / 50_000,
                    "{a} {b}: {fa:?} vs {back:?}"
                );
            }
        }
        assert_eq!(Fx::from_int(3) * Fx::HALF, Fx::from_ratio(3, 2));
        assert_eq!(Fx::from_int(-3).mul(Fx::HALF), Fx::from_ratio(-3, 2));
        assert_eq!(Fx::from_int(1) / Fx::from_int(3), Fx(21845));
        assert_eq!(Fx::from_int(2) / Fx::from_int(3), Fx(43691));
        assert_eq!(Fx::from_int(-2) / Fx::from_int(3), Fx(-43691));
        assert_eq!(Fx::MAX * Fx::from_int(2), Fx::MAX);
        assert_eq!(Fx::MIN * Fx::from_int(2), Fx::MIN);
    }

    #[test]
    fn fx_div_by_zero_saturates() {
        assert_eq!(Fx::ONE / Fx::ZERO, Fx::MAX);
        assert_eq!(-Fx::ONE / Fx::ZERO, Fx::MIN);
        assert_eq!(Fx::ZERO / Fx::ZERO, Fx::ZERO);
        assert_eq!(Fx::from_ratio(5, 0), Fx::ZERO);
    }

    #[test]
    fn fx_rounding() {
        let h = Fx::from_ratio(5, 2); // 2.5
        assert_eq!((h.to_int_floor(), h.to_int_round(), h.to_int_ceil()), (2, 3, 3));
        let n = -h;
        assert_eq!(
            (n.to_int_floor(), n.to_int_round(), n.to_int_ceil()),
            (-3, -3, -2)
        );
        assert_eq!(Fx::from_int(4).to_int_ceil(), 4);
        assert_eq!(Fx::from_ratio(-1, 4).frac(), 0xC000);
        assert_eq!(Fx::MIN.abs(), Fx::MAX);
        assert_eq!(Fx::from_int(40_000), Fx::MAX);
        assert_eq!(Fx::MAX + Fx::ONE, Fx::MAX);
        assert_eq!(Fx::MIN - Fx::ONE, Fx::MIN);
        assert_eq!(-Fx::MIN, Fx::MAX);
        assert_eq!(Fx::lerp(Fx::ZERO, Fx::ONE, 512), Fx::HALF);
        assert_eq!(Fx::MAX.to_int_ceil(), 32768);
    }

    #[test]
    fn fx_display() {
        assert_eq!(format!("{}", Fx::from_ratio(-5, 4)), "-1.2500");
        assert_eq!(format!("{}", Fx::ONE), "1.0000");
        assert_eq!(format!("{}", Fx(65535)), "1.0000");
        assert_eq!(format!("{}", Fx::ZERO), "0.0000");
    }

    #[test]
    fn scale_ops() {
        assert_eq!(Scale::pct(100), Scale::ONE);
        assert_eq!(Scale::pct(u16::MAX), Scale(u16::MAX));
        assert_eq!(Scale::ONE.apply(-37), -37);
        assert_eq!(Scale(128).apply(3), 2);
        assert_eq!(Scale(128).apply(-3), -2);
        assert_eq!(Scale(0).inverse_apply(9), 9);
        assert_eq!(Scale(128).inverse_apply(-9), -18);
        assert_eq!(Scale(384).to_fx(), Fx::from_ratio(3, 2));
        assert_eq!(Scale::default(), Scale::ONE);
        assert_eq!(Scale(u16::MAX).apply(i32::MAX), i32::MAX);
    }
}
