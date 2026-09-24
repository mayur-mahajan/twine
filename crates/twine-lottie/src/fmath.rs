//! Small `f32` helpers for `no_std` (no `libm`): trigonometry, square root, `exp`/`ln` and
//! conversions to fixed point. Accuracy is far below a pixel for the value ranges Lottie uses
//! (relative error ≲ 1e-6 for `sin`/`cos`/`sqrt`, ≲ 1e-5 rad for `atan2`), and every function is
//! deterministic and total (no panics, NaN maps to 0 where it matters).

use twine_core::{Fx, Opa};

/// π.
pub(crate) const PI: f32 = core::f32::consts::PI;
/// Degrees → radians.
pub(crate) const DEG: f32 = PI / 180.0;

/// `|v|`.
#[inline]
pub(crate) fn abs(v: f32) -> f32 {
    f32::from_bits(v.to_bits() & 0x7FFF_FFFF)
}

/// `v` rounded to the nearest integer (half away from zero), saturating.
#[inline]
pub(crate) fn round_i32(v: f32) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i32
    } else {
        (v - 0.5) as i32
    }
}

/// `⌊v⌋` (values beyond ±2^23 are already integers and returned unchanged).
pub(crate) fn floor(v: f32) -> f32 {
    if !(abs(v) < 8_388_608.0) {
        return if v.is_nan() { 0.0 } else { v };
    }
    let i = v as i32;
    let f = i as f32;
    if f > v { f - 1.0 } else { f }
}

/// `v` clamped to `[lo, hi]` (NaN → `lo`).
#[inline]
pub(crate) fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v >= lo { if v > hi { hi } else { v } } else { lo }
}

/// `√v` (0 for `v ≤ 0` and NaN).
pub(crate) fn sqrt(v: f32) -> f32 {
    if !(v > 0.0) {
        return 0.0;
    }
    if v == f32::INFINITY {
        return v;
    }
    // Bit-level initial guess, then Newton iterations (quadratic convergence).
    let mut x = f32::from_bits((v.to_bits() >> 1) + 0x1FBD_1DF5);
    for _ in 0..4 {
        x = f32::midpoint(x, v / x);
    }
    x
}

/// `sin(x)` (radians).
pub(crate) fn sin(x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    // Reduce to [-π, π], then fold to [-π/2, π/2].
    let two_pi = 2.0 * PI;
    let k = round_i32(x / two_pi) as f32;
    let mut r = x - k * two_pi;
    if r > PI / 2.0 {
        r = PI - r;
    } else if r < -PI / 2.0 {
        r = -PI - r;
    }
    let r2 = r * r;
    // Taylor series to r^13 (error < 6e-8 on [-π/2, π/2]).
    let mut t = 1.0 - r2 / 156.0;
    t = 1.0 - r2 / 110.0 * t;
    t = 1.0 - r2 / 72.0 * t;
    t = 1.0 - r2 / 42.0 * t;
    t = 1.0 - r2 / 20.0 * t;
    t = 1.0 - r2 / 6.0 * t;
    r * t
}

/// `cos(x)` (radians).
pub(crate) fn cos(x: f32) -> f32 {
    sin(x + PI / 2.0)
}

/// `tan(x)` (radians), saturated to ±1e6 near the poles.
pub(crate) fn tan(x: f32) -> f32 {
    let c = cos(x);
    let s = sin(x);
    if abs(c) < 1e-6 {
        return if (s >= 0.0) == (c >= 0.0) { 1e6 } else { -1e6 };
    }
    s / c
}

/// `atan(z)` for `|z| ≤ 1`.
fn atan_unit(z: f32) -> f32 {
    let z2 = z * z;
    z * (0.999_977_26
        + z2 * (-0.332_623_47
            + z2 * (0.193_543_46 + z2 * (-0.116_432_87 + z2 * (0.052_653_32 + z2 * -0.011_721_2)))))
}

/// `atan2(y, x)` in radians (0 for the origin).
pub(crate) fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (abs(x), abs(y));
    if !(ax > 0.0 || ay > 0.0) {
        return 0.0;
    }
    let a = if ax >= ay {
        atan_unit(ay / ax)
    } else {
        PI / 2.0 - atan_unit(ax / ay)
    };
    let a = if x < 0.0 { PI - a } else { a };
    if y < 0.0 { -a } else { a }
}

