//! Angles in 0.1° and integer trigonometry.

use core::fmt;
use core::ops::{Add, Neg, Sub};

use super::Fx;

/// Shift that normalizes [`sin`]/[`cos`] results (`>> TRIG_SHIFT`).
pub const TRIG_SHIFT: u32 = 15;
/// Largest value returned by [`sin`]/[`cos`] (`sin(90°)`).
pub const TRIG_MAX: i32 = 32_767;

/// `round(sin(d°) · 32767)` for `d = 0..=90` (verified against `f64` by a test).
const SIN_TABLE: [i16; 91] = [
    0, 572, 1144, 1715, 2286, 2856, 3425, 3993, 4560, 5126, 5690, 6252, 6813, 7371, 7927, 8481, 9032, 9580,
    10126, 10668, 11207, 11743, 12275, 12803, 13328, 13848, 14364, 14876, 15383, 15886, 16383, 16876, 17364,
    17846, 18323, 18794, 19260, 19720, 20173, 20621, 21062, 21497, 21925, 22347, 22762, 23170, 23571, 23964,
    24351, 24730, 25101, 25465, 25821, 26169, 26509, 26841, 27165, 27481, 27788, 28087, 28377, 28659, 28932,
    29196, 29451, 29697, 29934, 30162, 30381, 30591, 30791, 30982, 31163, 31335, 31498, 31650, 31794, 31927,
    32051, 32165, 32269, 32364, 32448, 32523, 32587, 32642, 32687, 32722, 32747, 32762, 32767,
];

/// `round(atan(2^-i) · 4096)` in degrees·4096 (CORDIC rotation angles, `i = 0..16`).
const ATAN_TABLE: [i64; 16] = [
    184_320, 108_810, 57_492, 29_184, 14_649, 7331, 3667, 1833, 917, 458, 229, 115, 57, 29, 14, 7,
];

/// An angle, stored in tenths of a degree. Angles grow clockwise on screen (y points down),
/// like LVGL.
///
/// The field is private: build angles with [`Angle::deg`] (whole degrees) or
/// [`Angle::deci_deg`] (tenths), so `30°` can never be confused with `3.0°` or `300°`.
/// The newtype is exactly an `i32` (no runtime cost).
///
/// ```
/// use twine_core::Angle;
/// assert_eq!(Angle::deg(-90).normalized(), Angle::deg(270));
/// assert_eq!(Angle::deci_deg(455).as_deg(), 45);
/// assert_eq!(Angle::deg(30), Angle::deci_deg(300));
/// assert_eq!(Angle::deg(30).as_deci_deg(), 300);
/// ```
///
/// A bare number is not an angle:
///
/// ```compile_fail
/// let _ = twine_core::Angle(300); // private field: write Angle::deg(30) or Angle::deci_deg(300)
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Angle(pub(crate) i32);

impl Angle {
    /// 0°.
    pub const ZERO: Angle = Angle(0);

    /// `d` whole degrees (saturating).
    #[inline]
    #[must_use]
    pub const fn deg(d: i32) -> Angle {
        Angle(d.saturating_mul(10))
    }

    /// `d` tenths of a degree (the stored representation).
    #[inline]
    #[must_use]
    pub const fn deci_deg(d: i32) -> Angle {
        Angle(d)
    }

    /// The angle in tenths of a degree (the stored representation).
    #[inline]
    #[must_use]
    pub const fn as_deci_deg(self) -> i32 {
        self.0
    }

    /// The equivalent angle in `0..3600`.
    #[must_use]
    pub const fn normalized(self) -> Angle {
        Angle(self.0.rem_euclid(3600))
    }

    /// Whole degrees, rounded towards negative infinity.
    #[must_use]
    pub const fn as_deg(self) -> i32 {
        self.0.div_euclid(10)
    }
}

impl Add for Angle {
    type Output = Angle;
    #[inline]
    fn add(self, o: Angle) -> Angle {
        Angle(self.0.saturating_add(o.0))
    }
}

impl Sub for Angle {
    type Output = Angle;
    #[inline]
    fn sub(self, o: Angle) -> Angle {
        Angle(self.0.saturating_sub(o.0))
    }
}

impl Neg for Angle {
    type Output = Angle;
    #[inline]
    fn neg(self) -> Angle {
        Angle(self.0.saturating_neg())
    }
}

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let a = self.0.unsigned_abs();
        write!(f, "{sign}{}.{}°", a / 10, a % 10)
    }
}

