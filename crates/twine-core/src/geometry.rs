//! Integer geometry: [`Point`], [`Size`], [`Rect`] (half-open), [`Insets`] and [`Rotation`].
//!
//! Pixels are `i32`. Every operation saturates instead of overflowing and never panics.
//!
//! ```
//! use twine_core::{Point, Rect};
//!
//! let a = Rect::from_xywh(0, 0, 10, 10);
//! let b = Rect::new(5, 5, 20, 20);
//! assert_eq!(a.intersection(&b), Some(Rect::new(5, 5, 10, 10)));
//! assert!(a.contains(Point::new(9, 9)));
//! assert!(!a.contains(Point::new(10, 0))); // x1 is exclusive
//! ```

use core::fmt;
use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Clamps an `i64` into the `i32` range.
#[inline]
pub(crate) const fn sat_i32(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

const fn min_i32(a: i32, b: i32) -> i32 {
    if a < b { a } else { b }
}

const fn max_i32(a: i32, b: i32) -> i32 {
    if a > b { a } else { b }
}

/// A point (or vector) in pixel coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Point {
    /// Horizontal coordinate (grows to the right).
    pub x: i32,
    /// Vertical coordinate (grows downwards).
    pub y: i32,
}

impl Point {
    /// The origin `(0, 0)`.
    pub const ZERO: Point = Point { x: 0, y: 0 };

    /// Creates a point.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Returns the point moved by `(dx, dy)` (saturating).
    #[must_use]
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
        }
    }
}

impl Add for Point {
    type Output = Point;
    #[inline]
    fn add(self, o: Point) -> Point {
        self.offset(o.x, o.y)
    }
}

impl Sub for Point {
    type Output = Point;
    #[inline]
    fn sub(self, o: Point) -> Point {
        Point::new(self.x.saturating_sub(o.x), self.y.saturating_sub(o.y))
    }
}

impl Neg for Point {
    type Output = Point;
    #[inline]
    fn neg(self) -> Point {
        Point::new(self.x.saturating_neg(), self.y.saturating_neg())
    }
}

impl AddAssign for Point {
    #[inline]
    fn add_assign(&mut self, o: Point) {
        *self = *self + o;
    }
}

impl SubAssign for Point {
    #[inline]
    fn sub_assign(&mut self, o: Point) {
        *self = *self - o;
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// A width × height pair in pixels. Negative or zero dimensions mean "empty".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Size {
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl Size {
    /// `0 × 0`.
    pub const ZERO: Size = Size { w: 0, h: 0 };

    /// Creates a size.
    #[must_use]
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }

    /// `true` when `w <= 0 || h <= 0`.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Number of pixels (`0` if empty).
    #[must_use]
    pub fn area(self) -> u64 {
        if self.is_empty() {
            0
        } else {
            u64::from(self.w as u32) * u64::from(self.h as u32)
        }
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.w, self.h)
    }
}

/// Display rotation, in LVGL's convention: the panel is turned clockwise by this angle and the
/// picture stays upright, so `Deg90` draws logical `(x, y)` on native pixel `(y, w − 1 − x)`
/// (see `Rect::rotate_in`). Software rotation and the panel drivers' hardware rotation
/// (`MADCTL`) follow the same convention. Re-exported by `twine-hal`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Rotation {
    /// No rotation.
    #[default]
    Deg0,
    /// 90° rotation.
    Deg90,
    /// 180° rotation.
    Deg180,
    /// 270° rotation.
    Deg270,
}

impl Rotation {
    /// Whether the logical width and height are swapped on the physical panel (90° and 270°).
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Deg90 | Rotation::Deg270)
    }

    /// The rotation in whole degrees (0, 90, 180 or 270).
    #[must_use]
    pub const fn degrees(self) -> u16 {
        match self {
            Rotation::Deg0 => 0,
            Rotation::Deg90 => 90,
            Rotation::Deg180 => 180,
            Rotation::Deg270 => 270,
        }
    }
}

/// Distances from the four edges of a rectangle (padding, margins, borders).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Insets {
    /// Left inset.
    pub left: i32,
    /// Top inset.
    pub top: i32,
    /// Right inset.
    pub right: i32,
    /// Bottom inset.
    pub bottom: i32,
}

