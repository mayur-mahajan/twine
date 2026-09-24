//! Flattening: curves → line segments within a tolerance (default 0.25 px).
//!
//! The number of segments of a Bézier curve of degree `d` comes from Wang's formula,
//! `n = ⌈√(d(d−1)/8 · M / tol)⌉` with `M` the largest second difference of the control points
//! (`|p0 − 2p1 + p2|` for quadratics, the max of both for cubics), computed with integer square
//! roots and capped at [`MAX_CURVE_SEGMENTS`]. Points are evaluated exactly with the Bernstein
//! form in `i64` (no accumulated error). Transforms are applied to the control points *before*
//! flattening, so the tolerance holds in device space.

use alloc::vec::Vec;

use twine_core::math::isqrt64;
use twine_core::{Fx, Transform};

use crate::geom::{FxPoint, fxp};
use crate::path::{Path, PathEl};

/// Default flattening tolerance: 0.25 px.
pub const DEFAULT_TOLERANCE: Fx = Fx(1 << 14);

/// Most segments one curve is split into.
pub const MAX_CURVE_SEGMENTS: u32 = 64;

/// A line segment in device space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Line {
    /// Start.
    pub p0: FxPoint,
    /// End.
    pub p1: FxPoint,
}

/// Number of segments for a curve of degree 2 or 3 with largest second difference `m`
/// (raw 16.16 length) at tolerance `tol`.
fn segment_count(m: i64, degree: i64, tol: Fx) -> u32 {
    let tol = i64::from(tol.0.max(1));
    // r = d(d−1)/8 · m / tol, in 16.16.
    let num = (i128::from(m) * i128::from(degree * (degree - 1))) << 16;
    let r = num / i128::from(8 * tol);
    let r = u64::try_from(r).unwrap_or(u64::MAX).min(1 << 40);
    // √r in 16.16 = √(r · 2^16); round up to an integer.
    let s = u64::from(isqrt64(r << 16));
    let n = s.div_ceil(1 << 16);
    (n as u32).clamp(1, MAX_CURVE_SEGMENTS)
}

/// Length of a raw second difference vector (overflow-safe).
fn diff_len(a: FxPoint, b: FxPoint, c: FxPoint) -> i64 {
    let (ax, ay) = a.raw();
    let (bx, by) = b.raw();
    let (cx, cy) = c.raw();
    crate::geom::hypot(ax - 2 * bx + cx, ay - 2 * by + cy)
}

/// Calls `f` with the points of the flattened quadratic `p0 → p2` (excluding `p0`).
pub(crate) fn flatten_quad(p0: FxPoint, p1: FxPoint, p2: FxPoint, tol: Fx, mut f: impl FnMut(FxPoint)) {
    let n = i64::from(segment_count(diff_len(p0, p1, p2), 2, tol));
    let (x0, y0) = p0.raw();
    let (x1, y1) = p1.raw();
    let (x2, y2) = p2.raw();
    let nn = n * n;
    for i in 1..n {
        let (a, b) = (n - i, i);
        let w = [a * a, 2 * a * b, b * b];
        let ev = |v0: i64, v1: i64, v2: i64| -> i64 {
            let s = i128::from(w[0]) * i128::from(v0)
                + i128::from(w[1]) * i128::from(v1)
                + i128::from(w[2]) * i128::from(v2);
            div_round(s, i128::from(nn))
        };
        f(fxp(ev(x0, x1, x2), ev(y0, y1, y2)));
    }
    f(p2);
}

