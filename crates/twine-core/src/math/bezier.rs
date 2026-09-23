//! Bezier curves for easing (ports of LVGL `lv_bezier3` and `lv_cubic_bezier`).
//!
//! All values are scaled by [`CUBIC_BEZIER_ONE`] (1024 = 1.0).

/// 1.0 in the bezier functions' fixed-point scale (LVGL `LV_BEZIER_VAL_MAX`).
pub const CUBIC_BEZIER_ONE: i32 = 1024;

const SHIFT: u32 = 10;
const NEWTON_ITERATIONS: usize = 8;

/// One-dimensional cubic Bezier with control values `u0..u3` at time `t` (`0..=1024`, larger
/// values are clamped): `u0·(1−t)³ + 3·u1·(1−t)²·t + 3·u2·(1−t)·t² + u3·t³`
/// (the general Bernstein form of LVGL's `lv_bezier3`, evaluated exactly and rounded once
/// instead of LVGL's per-term truncation).
///
/// ```
/// use twine_core::math::bezier3;
/// assert_eq!(bezier3(0, 0, 100, 900, 1024), 0);
/// assert_eq!(bezier3(1024, 0, 100, 900, 1024), 1024);
/// ```
#[must_use]
pub const fn bezier3(t: u16, u0: i32, u1: i32, u2: i32, u3: i32) -> i32 {
    let t = if t > 1024 { 1024 } else { t as i64 };
    let r = 1024 - t;
    // Exact Bernstein sum in 30-bit fixed point (≤ 2^61, fits i64), rounded once.
    let sum =
        u0 as i64 * r * r * r + 3 * u1 as i64 * r * r * t + 3 * u2 as i64 * r * t * t + u3 as i64 * t * t * t;
    crate::geometry::sat_i32((sum + (1 << 29)) >> 30)
}

/// `a·t³ + b·t² + c·t` in 10-bit fixed point (LVGL `do_cubic_bezier`).
const fn eval(t: i64, a: i64, b: i64, c: i64) -> i64 {
    let r = (a * t) >> SHIFT;
    let r = ((r + b) * t) >> SHIFT;
    ((r + c) * t) >> SHIFT
}

