//! Triangles and polygons: [`TriangleDsc`] / [`PolygonDsc`], [`FillRule`],
//! [`Painter::triangle`], [`Painter::polygon`], [`Painter::polygon_with_rule`].
//!
//! Vertices are continuous coordinates (pixel `(x, y)` covers `[x, x+1) × [y, y+1)`), so the
//! square `(0,0) (10,0) (10,10) (0,10)` fills exactly 10 × 10 pixels. Each pixel row is sampled
//! at 4 sub-scanlines; on each, the edge crossings (24.8 fixed point, at most
//! [`MAX_POLYGON_POINTS`] edges, kept on the stack) are sorted, the fill rule selects the
//! inside spans, and each pixel accumulates its exact horizontal coverage. The row coverage is
//! the average of the 4 sub-scanlines.

use core::mem::take;
use core::sync::atomic::AtomicBool;

use twine_core::{Color, Opa, Point, Rect};

use crate::Painter;
use crate::blend::BlendMode;
use crate::dispatch::warn_once;
use crate::gradient::{GradSampler, Gradient};
use crate::num::{advance, sat32, sat64, span};
use crate::painter::Paint;

/// Maximum number of polygon vertices; extra points are ignored (with a one-time warning).
pub const MAX_POLYGON_POINTS: usize = 64;

/// Fill parameters of triangles and polygons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriangleDsc<'a> {
    /// Color (unless `grad` is set).
    pub color: Color,
    /// Opacity.
    pub opa: Opa,
    /// Gradient over the shape's bounding box.
    pub grad: Option<&'a Gradient>,
}

impl Default for TriangleDsc<'_> {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            opa: Opa::COVER,
            grad: None,
        }
    }
}

/// Polygon fill parameters: the same fields as [`TriangleDsc`] (a type alias keeps both names).
pub type PolygonDsc<'a> = TriangleDsc<'a>;

/// Which points are inside a self-intersecting shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FillRule {
    /// Inside where the winding number is not zero.
    #[default]
    NonZero,
    /// Inside where the number of crossings is odd.
    EvenOdd,
}

#[derive(Clone, Copy, Debug, Default)]
struct Edge {
    /// Top point (24.8).
    x0: i64,
    y0: i64,
    /// Bottom point (24.8).
    x1: i64,
    y1: i64,
    /// +1 downward, −1 upward.
    dir: i8,
}

static WARN_POINTS: AtomicBool = AtomicBool::new(false);

/// The x crossing (24.8) of edge `e` at the sub-scanline `ys` (`e.y0 ≤ ys < e.y1`).
#[inline]
fn crossing(e: &Edge, ys: i64) -> i64 {
    // SAFE-ARITH: the coordinates are `i32`s in 24.8 (`< 2^40`), so the differences are below
    // 2^41; the product is formed in `i64` when it fits (the common case, the same arithmetic
    // as before) and in `i128` otherwise (`< 2^82`). The quotient is at most `|x1 − x0|`
    // (`ys − y0 < y1 − y0`), so `x0 + q` stays between `x0` and `x1`. `y1 > y0`: the division
    // fallbacks are never taken.
    let (dy, dx, h) = (
        ys.wrapping_sub(e.y0),
        e.x1.wrapping_sub(e.x0),
        e.y1.wrapping_sub(e.y0),
    );
    let q = match dy.checked_mul(dx) {
        Some(n) => n.checked_div(h).unwrap_or(0),
        None => sat64(
            i128::from(dy)
                .wrapping_mul(i128::from(dx))
                .checked_div(i128::from(h))
                .unwrap_or(0),
        ),
    };
    e.x0.wrapping_add(q)
}