/// Calls `f` with the points of the flattened cubic `p0 → p3` (excluding `p0`).
pub(crate) fn flatten_cubic(
    p0: FxPoint,
    p1: FxPoint,
    p2: FxPoint,
    p3: FxPoint,
    tol: Fx,
    mut f: impl FnMut(FxPoint),
) {
    let m = diff_len(p0, p1, p2).max(diff_len(p1, p2, p3));
    let n = i64::from(segment_count(m, 3, tol));
    let (x0, y0) = p0.raw();
    let (x1, y1) = p1.raw();
    let (x2, y2) = p2.raw();
    let (x3, y3) = p3.raw();
    let n3 = n * n * n;
    for i in 1..n {
        let (a, b) = (n - i, i);
        let w = [a * a * a, 3 * a * a * b, 3 * a * b * b, b * b * b];
        let ev = |v0: i64, v1: i64, v2: i64, v3: i64| -> i64 {
            let s = i128::from(w[0]) * i128::from(v0)
                + i128::from(w[1]) * i128::from(v1)
                + i128::from(w[2]) * i128::from(v2)
                + i128::from(w[3]) * i128::from(v3);
            div_round(s, i128::from(n3))
        };
        f(fxp(ev(x0, x1, x2, x3), ev(y0, y1, y2, y3)));
    }
    f(p3);
}

fn div_round(n: i128, d: i128) -> i64 {
    let q = (n.abs() + d / 2) / d;
    let q = if n < 0 { -q } else { q };
    q.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// Flattens `path` (mapped by `t`) into device-space lines for filling, appending to `out`.
/// Open sub-paths are closed. Zero-length segments are dropped. `out` is not cleared, so
/// callers keep its capacity across frames.
///
/// ```
/// use twine_core::{Fx, Transform};
/// use twine_vector::{FxPoint, Path, flatten, DEFAULT_TOLERANCE};
///
/// let mut p = Path::new();
/// p.circle(FxPoint::from_int(0, 0), Fx::from_int(100));
/// let mut lines = Vec::new();
/// flatten(&p, &Transform::IDENTITY, DEFAULT_TOLERANCE, &mut lines);
/// assert!(lines.len() <= 64);
/// ```
pub fn flatten(path: &Path, t: &Transform, tol: Fx, out: &mut Vec<Line>) {
    let tp = |p: FxPoint| p.transformed(t);
    let seg = |out: &mut Vec<Line>, p0: FxPoint, p1: FxPoint| {
        if p0 != p1 {
            out.push(Line { p0, p1 });
        }
    };
    let mut start = FxPoint::ZERO;
    let mut cur = FxPoint::ZERO;
    for el in path {
        match el {
            PathEl::MoveTo(p) => {
                seg(out, cur, start);
                start = tp(p);
                cur = start;
            }
            PathEl::LineTo(p) => {
                let p = tp(p);
                seg(out, cur, p);
                cur = p;
            }
            PathEl::QuadTo(c, p) => {
                flatten_quad(cur, tp(c), tp(p), tol, |q| {
                    seg(out, cur, q);
                    cur = q;
                });
            }
            PathEl::CubicTo(c1, c2, p) => {
                flatten_cubic(cur, tp(c1), tp(c2), tp(p), tol, |q| {
                    seg(out, cur, q);
                    cur = q;
                });
            }
            PathEl::Close => {
                seg(out, cur, start);
                cur = start;
            }
        }
    }
    // Close the last open sub-path (a no-op when it was closed).
    seg(out, cur, start);
}

/// One flattened sub-path for stroking (see [`Polylines`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Polyline<'a> {
    /// Points, consecutive duplicates removed.
    pub points: &'a [FxPoint],
    /// Whether the sub-path was closed (`Close` verb).
    pub closed: bool,
}

/// Flattened sub-paths for stroking, stored flat (one point buffer, one range per sub-path)
/// so the capacity is reused across frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Polylines {
    points: Vec<FxPoint>,
    ranges: Vec<(u32, u32, bool)>,
}

