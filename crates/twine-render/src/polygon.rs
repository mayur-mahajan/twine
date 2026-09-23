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

impl Painter<'_> {
    /// Fills a triangle (collinear points draw nothing).
    pub fn triangle(&mut self, pts: [Point; 3], dsc: &TriangleDsc<'_>) {
        let [a, b, c] = pts;
        let cross = i64::from(b.x - a.x) * i64::from(c.y - a.y) - i64::from(b.y - a.y) * i64::from(c.x - a.x);
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
            &pts[..MAX_POLYGON_POINTS]
        } else {
            pts
        };
        let mut edges = [Edge::default(); MAX_POLYGON_POINTS];
        let mut ne = 0;
        let mut bb = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (i, &p) in pts.iter().enumerate() {
            let q = pts[(i + 1) % pts.len()];
            bb = Rect::new(bb.x0.min(p.x), bb.y0.min(p.y), bb.x1.max(p.x), bb.y1.max(p.y));
            if p.y == q.y {
                continue;
            }
            let (top, bot, dir) = if p.y < q.y { (p, q, 1) } else { (q, p, -1) };
            edges[ne] = Edge {
                x0: i64::from(top.x) << 8,
                y0: i64::from(top.y) << 8,
                x1: i64::from(bot.x) << 8,
                y1: i64::from(bot.y) << 8,
                dir,
            };
            ne += 1;
        }
        let edges = &edges[..ne];
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
        let paint = match (&sampler, &gc, map_idx) {
            (Some(s), Some(c), Some(i)) => Paint::Grad {
                s,
                map: c.map(i),
                dither: dsc.grad.is_some_and(|g| g.dither) && self.is_565(),
            },
            _ => Paint::Solid(dsc.color),
        };
        let mut sc = self.take_scratch();
        let chunk = acc.len().min(sc.cov.len()) as i32;
        let mut xs = [(0i64, 0i8); MAX_POLYGON_POINTS];
        for y in rows.y0..rows.y1 {
            let mut c0 = rows.x0;
            while c0 < rows.x1 {
                let c1 = (c0 + chunk).min(rows.x1);
                let acc = &mut acc[..(c1 - c0) as usize];
                acc.fill(0);
                let (lim0, lim1) = (i64::from(c0) << 8, i64::from(c1) << 8);
                let (mut minx, mut maxx) = (i64::MAX, i64::MIN);
                for k in 0..4 {
                    let ys = (i64::from(y) << 8) + (2 * k + 1) * 32;
                    let mut n = 0;
                    for e in edges {
                        if ys >= e.y0 && ys < e.y1 {
                            let x = e.x0 + (ys - e.y0) * (e.x1 - e.x0) / (e.y1 - e.y0);
                            // Insertion sort.
                            let mut j = n;
                            while j > 0 && xs[j - 1].0 > x {
                                xs[j] = xs[j - 1];
                                j -= 1;
                            }
                            xs[j] = (x, e.dir);
                            n += 1;
                        }
                    }
                    let mut wind = 0i32;
                    for j in 0..n.saturating_sub(1) {
                        wind += i32::from(xs[j].1);
                        let inside = match rule {
                            FillRule::NonZero => wind != 0,
                            FillRule::EvenOdd => wind & 1 != 0,
                        };
                        if !inside {
                            continue;
                        }
                        let (a, b) = (xs[j].0.max(lim0), xs[j + 1].0.min(lim1));
                        if a >= b {
                            continue;
                        }
                        minx = minx.min(a);
                        maxx = maxx.max(b);
                        add_span(acc, a - lim0, b - lim0);
                    }
                }
                if minx < maxx {
                    let (s0, s1) = ((minx >> 8) as i32, ((maxx + 255) >> 8) as i32);
                    let (s0, s1) = (s0.max(c0), s1.min(c1));
                    let accr = &*acc;
                    let mut f = |x0: i32, buf: &mut [u8]| {
                        let o = (x0 - c0) as usize;
                        for (v, &a) in buf.iter_mut().zip(&accr[o..]) {
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

/// Adds the coverage of the span `[a, b)` (24.8, relative to the accumulator start).
#[inline]
fn add_span(acc: &mut [u16], a: i64, b: i64) {
    let (pa, pb) = ((a >> 8) as usize, ((b - 1) >> 8) as usize);
    if pa == pb {
        acc[pa] += (b - a) as u16;
        return;
    }
    acc[pa] += (256 - (a & 255)) as u16;
    for v in &mut acc[pa + 1..pb] {
        *v += 256;
    }
    acc[pb] += (b - ((pb as i64) << 8)) as u16;
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
