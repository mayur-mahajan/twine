//! [`Path`]: move/line/quadratic/cubic/arc/close commands with 16.16 fixed-point points.
//!
//! The builder is `&mut Path` chaining (there is no separate `PathBuilder`):
//!
//! ```
//! use twine_core::Fx;
//! use twine_vector::{FxPoint, Path, Verb};
//!
//! let mut p = Path::new();
//! p.move_to(FxPoint::from_int(0, 0))
//!     .line_to(FxPoint::from_int(10, 0))
//!     .quad_to(FxPoint::from_int(10, 10), FxPoint::from_int(0, 10))
//!     .close();
//! assert_eq!(p.verbs(), &[Verb::MoveTo, Verb::LineTo, Verb::QuadTo, Verb::Close]);
//! assert_eq!(p.points().len(), 4);
//! ```

use alloc::vec::Vec;

use twine_core::math::{atan2, cos_fx, isqrt64, sin_fx};
use twine_core::{Angle, Fx, Rect, Transform};

use crate::geom::{FxPoint, FxRect, fxp, muldiv, sat};

/// Cubic Bézier circle constant `4/3 · (√2 − 1) ≈ 0.5523` in 16.16.
pub const KAPPA: Fx = Fx(36_195);

/// A path command. Each verb consumes points from [`Path::points`]: `MoveTo`/`LineTo` one,
/// `QuadTo` two (control, end), `CubicTo` three (control 1, control 2, end), `Close` none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum Verb {
    /// Starts a new sub-path.
    MoveTo,
    /// Straight line.
    LineTo,
    /// Quadratic Bézier curve.
    QuadTo,
    /// Cubic Bézier curve.
    CubicTo,
    /// Closes the current sub-path (back to its start).
    Close,
}

impl Verb {
    /// Number of points the verb consumes.
    #[must_use]
    pub const fn points(self) -> usize {
        match self {
            Verb::MoveTo | Verb::LineTo => 1,
            Verb::QuadTo => 2,
            Verb::CubicTo => 3,
            Verb::Close => 0,
        }
    }
}

/// A vector path: sub-paths of lines and Bézier curves in 16.16 fixed point.
///
/// Drawing commands without a preceding [`move_to`](Self::move_to) start at the current point
/// (the origin for an empty path). [`clear`](Self::clear) keeps the capacity, so a path
/// rebuilt every frame does not allocate after the first frame.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Path {
    verbs: Vec<Verb>,
    points: Vec<FxPoint>,
    /// Start of the current sub-path.
    start: FxPoint,
    /// Whether a sub-path is open (a `MoveTo` was emitted and not closed).
    open: bool,
}

