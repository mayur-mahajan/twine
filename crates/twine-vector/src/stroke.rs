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

use twine_core::{Fx, Rect, Transform};

use crate::flatten::{Line, Polyline, Polylines};
use crate::geom::{FxPoint, FxRect, fxp, hypot, muldiv};

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
            && self.pattern.iter().all(|v| v.raw() >= 0)
            && self.pattern.iter().any(|v| v.raw() > 0)
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

/// Raw 16.16 vector. The stroker works on path coordinates (`i32` raw values) plus offsets of
/// at most the half width or the miter length; its sums saturate (exact for every outline
/// that fits the coordinate range, which `fxp` saturates to anyway). Outline building is per
/// vertex, not per pixel.
type V = (i64, i64);

#[inline]
fn add(a: V, b: V) -> V {
    (a.0.saturating_add(b.0), a.1.saturating_add(b.1))
}

#[inline]
fn neg(a: V) -> V {
    (a.0.saturating_neg(), a.1.saturating_neg())
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
        with_len((ay.saturating_sub(by), bx.saturating_sub(ax)), self.hw)
    }

    /// Arc around `c` from offset `from` to `to` (both of length `hw`, angle < 180°), emitting
    /// the points after `from` up to `to`.
    fn arc(&self, o: &mut Outline<'_>, c: V, from: V, to: V, depth: u32) {
        let m = add(from, to);
        let ml = hypot(m.0, m.1);
        let sag = self.hw.saturating_sub(ml / 2);
        if depth >= 10 || sag <= self.tol || ml == 0 {
            o.to(add(c, to));
            return;
        }
        let mid = with_len(m, self.hw);
        // SAFE-ARITH: `depth < 10` (returned above).
        self.arc(o, c, from, mid, depth.wrapping_add(1));
        self.arc(o, c, mid, to, depth.wrapping_add(1));
    }

    /// Half circle from `n` to `−n` around `c` through the direction `(n.y, −n.x)`.
    fn half_circle(&self, o: &mut Outline<'_>, c: V, n: V) {
        let d = (n.1, n.0.saturating_neg());
        self.arc(o, c, n, d, 0);
        self.arc(o, c, d, neg(n), 0);
    }

    /// Join at `p`: the outline is at the end of the incoming offset `p + na` and continues
    /// with the outgoing offset `p + nb`.
    fn join(&self, o: &mut Outline<'_>, p: V, na: V, nb: V) {
        o.to(add(p, na));
        // Products of two `i64`s fit in `i128`; their sums saturate (exact for normals of
        // length `hw ≤ 2^30`).
        let mul = |a: i64, b: i64| i128::from(a).saturating_mul(i128::from(b));
        let cross = mul(nb.0, na.1).saturating_sub(mul(nb.1, na.0));
        let dot = mul(na.0, nb.0).saturating_add(mul(na.1, nb.1));
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
                let hw2 = mul(self.hw, self.hw);
                let den = hw2.saturating_add(dot);
                let lim = i128::from(self.s.miter_limit.raw().max(0));
                // ratio² = 2·hw² / (hw² + na·nb) ≤ limit²  (limit in 16.16).
                if den > 0
                    && hw2.saturating_mul(2).saturating_mul(1 << 32)
                        <= lim.saturating_mul(lim).saturating_mul(den)
                {
                    let m = add(na, nb);
                    // `den > 0` (the `if`): the division's fallback is never taken.
                    let k = |v: i64| {
                        let q = i128::from(v).saturating_mul(hw2).checked_div(den).unwrap_or(0);
                        q.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
                    };
                    o.to(add(p, (k(m.0), k(m.1))));
                }
            }
            LineJoin::Round => {
                if cross == 0 {
                    let d = (na.1, na.0.saturating_neg());
                    self.arc(o, p, na, d, 0);
                    self.arc(o, p, d, nb, 0);
                } else {
                    self.arc(o, p, na, nb, 0);
                }
            }
        }
        o.to(add(p, nb));
    }

    /// Cap `cap` at `p`: the outline is at `p + n` and continues at `p − n`; the cap extends
    /// in the direction `(n.y, −n.x)`.
    fn cap(&self, o: &mut Outline<'_>, p: V, n: V, cap: LineCap) {
        match cap {
            LineCap::Butt => {}
            LineCap::Square => {
                let d = (n.1, n.0.saturating_neg());
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
        // Point `i` modulo `n` (closed polylines wrap around); `n ≥ 1` past the check below,
        // so the `get` never falls back.
        let at = |i: usize| -> FxPoint {
            pts.get(i.checked_rem(n).unwrap_or(0))
                .copied()
                .unwrap_or_default()
        };
        let raw = |i: usize| at(i).raw();
        if n == 0 {
            return;
        }
        if n == 1 {
            // Zero-length sub-path: a dot for round and square caps.
            let c = raw(0);
            let h = self.hw;
            match self.s.cap {
                LineCap::Butt => {}
                LineCap::Round => {
                    o.start(add(c, (h, 0)));
                    self.half_circle(o, c, (h, 0));
                    self.half_circle(o, c, (h.saturating_neg(), 0));
                    o.close();
                }
                LineCap::Square => {
                    let (x0, x1) = (c.0.saturating_sub(h), c.0.saturating_add(h));
                    let (y0, y1) = (c.1.saturating_sub(h), c.1.saturating_add(h));
                    o.start((x0, y0));
                    o.to((x1, y0));
                    o.to((x1, y1));
                    o.to((x0, y1));
                    o.close();
                }
            }
            return;
        }
        // SAFE-ARITH (the index arithmetic below): `n ≥ 2` here and every index is below `2n`.
        let nrm = |i: usize| self.normal(at(i), at(i.wrapping_add(1)));
        if pl.closed {
            // Left side forward.
            o.start(add(raw(0), nrm(0)));
            for j in 1..=n {
                self.join(o, raw(j), nrm(j.wrapping_sub(1)), nrm(j));
            }
            o.close();
            // Right side backward.
            let last = n.wrapping_sub(1);
            o.start(add(raw(0), neg(nrm(last))));
            for j in (0..n).rev() {
                self.join(o, raw(j), neg(nrm(j)), neg(nrm(j.wrapping_add(last))));
            }
            o.close();
        } else {
            self.open(o, at, n, (self.s.cap, self.s.cap));
        }
    }

    /// The open polyline of the `n ≥ 2` points `at(0..n)` as one closed outline: the left side
    /// forward, the end cap `caps.1`, the right side backward, the start cap `caps.0`.
    fn open(&self, o: &mut Outline<'_>, at: impl Fn(usize) -> FxPoint, n: usize, caps: (LineCap, LineCap)) {
        // SAFE-ARITH (the index arithmetic): `n ≥ 2` and every index is below `n`.
        let raw = |i: usize| at(i).raw();
        let nrm = |i: usize| self.normal(at(i), at(i.wrapping_add(1)));
        let (last, before) = (n.wrapping_sub(1), n.wrapping_sub(2));
        o.start(add(raw(0), nrm(0)));
        for i in 1..last {
            self.join(o, raw(i), nrm(i.wrapping_sub(1)), nrm(i));
        }
        o.to(add(raw(last), nrm(before)));
        self.cap(o, raw(last), nrm(before), caps.1);
        for i in (1..last).rev() {
            self.join(o, raw(i), neg(nrm(i)), neg(nrm(i.wrapping_sub(1))));
        }
        o.to(add(raw(0), neg(nrm(0))));
        self.cap(o, raw(0), neg(nrm(0)), caps.0);
        o.close();
    }

    /// [`polyline`](Self::polyline) restricted to the segments that can reach `reach`: each
    /// maximal run of them is stroked as an open polyline, with the stroke's caps at the
    /// polyline's own ends and butt ends where a segment was left out (see [`Reach`]).
    /// `coarse(i)` is a cheaper first test of segment `i` (its chunk reaches the region): a
    /// segment it rejects is left out without being tested itself.
    fn polyline_within(
        &self,
        o: &mut Outline<'_>,
        pl: Polyline<'_>,
        reach: &Reach<'_>,
        coarse: impl Fn(usize) -> bool,
    ) {
        let pts = pl.points;
        let n = pts.len();
        // Point `i` modulo `n`; `n ≥ 1` past the check below, so the `get` never falls back.
        let at = |i: usize| -> FxPoint {
            pts.get(i.checked_rem(n).unwrap_or(0))
                .copied()
                .unwrap_or_default()
        };
        // SAFE-ARITH (the index arithmetic below): indices are below `2n`.
        let hits = |i: usize| coarse(i) && reach.hits(at(i), at(i.wrapping_add(1)));
        if n == 0 {
            return;
        }
        if n == 1 {
            if hits(0) {
                self.polyline(o, pl);
            }
            return;
        }
        let segs = if pl.closed { n } else { n.wrapping_sub(1) };
        let mut any = false;
        let mut cut = None;
        for i in 0..segs {
            if hits(i) {
                any = true;
            } else if cut.is_none() {
                cut = Some(i);
            }
        }
        let Some(cut) = cut else {
            // Every segment reaches the region: the whole polyline, as without `reach`.
            self.polyline(o, pl);
            return;
        };
        if !any {
            return;
        }
        // A closed polyline is walked from just after a left-out segment, so no run wraps
        // around; an open one from its first segment.
        let start = if pl.closed { cut.wrapping_add(1) } else { 0 };
        let mut run: Option<usize> = None;
        for j in 0..=segs {
            let i = start.wrapping_add(j);
            if j < segs && hits(i) {
                run.get_or_insert(i);
                continue;
            }
            if let Some(first) = run.take() {
                // Segments `first..i`: points `first..=i`.
                let caps = (
                    if !pl.closed && first == 0 {
                        self.s.cap
                    } else {
                        LineCap::Butt
                    },
                    if !pl.closed && i == segs {
                        self.s.cap
                    } else {
                        LineCap::Butt
                    },
                );
                self.open(
                    o,
                    |k| at(first.wrapping_add(k)),
                    i.wrapping_sub(first).wrapping_add(1),
                    caps,
                );
            }
        }
    }
}

/// The device-space region in which a stroke can change pixels: the clip grown by the
/// stroke's reach — half the width times the largest offset factor of its joins and caps
/// (miter limit, `√2` of square caps, else 1), times the transform's largest stretch — plus a
/// margin for rounding.
///
/// Leaving out the segments whose reach misses the region changes no pixel in the clip:
/// the outline differs from the full one by a closed curve (the left-out segments' sides,
/// joins and caps, and the butt ends that replace their joins) that lies within the reach of
/// the left-out segments. Shrinking the width to zero moves that curve only within this reach
/// and turns it into the segments walked forward and back, so its winding number is zero at
/// every point of the clip; the rasterizer accumulates each cell from the winding numbers
/// (exact signed areas), so every covered cell gets the same coverage, and cells it no longer
/// visits had none.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reach<'t> {
    t: &'t Transform,
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

/// Rounding margin of [`Reach`]: 2 px (16.16), well above the outline's rounding (a few raw
/// units per point).
const REACH_MARGIN: i64 = 2 << 16;

impl<'t> Reach<'t> {
    /// The region of `clip` for strokes `s` (half width `hw`, 16.16 path units) drawn with
    /// `t`; `None` when it does not fit the coordinate range (nothing is left out then).
    fn new(s: &Stroke, hw: i64, t: &'t Transform, clip: Rect) -> Option<Self> {
        let factor = match s.join {
            LineJoin::Miter => i64::from(s.miter_limit.raw()).max(1 << 16),
            LineJoin::Round | LineJoin::Bevel => 1 << 16,
        };
        let factor = if s.cap == LineCap::Square {
            factor.max(3 << 15)
        } else {
            factor
        };
        // The largest stretch of `t` is at most the Frobenius norm of its linear part.
        let stretch = hypot(
            hypot(i64::from(t.a.raw()), i64::from(t.b.raw())),
            hypot(i64::from(t.c.raw()), i64::from(t.d.raw())),
        );
        // SAFE-ARITH: `hw`, `factor` and `stretch` are below 2^33 each; the product is formed
        // in `i128` and must fit 2^40 (16.16 → 2^24 px) to be used.
        let r = i128::from(hw)
            .wrapping_mul(i128::from(factor))
            .wrapping_mul(i128::from(stretch))
            >> 32;
        let r = i64::try_from(r)
            .ok()
            .filter(|&r| r < 1 << 40)?
            .wrapping_add(REACH_MARGIN);
        // SAFE-ARITH: `i32` edges times 2^16 and `r < 2^41`.
        let edge = |v: i32| i64::from(v).wrapping_mul(1 << 16);
        Some(Self {
            t,
            x0: edge(clip.x0).wrapping_sub(r),
            y0: edge(clip.y0).wrapping_sub(r),
            x1: edge(clip.x1).wrapping_add(r),
            y1: edge(clip.y1).wrapping_add(r),
        })
    }

    /// Whether anything inside `r` (path space) can reach the clip: the device bounds of its
    /// corners, grown by 1 px for their rounding, meet the region.
    fn hits_rect(&self, r: &FxRect) -> bool {
        let d = r.transformed_bounds(self.t);
        let (x0, y0) = (i64::from(d.x0.raw()), i64::from(d.y0.raw()));
        let (x1, y1) = (i64::from(d.x1.raw()), i64::from(d.y1.raw()));
        // SAFE-ARITH: `i32` raw values plus or minus 2^16.
        let m = 1 << 16;
        x1.wrapping_add(m) >= self.x0
            && x0.wrapping_sub(m) <= self.x1
            && y1.wrapping_add(m) >= self.y0
            && y0.wrapping_sub(m) <= self.y1
    }

    /// Whether the segment `a → b` (path space) can reach the clip: its device bounding box
    /// meets the region.
    fn hits(&self, a: FxPoint, b: FxPoint) -> bool {
        let (ax, ay) = a.transformed(self.t).raw();
        let (bx, by) = b.transformed(self.t).raw();
        ax.max(bx) >= self.x0 && ax.min(bx) <= self.x1 && ay.max(by) >= self.y0 && ay.min(by) <= self.y1
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
    // An odd pattern is repeated once. `len ≤ MAX_DASHES = 8`.
    let count = if len % 2 == 1 { len.saturating_mul(2) } else { len };
    // Entry `k` of the (repeated) pattern; `len ≥ 1` (the only caller passes a valid pattern,
    // `Dash::is_valid`), so the fallbacks are never taken.
    let val = |k: usize| i64::from(pat.get(k.checked_rem(len).unwrap_or(0)).map_or(0, |v| v.raw()));
    let total = (0..count).fold(0i64, |s, k| s.saturating_add(val(k)));
    // Lengths are sums of segment lengths (each below 2^33): they saturate rather than wrap.
    for pl in input.iter() {
        let pts = pl.points;
        let nseg = if pl.closed {
            pts.len()
        } else {
            pts.len().saturating_sub(1)
        };
        // Segment `i` (the last one of a closed polyline wraps to the first point); `i <
        // nseg ≤ pts.len()`, so `pts` is not empty and the fallbacks are never taken.
        let seg = |i: usize| {
            let p = |j: usize| {
                pts.get(j.checked_rem(pts.len()).unwrap_or(0))
                    .map_or((0, 0), |p| p.raw())
            };
            (p(i), p(i.saturating_add(1)))
        };
        let seg_len =
            |(a, b): ((i64, i64), (i64, i64))| hypot(b.0.saturating_sub(a.0), b.1.saturating_sub(a.1));
        let length = (0..nseg).fold(0i64, |s, i| s.saturating_add(seg_len(seg(i))));
        if length.checked_div(total.max(1)).unwrap_or(0) > MAX_DASHES_PER_POLYLINE {
            twine_core::warn!(target: "twine::vector", "dash pattern too fine for the path; stroking solid");
            let Some((&first, rest)) = pts.split_first() else {
                continue;
            };
            out.begin(first);
            for &p in rest {
                out.push(p);
            }
            out.end(pl.closed);
            continue;
        }
        // Advance by the offset. `total > 0` (a valid pattern): the fallback is never taken.
        let mut k = 0usize;
        let mut rem = val(0);
        let mut off = i64::from(dash.offset.raw())
            .checked_rem_euclid(total)
            .unwrap_or(0);
        let next = |k: usize| k.saturating_add(1).checked_rem(count).unwrap_or(0);
        while off > 0 {
            if off >= rem {
                // SAFE-ARITH: `off ≥ rem` here.
                off = off.wrapping_sub(rem);
                k = next(k);
                rem = val(k);
            } else {
                // SAFE-ARITH: `rem > off` here.
                rem = rem.wrapping_sub(off);
                off = 0;
            }
        }
        let mut drawing = false;
        for i in 0..nseg {
            let (a, b) = seg(i);
            let l = seg_len((a, b));
            let at = |s: i64| {
                fxp(
                    a.0.saturating_add(muldiv(b.0.saturating_sub(a.0), s, l)),
                    a.1.saturating_add(muldiv(b.1.saturating_sub(a.1), s, l)),
                )
            };
            let mut pos = 0i64;
            loop {
                // SAFE-ARITH: `0 ≤ pos ≤ l` (the loop ends at `l`) and `0 ≤ step ≤ rem`.
                let step = rem.min(l.wrapping_sub(pos));
                let on = k % 2 == 0;
                if on {
                    if !drawing {
                        out.begin(at(pos));
                        drawing = true;
                    }
                    out.push(at(pos.wrapping_add(step)));
                }
                pos = pos.wrapping_add(step);
                rem = rem.wrapping_sub(step);
                if rem == 0 {
                    if on {
                        out.end(false);
                        drawing = false;
                    }
                    k = next(k);
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

/// The polylines a stroke draws: `polys` itself, or its dash pieces (written to `dashed`)
/// when the stroke has a valid dash pattern.
pub(crate) fn dash_source<'a>(polys: &'a Polylines, s: &Stroke, dashed: &'a mut Polylines) -> &'a Polylines {
    match &s.dash {
        Some(d) if d.is_valid() => {
            dashed.clear();
            dash_polylines(polys, d, dashed);
            dashed
        }
        _ => polys,
    }
}

/// Strokes `polys` (path space) into closed device-space outlines appended to `out`.
/// `dashed` is scratch for the dash pieces; `tol` is the arc tolerance in path units. With a
/// `clip` (device pixels) the segments that cannot change a pixel in it are left out (see
/// [`Reach`]): the cost follows the geometry near the clip, not the whole path.
pub(crate) fn stroke_polylines(
    polys: &Polylines,
    s: &Stroke,
    tol: Fx,
    t: &Transform,
    clip: Option<Rect>,
    dashed: &mut Polylines,
    out: &mut Vec<Line>,
) {
    if s.width.raw() <= 0 {
        return;
    }
    let src = dash_source(polys, s, dashed);
    stroke_source(src, None, s, tol, t, clip, out);
}

/// Most segments of one [`Chunk`].
const CHUNK_SEGS: usize = 32;

/// A piece of a stroke's polylines with its path-space bounds, the unit of the coarse
/// culling of prepared strokes: either up to [`CHUNK_SEGS`] consecutive segments of one long
/// polyline, or whole short polylines (dash pieces, small sub-paths) together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    /// Polylines `first..end`.
    first: u32,
    end: u32,
    /// Segments `seg..seg_end` of polyline `first`, or `WHOLE` for whole polylines.
    seg: u32,
    seg_end: u32,
    /// Bounds of the chunk's points (path space).
    bounds: FxRect,
}

/// [`Chunk::seg_end`] of chunks of whole polylines.
const WHOLE: u32 = u32::MAX;

/// Builds the chunk index of `polys` into `out` with fallible growth. Returns `false` (`out`
/// left empty) when the memory cannot be reserved.
pub(crate) fn build_chunks(polys: &Polylines, out: &mut Vec<Chunk>) -> bool {
    out.clear();
    // At most one chunk per polyline plus one per `CHUNK_SEGS` segments.
    let bound = polys.iter().fold(0usize, |n, pl| {
        n.saturating_add(pl.points.len() / CHUNK_SEGS).saturating_add(1)
    });
    if out.try_reserve(bound).is_err() {
        return false;
    }
    let bounds_of = |pts: &mut dyn Iterator<Item = FxPoint>| {
        pts.fold(None, |r: Option<FxRect>, p| {
            Some(r.map_or(FxRect::from_points(p, p), |r| r.include(p)))
        })
    };
    // The open group of whole polylines: first polyline, bounds, segments.
    let mut group: Option<(u32, FxRect, usize)> = None;
    let flush = |group: &mut Option<(u32, FxRect, usize)>, end: u32, out: &mut Vec<Chunk>| {
        if let Some((first, bounds, _)) = group.take() {
            out.push(Chunk {
                first,
                end,
                seg: 0,
                seg_end: WHOLE,
                bounds,
            });
        }
    };
    // Polyline indices fit `u32` (`Polylines` stores them so).
    for (k, pl) in polys.iter().enumerate() {
        let k = k as u32;
        let n = pl.points.len();
        let segs = if pl.closed { n } else { n.saturating_sub(1) };
        // Point `i` modulo `n` (`n ≥ 1` wherever it is called).
        let at = |i: usize| {
            pl.points
                .get(i.checked_rem(n).unwrap_or(0))
                .copied()
                .unwrap_or_default()
        };
        if segs > CHUNK_SEGS {
            flush(&mut group, k, out);
            let mut s0 = 0;
            while s0 < segs {
                let s1 = s0.saturating_add(CHUNK_SEGS).min(segs);
                // Segments `s0..s1` use points `s0..=s1`.
                if let Some(bounds) = bounds_of(&mut (s0..=s1).map(at)) {
                    out.push(Chunk {
                        first: k,
                        end: k.saturating_add(1),
                        seg: s0 as u32,
                        seg_end: s1 as u32,
                        bounds,
                    });
                }
                s0 = s1;
            }
            continue;
        }
        let Some(b) = bounds_of(&mut pl.points.iter().copied()) else {
            continue;
        };
        let size = segs.max(1);
        if group
            .as_ref()
            .is_some_and(|g| g.2.saturating_add(size) > CHUNK_SEGS)
        {
            flush(&mut group, k, out);
        }
        group = Some(match group {
            Some((first, gb, gs)) => (first, gb.union(&b), gs.saturating_add(size)),
            None => (k, b, size),
        });
    }
    flush(&mut group, polys.len() as u32, out);
    true
}

/// Strokes the polylines `src` (already dashed) into closed device-space outlines appended to
/// `out`, like [`stroke_polylines`]. With an `index` (the chunks of `src` and scratch for their
/// hit flags) whole chunks out of reach of the `clip` are skipped without testing their
/// segments.
pub(crate) fn stroke_source(
    src: &Polylines,
    index: Option<(&[Chunk], &mut Vec<bool>)>,
    s: &Stroke,
    tol: Fx,
    t: &Transform,
    clip: Option<Rect>,
    out: &mut Vec<Line>,
) {
    if s.width.raw() <= 0 {
        return;
    }
    let st = Stroker {
        s,
        hw: (i64::from(s.width.raw()) / 2).max(1),
        tol: i64::from(tol.raw().max(1)),
    };
    let reach = clip.and_then(|c| Reach::new(s, st.hw, t, c));
    let mut o = Outline {
        t,
        out,
        first: FxPoint::ZERO,
        last: FxPoint::ZERO,
    };
    let Some(reach) = reach else {
        for pl in src.iter() {
            if !pl.points.is_empty() {
                st.polyline(&mut o, pl);
            }
        }
        return;
    };
    let Some((chunks, hit)) = index else {
        for pl in src.iter() {
            st.polyline_within(&mut o, pl, &reach, |_| true);
        }
        return;
    };
    hit.clear();
    hit.extend(chunks.iter().map(|c| reach.hits_rect(&c.bounds)));
    let hit: &[bool] = hit;
    let mut ci = 0;
    while let Some(c) = chunks.get(ci) {
        if c.seg_end == WHOLE {
            if hit.get(ci).copied().unwrap_or(true) {
                for k in c.first..c.end {
                    if let Some(pl) = src.get(k as usize) {
                        st.polyline_within(&mut o, pl, &reach, |_| true);
                    }
                }
            }
            // SAFE-ARITH: `ci < chunks.len()`.
            ci = ci.wrapping_add(1);
            continue;
        }
        // The chunks `ci..cj` of one long polyline (consecutive, `CHUNK_SEGS` segments each).
        let mut cj = ci;
        while chunks
            .get(cj)
            .is_some_and(|d| d.first == c.first && d.seg_end != WHOLE)
        {
            // SAFE-ARITH: `cj < chunks.len()`.
            cj = cj.wrapping_add(1);
        }
        let flags = hit.get(ci..cj).unwrap_or_default();
        if flags.iter().any(|&h| h)
            && let Some(pl) = src.get(c.first as usize)
        {
            st.polyline_within(&mut o, pl, &reach, |i| {
                flags.get(i / CHUNK_SEGS).copied().unwrap_or(true)
            });
        }
        ci = cj;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatten::{DEFAULT_TOLERANCE, flatten_for_stroke};
    use crate::path::Path;

    fn outline(path: &Path, s: &Stroke) -> Vec<Line> {
        outline_in(path, s, None)
    }

    fn outline_in(path: &Path, s: &Stroke, clip: Option<Rect>) -> Vec<Line> {
        let mut pl = Polylines::new();
        flatten_for_stroke(path, &Transform::IDENTITY, DEFAULT_TOLERANCE, &mut pl);
        let mut d = Polylines::new();
        let mut out = Vec::new();
        stroke_polylines(
            &pl,
            s,
            DEFAULT_TOLERANCE,
            &Transform::IDENTITY,
            clip,
            &mut d,
            &mut out,
        );
        out
    }

    #[test]
    fn chunks_split_long_polylines_and_group_short_ones() {
        let mut pl = Polylines::new();
        // A 100-segment polyline: chunks of 32, 32, 32 and 4 segments.
        pl.begin(FxPoint::from_int(0, 0));
        for i in 1..=100 {
            pl.push(FxPoint::from_int(i, i % 3));
        }
        pl.end(false);
        // 40 one-segment pieces (dashes): grouped 32 + 8.
        for i in 0..40 {
            pl.begin(FxPoint::from_int(i * 4, 50));
            pl.push(FxPoint::from_int(i * 4 + 2, 50));
            pl.end(false);
        }
        let mut c = Vec::new();
        assert!(build_chunks(&pl, &mut c));
        let shape: Vec<_> = c.iter().map(|c| (c.first, c.end, c.seg, c.seg_end)).collect();
        assert_eq!(
            shape,
            [
                (0, 1, 0, 32),
                (0, 1, 32, 64),
                (0, 1, 64, 96),
                (0, 1, 96, 100),
                (1, 33, 0, WHOLE),
                (33, 41, 0, WHOLE),
            ]
        );
        // Bounds cover each chunk's points: segments 32..64 use points 32..=64.
        assert_eq!(c[1].bounds.x0, Fx::from_int(32));
        assert_eq!(c[1].bounds.x1, Fx::from_int(64));
    }

    #[test]
    fn clip_leaves_out_the_segments_out_of_reach() {
        // A 10-segment staircase; a 4 px clip around the third step.
        let mut p = Path::new();
        p.move_to(FxPoint::from_int(0, 0));
        for i in 1..=10 {
            p.line_to(FxPoint::from_int(20 * i, 20 * (i % 2)));
        }
        let s = Stroke {
            width: Fx::from_int(2),
            ..Stroke::default()
        };
        let all = outline(&p, &s);
        let near = outline_in(&p, &s, Some(Rect::new(48, 8, 52, 12)));
        assert!(
            !near.is_empty() && near.len() * 3 < all.len(),
            "{} of {}",
            near.len(),
            all.len()
        );
        // A clip around everything leaves nothing out: the same outline.
        assert_eq!(outline_in(&p, &s, Some(Rect::new(-50, -50, 300, 100))), all);
        // A clip out of reach: nothing.
        assert_eq!(outline_in(&p, &s, Some(Rect::new(0, 60, 200, 80))), []);
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
