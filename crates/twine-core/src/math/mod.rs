//! Deterministic integer math: [`Fx`] (16.16 fixed point), [`Angle`] (0.1°), [`Scale`]
//! (256 = 1.0), [`Fraction`] (0..=1), [`AngularSpeed`], integer square roots, trigonometry
//! and small helpers.
//!
//! Nothing here uses floating point (P5), so results are bit-identical on every target (P6).

mod bezier;
mod fixed;
mod sqrt;
mod trig;
mod units;

pub use bezier::{CUBIC_BEZIER_ONE, bezier3, cubic_bezier};
pub use fixed::{Fx, Scale};
pub use sqrt::{SqrtRes, isqrt64, sqrt, sqrt_fx};
pub use trig::{Angle, TRIG_MAX, TRIG_SHIFT, atan2, cos, cos_fx, sin, sin_fx};
pub use units::{AngularSpeed, Fraction};

/// `x / 255` (floor) without a division; exact for every `x <= 65 535`.
///
/// This is the rounding every blend path uses (P6).
///
/// `x` must be at most `65_535` (a product of two `u8`s; checked by a `debug_assert!`); larger
/// values give an unspecified result. Never panics.
///
/// ```
/// use twine_core::math::udiv255;
/// assert_eq!(udiv255(255 * 255), 255);
/// assert_eq!(udiv255(254), 0);
/// ```
#[inline]
#[must_use]
pub const fn udiv255(x: u32) -> u32 {
    debug_assert!(x <= 65_535);
    // SAFE-ARITH: callers pass a product of two `u8`s or a sum of such products with weights
    // summing to 255 (`x ≤ 65_535`, debug-asserted), so `x · 0x8081 < 2^32`.
    x.wrapping_mul(0x8081) >> 23
}

/// Maps `v` from `[in_min, in_max]` to `[out_min, out_max]`, clamped (port of LVGL `lv_map`).
///
/// Either range may be reversed; an empty input range returns `out_min`.
/// Never panics: exact for spans up to 2^31; when both spans exceed 2^30 the ratio is taken
/// with 4 bits less precision (an error of a few units in a span above 2^30).
///
/// ```
/// use twine_core::math::map;
/// assert_eq!(map(5, 0, 10, 0, 100), 50);
/// assert_eq!(map(20, 0, 10, 0, 100), 100); // clamped
/// assert_eq!(map(2, 10, 0, 0, 100), 80);   // reversed input range
/// ```
#[must_use]
pub const fn map(v: i32, in_min: i32, in_max: i32, out_min: i32, out_max: i32) -> i32 {
    if in_max == in_min {
        return out_min;
    }
    if in_max >= in_min && v >= in_max {
        return out_max;
    }
    if in_max >= in_min && v <= in_min {
        return out_min;
    }
    if in_max <= in_min && v <= in_max {
        return out_max;
    }
    if in_max <= in_min && v >= in_min {
        return out_min;
    }
    // SAFE-ARITH: differences of two `i32`s fit in `i64` (below 2^33 in magnitude).
    let delta_in = (in_max as i64).wrapping_sub(in_min as i64);
    let delta_out = (out_max as i64).wrapping_sub(out_min as i64);
    // SAFE-ARITH: as above.
    let offset = (v as i64).wrapping_sub(in_min as i64);
    // `delta_in != 0` (`in_max != in_min`, returned above) and `|offset| < |delta_in|`, so the
    // quotients exist and fit; the `checked_div` fallbacks are never taken.
    let q = match offset.checked_mul(delta_out) {
        Some(p) => match p.checked_div(delta_in) {
            Some(q) => q,
            None => 0,
        },
        // The product reaches 2^63 only when both spans exceed 2^30: keep the ratio
        // `offset / delta_in` with 4 bits less precision (`|delta_in| >> 4 ≥ 2^26`, non-zero).
        // SAFE-ARITH: `|offset >> 4| < 2^29` and `|delta_out| < 2^33`, so the product is below
        // 2^62.
        None => match (offset >> 4).wrapping_mul(delta_out).checked_div(delta_in >> 4) {
            Some(q) => q,
            None => 0,
        },
    };
    crate::geometry::sat_i32(q.saturating_add(out_min as i64))
}