/// CSS-style `cubic-bezier(x1, y1, x2, y2)` easing evaluated at `x` (port of LVGL
/// `lv_cubic_bezier`; all values scaled by 1024). Solves `t` for `x` with Newton iterations,
/// falling back to bisection, and returns `y(t)`.
///
/// `x1`/`x2` outside `0..=1024` are invalid: a warning is logged and 0 is returned (as LVGL).
///
/// ```
/// use twine_core::math::{cubic_bezier, CUBIC_BEZIER_ONE};
/// // CSS `ease` = cubic-bezier(0.25, 0.1, 0.25, 1.0)
/// let y = cubic_bezier(512, 256, 102, 256, 1024);
/// assert!((y - 821).abs() <= 4);
/// assert_eq!(cubic_bezier(CUBIC_BEZIER_ONE, 256, 102, 256, 1024), CUBIC_BEZIER_ONE);
/// ```
#[must_use]
pub fn cubic_bezier(x: i32, x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    if !(0..=CUBIC_BEZIER_ONE).contains(&x1) || !(0..=CUBIC_BEZIER_ONE).contains(&x2) {
        crate::warn!(target: "twine::core", "cubic_bezier: x1/x2 out of range ({}, {})", x1, x2);
        return 0;
    }
    if x == 0 || x == CUBIC_BEZIER_ONE {
        return x;
    }
    let one = i64::from(CUBIC_BEZIER_ONE);
    let (x, x1, y1, x2, y2) = (
        i64::from(x),
        i64::from(x1),
        i64::from(y1),
        i64::from(x2),
        i64::from(y2),
    );
    let cx = 3 * x1;
    let bx = 3 * (x2 - x1) - cx;
    let ax = one - cx - bx;
    let cy = 3 * y1;
    let by = 3 * (y2 - y1) - cy;
    let ay = one - cy - by;

    let t = 'solve: {
        // Newton's method first.
        let mut t = x;
        for _ in 0..NEWTON_ITERATIONS {
            let xs = eval(t, ax, bx, cx) - x;
            if xs.abs() <= 1 {
                break 'solve t;
            }
            let mut d = (3 * ax * t) >> SHIFT;
            d = ((d + 2 * bx) * t) >> SHIFT;
            d += cx;
            if d.abs() <= 1 {
                break;
            }
            let step = (xs << SHIFT) / d;
            if step == 0 {
                break;
            }
            t -= step;
        }
        // Bisection fallback for reliability.
        let (mut tl, mut tr) = (0, one);
        let mut t = x;
        if t < tl {
            break 'solve tl;
        }
        if t > tr {
            break 'solve tr;
        }
        while tl < tr {
            let xs = eval(t, ax, bx, cx);
            if (xs - x).abs() <= 1 {
                break 'solve t;
            }
            if x > xs {
                tl = t;
            } else {
                tr = t;
            }
            t = (tr - tl) / 2 + tl;
            if t == tl {
                break;
            }
        }
        t
    };
    crate::geometry::sat_i32(eval(t, ay, by, cy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier3_endpoints() {
        for (u0, u1, u2, u3) in [(0, 0, 0, 1024), (-50, 300, 700, 2000), (1024, 0, 0, 0)] {
            assert_eq!(bezier3(0, u0, u1, u2, u3), u0);
            assert_eq!(bezier3(1024, u0, u1, u2, u3), u3);
            assert_eq!(bezier3(u16::MAX, u0, u1, u2, u3), u3);
        }
        // Linear control values give a straight line (within rounding).
        for t in 0..=1024u16 {
            let v = bezier3(t, 0, 341, 683, 1024);
            assert!((v - i32::from(t)).abs() <= 1, "{t}: {v}");
        }
    }

    #[test]
    fn cubic_bezier_linear_is_identity() {
        for x in 0..=1024 {
            let y = cubic_bezier(x, 0, 0, 1024, 1024);
            assert!((y - x).abs() <= 2, "{x}: {y}");
        }
    }

    #[test]
    fn cubic_bezier_rejects_invalid_control_points() {
        assert_eq!(cubic_bezier(500, -1, 0, 1024, 1024), 0);
        assert_eq!(cubic_bezier(500, 0, 0, 1025, 1024), 0);
    }

    #[test]
    #[allow(clippy::float_arithmetic, clippy::cast_precision_loss)]
    fn cubic_bezier_ease_matches_css_samples() {
        // Reference: solve x(t) = x by bisection in f64, return y(t).
        fn reference(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
            let b = |t: f64, p1: f64, p2: f64| {
                let u = 1.0 - t;
                3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
            };
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..60 {
                let mid = f64::midpoint(lo, hi);
                if b(mid, x1, x2) < x {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            b(f64::midpoint(lo, hi), y1, y2)
        }
        let curves = [
            (256, 102, 256, 1024),
            (430, 0, 1024, 1024),
            (0, 0, 594, 1024),
            (430, 0, 594, 1024),
        ];
        for (x1, y1, x2, y2) in curves {
            for x in (0..=1024).step_by(8) {
                let got = cubic_bezier(x, x1, y1, x2, y2);
                let exp = reference(
                    f64::from(x) / 1024.0,
                    f64::from(x1) / 1024.0,
                    f64::from(y1) / 1024.0,
                    f64::from(x2) / 1024.0,
                    f64::from(y2) / 1024.0,
                ) * 1024.0;
                assert!(
                    (f64::from(got) - exp).abs() <= 4.0,
                    "{x1},{y1},{x2},{y2} @ {x}: {got} vs {exp}"
                );
            }
        }
    }
}
