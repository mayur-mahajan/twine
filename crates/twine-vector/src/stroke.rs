//! Strokes: [`Stroke`], [`LineJoin`], [`LineCap`], [`Dash`] and the stroker that turns
//! polylines into closed outlines filled with the non-zero rule.
//!
//! Every segment is offset by ± half the width along its normal (normalized with an integer
//! square root). An open polyline becomes one closed outline: the left side forward, the end
//! cap, the right side backward, the start cap. A closed polyline becomes two outlines (left
//! side forward, right side backward), so the inside stays empty. On the outer side of a
//! vertex the join is inserted (miter — or bevel past the miter limit —, round, bevel); on the
//! inner side the outline passes through the vertex itself, which keeps it watertight for any
//! angle. Round joins and caps are arcs subdivided by bisection until the sagitta is within the
//! flattening tolerance (no trigonometry). Dashes split the polylines into open pieces first.
//!
//! Width ≤ 1 px strokes (hairlines) take the same path: the coverage-based rasterizer already
//! draws them with sub-pixel precision and any paint.

use alloc::vec::Vec;

use twine_core::{Fx, Transform};

use crate::flatten::{Line, Polyline, Polylines};
use crate::geom::{FxPoint, fxp, hypot, muldiv};

/// Shape of the outer corner where two stroked segments meet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LineJoin {
    /// Sharp corner (bevel when longer than `miter_limit × width`).
    #[default]
    Miter,
    /// Circular corner.
    Round,
    /// Corner cut off straight.
    Bevel,
}

/// Shape of the ends of open strokes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LineCap {
    /// Ends exactly at the end point.
    #[default]
    Butt,
    /// Half circle around the end point.
    Round,
    /// Extends half the width past the end point.
    Square,
}

/// Maximum dash pattern entries.
pub const MAX_DASHES: usize = 8;

/// A dash pattern: alternating "on" and "off" lengths (path units), starting `offset` into the
/// pattern. An odd number of entries is repeated once (SVG), so `[4]` is 4 on, 4 off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Dash {
    /// On/off lengths (at most [`MAX_DASHES`]).
    pub pattern: heapless::Vec<Fx, MAX_DASHES>,
    /// Start offset into the pattern.
    pub offset: Fx,
}

impl Dash {
    /// A dash pattern (entries beyond [`MAX_DASHES`] are dropped).
    ///
    /// ```
    /// use twine_core::Fx;
    /// use twine_vector::Dash;
    /// let d = Dash::new(&[Fx::from_int(6), Fx::from_int(3)], Fx::ZERO);
    /// assert_eq!(d.pattern.len(), 2);
    /// ```
    #[must_use]
    pub fn new(pattern: &[Fx], offset: Fx) -> Self {
        let mut p = heapless::Vec::new();
        for &v in pattern.iter().take(MAX_DASHES) {
            let _ = p.push(v);
        }
        Self { pattern: p, offset }
    }

    /// Whether the pattern draws dashes (non-empty, no negative entry, positive sum).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.pattern.is_empty()
            && self.pattern.iter().all(|v| v.0 >= 0)
            && self.pattern.iter().map(|v| i64::from(v.0)).sum::<i64>() > 0
    }
}

/// Stroke parameters.
///
/// ```
/// use twine_core::Fx;
/// use twine_vector::{LineCap, LineJoin, Stroke};
/// let s = Stroke { width: Fx::from_int(4), cap: LineCap::Round, ..Stroke::default() };
/// assert_eq!(s.join, LineJoin::Miter);
/// assert_eq!(s.miter_limit, Fx::from_int(4));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stroke {
    /// Line width (path units).
    pub width: Fx,
    /// Corner shape.
    pub join: LineJoin,
    /// End shape.
    pub cap: LineCap,
    /// Longest miter as a multiple of the width (SVG `stroke-miterlimit`, default 4).
    pub miter_limit: Fx,
    /// Dash pattern.
    pub dash: Option<Dash>,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: Fx::ONE,
            join: LineJoin::Miter,
            cap: LineCap::Butt,
            miter_limit: Fx::from_int(4),
            dash: None,
        }
    }
}

/// Raw 16.16 vector.
type V = (i64, i64);

#[inline]
fn add(a: V, b: V) -> V {
    (a.0 + b.0, a.1 + b.1)
}

#[inline]
fn neg(a: V) -> V {
    (-a.0, -a.1)
}

/// `v` scaled to length `len`.
fn with_len(v: V, len: i64) -> V {
    let l = hypot(v.0, v.1);
    if l == 0 {
        return (0, 0);
    }
    (muldiv(v.0, len, l), muldiv(v.1, len, l))
}