impl Insets {
    /// All zero.
    pub const ZERO: Insets = Insets {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    /// Creates insets from left, top, right, bottom.
    #[must_use]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// The same inset `v` on every side.
    #[must_use]
    pub const fn all(v: i32) -> Self {
        Self::new(v, v, v, v)
    }

    /// `h` on left and right, `v` on top and bottom.
    #[must_use]
    pub const fn hor_ver(h: i32, v: i32) -> Self {
        Self::new(h, v, h, v)
    }

    /// `left + right` (saturating).
    #[must_use]
    pub const fn horizontal(self) -> i32 {
        self.left.saturating_add(self.right)
    }

    /// `top + bottom` (saturating).
    #[must_use]
    pub const fn vertical(self) -> i32 {
        self.top.saturating_add(self.bottom)
    }
}

/// Half-open rectangle `[x0, x1) × [y0, y1)`. Empty iff `x1 <= x0 || y1 <= y0`.
///
/// LVGL areas are inclusive (`x2 = x1 - 1` here); ported algorithms convert explicitly.
///
/// ```
/// use twine_core::Rect;
/// let r = Rect::from_xywh(10, 20, 30, 40);
/// assert_eq!((r.x1, r.y1), (40, 60));
/// assert_eq!(r.area(), 1200);
/// assert_eq!(r.to_string(), "[10,20 .. 40,60)");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Rect {
    /// Left edge (inclusive).
    pub x0: i32,
    /// Top edge (inclusive).
    pub y0: i32,
    /// Right edge (exclusive).
    pub x1: i32,
    /// Bottom edge (exclusive).
    pub y1: i32,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Rect = Rect {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    /// Creates a rectangle from its edges.
    #[must_use]
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// Creates a rectangle from its top-left corner and size (saturating).
    #[must_use]
    pub const fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self::new(x, y, x.saturating_add(w), y.saturating_add(h))
    }