impl Painter<'_> {
    /// Fills a triangle (collinear points draw nothing).
    pub fn triangle(&mut self, pts: [Point; 3], dsc: &TriangleDsc<'_>) {
        let [a, b, c] = pts;
        // SAFE-ARITH: differences of `i32`s are below 2^32, products below 2^64, in `i128`.
        let d = |p: i32, q: i32| i128::from(p).wrapping_sub(i128::from(q));
        let cross = d(b.x, a.x)
            .wrapping_mul(d(c.y, a.y))
            .wrapping_sub(d(b.y, a.y).wrapping_mul(d(c.x, a.x)));
        if cross == 0 {
            return;
        }
        self.polygon(&pts, dsc);
    }

    /// Fills a polygon with the non-zero rule.
    pub fn polygon(&mut self, pts: &[Point], dsc: &PolygonDsc<'_>) {
        self.polygon_with_rule(pts, FillRule::NonZero, dsc);
    }

    /// Fills a polygon with the given fill rule.
    pub fn polygon_with_rule(&mut self, pts: &[Point], rule: FillRule, dsc: &PolygonDsc<'_>) {
        if dsc.opa.is_transparent() || pts.len() < 3 {
            return;
        }
        let pts = if pts.len() > MAX_POLYGON_POINTS {
            if warn_once(&WARN_POINTS) {
                twine_core::warn!(
                    target: "twine::render",
                    "polygon with {} points; only the first {} are drawn",
                    pts.len(),
                    MAX_POLYGON_POINTS
                );
            }
            pts.get(..MAX_POLYGON_POINTS).unwrap_or(pts)
        } else {
            pts
        };
        let mut edges = [Edge::default(); MAX_POLYGON_POINTS];
        let mut ne = 0;
        let mut bb = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        // Each point with its successor (the last one with the first).
        for (&p, &q) in pts.iter().zip(pts.iter().cycle().skip(1)) {
            bb = Rect::new(bb.x0.min(p.x), bb.y0.min(p.y), bb.x1.max(p.x), bb.y1.max(p.y));
            if p.y == q.y {
                continue;
            }
            let (top, bot, dir) = if p.y < q.y { (p, q, 1) } else { (q, p, -1) };
            // SAFE-INDEX: at most `pts.len() ≤ MAX_POLYGON_POINTS` edges; the fallback is never
            // taken.
            if let Some(slot) = edges.get_mut(ne) {
                *slot = Edge {
                    x0: i64::from(top.x) << 8,
                    y0: i64::from(top.y) << 8,
                    x1: i64::from(bot.x) << 8,
                    y1: i64::from(bot.y) << 8,
                    dir,
                };
                // SAFE-ARITH: `ne < MAX_POLYGON_POINTS` (the slot exists).
                ne = ne.wrapping_add(1);
            }
        }
        let edges = edges.get(..ne).unwrap_or_default();
        if bb.is_empty() {
            return;
        }
        let Some(rows) = bb.intersection(&self.clip()) else {
            return;
        };
        twine_core::trace!(target: "twine::render", "polygon {} points, bbox {}", pts.len(), bb);
        let mut acc = take(&mut self.caches_mut().acc);
        let mut gc = dsc.grad.map(|_| take(&mut self.caches_mut().gradient));
        let sampler = dsc.grad.map(|g| GradSampler::new(g, bb));
        let map_idx = match (&mut gc, dsc.grad) {
            (Some(c), Some(g)) => Some(c.lookup(g)),
            _ => None,
        };
        // `lookup` returns a cached index: the map is always there.
        let map = gc.as_ref().zip(map_idx).and_then(|(c, i)| c.map(i));
        let paint = match (&sampler, map) {
            (Some(s), Some(map)) => Paint::Grad {
                s,
                map,
                dither: dsc.grad.is_some_and(|g| g.dither) && self.is_565(),
            },
            _ => Paint::Solid(dsc.color),
        };
        let mut sc = self.take_scratch();
        let chunk = acc.len().min(sc.cov.len()).max(1);
        let mut xs = [(0i64, 0i8); MAX_POLYGON_POINTS];
        for y in rows.y0..rows.y1 {
            let mut c0 = rows.x0;
            while c0 < rows.x1 {
                let c1 = advance(c0, chunk, rows.x1);
                // SAFE-INDEX: `c1 − c0 ≤ chunk ≤ acc.len()`; the fallback is never taken.
                let Some(acc) = acc.get_mut(..span(c0, c1)) else {
                    break;
                };
                acc.fill(0);
                let (lim0, lim1) = (i64::from(c0) << 8, i64::from(c1) << 8);
                let (mut minx, mut maxx) = (i64::MAX, i64::MIN);
                for k in 0..4i64 {
                    // SAFE-ARITH: `|y · 256| < 2^39` and `(2k + 1) · 32 ≤ 224`.
                    let ys = (i64::from(y) << 8).wrapping_add(k.wrapping_mul(64).wrapping_add(32));
                    let mut n = 0;
                    for e in edges {
                        if ys >= e.y0 && ys < e.y1 {
                            // SAFE-INDEX: at most one crossing per edge, `≤ MAX_POLYGON_POINTS`;
                            // the fallback is never taken.
                            if let Some(slot) = xs.get_mut(n) {
                                *slot = (crossing(e, ys), e.dir);
                                // SAFE-ARITH: `n < MAX_POLYGON_POINTS` (the slot exists).
                                n = n.wrapping_add(1);
                            }
                        }
                    }
                    let crossings = xs.get_mut(..n).unwrap_or_default();
                    // Crossings with equal x only bound empty spans, whose order does not
                    // matter: an unstable sort gives the same spans.
                    crossings.sort_unstable_by_key(|c| c.0);
                    let mut wind = 0i32;
                    for pair in crossings.windows(2) {
                        let &[(xa, dir), (xb, _)] = pair else { continue };
                        // SAFE-ARITH: `|wind| ≤ MAX_POLYGON_POINTS`.
                        wind = wind.wrapping_add(i32::from(dir));
                        let inside = match rule {
                            FillRule::NonZero => wind != 0,
                            FillRule::EvenOdd => wind & 1 != 0,
                        };
                        if !inside {
                            continue;
                        }
                        let (a, b) = (xa.max(lim0), xb.min(lim1));
                        if a >= b {
                            continue;
                        }
                        minx = minx.min(a);
                        maxx = maxx.max(b);
                        // SAFE-ARITH: `lim0 ≤ a < b ≤ lim1` (`< 2^40` in magnitude).
                        add_span(acc, a.wrapping_sub(lim0), b.wrapping_sub(lim0));
                    }
                }
                if minx < maxx {
                    // `lim0 ≤ minx < maxx ≤ lim1`: both pixel bounds fit in `i32`.
                    // SAFE-ARITH: `maxx + 255 < 2^40`.
                    let (s0, s1) = (sat32(minx >> 8), sat32(maxx.wrapping_add(255) >> 8));
                    let (s0, s1) = (s0.max(c0), s1.min(c1));
                    let accr = &*acc;
                    let mut f = |x0: i32, buf: &mut [u8]| {
                        let cells = accr.get(span(c0, x0)..).unwrap_or_default();
                        for (v, &a) in buf.iter_mut().zip(cells) {
                            *v = (a / 4).min(255) as u8;
                        }
                    };
                    self.span(
                        &mut sc,
                        y,
                        s0,
                        s1,
                        &paint,
                        dsc.opa,
                        BlendMode::Normal,
                        Some(&mut f),
                    );
                }
                c0 = c1;
            }
        }
        self.put_scratch(sc);
        self.caches_mut().acc = acc;
        if let Some(c) = gc {
            self.caches_mut().gradient = c;
        }
    }
}

