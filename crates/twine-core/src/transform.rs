//! 2-D affine transforms in 16.16 fixed point.

use core::num::{NonZeroI128, NonZeroU128};

use crate::geometry::{Point, Rect, sat_i32};
use crate::math::{Angle, Fx, Scale, cos_fx, sin_fx};

/// `(v · 2^16) / den` rounded half away from zero, in `i128`, saturated to `i32`.
fn div_i128(num: i128, den: NonZeroI128) -> i32 {
    let n = num.unsigned_abs();
    let d: NonZeroU128 = den.unsigned_abs();
    // SAFE-ARITH: `n ≤ 2^127` and `d / 2 < 2^127`, so the sum is below 2^128.
    let q = n.wrapping_add(d.get() / 2) / d;
    let q = i128::try_from(q).unwrap_or(i128::MAX);
    // SAFE-ARITH: `0 ≤ q ≤ i128::MAX`, whose negation fits.
    let q = if (num < 0) == (den.get() < 0) {
        q
    } else {
        q.wrapping_neg()
    };
    i32::try_from(q).unwrap_or(if q > 0 { i32::MAX } else { i32::MIN })
}

/// `v >> 16` rounded half away from zero, saturated to `i32` (the intermediate steps saturate
/// too, which gives the same result: anything near the `i64` range saturates the `i32`).
const fn round16(v: i64) -> i32 {
    sat_i32(if v >= 0 {
        v.saturating_add(0x8000) >> 16
    } else {
        (v.saturating_neg().saturating_add(0x8000) >> 16).saturating_neg()
    })
}

/// A 2-D affine transform:
///
/// ```text
/// x' = a·x + c·y + tx
/// y' = b·x + d·y + ty
/// ```
///
/// Rotation angles are clockwise on screen (y points down), like LVGL.
///
/// ```
/// use twine_core::{Angle, Point, Transform};
/// let t = Transform::rotate(Angle::deg(90)).around(Point::new(10, 10));
/// assert_eq!(t.map_point(Point::new(20, 10)), Point::new(10, 20));
/// let inv = t.invert().unwrap();
/// assert_eq!(inv.map_point(Point::new(10, 20)), Point::new(20, 10));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Transform {
    /// x scale / rotation component.
    pub a: Fx,
    /// y shear from x (rotation `sin`).
    pub b: Fx,
    /// x shear from y (rotation `−sin`).
    pub c: Fx,
    /// y scale / rotation component.
    pub d: Fx,
    /// x translation.
    pub tx: Fx,
    /// y translation.
    pub ty: Fx,
}

impl Default for Transform {
    /// [`Transform::IDENTITY`].
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The identity transform.
    pub const IDENTITY: Transform = Transform {
        a: Fx::ONE,
        b: Fx::ZERO,
        c: Fx::ZERO,
        d: Fx::ONE,
        tx: Fx::ZERO,
        ty: Fx::ZERO,
    };

    /// Translation by `(dx, dy)`.
    #[must_use]
    pub const fn translate(dx: Fx, dy: Fx) -> Self {
        Transform {
            tx: dx,
            ty: dy,
            ..Self::IDENTITY
        }
    }