/// `sin` for `0..=900` decidegrees: table lookup with rounded linear interpolation.
const fn sin_q1(a: i32) -> i32 {
    let d = (a / 10) as usize;
    let r = a % 10;
    // SAFE-INDEX: `a ∈ 0..=900`, so `d ≤ 90`, the table's last index; the fallback is never
    // taken. `hi` is only read when `r ≠ 0`, i.e. `a < 900` and `d + 1 ≤ 90`.
    let (lo, hi) = match SIN_TABLE.split_at_checked(d) {
        Some((_, [lo, hi, ..])) => (*lo as i32, *hi as i32),
        Some((_, [lo])) => (*lo as i32, *lo as i32),
        _ => (0, 0),
    };
    // SAFE-ARITH: the table rises by at most 572 per degree and `r ≤ 9`, so the step is
    // below 2^13 and `lo + step ≤ 32_767`.
    lo.wrapping_add(hi.wrapping_sub(lo).wrapping_mul(r).wrapping_add(5) / 10)
}

/// Sine scaled to `−32767..=32767` (`>> TRIG_SHIFT` normalizes).
///
/// ```
/// use twine_core::{Angle, math::sin};
/// assert_eq!(sin(Angle::deg(90)), 32_767);
/// assert_eq!(sin(Angle::deg(-30)), -16_383);
/// ```
#[must_use]
pub const fn sin(a: Angle) -> i32 {
    let a = a.normalized().0;
    // SAFE-ARITH: `a ∈ 0..3600` (normalized); each branch maps it into `0..=900`, and
    // `sin_q1` returns `0..=32_767`, whose negation fits.
    if a <= 900 {
        sin_q1(a)
    } else if a <= 1800 {
        sin_q1(1800i32.wrapping_sub(a))
    } else if a <= 2700 {
        sin_q1(a.wrapping_sub(1800)).wrapping_neg()
    } else {
        sin_q1(3600i32.wrapping_sub(a)).wrapping_neg()
    }
}

/// Cosine scaled to `−32767..=32767` (`cos(a) = sin(a + 90°)`).
#[must_use]
pub const fn cos(a: Angle) -> i32 {
    // SAFE-ARITH: a normalized angle is below 3600.
    sin(Angle(a.normalized().0.wrapping_add(900)))
}

/// Converts a `−32767..=32767` trig value to 16.16 fixed point (rounded, `32767` → `1.0`).
const fn trig_to_fx(s: i32) -> Fx {
    let half = if s < 0 { -16_383 } else { 16_383 };
    // SAFE-ARITH: `|s| ≤ 32_767`, so `|s · 65_536 ± 16_383| ≤ 2_147_434_495 < 2^31`.
    Fx(s.wrapping_mul(65_536).wrapping_add(half) / 32_767)
}

/// Sine as fixed point (`sin_fx(90°) == Fx::ONE`).
#[must_use]
pub const fn sin_fx(a: Angle) -> Fx {
    trig_to_fx(sin(a))
}

/// Cosine as fixed point (`cos_fx(0°) == Fx::ONE`).
#[must_use]
pub const fn cos_fx(a: Angle) -> Fx {
    trig_to_fx(cos(a))
}