impl Path {
    /// An empty path.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            verbs: Vec::new(),
            points: Vec::new(),
            start: FxPoint::ZERO,
            open: false,
        }
    }

    /// An empty path with room for `verbs` verbs and `points` points.
    #[must_use]
    pub fn with_capacity(verbs: usize, points: usize) -> Self {
        Self {
            verbs: Vec::with_capacity(verbs),
            points: Vec::with_capacity(points),
            ..Self::new()
        }
    }

    /// The verbs.
    #[must_use]
    pub fn verbs(&self) -> &[Verb] {
        &self.verbs
    }

    /// The points (see [`Verb`] for how many each verb uses).
    #[must_use]
    pub fn points(&self) -> &[FxPoint] {
        &self.points
    }

    /// Whether the path has no verbs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    /// Removes every command, keeping the capacity.
    pub fn clear(&mut self) {
        self.verbs.clear();
        self.points.clear();
        self.start = FxPoint::ZERO;
        self.open = false;
    }

    /// The current point: the end of the last command (the sub-path start after `close`).
    #[must_use]
    pub fn current_point(&self) -> FxPoint {
        match self.verbs.last() {
            Some(Verb::Close) | None => self.start,
            Some(_) => self.points.last().copied().unwrap_or(FxPoint::ZERO),
        }
    }

    fn ensure_open(&mut self) {
        if !self.open {
            let p = self.current_point();
            self.move_to(p);
        }
    }

    /// Starts a new sub-path at `p`.
    pub fn move_to(&mut self, p: FxPoint) -> &mut Self {
        // Consecutive moves: the last one wins.
        if self.open && self.verbs.last() == Some(&Verb::MoveTo) {
            if let Some(last) = self.points.last_mut() {
                *last = p;
            }
        } else {
            self.verbs.push(Verb::MoveTo);
            self.points.push(p);
        }
        self.start = p;
        self.open = true;
        self
    }

    /// Straight line to `p`.
    pub fn line_to(&mut self, p: FxPoint) -> &mut Self {
        self.ensure_open();
        self.verbs.push(Verb::LineTo);
        self.points.push(p);
        self
    }

    /// Quadratic Bézier with control point `c` to `p`.
    pub fn quad_to(&mut self, c: FxPoint, p: FxPoint) -> &mut Self {
        self.ensure_open();
        self.verbs.push(Verb::QuadTo);
        self.points.extend_from_slice(&[c, p]);
        self
    }

    /// Cubic Bézier with control points `c1`, `c2` to `p`.
    pub fn cubic_to(&mut self, c1: FxPoint, c2: FxPoint, p: FxPoint) -> &mut Self {
        self.ensure_open();
        self.verbs.push(Verb::CubicTo);
        self.points.extend_from_slice(&[c1, c2, p]);
        self
    }

    /// Closes the current sub-path (no-op when none is open).
    pub fn close(&mut self) -> &mut Self {
        if self.open {
            self.verbs.push(Verb::Close);
            self.open = false;
        }
        self
    }

    /// SVG elliptical arc from the current point to `p` with radii `radii`, x-axis rotation
    /// `x_rot` and the SVG `large-arc` / `sweep` flags (W3C SVG 1.1 appendix F.6), converted to
    /// cubic Béziers of at most 90° each. Radii that are too small are scaled up; a zero radius
    /// draws a line; `p` equal to the current point draws nothing. The end point is exact.
    pub fn arc_to(
        &mut self,
        radii: FxPoint,
        x_rot: Angle,
        large: bool,
        sweep: bool,
        p: FxPoint,
    ) -> &mut Self {
        self.ensure_open();
        let p0 = self.current_point();
        if p0 == p {
            return self;
        }
        let (rx, ry) = (i64::from(radii.x.0).abs(), i64::from(radii.y.0).abs());
        if rx == 0 || ry == 0 {
            return self.line_to(p);
        }
        arc_segments(self, p0, p, rx, ry, x_rot, large, sweep);
        self
    }

    /// Appends the closed rectangle `r` with corner radius `radius` (clamped to half the
    /// shorter side). Rounded corners are cubic quarter circles (`KAPPA`), so a rounded
    /// rectangle has 4 lines and 4 cubics.
    pub fn rect(&mut self, r: Rect, radius: i32) -> &mut Self {
        let rad = Fx::from_int(radius.max(0));
        self.rounded_rect(FxRect::from(r), rad, rad)
    }

    /// Appends the closed rectangle `r` with elliptic corners `(rx, ry)` (each clamped to half
    /// the side, SVG `<rect>` semantics).
    pub fn rounded_rect(&mut self, r: FxRect, rx: Fx, ry: Fx) -> &mut Self {
        if r.is_empty() {
            return self;
        }
        let rx = rx.max(Fx::ZERO).min(Fx(r.width().0 / 2));
        let ry = ry.max(Fx::ZERO).min(Fx(r.height().0 / 2));
        let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);
        let p = FxPoint::new;
        if rx == Fx::ZERO || ry == Fx::ZERO {
            return self
                .move_to(p(x0, y0))
                .line_to(p(x1, y0))
                .line_to(p(x1, y1))
                .line_to(p(x0, y1))
                .close();
        }
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        self.move_to(p(x0 + rx, y0))
            .line_to(p(x1 - rx, y0))
            .cubic_to(p(x1 - rx + kx, y0), p(x1, y0 + ry - ky), p(x1, y0 + ry))
            .line_to(p(x1, y1 - ry))
            .cubic_to(p(x1, y1 - ry + ky), p(x1 - rx + kx, y1), p(x1 - rx, y1))
            .line_to(p(x0 + rx, y1))
            .cubic_to(p(x0 + rx - kx, y1), p(x0, y1 - ry + ky), p(x0, y1 - ry))
            .line_to(p(x0, y0 + ry))
            .cubic_to(p(x0, y0 + ry - ky), p(x0 + rx - kx, y0), p(x0 + rx, y0))
            .close()
    }

    /// Appends a closed circle (4 cubics, clockwise on screen, starting at 3 o'clock).
    pub fn circle(&mut self, c: FxPoint, r: Fx) -> &mut Self {
        self.ellipse(c, r, r)
    }

    /// Appends a closed axis-aligned ellipse (4 cubics, clockwise on screen).
    pub fn ellipse(&mut self, c: FxPoint, rx: Fx, ry: Fx) -> &mut Self {
        if rx.0 <= 0 || ry.0 <= 0 {
            return self;
        }
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let p = |dx: Fx, dy: Fx| FxPoint::new(c.x + dx, c.y + dy);
        let z = Fx::ZERO;
        self.move_to(p(rx, z))
            .cubic_to(p(rx, ky), p(kx, ry), p(z, ry))
            .cubic_to(p(-kx, ry), p(-rx, ky), p(-rx, z))
            .cubic_to(p(-rx, -ky), p(-kx, -ry), p(z, -ry))
            .cubic_to(p(kx, -ry), p(rx, -ky), p(rx, z))
            .close()
    }

    /// Bounding box of all points (control points included, so it contains the curves).
    /// An empty path gives [`FxRect::ZERO`].
    #[must_use]
    pub fn bounds(&self) -> FxRect {
        let Some(&first) = self.points.first() else {
            return FxRect::ZERO;
        };
        self.points
            .iter()
            .fold(FxRect::from_points(first, first), |r, &p| r.include(p))
    }

    /// Maps every point by `t` (Bézier curves are affine invariant).
    pub fn transform(&mut self, t: &Transform) {
        for p in &mut self.points {
            *p = p.transformed(t);
        }
        self.start = self.start.transformed(t);
    }

    /// Iterates the commands as [`PathEl`]s with their points.
    pub fn iter(&self) -> PathIter<'_> {
        PathIter {
            path: self,
            v: 0,
            p: 0,
        }
    }
}