    /// Creates a rectangle from an origin and a size.
    #[must_use]
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::from_xywh(origin.x, origin.y, size.w, size.h)
    }

    /// Width, `max(0, x1 - x0)` (saturating).
    #[must_use]
    pub const fn width(&self) -> i32 {
        max_i32(0, self.x1.saturating_sub(self.x0))
    }

    /// Height, `max(0, y1 - y0)` (saturating).
    #[must_use]
    pub const fn height(&self) -> i32 {
        max_i32(0, self.y1.saturating_sub(self.y0))
    }

    /// `width × height`.
    #[must_use]
    pub const fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }

    /// Top-left corner.
    #[must_use]
    pub const fn origin(&self) -> Point {
        Point::new(self.x0, self.y0)
    }

    /// `true` when the rectangle contains no pixel.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// Number of pixels (exact, `0` if empty).
    #[must_use]
    pub const fn area(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            ((self.x1 as i64 - self.x0 as i64) as u64) * ((self.y1 as i64 - self.y0 as i64) as u64)
        }
    }

    /// Whether the pixel `p` lies inside.
    #[must_use]
    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x < self.x1 && p.y >= self.y0 && p.y < self.y1
    }

    /// Whether `other` lies completely inside. An empty rectangle is contained in anything.
    #[must_use]
    pub const fn contains_rect(&self, other: &Rect) -> bool {
        other.is_empty()
            || (other.x0 >= self.x0 && other.x1 <= self.x1 && other.y0 >= self.y0 && other.y1 <= self.y1)
    }

    /// Whether the two rectangles share at least one pixel.
    #[must_use]
    pub const fn intersects(&self, other: &Rect) -> bool {
        max_i32(self.x0, other.x0) < min_i32(self.x1, other.x1)
            && max_i32(self.y0, other.y0) < min_i32(self.y1, other.y1)
    }

    /// The common part, or `None` if it is empty.
    #[must_use]
    pub const fn intersection(&self, other: &Rect) -> Option<Rect> {
        let r = Rect::new(
            max_i32(self.x0, other.x0),
            max_i32(self.y0, other.y0),
            min_i32(self.x1, other.x1),
            min_i32(self.y1, other.y1),
        );
        if r.is_empty() { None } else { Some(r) }
    }

    /// Bounding box of both. The union with an empty rectangle returns the other one.
    #[must_use]
    pub const fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            *other
        } else if other.is_empty() {
            *self
        } else {
            Rect::new(
                min_i32(self.x0, other.x0),
                min_i32(self.y0, other.y0),
                max_i32(self.x1, other.x1),
                max_i32(self.y1, other.y1),
            )
        }
    }

    /// Whether the rectangles overlap or share (part of) an edge. Touching only at a corner
    /// point does not count. Empty rectangles never touch anything.
    #[must_use]
    pub const fn touches_or_overlaps(&self, other: &Rect) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        let (xl, xh) = (max_i32(self.x0, other.x0), min_i32(self.x1, other.x1));
        let (yl, yh) = (max_i32(self.y0, other.y0), min_i32(self.y1, other.y1));
        xl <= xh && yl <= yh && (xl < xh || yl < yh)
    }

    /// Moved by `(dx, dy)` (saturating).
    #[must_use]
    pub const fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.x0.saturating_add(dx),
            self.y0.saturating_add(dy),
            self.x1.saturating_add(dx),
            self.y1.saturating_add(dy),
        )
    }

    /// Moved by the vector `p`.
    #[must_use]
    pub const fn offset(&self, p: Point) -> Rect {
        self.translate(p.x, p.y)
    }

    /// Clamps one axis after growing/shrinking: an inverted interval collapses to its center.
    const fn axis(lo: i64, hi: i64) -> (i32, i32) {
        if lo > hi {
            let c = sat_i32((lo + hi).div_euclid(2));
            (c, c)
        } else {
            (sat_i32(lo), sat_i32(hi))
        }
    }

    /// Grows by `n` on every side; a negative `n` shrinks. Shrinking past the center yields an
    /// empty rectangle at the center instead of an inverted one.
    #[must_use]
    pub const fn expand(&self, n: i32) -> Rect {
        self.inset(Insets::all(n.saturating_neg()))
    }

    /// Shrinks by the given insets (negative insets grow). Clamped to empty at the center.
    #[must_use]
    pub const fn inset(&self, i: Insets) -> Rect {
        let (x0, x1) = Self::axis(self.x0 as i64 + i.left as i64, self.x1 as i64 - i.right as i64);
        let (y0, y1) = Self::axis(self.y0 as i64 + i.top as i64, self.y1 as i64 - i.bottom as i64);
        Rect::new(x0, y0, x1, y1)
    }

    /// Grows by the given insets (negative insets shrink). Clamped to empty at the center.
    #[must_use]
    pub const fn outset(&self, i: Insets) -> Rect {
        self.inset(Insets::new(
            i.left.saturating_neg(),
            i.top.saturating_neg(),
            i.right.saturating_neg(),
            i.bottom.saturating_neg(),
        ))
    }

    /// Center point, rounded towards negative infinity.
    #[must_use]
    pub const fn center(&self) -> Point {
        Point::new(
            sat_i32((self.x0 as i64 + self.x1 as i64).div_euclid(2)),
            sat_i32((self.y0 as i64 + self.y1 as i64).div_euclid(2)),
        )
    }

    /// The nearest pixel inside the rectangle (`x <= x1 - 1`). For an empty rectangle the
    /// result is clamped to the top-left corner.
    #[must_use]
    pub const fn clamp_point(&self, p: Point) -> Point {
        Point::new(
            max_i32(min_i32(p.x, self.x1.saturating_sub(1)), self.x0),
            max_i32(min_i32(p.y, self.y1.saturating_sub(1)), self.y0),
        )
    }

    /// Rounds `x0, y0` down and `x1, y1` up to multiples of `align` (0 or 1: unchanged).
    /// Used for `DisplayInfo::align`.
    #[must_use]
    pub const fn round_out(&self, align: u8) -> Rect {
        const fn down(v: i32, a: i64) -> i32 {
            sat_i32((v as i64).div_euclid(a) * a)
        }
        const fn up(v: i32, a: i64) -> i32 {
            sat_i32((v as i64 + a - 1).div_euclid(a) * a)
        }
        if align <= 1 {
            return *self;
        }
        let a = align as i64;
        Rect::new(down(self.x0, a), down(self.y0, a), up(self.x1, a), up(self.y1, a))
    }

    /// Iterates over horizontal bands of at most `max_rows` rows, top to bottom, covering the
    /// rectangle exactly. `max_rows <= 0` yields nothing (and logs a warning).
    #[must_use]
    pub fn rows(&self, max_rows: i32) -> RowChunks {
        if max_rows <= 0 {
            crate::warn!(target: "twine::core", "Rect::rows: max_rows must be > 0 (got {})", max_rows);
        }
        let done = self.is_empty() || max_rows <= 0;
        RowChunks {
            rect: *self,
            y: if done { self.y1 } else { self.y0 },
            step: max_rows,
        }
    }

    /// Splits at row `y` (clamped into the rectangle) into `(top, bottom)`.
    #[must_use]
    pub const fn split_at_y(&self, y: i32) -> (Rect, Rect) {
        let y = max_i32(self.y0, min_i32(y, self.y1));
        (
            Rect::new(self.x0, self.y0, self.x1, y),
            Rect::new(self.x0, y, self.x1, self.y1),
        )
    }

    /// Corner coordinates clockwise from the top-left: `(x0,y0) (x1,y0) (x1,y1) (x0,y1)`.
    /// These are edge coordinates (`x1`, `y1` exclusive), as used for transforms.
    #[must_use]
    pub const fn corners(&self) -> [Point; 4] {
        [
            Point::new(self.x0, self.y0),
            Point::new(self.x1, self.y0),
            Point::new(self.x1, self.y1),
            Point::new(self.x0, self.y1),
        ]
    }

    /// Up to 4 non-overlapping rectangles covering `self \ other` (no allocation).
    ///
    /// ```
    /// use twine_core::Rect;
    /// let parts: Vec<Rect> = Rect::new(0, 0, 10, 10).subtract(&Rect::new(2, 2, 8, 8)).collect();
    /// assert_eq!(parts.len(), 4);
    /// assert_eq!(parts.iter().map(Rect::area).sum::<u64>(), 100 - 36);
    /// ```
    #[must_use]
    pub fn subtract(&self, other: &Rect) -> SmallRects {
        let mut out = SmallRects::default();
        let Some(i) = self.intersection(other) else {
            out.push(*self);
            return out;
        };
        out.push(Rect::new(self.x0, self.y0, self.x1, i.y0));
        out.push(Rect::new(self.x0, i.y1, self.x1, self.y1));
        out.push(Rect::new(self.x0, i.y0, i.x0, i.y1));
        out.push(Rect::new(i.x1, i.y0, self.x1, i.y1));
        out
    }

    /// Maps a rectangle of a logical `w × h` screen to the physical panel for software rotation
    /// (same convention as LVGL `lv_display_rotate_area`; `w`, `h` are the *logical* size):
    /// - 90°: `(x, y) → (y, w − 1 − x)`
    /// - 180°: `(x, y) → (w − 1 − x, h − 1 − y)`
    /// - 270°: `(x, y) → (h − 1 − y, x)`
    #[must_use]
    pub const fn rotate_in(&self, rotation: Rotation, w: i32, h: i32) -> Rect {
        let r = self;
        match rotation {
            Rotation::Deg0 => *r,
            Rotation::Deg90 => Rect::new(r.y0, w.saturating_sub(r.x1), r.y1, w.saturating_sub(r.x0)),
            Rotation::Deg180 => Rect::new(
                w.saturating_sub(r.x1),
                h.saturating_sub(r.y1),
                w.saturating_sub(r.x0),
                h.saturating_sub(r.y0),
            ),
            Rotation::Deg270 => Rect::new(h.saturating_sub(r.y1), r.x0, h.saturating_sub(r.y0), r.x1),
        }
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{},{} .. {},{})", self.x0, self.y0, self.x1, self.y1)
    }
}

