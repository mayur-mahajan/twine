//! Lines: [`LineDsc`], [`Painter::line`] and [`Painter::polyline`].
//!
//! Points are pixel coordinates; the line runs between the pixel centers. Horizontal and
//! vertical lines without round caps are filled rectangles (the width extends `(w − 1) / 2`
//! pixels to the top/left and `w / 2` to the bottom/right, like LVGL). Other lines use
//! distance-based anti-aliasing in fixed point: for each row only the x-interval where the band
//! `|d| ≤ w/2 + 1` meets the row is visited; a pixel's coverage is
//! `clamp(w/2 + ½ − |d|, 0, 1)` times the end factor. Butt ends cover the end pixels fully
//! (the line reaches half a pixel past each end point, like the fast path); round ends use the
//! distance to the end point.

use core::num::NonZeroI128;

use twine_core::math::{div_rem_i128, isqrt64};
use twine_core::{Color, Opa, Point, Rect};

use crate::Painter;
use crate::blend::BlendMode;
use crate::num::{sat32, sat64};
use crate::painter::Paint;
use crate::rect::RADIUS_CIRCLE;

/// Line parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LineDsc {
    /// Color.
    pub color: Color,
    /// Opacity.
    pub opa: Opa,
    /// Width in pixels (`≤ 0` draws nothing; widths above 2^22 px are clamped to 2^22 px).
    /// Never panics.
    pub width: i32,
    /// Length of the dashes (0 = solid).
    pub dash_width: i32,
    /// Length of the gaps between dashes.
    pub dash_gap: i32,
    /// Round start cap.
    pub round_start: bool,
    /// Round end cap.
    pub round_end: bool,
    /// Blend mode.
    pub blend_mode: BlendMode,
}

impl Default for LineDsc {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            opa: Opa::COVER,
            width: 1,
            dash_width: 0,
            dash_gap: 0,
            round_start: false,
            round_end: false,
            blend_mode: BlendMode::Normal,
        }
    }
}

impl LineDsc {
    fn dashed(&self) -> bool {
        self.dash_width > 0 && self.dash_gap > 0
    }
}

/// Widest line drawn: wider lines are drawn this wide (2^22 px, far beyond any screen). Keeps
/// the anti-aliasing arithmetic in range (see `line_general`).
pub(crate) const MAX_LINE_WIDTH: i32 = 1 << 22;

/// 65536 as a divisor.
const K65536: NonZeroI128 = match NonZeroI128::new(65536) {
    Some(k) => k,
    None => NonZeroI128::MAX,
};

/// `⌊a / b⌋` (`b` is non-zero by its type; callers pass `|a| < 2^100`). Divides in `i64` when
/// both values fit (the common case, inline); otherwise in the shared out-of-line
/// [`div_rem_i128`] (a 128-bit division is a long inline loop on 32-bit targets).
#[inline]
fn div_floor(a: i128, b: NonZeroI128) -> i128 {
    if let (Ok(a), Ok(b)) = (i64::try_from(a), i64::try_from(b.get())) {
        // `b != 0`; `None` only for `i64::MIN / −1`, which takes the `i128` path below.
        if let (Some(q), Some(r)) = (a.checked_div_euclid(b), a.checked_rem_euclid(b)) {
            // Euclidean division rounds towards −∞ for a positive divisor; for a negative one
            // the floor is one less when there is a remainder.
            // SAFE-ARITH: `|q| ≤ |a| ≤ 2^63`, and `q − 1` only for `q ≤ |a| / 2`.
            return i128::from(if b < 0 && r != 0 { q.wrapping_sub(1) } else { q });
        }
    }
    // `b != 0` and `a > i128::MIN`: the fallback is never taken. The truncated quotient is one
    // above the floor when the remainder has the other sign than `b`.
    div_rem_i128(a, b.get()).map_or(0, |(q, r)| {
        // SAFE-ARITH: `|q| ≤ |a| < 2^100`.
        if r != 0 && (r < 0) != (b.get() < 0) {
            q.wrapping_sub(1)
        } else {
            q
        }
    })
}

