//! Fixed-point geometry of the vector crate: [`FxPoint`], [`FxSize`], [`FxRect`].

use core::ops::{Add, Neg, Sub};

use twine_core::{Fx, Point, Rect, Transform};

/// A point with 16.16 fixed-point coordinates.
///
/// ```
/// use twine_core::Fx;
/// use twine_vector::FxPoint;
/// let p = FxPoint::from_int(3, 4) + FxPoint::new(Fx::HALF, Fx::ZERO);
/// assert_eq!(p, FxPoint::new(Fx::from_ratio(7, 2), Fx::from_int(4)));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FxPoint {
    /// x coordinate.
    pub x: Fx,
    /// y coordinate.
    pub y: Fx,
}

impl FxPoint {
    /// The origin.
    pub const ZERO: FxPoint = FxPoint {
        x: Fx::ZERO,
        y: Fx::ZERO,
    };

    /// A point from fixed-point coordinates.
    #[must_use]
    pub const fn new(x: Fx, y: Fx) -> Self {
        Self { x, y }
    }

    /// A point from integer pixel coordinates.
    #[must_use]
    pub const fn from_int(x: i32, y: i32) -> Self {
        Self {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
        }
    }

    /// The point mapped by `t`.
    #[must_use]
    pub const fn transformed(self, t: &Transform) -> Self {
        let (x, y) = t.map(self.x, self.y);
        Self { x, y }
    }

    /// Raw 16.16 coordinates as `i64`.
    #[must_use]
    pub(crate) const fn raw(self) -> (i64, i64) {
        (self.x.0 as i64, self.y.0 as i64)
    }
}

impl From<Point> for FxPoint {
    fn from(p: Point) -> Self {
        Self::from_int(p.x, p.y)
    }
}

impl Add for FxPoint {
    type Output = FxPoint;
    fn add(self, o: FxPoint) -> FxPoint {
        FxPoint::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for FxPoint {
    type Output = FxPoint;
    fn sub(self, o: FxPoint) -> FxPoint {
        FxPoint::new(self.x - o.x, self.y - o.y)
    }
}

impl Neg for FxPoint {
    type Output = FxPoint;
    fn neg(self) -> FxPoint {
        FxPoint::new(-self.x, -self.y)
    }
}

/// A size with 16.16 fixed-point dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FxSize {
    /// Width.
    pub w: Fx,
    /// Height.
    pub h: Fx,
}

impl FxSize {
    /// A size.
    #[must_use]
    pub const fn new(w: Fx, h: Fx) -> Self {
        Self { w, h }
    }
}

/// A fixed-point rectangle `[x0, x1) × [y0, y1)`.
///
/// ```
/// use twine_vector::{FxPoint, FxRect};
/// let r = FxRect::from_points(FxPoint::from_int(3, 1), FxPoint::from_int(-2, 5));
/// assert_eq!(r.to_rect_out(), twine_core::Rect::new(-2, 1, 3, 5));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FxRect {
    /// Left.
    pub x0: Fx,
    /// Top.
    pub y0: Fx,
    /// Right (exclusive).
    pub x1: Fx,
    /// Bottom (exclusive).
    pub y1: Fx,
}

impl FxRect {
    /// The empty rectangle at the origin.
    pub const ZERO: FxRect = FxRect {
        x0: Fx::ZERO,
        y0: Fx::ZERO,
        x1: Fx::ZERO,
        y1: Fx::ZERO,
    };

    /// A rectangle from its edges.
    #[must_use]
    pub const fn new(x0: Fx, y0: Fx, x1: Fx, y1: Fx) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// A rectangle from its origin and size.
    #[must_use]
    pub fn from_xywh(x: Fx, y: Fx, w: Fx, h: Fx) -> Self {
        Self::new(x, y, x + w, y + h)
    }

    /// The smallest rectangle containing both points.
    #[must_use]
    pub fn from_points(a: FxPoint, b: FxPoint) -> Self {
        Self::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }

    /// Width (never negative).
    #[must_use]
    pub fn width(&self) -> Fx {
        (self.x1 - self.x0).max(Fx::ZERO)
    }