/// Emits the outline as device-space lines.
struct Outline<'a> {
    t: &'a Transform,
    out: &'a mut Vec<Line>,
    first: FxPoint,
    last: FxPoint,
}

impl Outline<'_> {
    fn start(&mut self, p: V) {
        let d = fxp(p.0, p.1).transformed(self.t);
        self.first = d;
        self.last = d;
    }

    fn to(&mut self, p: V) {
        let d = fxp(p.0, p.1).transformed(self.t);
        if d != self.last {
            self.out.push(Line { p0: self.last, p1: d });
            self.last = d;
        }
    }

    fn close(&mut self) {
        if self.first != self.last {
            self.out.push(Line {
                p0: self.last,
                p1: self.first,
            });
        }
        self.last = self.first;
    }
}

/// Stroker state for one draw.
struct Stroker<'s> {
    s: &'s Stroke,
    hw: i64,
    tol: i64,
}

impl Stroker<'_> {
    /// Left normal of `a → b` with length `hw`.
    fn normal(&self, a: FxPoint, b: FxPoint) -> V {
        let (ax, ay) = a.raw();
        let (bx, by) = b.raw();
        with_len((ay - by, bx - ax), self.hw)
    }

    /// Arc around `c` from offset `from` to `to` (both of length `hw`, angle < 180°), emitting
    /// the points after `from` up to `to`.
    fn arc(&self, o: &mut Outline<'_>, c: V, from: V, to: V, depth: u32) {
        let m = add(from, to);
        let ml = hypot(m.0, m.1);
        let sag = self.hw - ml / 2;
        if depth >= 10 || sag <= self.tol || ml == 0 {
            o.to(add(c, to));
            return;
        }
        let mid = with_len(m, self.hw);
        self.arc(o, c, from, mid, depth + 1);
        self.arc(o, c, mid, to, depth + 1);
    }

    /// Half circle from `n` to `−n` around `c` through the direction `(n.y, −n.x)`.
    fn half_circle(&self, o: &mut Outline<'_>, c: V, n: V) {
        let d = (n.1, -n.0);
        self.arc(o, c, n, d, 0);
        self.arc(o, c, d, neg(n), 0);
    }

    /// Join at `p`: the outline is at the end of the incoming offset `p + na` and continues
    /// with the outgoing offset `p + nb`.
    fn join(&self, o: &mut Outline<'_>, p: V, na: V, nb: V) {
        o.to(add(p, na));
        let cross = i128::from(nb.0) * i128::from(na.1) - i128::from(nb.1) * i128::from(na.0);
        let dot = i128::from(na.0) * i128::from(nb.0) + i128::from(na.1) * i128::from(nb.1);
        if cross == 0 && dot > 0 {
            o.to(add(p, nb));
            return;
        }
        let outer = cross > 0 || (cross == 0 && dot < 0);
        if !outer {
            o.to(p);
            o.to(add(p, nb));
            return;
        }
        match self.s.join {
            LineJoin::Bevel => {}
            LineJoin::Miter => {
                let hw2 = i128::from(self.hw) * i128::from(self.hw);
                let den = hw2 + dot;
                let lim = i128::from(self.s.miter_limit.0.max(0));
                // ratio² = 2·hw² / (hw² + na·nb) ≤ limit²  (limit in 16.16).
                if den > 0 && (2 * hw2) << 32 <= lim * lim * den {
                    let m = add(na, nb);
                    let k = |v: i64| (i128::from(v) * hw2 / den) as i64;
                    o.to((p.0 + k(m.0), p.1 + k(m.1)));
                }
            }
            LineJoin::Round => {
                if cross == 0 {
                    let d = (na.1, -na.0);
                    self.arc(o, p, na, d, 0);
                    self.arc(o, p, d, nb, 0);
                } else {
                    self.arc(o, p, na, nb, 0);
                }
            }
        }
        o.to(add(p, nb));
    }

    /// Cap at `p`: the outline is at `p + n` and continues at `p − n`; the cap extends in the
    /// direction `(n.y, −n.x)`.
    fn cap(&self, o: &mut Outline<'_>, p: V, n: V) {
        match self.s.cap {
            LineCap::Butt => {}
            LineCap::Square => {
                let d = (n.1, -n.0);
                o.to(add(add(p, n), d));
                o.to(add(add(p, neg(n)), d));
            }
            LineCap::Round => self.half_circle(o, p, n),
        }
        o.to(add(p, neg(n)));
    }

    fn polyline(&self, o: &mut Outline<'_>, pl: Polyline<'_>) {
        let pts = pl.points;
        let n = pts.len();
        let raw = |i: usize| pts[i].raw();
        if n == 1 {
            // Zero-length sub-path: a dot for round and square caps.
            let c = raw(0);
            let h = self.hw;
            match self.s.cap {
                LineCap::Butt => {}
                LineCap::Round => {
                    o.start(add(c, (h, 0)));
                    self.half_circle(o, c, (h, 0));
                    self.half_circle(o, c, (-h, 0));
                    o.close();
                }
                LineCap::Square => {
                    o.start((c.0 - h, c.1 - h));
                    o.to((c.0 + h, c.1 - h));
                    o.to((c.0 + h, c.1 + h));
                    o.to((c.0 - h, c.1 + h));
                    o.close();
                }
            }
            return;
        }
        let nrm = |i: usize| self.normal(pts[i % n], pts[(i + 1) % n]);
        if pl.closed {
            // Left side forward.
            o.start(add(raw(0), nrm(0)));
            for j in 1..=n {
                self.join(o, raw(j % n), nrm(j - 1), nrm(j % n));
            }
            o.close();
            // Right side backward.
            o.start(add(raw(0), neg(nrm(n - 1))));
            for j in (0..n).rev() {
                self.join(o, raw(j), neg(nrm(j)), neg(nrm((j + n - 1) % n)));
            }
            o.close();
        } else {
            o.start(add(raw(0), nrm(0)));
            for i in 1..n - 1 {
                self.join(o, raw(i), nrm(i - 1), nrm(i));
            }
            o.to(add(raw(n - 1), nrm(n - 2)));
            self.cap(o, raw(n - 1), nrm(n - 2));
            for i in (1..n - 1).rev() {
                self.join(o, raw(i), neg(nrm(i)), neg(nrm(i - 1)));
            }
            o.to(add(raw(0), neg(nrm(0))));
            self.cap(o, raw(0), neg(nrm(0)));
            o.close();
        }
    }
}

