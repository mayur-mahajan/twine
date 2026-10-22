//! The anti-aliasing rasterizer: device-space lines → coverage rows → [`Painter::coverage_span`].
//!
//! Two paths, chosen by the fill rule:
//!
//! - [`FillRule::NonZero`]: **sparse signed-area accumulation** (the font-rs /
//!   `stb_truetype` v2 idea) in integers. Coordinates are 24.8 fixed point. Every edge crossing a
//!   pixel row adds, per cell it passes, its signed height times the covered fraction of the
//!   cell (`acc[c]`) and the rest to the next cell (`acc[c + 1]`); a prefix sum over the row
//!   then gives the signed area covered left of each pixel, and `min(|sum|, 1)` is the
//!   coverage. The accumulator is one row of `i32` sized to the clip width + 2.
//! - [`FillRule::EvenOdd`]: a **scanline** variant (the accumulation cannot see winding
//!   parity): each pixel row is sampled at 4 sub-scanlines; on each, the edge crossings are
//!   sorted, odd-parity spans add their exact horizontal coverage (1/256 px) into a `u16` row
//!   accumulator, and the coverage is the average of the 4 sub-scanlines.
//!
//! Only rows inside the painter's clip are processed, and each row's result does not depend
//! on which rows are processed, so rendering in chunks is pixel-identical to one pass.
//! Horizontal clipping is exact: the part of an edge left of the clip is accumulated as a
//! vertical edge on the clip's left border (it covers the whole row to its right), parts right
//! of the clip are dropped.

use alloc::vec::Vec;

use twine_render::{BlendMode, FillRule, Painter, SpanSource};

use crate::flatten::Line;
use crate::geom::sat;

/// One row = 256 sub-pixel units (24.8 fixed point).
const ONE: i64 = 256;

// Range argument of this module (SAFE-ARITH for the arithmetic below unless a line says
// otherwise): edges come from 16.16 `Fx` coordinates rounded to 24.8 (`to8`), so every edge
// coordinate is within `±2^23` (24.8, i.e. ±32_768 px). The processed columns lie between the
// edges' extremes (`xs ≥ minx / 256`, `xe ≤ maxx / 256 + 1`), so the row width `w ≤ 2^16 + 2`
// and every row-local x is within `±2^25`; sub-scanline y values are within `±2^24`.

/// A non-horizontal edge in 24.8 fixed point, oriented top → bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Edge {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    /// +1 when the original line pointed down, −1 when up.
    dir: i32,
}

impl Edge {
    /// x at `y` (24.8), floor division (identical for every row that asks).
    #[inline]
    fn x_at(&self, y: i64) -> i64 {
        let (x0, y0, x1, y1) = (
            i64::from(self.x0),
            i64::from(self.y0),
            i64::from(self.x1),
            i64::from(self.y1),
        );
        // `|y − y0|, |x1 − x0| ≤ 2^25` (module range argument): the product is below 2^50.
        // Edges are non-horizontal and oriented top → bottom (`y0 < y1`, horizontal lines are
        // skipped when the edges are built): the division's fallback is never taken.
        let q = y
            .wrapping_sub(y0)
            .wrapping_mul(x1.wrapping_sub(x0))
            .checked_div_euclid(y1.wrapping_sub(y0))
            .unwrap_or(0);
        x0.wrapping_add(q)
    }
}

/// Reusable scratch memory of the rasterizer (part of [`VectorCaches`](crate::VectorCaches)).
#[derive(Clone, Debug, Default)]
pub(crate) struct RasterScratch {
    edges: Vec<Edge>,
    active: Vec<u32>,
    acc: Vec<i32>,
    acc16: Vec<u16>,
    cov: Vec<u8>,
    cross: Vec<(i32, i32)>,
}