/// One path command with its points (see [`Path::iter`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathEl {
    /// Start of a sub-path.
    MoveTo(FxPoint),
    /// Line to the point.
    LineTo(FxPoint),
    /// Quadratic curve (control, end).
    QuadTo(FxPoint, FxPoint),
    /// Cubic curve (control 1, control 2, end).
    CubicTo(FxPoint, FxPoint, FxPoint),
    /// Close the sub-path.
    Close,
}

/// Iterator over the commands of a [`Path`].
#[derive(Clone, Debug)]
pub struct PathIter<'a> {
    path: &'a Path,
    v: usize,
    p: usize,
}

impl Iterator for PathIter<'_> {
    type Item = PathEl;
    fn next(&mut self) -> Option<PathEl> {
        let verb = *self.path.verbs.get(self.v)?;
        let n = verb.points();
        let pts = self.path.points.get(self.p..self.p + n)?;
        self.v += 1;
        self.p += n;
        Some(match verb {
            Verb::MoveTo => PathEl::MoveTo(pts[0]),
            Verb::LineTo => PathEl::LineTo(pts[0]),
            Verb::QuadTo => PathEl::QuadTo(pts[0], pts[1]),
            Verb::CubicTo => PathEl::CubicTo(pts[0], pts[1], pts[2]),
            Verb::Close => PathEl::Close,
        })
    }
}

impl<'a> IntoIterator for &'a Path {
    type Item = PathEl;
    type IntoIter = PathIter<'a>;
    fn into_iter(self) -> PathIter<'a> {
        self.iter()
    }
}

/// `√v` of a non-negative 16.16 `i128` value, as 16.16.
fn sqrt_fx128(v: i128) -> i64 {
    if v <= 0 {
        return 0;
    }
    let s = u64::try_from(v).unwrap_or(u64::MAX);
    // √(v / 2^16) · 2^16 = √(v · 2^16)
    if s <= u64::MAX >> 16 {
        i64::from(isqrt64(s << 16))
    } else {
        i64::from(isqrt64(s)) << 8
    }
}