/// Most dashes emitted per polyline; beyond it the polyline is stroked solid (a warning is
/// logged) so tiny patterns on long paths cannot exhaust memory.
pub const MAX_DASHES_PER_POLYLINE: i64 = 4096;

/// Splits `input` into the "on" pieces of `dash` (appended to `out` as open polylines). The
/// pattern restarts (at `offset`) for every sub-path.
fn dash_polylines(input: &Polylines, dash: &Dash, out: &mut Polylines) {
    let pat = &dash.pattern;
    let len = pat.len();
    let count = if len % 2 == 1 { 2 * len } else { len };
    let val = |k: usize| i64::from(pat[k % len].0);
    let total: i64 = (0..count).map(val).sum();
    for pl in input.iter() {
        let pts = pl.points;
        let nseg = if pl.closed {
            pts.len()
        } else {
            pts.len().saturating_sub(1)
        };
        let seg = |i: usize| (pts[i].raw(), pts[(i + 1) % pts.len()].raw());
        let length: i64 = (0..nseg)
            .map(|i| {
                let (a, b) = seg(i);
                hypot(b.0 - a.0, b.1 - a.1)
            })
            .sum();
        if length / total.max(1) > MAX_DASHES_PER_POLYLINE {
            twine_core::warn!(target: "twine::vector", "dash pattern too fine for the path; stroking solid");
            out.begin(pts[0]);
            for &p in &pts[1..] {
                out.push(p);
            }
            out.end(pl.closed);
            continue;
        }
        // Advance by the offset.
        let mut k = 0usize;
        let mut rem = val(0);
        let mut off = i64::from(dash.offset.0).rem_euclid(total);
        while off > 0 {
            if off >= rem {
                off -= rem;
                k = (k + 1) % count;
                rem = val(k);
            } else {
                rem -= off;
                off = 0;
            }
        }
        let mut drawing = false;
        for i in 0..nseg {
            let (a, b) = seg(i);
            let l = hypot(b.0 - a.0, b.1 - a.1);
            let at = |s: i64| fxp(a.0 + muldiv(b.0 - a.0, s, l), a.1 + muldiv(b.1 - a.1, s, l));
            let mut pos = 0;
            loop {
                let step = rem.min(l - pos);
                let on = k % 2 == 0;
                if on {
                    if !drawing {
                        out.begin(at(pos));
                        drawing = true;
                    }
                    out.push(at(pos + step));
                }
                pos += step;
                rem -= step;
                if rem == 0 {
                    if on {
                        out.end(false);
                        drawing = false;
                    }
                    k = (k + 1) % count;
                    rem = val(k);
                }
                if pos >= l {
                    break;
                }
            }
        }
        if drawing {
            out.end(false);
        }
    }
}