/// Angle of the vector `(x, y)` in `0..3600` (`(0, 0)` → 0). Integer CORDIC in vectoring mode
/// (16 iterations, inputs pre-scaled to 30 bits); accurate to 1 decidegree.
///
/// ```
/// use twine_core::{Angle, math::atan2};
/// assert_eq!(atan2(1, 0), Angle::deg(90)); // +y points down: 90° is "south"
/// assert_eq!(atan2(-5, -5), Angle::deg(225));
/// ```
#[must_use]
pub const fn atan2(y: i32, x: i32) -> Angle {
    const FULL: i64 = 360 * 4096;
    if x == 0 && y == 0 {
        return Angle(0);
    }
    let (mut x, mut y) = (x as i64, y as i64);
    let mut z: i64 = 0;
    if x < 0 {
        // SAFE-ARITH: `x`, `y` are `i32`s widened to `i64`.
        x = x.wrapping_neg();
        y = y.wrapping_neg();
        z = 180 * 4096;
    }
    // Pre-scale so that max(|x|, |y|) is in [2^29, 2^30): precision for small inputs,
    // headroom for the CORDIC gain (≈ 1.647) on large ones.
    let mut m = if x > y.abs() { x } else { y.abs() };
    while m < (1 << 29) {
        m <<= 1;
        x <<= 1;
        y <<= 1;
    }
    while m >= (1 << 30) {
        m >>= 1;
        x >>= 1;
        y >>= 1;
    }
    let mut i = 0u32;
    let mut steps: &[i64] = &ATAN_TABLE;
    // SAFE-ARITH: `|x|, |y| < 2^30` after the pre-scaling and the CORDIC gain is below 1.65,
    // so `x`, `y` stay below 2^31 in magnitude; `|z| < 2^21` (start ≤ 180 · 4096 plus the
    // table's sum); `i < 16`.
    while let [step, rest @ ..] = steps {
        // SHIFT: `i < 16` (one step per table entry), on `i64`.
        let (dx, dy) = (x >> i, y >> i);
        if y > 0 {
            x = x.wrapping_add(dy);
            y = y.wrapping_sub(dx);
            z = z.wrapping_add(*step);
        } else {
            x = x.wrapping_sub(dy);
            y = y.wrapping_add(dx);
            z = z.wrapping_sub(*step);
        }
        i = i.wrapping_add(1);
        steps = rest;
    }
    let z = z.rem_euclid(FULL);
    // SAFE-ARITH: `z ∈ 0..FULL` (`< 2^21`), so `z · 10 + 2048 < 2^25`, and `d ≤ 3600`.
    let d = (z.wrapping_mul(10).wrapping_add(2048) / 4096) as i32;
    Angle(if d >= 3600 { d.wrapping_sub(3600) } else { d })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_arithmetic, clippy::cast_precision_loss)]
    fn sin_table_matches_f64() {
        for (d, &v) in SIN_TABLE.iter().enumerate() {
            let exact = (d as f64).to_radians().sin() * 32_767.0;
            assert!((f64::from(v) - exact).abs() <= 1.0, "{d}: {v} vs {exact}");
        }
    }

    #[test]
    fn sin_cos_quadrants() {
        for (deg, s, c) in [
            (0, 0, 32_767),
            (90, 32_767, 0),
            (180, 0, -32_767),
            (270, -32_767, 0),
        ] {
            assert_eq!(sin(Angle::deg(deg)), s, "sin {deg}");
            assert_eq!(cos(Angle::deg(deg)), c, "cos {deg}");
        }
        assert_eq!(sin(Angle::deg(30)), 16_383);
        assert_eq!(sin(Angle::deg(150)), 16_383);
        assert_eq!(sin(Angle::deg(210)), -16_383);
        assert_eq!(sin(Angle::deg(330)), -16_383);
        assert_eq!(sin(Angle::deg(390)), 16_383);
        assert_eq!(sin(Angle::deg(-330)), 16_383);
        assert_eq!(sin_fx(Angle::deg(90)), Fx::ONE);
        assert_eq!(cos_fx(Angle::deg(180)), -Fx::ONE);
        assert!((sin_fx(Angle::deg(30)).0 - Fx::HALF.0).abs() <= 1);
        assert_eq!(sin(Angle(i32::MIN)), sin(Angle(i32::MIN).normalized()));
    }

    #[test]
    fn atan2_axes() {
        assert_eq!(atan2(0, 1), Angle::deg(0));
        assert_eq!(atan2(1, 0), Angle::deg(90));
        assert_eq!(atan2(0, -1), Angle::deg(180));
        assert_eq!(atan2(-1, 0), Angle::deg(270));
        assert_eq!(atan2(0, 0), Angle(0));
        assert_eq!(atan2(7, 7), Angle::deg(45));
        assert_eq!(atan2(i32::MIN, i32::MIN), Angle::deg(225));
        assert_eq!(atan2(i32::MAX, i32::MAX), Angle::deg(45));
        assert_eq!(atan2(-1, 1_000_000), Angle(0));
    }

    #[test]
    fn angle_ops() {
        assert_eq!(Angle::deg(10) + Angle::deg(5), Angle(150));
        assert_eq!(Angle::deg(10) - Angle::deg(15), Angle(-50));
        assert_eq!(-Angle(5), Angle(-5));
        assert_eq!(Angle(-5).as_deg(), -1);
        assert_eq!(alloc::format!("{}", Angle(-455)), "-45.5°");
    }
}
