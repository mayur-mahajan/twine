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

/// An angle in tenths of a degree (`Angle(900)` is 90°). Angles grow clockwise on screen
/// (y points down), like LVGL.
///
/// ```
/// use twine_core::Angle;
/// assert_eq!(Angle::deg(-90).normalized(), Angle::deg(270));
/// assert_eq!(Angle::decideg(455).as_deg(), 45);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Angle(pub i32);

impl Angle {
    /// `d` whole degrees (saturating).
    #[must_use]
    pub const fn deg(d: i32) -> Angle {
        Angle(d.saturating_mul(10))
    }

    /// `d` tenths of a degree.
    #[must_use]
    pub const fn decideg(d: i32) -> Angle {
        Angle(d)
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
    let lo = SIN_TABLE[d] as i32;
    if r == 0 {
        lo
    } else {
        let hi = SIN_TABLE[d + 1] as i32;
        lo + ((hi - lo) * r + 5) / 10
    }
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
    if a <= 900 {
        sin_q1(a)
    } else if a <= 1800 {
        sin_q1(1800 - a)
    } else if a <= 2700 {
        -sin_q1(a - 1800)
    } else {
        -sin_q1(3600 - a)
    }
}

/// Cosine scaled to `−32767..=32767` (`cos(a) = sin(a + 90°)`).
#[must_use]
pub const fn cos(a: Angle) -> i32 {
    sin(Angle(a.normalized().0 + 900))
}

/// Converts a `−32767..=32767` trig value to 16.16 fixed point (rounded, `32767` → `1.0`).
const fn trig_to_fx(s: i32) -> Fx {
    let sign = if s < 0 { -1 } else { 1 };
    Fx((s * 65_536 + sign * 16_383) / 32_767)
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
        x = -x;
        y = -y;
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
    let mut i = 0;
    while i < 16 {
        let (dx, dy) = (x >> i, y >> i);
        if y > 0 {
            x += dy;
            y -= dx;
            z += ATAN_TABLE[i];
        } else {
            x -= dy;
            y += dx;
            z -= ATAN_TABLE[i];
        }
        i += 1;
    }
    let z = z.rem_euclid(FULL);
    let d = ((z * 10 + 2048) / 4096) as i32;
    Angle(if d >= 3600 { d - 3600 } else { d })
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