/// Iterator returned by [`Rect::rows`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowChunks {
    rect: Rect,
    y: i32,
    step: i32,
}

impl Iterator for RowChunks {
    type Item = Rect;

    fn next(&mut self) -> Option<Rect> {
        if self.y >= self.rect.y1 {
            return None;
        }
        let y1 = min_i32(self.y.saturating_add(self.step), self.rect.y1);
        let r = Rect::new(self.rect.x0, self.y, self.rect.x1, y1);
        self.y = y1;
        Some(r)
    }
}

/// Up to four rectangles without allocation (result of [`Rect::subtract`]). Empty rectangles
/// are never stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SmallRects {
    rects: [Rect; 4],
    len: u8,
    pos: u8,
}

impl SmallRects {
    fn push(&mut self, r: Rect) {
        if !r.is_empty() && usize::from(self.len) < self.rects.len() {
            self.rects[usize::from(self.len)] = r;
            self.len += 1;
        }
    }
}

impl Iterator for SmallRects {
    type Item = Rect;

    fn next(&mut self) -> Option<Rect> {
        if self.pos < self.len {
            self.pos += 1;
            Some(self.rects[usize::from(self.pos - 1)])
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = usize::from(self.len - self.pos);
        (n, Some(n))
    }
}

impl ExactSizeIterator for SmallRects {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn rect_empty_when_inverted_or_zero_width() {
        assert!(Rect::new(0, 0, 0, 10).is_empty());
        assert!(Rect::new(5, 0, 4, 10).is_empty());
        assert!(Rect::new(0, 3, 10, 3).is_empty());
        assert!(!Rect::new(0, 0, 1, 1).is_empty());
        assert_eq!(Rect::new(5, 0, 4, 10).width(), 0);
        assert_eq!(Rect::new(5, 0, 4, 10).area(), 0);
        assert!(Size::new(0, 5).is_empty());
        assert_eq!(Size::new(-1, 5).area(), 0);
        assert_eq!(Size::new(3, 5).area(), 15);
    }