impl RasterScratch {
    pub(crate) fn bytes_reserved(&self) -> usize {
        [
            self.edges.capacity().saturating_mul(core::mem::size_of::<Edge>()),
            self.active.capacity().saturating_mul(4),
            self.acc.capacity().saturating_mul(4),
            self.acc16.capacity().saturating_mul(2),
            self.cov.capacity(),
            self.cross.capacity().saturating_mul(8),
        ]
        .iter()
        .fold(0usize, |s, &v| s.saturating_add(v))
    }

    /// Reserves room for `n` edges (edge list, active list, crossings).
    pub(crate) fn reserve_edges(&mut self, n: usize) {
        self.edges.reserve(n);
        self.active.reserve(n);
        self.cross.reserve(n);
    }

    /// Grows the row buffers to at least `w` pixels (warm-up).
    pub(crate) fn reserve_width(&mut self, w: usize) {
        let n = w.saturating_add(2);
        if self.acc.len() < n {
            self.acc.resize(n, 0);
            self.acc16.resize(n, 0);
            self.cov.resize(n, 0);
        }
    }
}

/// Converts a 16.16 coordinate to 24.8 (rounded).
#[inline]
fn to8(v: twine_core::Fx) -> i32 {
    // An `i32` plus 128 fits in `i64`; the result is within `±2^23`.
    sat(i64::from(v.raw()).wrapping_add(128) >> 8)
}