    /// Height (never negative).
    #[must_use]
    pub fn height(&self) -> Fx {
        (self.y1 - self.y0).max(Fx::ZERO)
    }

    /// Whether the rectangle has no area.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// Whether `p` lies inside or on the border.
    #[must_use]
    pub fn contains_incl(&self, p: FxPoint) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }

    /// Grows the rectangle to include `p`.
    #[must_use]
    pub fn include(self, p: FxPoint) -> Self {
        Self::new(
            self.x0.min(p.x),
            self.y0.min(p.y),
            self.x1.max(p.x),
            self.y1.max(p.y),
        )
    }

    /// The union of both rectangles.
    #[must_use]
    pub fn union(&self, o: &FxRect) -> Self {
        Self::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }

    /// Grows every side by `d`.
    #[must_use]
    pub fn outset(&self, d: Fx) -> Self {
        Self::new(self.x0 - d, self.y0 - d, self.x1 + d, self.y1 + d)
    }

    /// The smallest integer rectangle containing this one (floor / ceil).
    #[must_use]
    pub const fn to_rect_out(&self) -> Rect {
        Rect::new(
            self.x0.to_int_floor(),
            self.y0.to_int_floor(),
            self.x1.to_int_ceil(),
            self.y1.to_int_ceil(),
        )
    }

    /// Bounding box of the four mapped corners.
    #[must_use]
    pub fn transformed_bounds(&self, t: &Transform) -> Self {
        let c = [
            FxPoint::new(self.x0, self.y0),
            FxPoint::new(self.x1, self.y0),
            FxPoint::new(self.x0, self.y1),
            FxPoint::new(self.x1, self.y1),
        ]
        .map(|p| p.transformed(t));
        let first = FxRect::from_points(c[0], c[0]);
        c.iter().fold(first, |r, &p| r.include(p))
    }
}

impl From<Rect> for FxRect {
    fn from(r: Rect) -> Self {
        Self::new(
            Fx::from_int(r.x0),
            Fx::from_int(r.y0),
            Fx::from_int(r.x1),
            Fx::from_int(r.y1),
        )
    }
}

/// `√(dx² + dy²)` for raw 16.16 components, exact to 1 LSB, without overflow.
#[must_use]
pub(crate) fn hypot(dx: i64, dy: i64) -> i64 {
    let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
    // Scale down so the squares fit in u64 (keep ≤ 2^31 per component).
    let m = ax.max(ay);
    let mut shift = 0;
    while (m >> shift) > (1 << 31) {
        shift += 1;
    }
    let (sx, sy) = (ax >> shift, ay >> shift);
    (u64::from(twine_core::math::isqrt64(sx * sx + sy * sy)) << shift) as i64
}

/// Saturates an `i64` into the `i32` range.
#[inline]
pub(crate) const fn sat(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

/// Saturating conversion of raw 16.16 `i64` values into an [`FxPoint`].
#[inline]
pub(crate) const fn fxp(x: i64, y: i64) -> FxPoint {
    FxPoint::new(Fx(sat(x)), Fx(sat(y)))
}

/// `a · b / c` in `i128`, rounded half away from zero, saturated (`c == 0` → 0).
#[inline]
pub(crate) fn muldiv(a: i64, b: i64, c: i64) -> i64 {
    if c == 0 {
        return 0;
    }
    let n = i128::from(a) * i128::from(b);
    let d = i128::from(c);
    let q = (n.abs() + d.abs() / 2) / d.abs();
    let q = if (n < 0) == (d < 0) { q } else { -q };
    q.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot_exact_and_large() {
        assert_eq!(hypot(3 << 16, 4 << 16), 5 << 16);
        let big = i64::from(i32::MAX);
        let h = hypot(big, big);
        assert!((h - big * 14142 / 10000).abs() < big / 1000);
        assert_eq!(hypot(0, -7), 7);
    }

    #[test]
    fn muldiv_rounds() {
        assert_eq!(muldiv(7, 3, 2), 11);
        assert_eq!(muldiv(-7, 3, 2), -11);
        assert_eq!(muldiv(5, 5, 0), 0);
    }
}