/// Converts an SVG arc (endpoint parameterization) into cubics appended to `path`.
#[allow(clippy::too_many_arguments)]
fn arc_segments(
    path: &mut Path,
    p0: FxPoint,
    p: FxPoint,
    mut rx: i64,
    mut ry: i64,
    x_rot: Angle,
    large: bool,
    sweep: bool,
) {
    let (c, s) = (i64::from(cos_fx(x_rot).0), i64::from(sin_fx(x_rot).0));
    let (x0, y0) = p0.raw();
    let (x1, y1) = p.raw();
    let (hx, hy) = ((x0 - x1) / 2, (y0 - y1) / 2);
    // Step 1: (x1', y1') in 16.16.
    let xp = (c * hx + s * hy) >> 16;
    let yp = (-s * hx + c * hy) >> 16;
    // Step 2: scale up radii that are too small. Λ in 16.16.
    let (xp2, yp2) = (i128::from(xp) * i128::from(xp), i128::from(yp) * i128::from(yp));
    let lambda = (xp2 << 16) / (i128::from(rx) * i128::from(rx)).max(1)
        + (yp2 << 16) / (i128::from(ry) * i128::from(ry)).max(1);
    if lambda > 1 << 16 {
        let f = sqrt_fx128(lambda);
        rx = muldiv(rx, f, 1 << 16) + 1;
        ry = muldiv(ry, f, 1 << 16) + 1;
    }
    // Step 3: center (cx', cy'). Work in 24.8 to keep the 4th powers inside i128.
    let (rx8, ry8, xp8, yp8) = (
        i128::from(rx >> 8),
        i128::from(ry >> 8),
        i128::from(xp >> 8),
        i128::from(yp >> 8),
    );
    let (rx2, ry2) = (rx8 * rx8, ry8 * ry8);
    let num = rx2 * ry2 - rx2 * yp8 * yp8 - ry2 * xp8 * xp8;
    let den = rx2 * yp8 * yp8 + ry2 * xp8 * xp8;
    let mut coef = if num <= 0 || den == 0 {
        0
    } else {
        sqrt_fx128((num << 16) / den)
    };
    if large == sweep {
        coef = -coef;
    }
    let cxp = muldiv(muldiv(coef, rx, 1 << 16), yp, ry);
    let cyp = -muldiv(muldiv(coef, ry, 1 << 16), xp, rx);
    // Step 4: center in user space.
    let cx = ((c * cxp - s * cyp) >> 16) + (x0 + x1) / 2;
    let cy = ((s * cxp + c * cyp) >> 16) + (y0 + y1) / 2;
    // Unit vectors (16.16) of the start and end points on the unit circle.
    let u = (muldiv(xp - cxp, 1 << 16, rx), muldiv(yp - cyp, 1 << 16, ry));
    let v = (muldiv(-xp - cxp, 1 << 16, rx), muldiv(-yp - cyp, 1 << 16, ry));
    let th1 = atan2(sat(u.1), sat(u.0)).0;
    let th2 = atan2(sat(v.1), sat(v.0)).0;
    let mut dth = (th2 - th1).rem_euclid(3600);
    if !sweep && dth > 0 {
        dth -= 3600;
    }
    if dth == 0 {
        path.line_to(p);
        return;
    }
    let n = (dth.abs() + 899) / 900;
    let delta = dth / n;
    // Maps a unit-circle point to user space.
    let map = |ux: i64, uy: i64| -> FxPoint {
        let ex = (rx * ux) >> 16;
        let ey = (ry * uy) >> 16;
        fxp(((c * ex - s * ey) >> 16) + cx, ((s * ex + c * ey) >> 16) + cy)
    };
    // k = 4/3 · tan(δ/4), signed.
    let q = Angle(delta / 4);
    let k = muldiv(4 << 16, i64::from(sin_fx(q).0), 3 * i64::from(cos_fx(q).0).max(1));
    let point_at = |i: i32| -> (i64, i64) {
        if i == n {
            return v;
        }
        let a = Angle(delta * i);
        let (ca, sa) = (i64::from(cos_fx(a).0), i64::from(sin_fx(a).0));
        // u·cos a + perp(u)·sin a, perp(x, y) = (−y, x).
        ((u.0 * ca - u.1 * sa) >> 16, (u.1 * ca + u.0 * sa) >> 16)
    };
    let mut a = u;
    for i in 1..=n {
        let b = point_at(i);
        let c1 = (a.0 - ((k * a.1) >> 16), a.1 + ((k * a.0) >> 16));
        let c2 = (b.0 + ((k * b.1) >> 16), b.1 - ((k * b.0) >> 16));
        let end = if i == n { p } else { map(b.0, b.1) };
        path.cubic_to(map(c1.0, c1.1), map(c2.0, c2.1), end);
        a = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_move_and_close() {
        let mut p = Path::new();
        p.line_to(FxPoint::from_int(5, 5))
            .close()
            .line_to(FxPoint::from_int(1, 1));
        assert_eq!(
            p.verbs(),
            &[
                Verb::MoveTo,
                Verb::LineTo,
                Verb::Close,
                Verb::MoveTo,
                Verb::LineTo
            ]
        );
        assert_eq!(p.points()[2], FxPoint::ZERO);
        let mut q = Path::new();
        q.move_to(FxPoint::from_int(1, 1))
            .move_to(FxPoint::from_int(2, 2));
        assert_eq!(q.verbs(), &[Verb::MoveTo]);
        assert_eq!(q.current_point(), FxPoint::from_int(2, 2));
    }

    #[test]
    fn arc_degenerate_cases() {
        let mut p = Path::new();
        p.move_to(FxPoint::from_int(0, 0));
        p.arc_to(
            FxPoint::from_int(0, 5),
            Angle(0),
            false,
            true,
            FxPoint::from_int(10, 0),
        );
        assert_eq!(p.verbs().last(), Some(&Verb::LineTo));
        let n = p.verbs().len();
        p.arc_to(
            FxPoint::from_int(5, 5),
            Angle(0),
            false,
            true,
            FxPoint::from_int(10, 0),
        );
        assert_eq!(p.verbs().len(), n);
        // Radii too small: scaled to a half circle (2 cubics).
        p.arc_to(
            FxPoint::from_int(1, 1),
            Angle(0),
            false,
            true,
            FxPoint::from_int(30, 0),
        );
        assert_eq!(p.verbs().len(), n + 2);
        assert_eq!(p.current_point(), FxPoint::from_int(30, 0));
    }
}
