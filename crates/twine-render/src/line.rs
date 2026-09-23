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

use twine_core::math::isqrt64;
use twine_core::{Color, Opa, Point, Rect};

use crate::Painter;
use crate::blend::BlendMode;
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
    /// Width in pixels (`≤ 0` draws nothing).
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

#[inline]
fn div_floor(a: i64, b: i64) -> i64 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

#[inline]
fn div_ceil(a: i64, b: i64) -> i64 {
    -div_floor(-a, b)
}

/// Overlap length of `[a, b)` and `[c, d)` (0 if disjoint).
#[inline(always)]
fn overlap(a: i64, b: i64, c: i64, d: i64) -> i64 {
    (b.min(d) - a.max(c)).max(0)
}

impl Painter<'_> {
    /// Draws a line from `p1` to `p2`.
    pub fn line(&mut self, p1: Point, p2: Point, dsc: &LineDsc) {
        if dsc.width <= 0 || dsc.opa.is_transparent() {
            return;
        }
        twine_core::trace!(target: "twine::render", "line {} -> {} w {}", p1, p2, dsc.width);
        let w = dsc.width;
        if p1 == p2 {
            if dsc.round_start || dsc.round_end {
                self.disc(p1, w, dsc);
            } else {
                let r = Rect::from_xywh(p1.x - (w - 1) / 2, p1.y - (w - 1) / 2, w, w);
                self.fill_mode(r, dsc.color, dsc.opa, dsc.blend_mode);
            }
            return;
        }
        let round = dsc.round_start || dsc.round_end;
        if !round && (p1.y == p2.y || p1.x == p2.x) {
            self.line_axis(p1, p2, dsc);
            return;
        }
        self.line_general(p1, p2, dsc);
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
        let last = pts.len() - 2;
        for (i, seg) in pts.windows(2).enumerate() {
            let d = LineDsc {
                round_start: dsc.round_start && i == 0,
                round_end: dsc.round_end && i == last,
                ..*dsc
            };
            self.line(seg[0], seg[1], &d);
            if joins && i < last {
                self.disc(seg[1], dsc.width, dsc);
            }
        }
    }

    /// A filled circle of diameter `w` centered on pixel `c`.
    fn disc(&mut self, c: Point, w: i32, dsc: &LineDsc) {
        let area = Rect::from_xywh(c.x - (w - 1) / 2, c.y - (w - 1) / 2, w, w);
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
    fn line_axis(&mut self, p1: Point, p2: Point, dsc: &LineDsc) {
        let w = dsc.width;
        let hor = p1.y == p2.y;
        let (a, b) = if hor { (p1.x, p2.x) } else { (p1.y, p2.y) };
        let dir = if b >= a { 1 } else { -1 };
        let len = (b - a).abs() + 1;
        let across = if hor { p1.y } else { p1.x };
        let (c0, c1) = (across - (w - 1) / 2, across + w / 2 + 1);
        let mut seg = |s: i32, e: i32| {
            // [s, e) along the axis in pixels from `a`, in the line's direction.
            let (u0, u1) = if dir > 0 {
                (a + s, a + e)
            } else {
                (a - e + 1, a - s + 1)
            };
            let r = if hor {
                Rect::new(u0, c0, u1, c1)
            } else {
                Rect::new(c0, u0, c1, u1)
            };
            self.fill_mode(r, dsc.color, dsc.opa, dsc.blend_mode);
        };
        if dsc.dashed() {
            let period = dsc.dash_width + dsc.dash_gap;
            let mut s = 0;
            while s < len {
                seg(s, (s + dsc.dash_width).min(len));
                s += period;
            }
        } else {
            seg(0, len);
        }
    }

    fn line_general(&mut self, p1: Point, p2: Point, dsc: &LineDsc) {
        let (dx, dy) = (i64::from(p2.x - p1.x), i64::from(p2.y - p1.y));
        let len2 = dx * dx + dy * dy;
        let l8 = i64::from(isqrt64((len2 as u64) << 16)).max(1); // length in 1/256 px
        let hw = i64::from(dsc.width) * 128; // half width, 1/256 px
        let ext = hw + 256; // reach beyond the segment ends and the band
        // Bounding box.
        let e = (ext >> 8) as i32 + 1;
        let bb = Rect::new(
            p1.x.min(p2.x) - e,
            p1.y.min(p2.y) - e,
            p1.x.max(p2.x) + e + 1,
            p1.y.max(p2.y) + e + 1,
        );
        let Some(rows) = bb.intersection(&self.clip()) else {
            return;
        };
        // d and t accumulate in 1/256 px with 16 extra fractional bits.
        let scale = |v: i64| -> i64 { (i128::from(v) * (1i128 << 32) / i128::from(l8)) as i64 };
        let (d_step, t_step) = (scale(-dy), scale(dx));
        // |cross|·65536 ≤ ext·l8 ⇔ |d| ≤ ext
        let k_band = div_ceil(ext * l8, 65536);
        let k_t = div_ceil(ext * l8, 65536);
        let (dashed, period, dw) = (
            dsc.dashed(),
            i64::from(dsc.dash_width + dsc.dash_gap) * 256,
            i64::from(dsc.dash_width) * 256,
        );
        let mut sc = self.take_scratch();
        for y in rows.y0..rows.y1 {
            let ry = i64::from(y - p1.y);
            // x-range where |cross| ≤ k_band, cross = dx·ry − dy·rx.
            let (mut lo, mut hi) = (i64::from(rows.x0 - p1.x), i64::from(rows.x1 - 1 - p1.x));
            if dy != 0 {
                let (a, b) = (dx * ry - k_band, dx * ry + k_band);
                let (u, v) = if dy > 0 {
                    (div_floor(a, dy), div_ceil(b, dy))
                } else {
                    (div_floor(b, dy), div_ceil(a, dy))
                };
                lo = lo.max(u - 1);
                hi = hi.min(v + 1);
            }
            // t-range: −ext ≤ t ≤ len + ext  ⇔  −k_t ≤ dot ≤ len2 + k_t, dot = dx·rx + dy·ry.
            if dx != 0 {
                let (a, b) = (-k_t - dy * ry, len2 + k_t - dy * ry);
                let (u, v) = if dx > 0 {
                    (div_floor(a, dx), div_ceil(b, dx))
                } else {
                    (div_floor(b, dx), div_ceil(a, dx))
                };
                lo = lo.max(u - 1);
                hi = hi.min(v + 1);
            }
            if lo > hi {
                continue;
            }
            let x0 = p1.x + lo as i32;
            let x1 = p1.x + hi as i32 + 1;
            let cross0 = dx * ry - dy * lo;
            let dot0 = dx * lo + dy * ry;
            let d0 = scale(cross0);
            let t0 = scale(dot0);
            let len256 = l8;
            let mut f = |cx0: i32, buf: &mut [u8]| {
                let off = i64::from(cx0 - x0);
                let mut d = d0 + d_step * off;
                let mut t = t0 + t_step * off;
                for (i, c) in buf.iter_mut().enumerate() {
                    let (dd, tt) = ((d >> 16).abs(), t >> 16);
                    let mut v = if tt < 0 && dsc.round_start {
                        let rx = lo + off + i as i64;
                        let dist = i64::from(isqrt64(((rx * rx + ry * ry) as u64) << 16));
                        (hw + 128 - dist).clamp(0, 256)
                    } else if tt > len256 && dsc.round_end {
                        let rx = lo + off + i as i64 - dx;
                        let ry2 = ry - dy;
                        let dist = i64::from(isqrt64(((rx * rx + ry2 * ry2) as u64) << 16));
                        (hw + 128 - dist).clamp(0, 256)
                    } else {
                        let side = (hw + 128 - dd).clamp(0, 256);
                        let cap = if tt < 128 {
                            (tt + 256).clamp(0, 256)
                        } else if tt > len256 - 128 {
                            (len256 - tt + 256).clamp(0, 256)
                        } else {
                            256
                        };
                        (side * cap) >> 8
                    };
                    if dashed && v > 0 {
                        let u = (tt + 128).rem_euclid(period);
                        let (a, b) = (u - 128, u + 128);
                        let on = overlap(a, b, 0, dw)
                            + overlap(a, b, period, period + dw)
                            + overlap(a, b, -period, -period + dw);
                        v = (v * on.min(256)) >> 8;
                    }
                    *c = v.min(255) as u8;
                    d += d_step;
                    t += t_step;
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