/// `e^x` (clamped to `x ∈ [-80, 80]`).
pub(crate) fn exp(x: f32) -> f32 {
    let x = clamp(x, -80.0, 80.0);
    let ln2 = core::f32::consts::LN_2;
    let k = round_i32(x / ln2);
    let r = x - k as f32 * ln2;
    // Taylor series of e^r for |r| ≤ ln2/2.
    let mut t = 1.0 + r / 8.0;
    t = 1.0 + r / 7.0 * t;
    t = 1.0 + r / 6.0 * t;
    t = 1.0 + r / 5.0 * t;
    t = 1.0 + r / 4.0 * t;
    t = 1.0 + r / 3.0 * t;
    t = 1.0 + r / 2.0 * t;
    t = 1.0 + r * t;
    // 2^k as a float (k ∈ [-116, 116]: split to stay in the normal range).
    let half = k / 2;
    t * pow2(half) * pow2(k - half)
}

/// `2^k` for `k ∈ [-126, 127]`.
fn pow2(k: i32) -> f32 {
    let k = k.clamp(-126, 127);
    f32::from_bits(((k + 127) as u32) << 23)
}

/// `ln(x)` (−80 for `x ≤ 0`).
pub(crate) fn ln(x: f32) -> f32 {
    if !(x > 0.0) {
        return -80.0;
    }
    if x == f32::INFINITY {
        return 89.0;
    }
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xFF) as i32 - 127;
    if e == -127 {
        // Subnormal: tiny enough for our purposes.
        return -80.0;
    }
    let mut m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    if m > core::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let f = (m - 1.0) / (m + 1.0);
    let f2 = f * f;
    let s = f * (2.0 + f2 * (2.0 / 3.0 + f2 * (2.0 / 5.0 + f2 * (2.0 / 7.0 + f2 * (2.0 / 9.0)))));
    e as f32 * core::f32::consts::LN_2 + s
}

/// `b^e` for `b > 0` (0 for `b ≤ 0`).
pub(crate) fn powf(b: f32, e: f32) -> f32 {
    if !(b > 0.0) {
        return 0.0;
    }
    exp(e * ln(b))
}

/// `v` as 16.16 fixed point (rounded, saturating; NaN → 0).
#[inline]
pub(crate) fn fx(v: f32) -> Fx {
    Fx(round_i32(v * 65_536.0))
}

/// A `0..=1` fraction as an [`Opa`] (clamped; NaN → transparent).
#[inline]
pub(crate) fn opa(v: f32) -> Opa {
    Opa(round_i32(clamp(v, 0.0, 1.0) * 255.0) as u8)
}

/// A `0..=1` color channel as a byte.
#[inline]
pub(crate) fn channel(v: f32) -> u8 {
    round_i32(clamp(v, 0.0, 1.0) * 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        abs(a - b) <= eps
    }

    #[test]
    fn trig_matches_std() {
        let mut x = -20.0f32;
        while x < 20.0 {
            assert!(close(sin(x), x.sin(), 2e-6), "sin {x}");
            assert!(close(cos(x), x.cos(), 2e-6), "cos {x}");
            x += 0.013;
        }
        for &(y, x) in &[
            (1.0f32, 2.0f32),
            (-3.0, 0.5),
            (0.0, -1.0),
            (-1.0, -1.0),
            (5.0, 0.0),
            (0.0, 0.0),
        ] {
            assert!(close(atan2(y, x), y.atan2(x), 2e-5), "atan2 {y} {x}");
        }
    }

    #[test]
    fn sqrt_exp_ln_match_std() {
        for &v in &[1e-6f32, 0.25, 1.0, 2.0, 10.0, 12_345.0, 1e9] {
            assert!(close(sqrt(v), v.sqrt(), v.sqrt() * 1e-6), "sqrt {v}");
            assert!(close(ln(v), v.ln(), 1e-5), "ln {v}");
        }
        for &v in &[-10.0f32, -1.0, 0.0, 0.5, 1.0, 3.3, 20.0] {
            assert!(close(exp(v), v.exp(), v.exp() * 2e-6), "exp {v}");
        }
        assert!(close(powf(1.1, 3.0), 1.331, 1e-5));
        assert_eq!(sqrt(-1.0), 0.0);
        assert_eq!(sqrt(f32::NAN), 0.0);
    }

    #[test]
    fn floor_and_conversions() {
        assert_eq!(floor(1.5), 1.0);
        assert_eq!(floor(-1.5), -2.0);
        assert_eq!(floor(-2.0), -2.0);
        assert_eq!(floor(1e20), 1e20);
        assert_eq!(fx(1.5), Fx(98_304));
        assert_eq!(fx(f32::NAN), Fx(0));
        assert_eq!(fx(1e30), Fx(i32::MAX));
        assert_eq!(opa(0.5), Opa(128));
        assert_eq!(channel(2.0), 255);
    }
}
