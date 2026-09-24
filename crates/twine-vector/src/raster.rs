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
        x0 + ((y - y0) * (x1 - x0)).div_euclid(y1 - y0)
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
        self.edges.capacity() * core::mem::size_of::<Edge>()
            + self.active.capacity() * 4
            + self.acc.capacity() * 4
            + self.acc16.capacity() * 2
            + self.cov.capacity()
            + self.cross.capacity() * 8
    }

    /// Reserves room for `n` edges (edge list, active list, crossings).
    pub(crate) fn reserve_edges(&mut self, n: usize) {
        self.edges.reserve(n);
        self.active.reserve(n);
        self.cross.reserve(n);
    }

    /// Grows the row buffers to at least `w` pixels (warm-up).
    pub(crate) fn reserve_width(&mut self, w: usize) {
        if self.acc.len() < w + 2 {
            self.acc.resize(w + 2, 0);
            self.acc16.resize(w + 2, 0);
            self.cov.resize(w + 2, 0);
        }
    }
}

/// Converts a 16.16 coordinate to 24.8 (rounded).
#[inline]
fn to8(v: twine_core::Fx) -> i32 {
    sat((i64::from(v.0) + 128) >> 8)
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
    let row0 = clip.y0.max(miny >> 8);
    let row1 = clip.y1.min(sat((i64::from(maxy) + ONE - 1) >> 8));
    let xs = clip.x0.max(minx >> 8);
    let xe = clip.x1.min(sat((i64::from(maxx) + ONE - 1) >> 8));
    if row0 >= row1 || xs >= xe {
        return;
    }
    let w = (xe - xs) as usize;
    r.reserve_width(w);
    r.edges.sort_unstable_by_key(|e| e.y0);
    r.active.clear();
    let x_origin = i64::from(xs) * ONE;
    let wlim = w as i64 * ONE;
    let mut next = 0usize;
    // Skip edges that end above the first row.
    let top = i64::from(row0) * ONE;
    for y in row0..row1 {
        let ry0 = i64::from(y) * ONE;
        let ry1 = ry0 + ONE;
        while next < r.edges.len() && i64::from(r.edges[next].y0) < ry1 {
            if i64::from(r.edges[next].y1) > top {
                r.active.push(next as u32);
            }
            next += 1;
        }
        let edges = &r.edges;
        r.active.retain(|&i| i64::from(edges[i as usize].y1) > ry0);
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
            p.coverage_span_mode(y, xs + a as i32, &r.cov[a..b], src, mode);
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
    let ry1 = ry0 + ONE;
    let (mut minc, mut maxc) = (usize::MAX, 0usize);
    let acc = &mut r.acc[..w + 2];
    for &i in &r.active {
        let e = r.edges[i as usize];
        let top = i64::from(e.y0).max(ry0);
        let bot = i64::from(e.y1).min(ry1);
        if top >= bot {
            continue;
        }
        let xa = e.x_at(top) - x_origin;
        let xb = e.x_at(bot) - x_origin;
        let (ya, yb) = (top - ry0, bot - ry0);
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
    let mut s: i64 = 0;
    let mut end = minc;
    let mut c = minc;
    while c < w {
        s += i64::from(acc[c]);
        acc[c] = 0;
        let v = s.unsigned_abs().min(65_536);
        r.cov[c] = ((v * 255 + 32_768) >> 16) as u8;
        c += 1;
        if r.cov[c - 1] != 0 {
            end = c;
        }
        if c > maxc && s == 0 {
            break;
        }
    }
    // Clear the rest of the touched range (cells before `c` were cleared by the prefix sum).
    if c <= maxc {
        acc[c..=maxc.min(w + 1)].fill(0);
    }
    (end > minc).then_some((minc, end))
}

/// Accumulates the row-local segment `(xa, ya) → (xb, yb)` (x relative to the clip origin,
/// y in `0..=256`, `ya < yb`) with winding `dir`. Returns the touched cell range.
fn add_segment(acc: &mut [i32], xa: i64, ya: i64, xb: i64, yb: i64, dir: i64, wlim: i64) -> (usize, usize) {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    let mut cell = |acc: &mut [i32], x0: i64, x1: i64, dy: i64| {
        // Piece fully inside one cell column (or left of the clip / right of it).
        let d = dy * dir;
        if d == 0 {
            return;
        }
        if x1 <= 0 && x0 <= 0 {
            acc[0] += (d * ONE) as i32;
            lo = 0;
            return;
        }
        if x0 >= wlim && x1 >= wlim {
            return;
        }
        let c = (x0.min(x1).max(0) >> 8) as usize;
        let mx2 = x0 + x1 - 2 * (c as i64) * ONE; // 2 · mean x within the cell, 0..=512
        let part = d * (2 * ONE - mx2) / 2;
        acc[c] += part as i32;
        acc[c + 1] += (d * ONE - part) as i32;
        lo = lo.min(c);
        hi = hi.max(c + 1);
    };
    if xa == xb {
        cell(acc, xa, xa, yb - ya);
        return (lo, hi);
    }
    // Walk left → right through the cell boundaries (and the clip's left border at 0).
    let (xl, yl, xr, yr) = if xa < xb {
        (xa, ya, xb, yb)
    } else {
        (xb, yb, xa, ya)
    };
    let y_at = |x: i64| -> i64 { yl + ((x - xl) * (yr - yl)).div_euclid(xr - xl) };
    let mut x = xl;
    let mut y = yl;
    while x < xr {
        let nx = if x < 0 {
            0.min(xr)
        } else if x >= wlim {
            xr
        } else {
            ((x >> 8) + 1).saturating_mul(ONE).min(xr)
        };
        let ny = if nx == xr { yr } else { y_at(nx) };
        cell(acc, x, nx, (ny - y).abs());
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
    let acc = &mut r.acc16[..w + 2];
    let (mut minx, mut maxx) = (i64::MAX, i64::MIN);
    for k in 0..4 {
        let ys = ry0 + 32 + 64 * k;
        r.cross.clear();
        for &i in &r.active {
            let e = r.edges[i as usize];
            if ys >= i64::from(e.y0) && ys < i64::from(e.y1) {
                r.cross.push((sat(e.x_at(ys) - x_origin), e.dir));
            }
        }
        r.cross.sort_unstable_by_key(|c| c.0);
        let mut parity = false;
        for j in 0..r.cross.len().saturating_sub(1) {
            parity = !parity;
            if !parity {
                continue;
            }
            let a = i64::from(r.cross[j].0).max(0);
            let b = i64::from(r.cross[j + 1].0).min(wlim);
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
    let (a, b) = ((minx >> 8) as usize, (((maxx + ONE - 1) >> 8) as usize).min(w));
    for (v, a) in r.cov[a..b].iter_mut().zip(&mut acc[a..b]) {
        *v = (*a / 4).min(255) as u8;
        *a = 0;
    }
    Some((a, b))
}

/// Adds the coverage of `[a, b)` (24.8, relative to the row start) to `acc`.
#[inline]
fn add_span(acc: &mut [u16], a: i64, b: i64) {
    let (pa, pb) = ((a >> 8) as usize, ((b - 1) >> 8) as usize);
    if pa == pb {
        acc[pa] += (b - a) as u16;
        return;
    }
    acc[pa] += (ONE - (a & 255)) as u16;
    for v in &mut acc[pa + 1..pb] {
        *v += ONE as u16;
    }
    acc[pb] += (b - ((pb as i64) << 8)) as u16;
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