/// Adds the coverage of the span `[a, b)` (24.8, relative to the accumulator start;
/// `0 ≤ a < b ≤ 256 · acc.len()`). Each cell gains at most 256 per call; per pixel row the
/// polygon adds at most `4 · 256` to a cell (disjoint spans on 4 sub-scanlines).
#[inline]
fn add_span(acc: &mut [u16], a: i64, b: i64) {
    // SAFE-ARITH: `0 ≤ a < b`, so `b − 1 ≥ 0`; the cell indices are below `acc.len()`.
    let (pa, pb) = ((a >> 8) as usize, (b.wrapping_sub(1) >> 8) as usize);
    // SAFE-INDEX: `pa ≤ pb < acc.len()`; the fallback is never taken.
    let Some(cells) = acc.get_mut(pa..=pb) else {
        return;
    };
    // SAFE-ARITH: every value added is at most 256 and a cell receives at most 1024 per row
    // (see above), far below `u16::MAX`; `b − a ≤ 256` for a single cell.
    match cells {
        [only] => *only = only.wrapping_add(b.wrapping_sub(a) as u16),
        [first, middle @ .., last] => {
            *first = first.wrapping_add(256u16.wrapping_sub((a & 255) as u16));
            for v in middle {
                *v = v.wrapping_add(256);
            }
            // `b − 256·pb ∈ 1..=256`.
            *last = last.wrapping_add(b.wrapping_sub((pb as i64) << 8) as u16);
        }
        [] => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_span_exact() {
        let mut acc = [0u16; 4];
        add_span(&mut acc, 128, 3 * 256 + 64);
        assert_eq!(acc, [128, 256, 256, 64]);
        let mut acc = [0u16; 2];
        add_span(&mut acc, 10, 20);
        assert_eq!(acc, [10, 0]);
    }
}