/// Clamps `v` into `[lo, hi]`. Unlike [`Ord::clamp`] it never panics: if `lo > hi` the result
/// is `lo`.
#[must_use]
pub const fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    let v = if v > hi { hi } else { v };
    if v < lo { lo } else { v }
}

/// Linear interpolation `a + (b − a)·t/1024` with `t` in `0..=1024` (values above 1024
/// extrapolate), rounded half up. `t = 0` gives `a`, `t = 1024` gives `b` exactly.
///
/// ```
/// use twine_core::math::lerp_i32;
/// assert_eq!(lerp_i32(0, 100, 512), 50);
/// assert_eq!(lerp_i32(-7, 13, 1024), 13);
/// ```
#[must_use]
pub const fn lerp_i32(a: i32, b: i32, t: u16) -> i32 {
    // SAFE-ARITH: `|d| < 2^33` and `t < 2^16`, so every step stays below 2^50 in magnitude.
    let d = (b as i64).wrapping_sub(a as i64);
    crate::geometry::sat_i32((a as i64).wrapping_add(d.wrapping_mul(t as i64).wrapping_add(512) >> 10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_map_extreme_ranges_do_not_overflow() {
        // `(v − in_min) · delta_out` reached 2^63 for spans near 2^32 on both sides.
        let exact = |v: i32, a: i32, b: i32, c: i32, d: i32| {
            let r = (i128::from(v) - i128::from(a)) * (i128::from(d) - i128::from(c))
                / (i128::from(b) - i128::from(a))
                + i128::from(c);
            i64::try_from(r).unwrap()
        };
        for (v, a, b, c, d) in [
            (i32::MAX - 1, i32::MIN, i32::MAX, i32::MIN, i32::MAX),
            (0, i32::MIN, i32::MAX, i32::MAX, i32::MIN),
            (1 << 30, i32::MIN, i32::MAX, -7, i32::MAX),
            (-5, i32::MAX, i32::MIN, i32::MIN, i32::MAX),
        ] {
            let got = i64::from(map(v, a, b, c, d));
            assert!(
                (got - exact(v, a, b, c, d)).abs() <= 16,
                "{v} {a} {b} {c} {d}: {got}"
            );
        }
        assert_eq!(map(5, 0, 10, 0, 100), 50);
    }

    #[test]
    fn map_matches_lvgl() {
        assert_eq!(map(0, 0, 0, 3, 9), 3);
        assert_eq!(map(-5, 0, 10, 0, 100), 0);
        assert_eq!(map(3, 0, 10, 100, 0), 70);
        assert_eq!(map(15, 10, 0, 0, 100), 0);
        assert_eq!(map(-1, 10, 0, 0, 100), 100);
        assert_eq!(map(i32::MAX - 1, i32::MIN, i32::MAX, 0, 1000), 999);
    }

    #[test]
    fn clamp_and_lerp() {
        assert_eq!(clamp(5, 0, 3), 3);
        assert_eq!(clamp(-5, 0, 3), 0);
        assert_eq!(clamp(2, 0, 3), 2);
        assert_eq!(clamp(2, 5, 3), 5);
        assert_eq!(lerp_i32(10, 20, 0), 10);
        assert_eq!(lerp_i32(10, 20, 1024), 20);
        assert_eq!(lerp_i32(20, 10, 512), 15);
        assert_eq!(lerp_i32(0, 1, 511), 0);
        assert_eq!(lerp_i32(0, 1, 512), 1);
        assert_eq!(lerp_i32(i32::MIN, i32::MAX, 2048), i32::MAX);
    }
}