/// Strokes `polys` (path space) into closed device-space outlines appended to `out`.
/// `dashed` is scratch for the dash pieces; `tol` is the arc tolerance in path units.
pub(crate) fn stroke_polylines(
    polys: &Polylines,
    s: &Stroke,
    tol: Fx,
    t: &Transform,
    dashed: &mut Polylines,
    out: &mut Vec<Line>,
) {
    if s.width.0 <= 0 {
        return;
    }
    let st = Stroker {
        s,
        hw: (i64::from(s.width.0) / 2).max(1),
        tol: i64::from(tol.0.max(1)),
    };
    let mut o = Outline {
        t,
        out,
        first: FxPoint::ZERO,
        last: FxPoint::ZERO,
    };
    let src = match &s.dash {
        Some(d) if d.is_valid() => {
            dashed.clear();
            dash_polylines(polys, d, dashed);
            &*dashed
        }
        _ => polys,
    };
    for pl in src.iter() {
        if !pl.points.is_empty() {
            st.polyline(&mut o, pl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatten::{DEFAULT_TOLERANCE, flatten_for_stroke};
    use crate::path::Path;

    fn outline(path: &Path, s: &Stroke) -> Vec<Line> {
        let mut pl = Polylines::new();
        flatten_for_stroke(path, &Transform::IDENTITY, DEFAULT_TOLERANCE, &mut pl);
        let mut d = Polylines::new();
        let mut out = Vec::new();
        stroke_polylines(&pl, s, DEFAULT_TOLERANCE, &Transform::IDENTITY, &mut d, &mut out);
        out
    }

    /// Signed area (2×, in px²) of the closed outlines.
    fn area2(lines: &[Line]) -> i64 {
        let s: i128 = lines
            .iter()
            .map(|l| {
                let (a, b) = (l.p0.raw(), l.p1.raw());
                i128::from(a.0) * i128::from(b.1) - i128::from(b.0) * i128::from(a.1)
            })
            .sum();
        (s >> 32) as i64
    }

    #[test]
    fn straight_line_butt_is_a_rectangle() {
        let mut p = Path::new();
        p.move_to(FxPoint::from_int(0, 0))
            .line_to(FxPoint::from_int(10, 0));
        let s = Stroke {
            width: Fx::from_int(2),
            ..Stroke::default()
        };
        let o = outline(&p, &s);
        assert_eq!(o.len(), 4);
        assert_eq!(area2(&o).abs(), 40);
    }

    #[test]
    fn caps_extend_area() {
        let mut p = Path::new();
        p.move_to(FxPoint::from_int(0, 0))
            .line_to(FxPoint::from_int(10, 0));
        let sq = Stroke {
            width: Fx::from_int(2),
            cap: LineCap::Square,
            ..Stroke::default()
        };
        assert_eq!(area2(&outline(&p, &sq)).abs(), 48);
        let rd = Stroke {
            width: Fx::from_int(2),
            cap: LineCap::Round,
            ..Stroke::default()
        };
        // 10·2 + π·1² ≈ 23.14 → 2× ≈ 46.
        assert!((area2(&outline(&p, &rd)).abs() - 46).abs() <= 1);
    }

    #[test]
    fn dash_splits() {
        let mut pl = Polylines::new();
        let mut p = Path::new();
        p.move_to(FxPoint::from_int(0, 0))
            .line_to(FxPoint::from_int(20, 0));
        flatten_for_stroke(&p, &Transform::IDENTITY, DEFAULT_TOLERANCE, &mut pl);
        let mut out = Polylines::new();
        dash_polylines(
            &pl,
            &Dash::new(&[Fx::from_int(4), Fx::from_int(2)], Fx::from_int(1)),
            &mut out,
        );
        let v: Vec<_> = out
            .iter()
            .map(|p| (p.points[0].x.to_int_round(), p.points[1].x.to_int_round()))
            .collect();
        assert_eq!(v, [(0, 3), (5, 9), (11, 15), (17, 20)]);
        // Odd pattern repeats: [3] = 3 on, 3 off.
        let mut out = Polylines::new();
        dash_polylines(&pl, &Dash::new(&[Fx::from_int(3)], Fx::ZERO), &mut out);
        assert_eq!(out.len(), 4);
    }
}