    #[test]
    fn intersection_of_disjoint_is_none() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersection(&Rect::new(10, 0, 20, 10)), None);
        assert_eq!(a.intersection(&Rect::new(20, 20, 30, 30)), None);
        assert!(!a.intersects(&Rect::new(10, 0, 20, 10)));
    }

    #[test]
    fn union_with_empty_returns_other() {
        let a = Rect::new(3, 4, 10, 12);
        let e = Rect::new(100, 100, 100, 200);
        assert_eq!(a.union(&e), a);
        assert_eq!(e.union(&a), a);
        assert_eq!(a.union(&Rect::new(0, 0, 1, 1)), Rect::new(0, 0, 10, 12));
    }

    #[test]
    fn touching_rects_touch_but_do_not_intersect() {
        let a = Rect::new(0, 0, 10, 10);
        let right = Rect::new(10, 2, 20, 8);
        let below = Rect::new(0, 10, 10, 20);
        let corner = Rect::new(10, 10, 20, 20);
        assert!(a.touches_or_overlaps(&right) && !a.intersects(&right));
        assert!(a.touches_or_overlaps(&below) && !a.intersects(&below));
        assert!(!a.touches_or_overlaps(&corner));
        assert!(!a.touches_or_overlaps(&Rect::new(11, 0, 20, 10)));
        assert!(a.touches_or_overlaps(&Rect::new(5, 5, 6, 6)));
    }

    #[test]
    fn round_out_aligns_to_8() {
        assert_eq!(Rect::new(3, 9, 17, 16).round_out(8), Rect::new(0, 8, 24, 16));
        assert_eq!(Rect::new(-3, -9, 1, 1).round_out(8), Rect::new(-8, -16, 8, 8));
        let r = Rect::new(3, 9, 17, 16);
        assert_eq!(r.round_out(0), r);
        assert_eq!(r.round_out(1), r);
    }

    #[test]
    fn rows_cover_rect_exactly() {
        let r = Rect::new(2, 5, 12, 28);
        let chunks: Vec<Rect> = r.rows(10).collect();
        assert_eq!(
            chunks,
            [
                Rect::new(2, 5, 12, 15),
                Rect::new(2, 15, 12, 25),
                Rect::new(2, 25, 12, 28)
            ]
        );
        assert_eq!(r.rows(0).count(), 0);
        assert_eq!(r.rows(-3).count(), 0);
        assert_eq!(Rect::ZERO.rows(4).count(), 0);
        assert_eq!(r.rows(1000).collect::<Vec<_>>(), [r]);
    }

    #[test]
    fn expand_negative_clamps_to_empty() {
        let r = Rect::new(0, 0, 10, 4);
        assert_eq!(r.expand(2), Rect::new(-2, -2, 12, 6));
        assert_eq!(r.expand(-1), Rect::new(1, 1, 9, 3));
        let e = r.expand(-3);
        assert!(e.is_empty());
        assert_eq!(e, Rect::new(3, 2, 7, 2));
        let e = r.expand(-100);
        assert_eq!(e, Rect::new(5, 2, 5, 2));
        assert_eq!(r.expand(i32::MIN).width(), 0);
    }

    #[test]
    fn insets_and_outsets() {
        let r = Rect::new(0, 0, 100, 50);
        let i = Insets::new(1, 2, 3, 4);
        assert_eq!(r.inset(i), Rect::new(1, 2, 97, 46));
        assert_eq!(r.outset(i), Rect::new(-1, -2, 103, 54));
        assert_eq!(i.horizontal(), 4);
        assert_eq!(i.vertical(), 6);
        assert_eq!(Insets::hor_ver(1, 2), Insets::new(1, 2, 1, 2));
        assert_eq!(Insets::all(3), Insets::new(3, 3, 3, 3));
    }

    #[test]
    fn misc_accessors() {
        let r = Rect::from_origin_size(Point::new(1, 2), Size::new(3, 4));
        assert_eq!(r, Rect::new(1, 2, 4, 6));
        assert_eq!(r.size(), Size::new(3, 4));
        assert_eq!(r.origin(), Point::new(1, 2));
        assert_eq!(r.center(), Point::new(2, 4));
        assert_eq!(Rect::new(-3, -3, 0, 0).center(), Point::new(-2, -2));
        assert_eq!(r.clamp_point(Point::new(100, -100)), Point::new(3, 2));
        assert_eq!(Rect::ZERO.clamp_point(Point::new(5, 5)), Point::ZERO);
        assert_eq!(r.split_at_y(4), (Rect::new(1, 2, 4, 4), Rect::new(1, 4, 4, 6)));
        assert_eq!(r.split_at_y(-10).0.height(), 0);
        assert_eq!(r.offset(Point::new(1, 1)), Rect::new(2, 3, 5, 7));
        assert_eq!(r.corners()[2], Point::new(4, 6));
        assert!(Rect::new(0, 0, 10, 10).contains_rect(&Rect::new(2, 2, 10, 10)));
        assert!(!Rect::new(0, 0, 10, 10).contains_rect(&Rect::new(2, 2, 11, 10)));
        assert!(Rect::ZERO.contains_rect(&Rect::new(5, 5, 5, 5)));
        let big = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        assert_eq!(big.width(), i32::MAX);
        assert_eq!(big.area(), (u64::from(u32::MAX)) * u64::from(u32::MAX));
    }

    #[test]
    fn point_ops_saturate() {
        let p = Point::new(i32::MAX, 1) + Point::new(1, 1);
        assert_eq!(p, Point::new(i32::MAX, 2));
        assert_eq!(-Point::new(i32::MIN, 3), Point::new(i32::MAX, -3));
        let mut q = Point::new(1, 1);
        q += Point::new(2, 3);
        q -= Point::new(1, 1);
        assert_eq!(q, Point::new(2, 3));
        assert_eq!(q - Point::new(5, 5), Point::new(-3, -2));
        assert_eq!(alloc::format!("{q}"), "(2, 3)");
        assert_eq!(alloc::format!("{}", Size::new(3, 4)), "3x4");
    }

    #[test]
    fn subtract_covers_difference() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.subtract(&Rect::new(20, 20, 30, 30)).collect::<Vec<_>>(), [a]);
        assert_eq!(a.subtract(&Rect::new(-5, -5, 50, 50)).len(), 0);
        let parts: Vec<Rect> = a.subtract(&Rect::new(0, 0, 10, 5)).collect();
        assert_eq!(parts, [Rect::new(0, 5, 10, 10)]);
        let parts: Vec<Rect> = a.subtract(&Rect::new(3, 3, 6, 6)).collect();
        for (i, p) in parts.iter().enumerate() {
            for q in &parts[i + 1..] {
                assert!(!p.intersects(q));
            }
        }
        assert_eq!(parts.iter().map(Rect::area).sum::<u64>(), 91);
    }

    #[test]
    fn rotate_in_maps_like_lvgl() {
        // Logical 320x240 screen, a 10x20 rect at (5, 7).
        let r = Rect::from_xywh(5, 7, 10, 20);
        assert_eq!(r.rotate_in(Rotation::Deg0, 320, 240), r);
        // LVGL 90°: y2 = w - x1 - 1 (inclusive) → half-open [w - x1_ho, w - x0).
        assert_eq!(r.rotate_in(Rotation::Deg90, 320, 240), Rect::new(7, 305, 27, 315));
        assert_eq!(
            r.rotate_in(Rotation::Deg180, 320, 240),
            Rect::new(305, 213, 315, 233)
        );
        assert_eq!(
            r.rotate_in(Rotation::Deg270, 320, 240),
            Rect::new(213, 5, 233, 15)
        );
        // The edge pixel (0,0) in 90° goes to physical (0, w-1).
        let px = Rect::from_xywh(0, 0, 1, 1).rotate_in(Rotation::Deg90, 320, 240);
        assert_eq!(px, Rect::from_xywh(0, 319, 1, 1));
        assert!(Rotation::Deg90.swaps_axes() && !Rotation::Deg180.swaps_axes());
        assert_eq!(Rotation::Deg270.degrees(), 270);
    }
}