/// Rasterizes the closed polygon(s) given as `lines` into `p` with `rule`, painting with `src`.
pub(crate) fn fill_lines(
    p: &mut Painter<'_>,
    r: &mut RasterScratch,
    lines: &[Line],
    rule: FillRule,
    src: &SpanSource<'_>,
    mode: BlendMode,
) {
    let clip = p.clip();
    if clip.is_empty() || lines.is_empty() {
        return;
    }
    r.edges.clear();
    let (mut minx, mut miny, mut maxx, mut maxy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for l in lines {
        let (ax, ay, bx, by) = (to8(l.p0.x), to8(l.p0.y), to8(l.p1.x), to8(l.p1.y));
        if ay == by {
            continue;
        }
        let e = if ay < by {
            Edge {
                x0: ax,
                y0: ay,
                x1: bx,
                y1: by,
                dir: 1,
            }
        } else {
            Edge {
                x0: bx,
                y0: by,
                x1: ax,
                y1: ay,
                dir: -1,
            }
        };
        minx = minx.min(ax.min(bx));
        maxx = maxx.max(ax.max(bx));
        miny = miny.min(e.y0);
        maxy = maxy.max(e.y1);
        r.edges.push(e);
    }
    if r.edges.is_empty() {
        return;
    }
    // `maxx`, `maxy` are within `±2^23` (24.8): the rounding sums fit.
    let row0 = clip.y0.max(miny >> 8);
    let row1 = clip.y1.min(sat(i64::from(maxy).wrapping_add(ONE - 1) >> 8));
    let xs = clip.x0.max(minx >> 8);
    let xe = clip.x1.min(sat(i64::from(maxx).wrapping_add(ONE - 1) >> 8));
    if row0 >= row1 || xs >= xe {
        return;
    }
    let w = crate::geom::span(xs, xe);
    r.reserve_width(w);
    r.edges.sort_unstable_by_key(|e| e.y0);
    r.active.clear();
    // `|xs| ≤ 2^15 + 1` and `w ≤ 2^16 + 2`: products with 256 are below 2^25.
    let x_origin = i64::from(xs).wrapping_mul(ONE);
    let wlim = (w as i64).wrapping_mul(ONE);
    let mut next = 0usize;
    // Skip edges that end above the first row.
    let top = i64::from(row0).wrapping_mul(ONE);
    for y in row0..row1 {
        // `row0 ≤ y < row1 ≤ 2^15 + 1`.
        let ry0 = i64::from(y).wrapping_mul(ONE);
        let ry1 = ry0.wrapping_add(ONE);
        while let Some(e) = r.edges.get(next).filter(|e| i64::from(e.y0) < ry1) {
            if i64::from(e.y1) > top {
                // `next < edges.len() ≤ u32::MAX` for any path that fits memory… the index is
                // kept as `u32` (half the memory); a larger index is skipped.
                if let Ok(i) = u32::try_from(next) {
                    r.active.push(i);
                }
            }
            // `next < edges.len()` (the edge exists).
            next = next.wrapping_add(1);
        }
        let edges = &r.edges;
        r.active
            .retain(|&i| edges.get(i as usize).is_some_and(|e| i64::from(e.y1) > ry0));
        if r.active.is_empty() {
            if next >= r.edges.len() {
                break;
            }
            continue;
        }
        let span = match rule {
            FillRule::NonZero => accumulate_row(r, ry0, x_origin, wlim, w),
            FillRule::EvenOdd => scan_row_even_odd(r, ry0, x_origin, wlim, w),
        };
        if let Some((a, b)) = span {
            // `a < b ≤ w`: the start column `xs + a` lies inside the clip.
            let x = xs.saturating_add(i32::try_from(a).unwrap_or(i32::MAX));
            p.coverage_span_mode(y, x, r.cov.get(a..b).unwrap_or_default(), src, mode);
        }
    }
}

/// Non-zero accumulation of one row. Returns the covered cell range `[a, b)` with the coverage
/// in `r.cov`.
fn accumulate_row(
    r: &mut RasterScratch,
    ry0: i64,
    x_origin: i64,
    wlim: i64,
    w: usize,
) -> Option<(usize, usize)> {
    let ry1 = ry0.wrapping_add(ONE);
    let (mut minc, mut maxc) = (usize::MAX, 0usize);
    // `reserve_width(w)` sized the row buffers: the `?`s are never taken.
    let acc = r.acc.get_mut(..w.saturating_add(2))?;
    for &i in &r.active {
        let Some(e) = r.edges.get(i as usize) else {
            continue;
        };
        let top = i64::from(e.y0).max(ry0);
        let bot = i64::from(e.y1).min(ry1);
        if top >= bot {
            continue;
        }
        let xa = e.x_at(top).wrapping_sub(x_origin);
        let xb = e.x_at(bot).wrapping_sub(x_origin);
        let (ya, yb) = (top.wrapping_sub(ry0), bot.wrapping_sub(ry0));
        let dir = i64::from(e.dir);
        let (lo, hi) = add_segment(acc, xa, ya, xb, yb, dir, wlim);
        if lo <= hi {
            minc = minc.min(lo);
            maxc = maxc.max(hi);
        }
    }
    if minc == usize::MAX {
        return None;
    }
    // Prefix sum → coverage; continue past the last touched cell while the sum is not zero
    // (the shape extends beyond the clip's right edge).
    // `|s| ≤ (w + 2) · 2^31 < 2^49` (`w ≤ 2^16 + 2`, cells are `i32`s).
    let mut s: i64 = 0;
    let mut end = minc;
    let mut c = minc;
    let cells = acc.get_mut(minc..w).unwrap_or_default();
    let covs = r.cov.get_mut(minc..w).unwrap_or_default();
    for ((cell, out), k) in cells.iter_mut().zip(covs.iter_mut()).zip(minc..w) {
        s = s.wrapping_add(i64::from(*cell));
        *cell = 0;
        let v = s.unsigned_abs().min(65_536);
        // `v ≤ 2^16`: `v · 255 + 2^15 < 2^24`.
        *out = (v.wrapping_mul(255).wrapping_add(32_768) >> 16) as u8;
        // `k < w`.
        c = k.wrapping_add(1);
        if *out != 0 {
            end = c;
        }
        if c > maxc && s == 0 {
            break;
        }
    }
    // Clear the rest of the touched range (cells before `c` were cleared by the prefix sum).
    if c <= maxc {
        if let Some(rest) = acc.get_mut(c..=maxc.min(w.saturating_add(1))) {
            rest.fill(0);
        }
    }
    (end > minc).then_some((minc, end))
}

/// Accumulates the row-local segment `(xa, ya) → (xb, yb)` (x relative to the clip origin,
/// within `±2^25`; y in `0..=256`, `ya < yb`) with winding `dir`. Returns the touched cell
/// range.
fn add_segment(acc: &mut [i32], xa: i64, ya: i64, xb: i64, yb: i64, dir: i64, wlim: i64) -> (usize, usize) {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    // A cell receives `±256 · 256` per edge; summed over many overlapping edges it saturates
    // (only reached by more than 2^15 edges crossing one cell in one row).
    let mut cell = |acc: &mut [i32], x0: i64, x1: i64, dy: i64| {
        // Piece fully inside one cell column (or left of the clip / right of it).
        // `|dy| ≤ 256`, `|dir| = 1`: `|d| ≤ 256` and `d · 256 ≤ 2^16`.
        let d = dy.wrapping_mul(dir);
        if d == 0 {
            return;
        }
        if x1 <= 0 && x0 <= 0 {
            if let Some(first) = acc.first_mut() {
                *first = first.saturating_add(d.wrapping_mul(ONE) as i32);
            }
            lo = 0;
            return;
        }
        if x0 >= wlim && x1 >= wlim {
            return;
        }
        // `0 ≤ min(x0, x1) < wlim`, so `c < w` and `c + 1 ≤ w < acc.len()`.
        let c = (x0.min(x1).max(0) >> 8) as usize;
        // 2 · mean x within the cell, `0..=512`.
        let mx2 = x0.wrapping_add(x1).wrapping_sub((c as i64).wrapping_mul(2 * ONE));
        let part = d.wrapping_mul((2 * ONE).wrapping_sub(mx2)) / 2;
        // SAFE-INDEX: `c + 1 < acc.len()` (above); the fallback is never taken.
        if let Some([a, b]) = acc.get_mut(c..).and_then(<[i32]>::first_chunk_mut::<2>) {
            *a = a.saturating_add(part as i32);
            *b = b.saturating_add(d.wrapping_mul(ONE).wrapping_sub(part) as i32);
        }
        lo = lo.min(c);
        hi = hi.max(c.wrapping_add(1));
    };
    if xa == xb {
        cell(acc, xa, xa, yb.wrapping_sub(ya));
        return (lo, hi);
    }
    // Walk left → right through the cell boundaries (and the clip's left border at 0).
    let (xl, yl, xr, yr) = if xa < xb {
        (xa, ya, xb, yb)
    } else {
        (xb, yb, xa, ya)
    };
    // `xa != xb` (returned above), so `xl < xr`: the division's fallback is never taken.
    let y_at = |x: i64| -> i64 {
        let q = x
            .wrapping_sub(xl)
            .wrapping_mul(yr.wrapping_sub(yl))
            .checked_div_euclid(xr.wrapping_sub(xl))
            .unwrap_or(0);
        yl.wrapping_add(q)
    };
    let mut x = xl;
    let mut y = yl;
    while x < xr {
        let nx = if x < 0 {
            0.min(xr)
        } else if x >= wlim {
            xr
        } else {
            ((x >> 8).wrapping_add(1)).saturating_mul(ONE).min(xr)
        };
        let ny = if nx == xr { yr } else { y_at(nx) };
        cell(acc, x, nx, ny.wrapping_sub(y).abs());
        x = nx;
        y = ny;
    }
    (lo, hi)
}

/// Even-odd scanline coverage of one row (4 sub-scanlines). Returns the covered range.
fn scan_row_even_odd(
    r: &mut RasterScratch,
    ry0: i64,
    x_origin: i64,
    wlim: i64,
    w: usize,
) -> Option<(usize, usize)> {
    // `reserve_width(w)` sized the row buffers: the `?`s are never taken.
    let acc = r.acc16.get_mut(..w.saturating_add(2))?;
    let (mut minx, mut maxx) = (i64::MAX, i64::MIN);
    for k in 0..4i64 {
        let ys = ry0.wrapping_add(32).wrapping_add(k.wrapping_mul(64));
        r.cross.clear();
        for &i in &r.active {
            let Some(e) = r.edges.get(i as usize) else {
                continue;
            };
            if ys >= i64::from(e.y0) && ys < i64::from(e.y1) {
                r.cross.push((sat(e.x_at(ys).wrapping_sub(x_origin)), e.dir));
            }
        }
        r.cross.sort_unstable_by_key(|c| c.0);
        let mut parity = false;
        for pair in r.cross.windows(2) {
            let &[(xa, _), (xb, _)] = pair else { continue };
            parity = !parity;
            if !parity {
                continue;
            }
            let a = i64::from(xa).max(0);
            let b = i64::from(xb).min(wlim);
            if a >= b {
                continue;
            }
            minx = minx.min(a);
            maxx = maxx.max(b);
            add_span(acc, a, b);
        }
    }
    if minx >= maxx {
        return None;
    }
    // `0 ≤ minx < maxx ≤ wlim < 2^25`.
    let (a, b) = (
        (minx >> 8) as usize,
        ((maxx.wrapping_add(ONE - 1) >> 8) as usize).min(w),
    );
    let covs = r.cov.get_mut(a..b).unwrap_or_default();
    for (v, c) in covs.iter_mut().zip(acc.get_mut(a..b).unwrap_or_default()) {
        *v = (*c / 4).min(255) as u8;
        *c = 0;
    }
    Some((a, b))
}

/// Adds the coverage of `[a, b)` (24.8, relative to the row start; `0 ≤ a < b ≤ 256 ·
/// acc.len()`) to `acc`. The spans of one sub-scanline are disjoint, so a cell gains at most
/// `4 · 256` per row.
#[inline]
fn add_span(acc: &mut [u16], a: i64, b: i64) {
    // `0 ≤ a < b`, so `b − 1 ≥ 0`; the cell indices are below `acc.len()`.
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
            *first = first.wrapping_add((ONE as u16).wrapping_sub((a & 255) as u16));
            for v in middle {
                *v = v.wrapping_add(ONE as u16);
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

    fn row(segs: &[(i64, i64, i64, i64, i64)], w: usize) -> Vec<i64> {
        use alloc::vec;
        let mut acc = vec![0i32; w + 2];
        for &(xa, ya, xb, yb, dir) in segs {
            add_segment(&mut acc, xa, ya, xb, yb, dir, w as i64 * ONE);
        }
        let mut s = 0i64;
        acc[..w]
            .iter()
            .map(|&v| {
                s += i64::from(v);
                s
            })
            .collect()
    }

    #[test]
    fn vertical_edges_cover_exactly() {
        // Rect from x = 1.5 to x = 3 over the whole row.
        let r = row(&[(384, 0, 384, 256, 1), (768, 0, 768, 256, -1)], 5);
        assert_eq!(r, [0, 32_768, 65_536, 0, 0]);
    }

    #[test]
    fn diagonal_area_is_exact() {
        // Edge from (0, 0) to (2, 1) px, winding up: triangle area right of it.
        let r = row(&[(0, 0, 512, 256, 1), (768, 0, 768, 256, -1)], 4);
        // Region x ∈ [2y, 3): cell 0 covers ∫(1 − 2y) over y ∈ [0, ½] = ¼, cell 1 covers ¾.
        assert_eq!(r, [16_384, 49_152, 65_536, 0]);
    }

    #[test]
    fn left_of_clip_is_a_border_edge() {
        // An edge entirely left of the clip covers the full row to its right.
        let r = row(&[(-1000, 0, -600, 256, 1), (512, 0, 512, 256, -1)], 3);
        assert_eq!(r, [65_536, 65_536, 0]);
        // Crossing the border at x = 0 mid-row.
        let r = row(&[(-256, 0, 256, 256, 1), (512, 0, 512, 256, -1)], 3);
        assert_eq!(r[0], 49_152);
    }
}