    /// Scaling by `(sx, sy)` around the origin.
    #[must_use]
    pub const fn scale(sx: Fx, sy: Fx) -> Self {
        Transform {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// Rotation by `angle` (clockwise on screen) around the origin.
    #[must_use]
    pub const fn rotate(angle: Angle) -> Self {
        let (s, c) = (sin_fx(angle), cos_fx(angle));
        Transform {
            a: c,
            b: s,
            c: Fx(s.0.saturating_neg()),
            d: c,
            tx: Fx::ZERO,
            ty: Fx::ZERO,
        }
    }

    /// Skew: `x' = x + tan(ax)·y`, `y' = tan(ay)·x + y`. Where `cos` is 0 the tangent is
    /// clamped to [`Fx::MAX`] (or [`Fx::MIN`] for a negative sine).
    #[must_use]
    pub const fn skew(ax: Angle, ay: Angle) -> Self {
        const fn tan(a: Angle) -> Fx {
            let (s, c) = (sin_fx(a), cos_fx(a));
            if c.0 == 0 {
                if s.0 < 0 { Fx::MIN } else { Fx::MAX }
            } else {
                // Same rounding as `Fx::div`, without its zero-division warning.
                let n = (s.0 as i64) << 16;
                let d = c.0 as i64;
                let d_abs = d.unsigned_abs();
                // SAFE-ARITH: `|n| ≤ 2^47` and `|d| < 2^31`, so the sum is below 2^48; the
                // quotient is below 2^48 too, so its negation fits.
                let num = n.unsigned_abs().wrapping_add(d_abs / 2);
                // `c.0 != 0` (the `if` above): the fallback is never taken.
                let q = match num.checked_div(d_abs) {
                    Some(q) => q as i64,
                    None => 0,
                };
                Fx(sat_i32(if (n < 0) == (d < 0) { q } else { q.wrapping_neg() }))
            }
        }
        Transform {
            c: tan(ax),
            b: tan(ay),
            ..Self::IDENTITY
        }
    }

    /// Composition: apply `self`, then `next`. Never panics: coefficients beyond the 16.16
    /// range saturate.
    #[must_use]
    pub const fn then(self, next: Transform) -> Self {
        const fn m(p: Fx, q: Fx) -> i64 {
            // SAFE-ARITH: a product of two `i32`s is at most 2^62 in magnitude.
            (p.0 as i64).wrapping_mul(q.0 as i64)
        }
        // Two products can reach 2^63: the sums saturate, which `round16` saturates to the same
        // `i32` as the exact value.
        const fn sum(a: i64, b: i64, c: i64) -> i64 {
            a.saturating_add(b).saturating_add(c)
        }
        let (s, n) = (self, next);
        Transform {
            a: Fx(round16(sum(m(n.a, s.a), m(n.c, s.b), 0))),
            b: Fx(round16(sum(m(n.b, s.a), m(n.d, s.b), 0))),
            c: Fx(round16(sum(m(n.a, s.c), m(n.c, s.d), 0))),
            d: Fx(round16(sum(m(n.b, s.c), m(n.d, s.d), 0))),
            tx: Fx(round16(sum(m(n.a, s.tx), m(n.c, s.ty), (n.tx.0 as i64) << 16))),
            ty: Fx(round16(sum(m(n.b, s.tx), m(n.d, s.ty), (n.ty.0 as i64) << 16))),
        }
    }

    /// The same transform applied around `pivot` instead of the origin:
    /// `translate(−pivot) · self · translate(pivot)`.
    #[must_use]
    pub const fn around(self, pivot: Point) -> Self {
        let (px, py) = (Fx::from_int(pivot.x), Fx::from_int(pivot.y));
        Self::translate(Fx(px.0.saturating_neg()), Fx(py.0.saturating_neg()))
            .then(self)
            .then(Self::translate(px, py))
    }

    /// Scale by `(sx, sy)`, then rotate by `angle`, both around `pivot` (LVGL image/layer
    /// transform order).
    #[must_use]
    pub const fn from_rotate_scale(angle: Angle, sx: Scale, sy: Scale, pivot: Point) -> Self {
        Self::scale(sx.to_fx(), sy.to_fx())
            .then(Self::rotate(angle))
            .around(pivot)
    }

    /// The inverse transform, or `None` when the determinant is below 1/65536 in magnitude.
    #[must_use]
    pub fn invert(&self) -> Option<Transform> {
        // SAFE-ARITH: every coefficient is an `i32` widened to `i128`: the products below are
        // at most 2^62, their differences 2^63 and the `2^32`-scaled values 2^63 in magnitude
        // (`<< 16` of a difference stays below 2^80), far inside `i128`.
        let w = |v: Fx| i128::from(v.0);
        let mul = |p: i128, q: i128| p.wrapping_mul(q);
        // Determinant in 32.32 fixed point.
        let det = mul(w(self.a), w(self.d)).wrapping_sub(mul(w(self.b), w(self.c)));
        if det.unsigned_abs() < (1 << 16) {
            return None;
        }
        let det = NonZeroI128::new(det)?;
        let s16 = 1i128 << 32;
        Some(Transform {
            a: Fx(div_i128(mul(w(self.d), s16), det)),
            b: Fx(div_i128(mul(w(self.b).wrapping_neg(), s16), det)),
            c: Fx(div_i128(mul(w(self.c).wrapping_neg(), s16), det)),
            d: Fx(div_i128(mul(w(self.a), s16), det)),
            tx: Fx(div_i128(
                mul(w(self.c), w(self.ty)).wrapping_sub(mul(w(self.d), w(self.tx))) << 16,
                det,
            )),
            ty: Fx(div_i128(
                mul(w(self.b), w(self.tx)).wrapping_sub(mul(w(self.a), w(self.ty))) << 16,
                det,
            )),
        })
    }

    /// Maps a fixed-point coordinate (saturating; never panics).
    #[must_use]
    pub const fn map(&self, x: Fx, y: Fx) -> (Fx, Fx) {
        let (x, y) = (x.0 as i64, y.0 as i64);
        // SAFE-ARITH: products of two `i32`s are at most 2^62 in magnitude; their sums can
        // reach 2^63 and saturate, which `round16` saturates to the same `i32`.
        let (ax, cy) = (
            (self.a.0 as i64).wrapping_mul(x),
            (self.c.0 as i64).wrapping_mul(y),
        );
        // SAFE-ARITH: as above.
        let (bx, dy) = (
            (self.b.0 as i64).wrapping_mul(x),
            (self.d.0 as i64).wrapping_mul(y),
        );
        (
            Fx(round16(
                ax.saturating_add(cy).saturating_add((self.tx.0 as i64) << 16),
            )),
            Fx(round16(
                bx.saturating_add(dy).saturating_add((self.ty.0 as i64) << 16),
            )),
        )
    }

    /// Maps an integer coordinate exactly (no 16-bit range limit), in 16.16.
    const fn map_raw(&self, p: Point) -> (i64, i64) {
        let (x, y) = (p.x as i64, p.y as i64);
        // SAFE-ARITH: products of two `i32`s are at most 2^62 in magnitude; their sums can
        // reach 2^63 and saturate (every caller saturates the result to `i32`).
        let (ax, cy) = (
            (self.a.0 as i64).wrapping_mul(x),
            (self.c.0 as i64).wrapping_mul(y),
        );
        // SAFE-ARITH: as above.
        let (bx, dy) = (
            (self.b.0 as i64).wrapping_mul(x),
            (self.d.0 as i64).wrapping_mul(y),
        );
        (
            ax.saturating_add(cy).saturating_add(self.tx.0 as i64),
            bx.saturating_add(dy).saturating_add(self.ty.0 as i64),
        )
    }

    /// Maps a pixel coordinate, rounded to the nearest integer.
    #[must_use]
    pub const fn map_point(&self, p: Point) -> Point {
        let (x, y) = self.map_raw(p);
        Point::new(round16(x), round16(y))
    }

    /// Bounding box of the four mapped corners (see [`Rect::corners`]), floored/ceiled
    /// outwards.
    #[must_use]
    pub fn map_rect_bounds(&self, r: Rect) -> Rect {
        let pts = r.corners().map(|p| self.map_raw(p));
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Rect::new(
            sat_i32(x0 >> 16),
            sat_i32(y0 >> 16),
            sat_i32(x1.saturating_add(0xFFFF) >> 16),
            sat_i32(y1.saturating_add(0xFFFF) >> 16),
        )
    }

    /// Whether this is exactly the identity.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Whether the transform only translates (no scale, rotation or skew).
    #[must_use]
    pub fn is_translation_only(&self) -> bool {
        self.a == Fx::ONE && self.d == Fx::ONE && self.b == Fx::ZERO && self.c == Fx::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_extreme_transforms_saturate() {
        // Two products of `i32::MIN` coefficients reached 2^63 in `then` and `map`.
        let m = Fx::from_raw(i32::MIN);
        let t = Transform::scale(m, m).then(Transform::scale(m, m));
        assert_eq!((t.a, t.d), (Fx::MAX, Fx::MAX));
        let big = Transform {
            a: m,
            c: m,
            ..Transform::IDENTITY
        };
        assert_eq!(big.map(m, m).0, Fx::MAX);
        assert_eq!(big.map_point(Point::new(i32::MIN, i32::MIN)).x, i32::MAX);
    }

    #[test]
    fn rotate_90_maps_axes() {
        let t = Transform::rotate(Angle::deg(90));
        assert_eq!(t.map_point(Point::new(1, 0)), Point::new(0, 1));
        assert_eq!(t.map_point(Point::new(0, 1)), Point::new(-1, 0));
        assert_eq!(t.map_point(Point::new(100, 0)), Point::new(0, 100));
        let t = Transform::rotate(Angle::deg(180));
        assert_eq!(t.map_point(Point::new(7, -3)), Point::new(-7, 3));
        assert_eq!(t.map(Fx::ONE, Fx::ZERO), (-Fx::ONE, Fx::ZERO));
    }

    #[test]
    fn map_rect_bounds_contains_mapped_corners() {
        let r = Rect::new(-5, 3, 40, 27);
        for t in [
            Transform::rotate(Angle::deg(33)).around(Point::new(10, 10)),
            Transform::scale(Fx::from_ratio(3, 2), Fx::from_ratio(1, 3)),
            Transform::skew(Angle::deg(20), Angle::deg(-10)),
            Transform::translate(Fx::from_ratio(1, 2), Fx::from_int(-4)),
        ] {
            let b = t.map_rect_bounds(r);
            for c in r.corners() {
                let (x, y) = t.map(Fx::from_int(c.x), Fx::from_int(c.y));
                assert!(
                    x.to_int_floor() >= b.x0 && x.to_int_ceil() <= b.x1,
                    "{t:?} {c:?} {b:?}"
                );
                assert!(
                    y.to_int_floor() >= b.y0 && y.to_int_ceil() <= b.y1,
                    "{t:?} {c:?} {b:?}"
                );
            }
        }
        assert_eq!(Transform::IDENTITY.map_rect_bounds(r), r);
    }

    #[test]
    fn composition_and_predicates() {
        let t = Transform::translate(Fx::from_int(3), Fx::from_int(4));
        assert!(t.is_translation_only() && !t.is_identity());
        assert!(Transform::default().is_identity());
        let s = Transform::scale(Fx::from_int(2), Fx::from_int(2));
        // scale, then translate
        assert_eq!(s.then(t).map_point(Point::new(1, 1)), Point::new(5, 6));
        // translate, then scale
        assert_eq!(t.then(s).map_point(Point::new(1, 1)), Point::new(8, 10));
        let rs = Transform::from_rotate_scale(Angle::deg(90), Scale(512), Scale::ONE, Point::new(5, 5));
        // (6,5): offset (1,0) → scaled (2,0) → rotated (0,2) → (5,7)
        assert_eq!(rs.map_point(Point::new(6, 5)), Point::new(5, 7));
        assert!(Transform::scale(Fx::ZERO, Fx::ONE).invert().is_none());
        let sk = Transform::skew(Angle::deg(90), Angle::deg(270));
        assert_eq!((sk.c, sk.b), (Fx::MAX, Fx::MIN));
        let sk = Transform::skew(Angle::deg(45), Angle(0));
        assert_eq!(sk.map_point(Point::new(0, 10)), Point::new(10, 10));
    }
}