/// `⌈a / b⌉` (`b` is non-zero by its type; callers pass `|a| < 2^100`).
#[inline]
fn div_ceil(a: i128, b: NonZeroI128) -> i128 {
    // SAFE-ARITH: `|a| < 2^100`, so negating cannot overflow.
    div_floor(a.wrapping_neg(), b).wrapping_neg()
}

/// Overlap length of `[a, b)` and `[c, d)` (0 if disjoint). Callers pass values below 2^45.
#[inline(always)]
fn overlap(a: i64, b: i64, c: i64, d: i64) -> i64 {
    // SAFE-ARITH: the values are below 2^45 in magnitude (`line_general`'s dash range).
    b.min(d).wrapping_sub(a.max(c)).max(0)
}

impl Painter<'_> {
    /// Draws a line from `p1` to `p2`. Widths above 2^22 px are clamped to 2^22 px; any `i32`
    /// end points are accepted. Never panics.
    pub fn line(&mut self, p1: Point, p2: Point, dsc: &LineDsc) {
        if dsc.width <= 0 || dsc.opa.is_transparent() {
            return;
        }
        twine_core::trace!(target: "twine::render", "line {} -> {} w {}", p1, p2, dsc.width);
        let w = dsc.width.min(MAX_LINE_WIDTH);
        if p1 == p2 {
            if dsc.round_start || dsc.round_end {
                self.disc(p1, w, dsc);
            } else {
                self.fill_mode(square(p1, w), dsc.color, dsc.opa, dsc.blend_mode);
            }
            return;
        }
        let round = dsc.round_start || dsc.round_end;
        if !round && (p1.y == p2.y || p1.x == p2.x) {
            self.line_axis(p1, p2, w, dsc);
            return;
        }
        self.line_general(p1, p2, w, dsc);
    }

    /// Draws connected segments. Interior vertices get a round join when `width > 2` and a
    /// round cap is requested. Overlapping parts of translucent polylines are blended twice;
    /// wrap the call in [`Painter::layer`] for an exact result.
    pub fn polyline(&mut self, pts: &[Point], dsc: &LineDsc) {
        if pts.len() < 2 {
            if let Some(&p) = pts.first() {
                self.line(p, p, dsc);
            }
            return;
        }
        let joins = dsc.width > 2 && (dsc.round_start || dsc.round_end);
        // SAFE-ARITH: `pts.len() ≥ 2`.
        let last = pts.len().wrapping_sub(2);
        for (i, seg) in pts.windows(2).enumerate() {
            let &[a, b] = seg else { continue };
            let d = LineDsc {
                round_start: dsc.round_start && i == 0,
                round_end: dsc.round_end && i == last,
                ..*dsc
            };
            self.line(a, b, &d);
            if joins && i < last {
                self.disc(b, dsc.width.min(MAX_LINE_WIDTH), dsc);
            }
        }
    }

    /// A filled circle of diameter `w` centered on pixel `c`.
    fn disc(&mut self, c: Point, w: i32, dsc: &LineDsc) {
        let area = square(c, w);
        let r = crate::rrect::eff_radius(area, RADIUS_CIRCLE);
        let mut sc = self.take_scratch();
        self.ring(
            &mut sc,
            area,
            r,
            None,
            &Paint::Solid(dsc.color),
            dsc.opa,
            dsc.blend_mode,
        );
        self.put_scratch(sc);
    }

    /// Horizontal or vertical line without round caps: rectangle fills (one per dash).
    fn line_axis(&mut self, p1: Point, p2: Point, w: i32, dsc: &LineDsc) {
        let hor = p1.y == p2.y;
        let (a, b) = if hor { (p1.x, p2.x) } else { (p1.y, p2.y) };
        let (a, b, w) = (i64::from(a), i64::from(b), i64::from(w));
        let dir = if b >= a { 1 } else { -1 };
        // SAFE-ARITH: differences of `i32`s are below 2^32 and `w ≤ 2^22`, so every value in
        // this function is below 2^34 in magnitude (the dash loop stops at `len + period`).
        let len = b.wrapping_sub(a).abs().wrapping_add(1);
        let across = i64::from(if hor { p1.y } else { p1.x });
        let (c0, c1) = (
            across.wrapping_sub(w.wrapping_sub(1) / 2),
            across.wrapping_add(w / 2).wrapping_add(1),
        );
        let mut seg = |s: i64, e: i64| {
            // [s, e) along the axis in pixels from `a`, in the line's direction.
            let (u0, u1) = if dir > 0 {
                (a.wrapping_add(s), a.wrapping_add(e))
            } else {
                (
                    a.wrapping_sub(e).wrapping_add(1),
                    a.wrapping_sub(s).wrapping_add(1),
                )
            };
            let (u0, u1, c0, c1) = (sat32(u0), sat32(u1), sat32(c0), sat32(c1));
            let r = if hor {
                Rect::new(u0, c0, u1, c1)
            } else {
                Rect::new(c0, u0, c1, u1)
            };
            self.fill_mode(r, dsc.color, dsc.opa, dsc.blend_mode);
        };
        if dsc.dashed() {
            let (dw, period) = (
                i64::from(dsc.dash_width),
                i64::from(dsc.dash_width).wrapping_add(i64::from(dsc.dash_gap)),
            );
            let mut s = 0;
            while s < len {
                seg(s, s.wrapping_add(dw).min(len));
                s = s.wrapping_add(period);
            }
        } else {
            seg(0, len);
        }
    }

    #[allow(clippy::too_many_lines)] // one row loop with its range arguments
    fn line_general(&mut self, p1: Point, p2: Point, w: i32, dsc: &LineDsc) {
        // Range argument (SAFE-ARITH for this function): the end points are `i32`s, so
        // `|dx|, |dy| < 2^32` and `len2 < 2^65` (in `i128`); `l8 = |p2 − p1| · 256 < 2^41`
        // and `l8 ≥ 256 · max(|dx|, |dy|)`. `w ≤ 2^22`, so `hw, ext < 2^30`. Products of these
        // with row/column offsets (`< 2^33`) are formed in `i128` (below 2^100).
        let (dx, dy) = (
            i64::from(p2.x).wrapping_sub(i64::from(p1.x)),
            i64::from(p2.y).wrapping_sub(i64::from(p1.y)),
        );
        let (dx128, dy128) = (i128::from(dx), i128::from(dy));
        let len2 = dx128.wrapping_mul(dx128).wrapping_add(dy128.wrapping_mul(dy128));
        // Length in 1/256 px (exact floor square root; `len2 · 2^16 < 2^81`).
        let l8 = sat64(((len2 as u128) << 16).isqrt() as i128).max(1);
        let hw = i64::from(w).wrapping_mul(128); // half width, 1/256 px
        let ext = hw.wrapping_add(256); // reach beyond the segment ends and the band
        // Bounding box.
        let e = sat32((ext >> 8).wrapping_add(1));
        let bb = Rect::new(
            p1.x.min(p2.x).saturating_sub(e),
            p1.y.min(p2.y).saturating_sub(e),
            p1.x.max(p2.x).saturating_add(e).saturating_add(1),
            p1.y.max(p2.y).saturating_add(e).saturating_add(1),
        );
        let Some(rows) = bb.intersection(&self.clip()) else {
            return;
        };
        // d and t accumulate in 1/256 px with 16 extra fractional bits. Values beyond `±2^58`
        // are clamped: such pixels are more than 2^42 / 256 px away from the line or its ends,
        // where every coverage is 0 either way.
        let lim = 1i128 << 58;
        let scale = |v: i128| -> i64 {
            // `l8 ≥ 1`: the fallback is never taken. `|v| < 2^66`, `· 2^32 < 2^98`.
            sat64(
                div_rem_i128(v.wrapping_mul(1 << 32), i128::from(l8))
                    .map_or(0, |(q, _)| q)
                    .clamp(lim.wrapping_neg(), lim),
            )
        };
        // `|d_step|, |t_step| ≤ 2^32 · max(|dx|, |dy|) / l8 ≤ 2^24`.
        let (d_step, t_step) = (scale(dy128.wrapping_neg()), scale(dx128));
        // |cross|·65536 ≤ ext·l8 ⇔ |d| ≤ ext
        let k_band = div_ceil(i128::from(ext).wrapping_mul(i128::from(l8)), K65536);
        let k_t = k_band;
        let (dashed, period, dw) = (
            dsc.dashed(),
            i64::from(dsc.dash_width)
                .wrapping_add(i64::from(dsc.dash_gap))
                .wrapping_mul(256),
            i64::from(dsc.dash_width).wrapping_mul(256),
        );
        let mut sc = self.take_scratch();
        for y in rows.y0..rows.y1 {
            let ry = i64::from(y).wrapping_sub(i64::from(p1.y));
            let ry128 = i128::from(ry);
            // x-range where |cross| ≤ k_band, cross = dx·ry − dy·rx.
            let (lo0, hi0) = (
                i64::from(rows.x0).wrapping_sub(i64::from(p1.x)),
                i64::from(rows.x1).wrapping_sub(1).wrapping_sub(i64::from(p1.x)),
            );
            let (mut lo, mut hi) = (i128::from(lo0), i128::from(hi0));
            if let Some(dy) = NonZeroI128::new(dy128) {
                let c = dx128.wrapping_mul(ry128);
                let (a, b) = (c.wrapping_sub(k_band), c.wrapping_add(k_band));
                let (u, v) = if dy.get() > 0 {
                    (div_floor(a, dy), div_ceil(b, dy))
                } else {
                    (div_floor(b, dy), div_ceil(a, dy))
                };
                lo = lo.max(u.wrapping_sub(1));
                hi = hi.min(v.wrapping_add(1));
            }
            // t-range: −ext ≤ t ≤ len + ext  ⇔  −k_t ≤ dot ≤ len2 + k_t, dot = dx·rx + dy·ry.
            if let Some(dx) = NonZeroI128::new(dx128) {
                let c = dy128.wrapping_mul(ry128);
                let (a, b) = (
                    k_t.wrapping_neg().wrapping_sub(c),
                    len2.wrapping_add(k_t).wrapping_sub(c),
                );
                let (u, v) = if dx.get() > 0 {
                    (div_floor(a, dx), div_ceil(b, dx))
                } else {
                    (div_floor(b, dx), div_ceil(a, dx))
                };
                lo = lo.max(u.wrapping_sub(1));
                hi = hi.min(v.wrapping_add(1));
            }
            if lo > hi {
                continue;
            }
            // Here `lo0 ≤ lo ≤ hi ≤ hi0`: the narrowing is exact.
            let (lo, hi) = (sat64(lo), sat64(hi));
            let x0 = sat32(i64::from(p1.x).wrapping_add(lo));
            let x1 = sat32(i64::from(p1.x).wrapping_add(hi).wrapping_add(1));
            let lo128 = i128::from(lo);
            let cross0 = dx128.wrapping_mul(ry128).wrapping_sub(dy128.wrapping_mul(lo128));
            let dot0 = dx128.wrapping_mul(lo128).wrapping_add(dy128.wrapping_mul(ry128));
            let d0 = scale(cross0);
            let t0 = scale(dot0);
            let len256 = l8;
            // Round caps: squared distances of pixels at most `ext / 256 < 2^22` px from an end
            // point decide; offsets are clamped to `±2^23` px first (farther pixels stay
            // farther than the cap reaches), so the sum of squares `· 2^16` fits in `u64`.
            let cap_dist = |rx: i64, ry: i64| -> i64 {
                let (rx, ry) = (rx.clamp(-(1 << 23), 1 << 23), ry.clamp(-(1 << 23), 1 << 23));
                i64::from(isqrt64(
                    (rx.wrapping_mul(rx).wrapping_add(ry.wrapping_mul(ry)) as u64) << 16,
                ))
            };
            let mut f = |cx0: i32, buf: &mut [u8]| {
                // `0 ≤ off < 2^32`; `|d|, |t| ≤ 2^58 + 2^24 · 2^32 < 2^59` here and after at
                // most `max_span ≤ 2^16` more steps.
                let off = i64::from(cx0).wrapping_sub(i64::from(x0));
                let mut d = d0.wrapping_add(d_step.wrapping_mul(off));
                let mut t = t0.wrapping_add(t_step.wrapping_mul(off));
                let mut rx = lo.wrapping_add(off);
                for c in buf.iter_mut() {
                    // `|d >> 16|, |t >> 16| < 2^43`, `hw + 128 < 2^30`, `len256 < 2^41`.
                    let (dd, tt) = ((d >> 16).abs(), t >> 16);
                    let mut v = if tt < 0 && dsc.round_start {
                        hw.wrapping_add(128).wrapping_sub(cap_dist(rx, ry)).clamp(0, 256)
                    } else if tt > len256 && dsc.round_end {
                        let d_end = cap_dist(rx.wrapping_sub(dx), ry.wrapping_sub(dy));
                        hw.wrapping_add(128).wrapping_sub(d_end).clamp(0, 256)
                    } else {
                        let side = hw.wrapping_add(128).wrapping_sub(dd).clamp(0, 256);
                        let cap = if tt < 128 {
                            tt.wrapping_add(256).clamp(0, 256)
                        } else if tt > len256.wrapping_sub(128) {
                            len256.wrapping_sub(tt).wrapping_add(256).clamp(0, 256)
                        } else {
                            256
                        };
                        side.wrapping_mul(cap) >> 8
                    };
                    if dashed && v > 0 {
                        // `period ≥ 512` (`dash_width > 0`, `dash_gap > 0`) and below 2^40, so
                        // `u` and the overlap bounds stay below 2^45. The fallback (a zero
                        // period) is never taken.
                        let u = tt.wrapping_add(128).checked_rem_euclid(period).unwrap_or(0);
                        let (a, b) = (u.wrapping_sub(128), u.wrapping_add(128));
                        let on = overlap(a, b, 0, dw)
                            .wrapping_add(overlap(a, b, period, period.wrapping_add(dw)))
                            .wrapping_add(overlap(a, b, period.wrapping_neg(), dw.wrapping_sub(period)));
                        v = v.wrapping_mul(on.min(256)) >> 8;
                    }
                    *c = v.min(255) as u8;
                    d = d.wrapping_add(d_step);
                    t = t.wrapping_add(t_step);
                    rx = rx.wrapping_add(1);
                }
            };
            self.span(
                &mut sc,
                y,
                x0,
                x1,
                &Paint::Solid(dsc.color),
                dsc.opa,
                dsc.blend_mode,
                Some(&mut f),
            );
        }
        self.put_scratch(sc);
    }
}

/// The `w × w` square centered on pixel `c` (the extra pixel of an even width goes right and
/// down, like LVGL).
fn square(c: Point, w: i32) -> Rect {
    // SAFE-ARITH: `1 ≤ w`, so `(w − 1) / 2` does not overflow.
    let h = w.wrapping_sub(1) / 2;
    Rect::from_xywh(c.x.saturating_sub(h), c.y.saturating_sub(h), w, w)
}