impl Polylines {
    /// No polylines.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            points: Vec::new(),
            ranges: Vec::new(),
        }
    }

    /// Removes every polyline, keeping the capacity.
    pub fn clear(&mut self) {
        self.points.clear();
        self.ranges.clear();
    }

    /// Number of polylines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Whether there are no polylines.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Iterates the polylines.
    pub fn iter(&self) -> impl Iterator<Item = Polyline<'_>> + '_ {
        self.ranges.iter().map(|&(a, b, closed)| Polyline {
            points: &self.points[a as usize..b as usize],
            closed,
        })
    }

    /// Starts a polyline at `p` (finish it with [`end`](Self::end)).
    pub(crate) fn begin(&mut self, p: FxPoint) {
        let n = self.points.len() as u32;
        self.ranges.push((n, n, false));
        self.push(p);
    }

    /// Appends `p` to the current polyline (duplicates of the last point are skipped).
    pub(crate) fn push(&mut self, p: FxPoint) {
        let Some(r) = self.ranges.last_mut() else {
            return;
        };
        if r.1 > r.0 && self.points.last() == Some(&p) {
            return;
        }
        self.points.push(p);
        r.1 = self.points.len() as u32;
    }

    /// Marks the current polyline closed or open. A closed polyline whose last point equals
    /// its first drops the duplicate.
    pub(crate) fn end(&mut self, closed: bool) {
        let Some(r) = self.ranges.last_mut() else {
            return;
        };
        r.2 = closed;
        if closed && r.1 - r.0 >= 2 && self.points[r.0 as usize] == self.points[r.1 as usize - 1] {
            self.points.pop();
            r.1 -= 1;
        }
    }
}

/// Flattens `path` (mapped by `t`) into polylines for stroking, keeping open/closed
/// information. `out` is not cleared.
pub fn flatten_for_stroke(path: &Path, t: &Transform, tol: Fx, out: &mut Polylines) {
    let tp = |p: FxPoint| p.transformed(t);
    let mut cur = FxPoint::ZERO;
    let mut open = false;
    for el in path {
        match el {
            PathEl::MoveTo(p) => {
                if open {
                    out.end(false);
                }
                cur = tp(p);
                out.begin(cur);
                open = true;
            }
            PathEl::LineTo(p) => {
                cur = tp(p);
                out.push(cur);
            }
            PathEl::QuadTo(c, p) => {
                flatten_quad(cur, tp(c), tp(p), tol, |q| out.push(q));
                cur = tp(p);
            }
            PathEl::CubicTo(c1, c2, p) => {
                flatten_cubic(cur, tp(c1), tp(c2), tp(p), tol, |q| out.push(q));
                cur = tp(p);
            }
            PathEl::Close => {
                if open {
                    out.end(true);
                    open = false;
                }
            }
        }
    }
    if open {
        out.end(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_counts() {
        // A straight curve (zero second difference) is one segment.
        assert_eq!(segment_count(0, 3, DEFAULT_TOLERANCE), 1);
        // M = 1 px, tol 0.25: quad √(1/4 · 4) = 1; cubic √(3/4 · 4) = 1.73 → 2.
        assert_eq!(segment_count(1 << 16, 2, DEFAULT_TOLERANCE), 1);
        assert_eq!(segment_count(1 << 16, 3, DEFAULT_TOLERANCE), 2);
        assert_eq!(segment_count(i64::MAX / 4, 3, Fx(1)), MAX_CURVE_SEGMENTS);
    }

    #[test]
    fn polylines_dedup_and_close() {
        let mut p = Path::new();
        p.move_to(FxPoint::from_int(0, 0))
            .line_to(FxPoint::from_int(0, 0))
            .line_to(FxPoint::from_int(5, 0))
            .line_to(FxPoint::from_int(0, 0))
            .close()
            .move_to(FxPoint::from_int(9, 9))
            .line_to(FxPoint::from_int(9, 12));
        let mut out = Polylines::new();
        flatten_for_stroke(&p, &Transform::IDENTITY, DEFAULT_TOLERANCE, &mut out);
        let v: Vec<_> = out.iter().collect();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].points.len(), 2);
        assert!(v[0].closed);
        assert!(!v[1].closed);
    }
}
