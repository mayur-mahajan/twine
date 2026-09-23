//! Arcs: [`ArcDsc`] and [`Painter::arc`] — the primitive behind arcs, spinners, scales and
//! round sliders.
//!
//! The ring lies between the circles of radius `radius` and `radius − width` around the
//! top-left corner of pixel `center` (so a ring of radius `r` fills a `2r × 2r` box, like
//! LVGL). It is the difference of two anti-aliased discs, limited by an anti-aliased angle
//! mask. Rounded ends are discs centered on the middle radius at both end angles; their
//! coverage is combined with the ring's by `max` before blending, so translucent arcs have no
//! double-blended seams.

use core::mem::take;

use twine_core::math::{cos, isqrt64, sin};
use twine_core::{Angle, Color, Opa, Point, Rect};

use crate::blend::{BlendMode, Source};
use crate::mask::Mask;
use crate::painter::Paint;
use crate::rrect::RowSpan;
use crate::transform_blit::{is_direct_format, warn_unsupported_image};
use crate::{ImagePixels, Painter};

/// Arc parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArcDsc<'a> {
    /// Color (unless `image` is set).
    pub color: Color,
    /// Opacity.
    pub opa: Opa,
    /// Ring width (`≥ radius`: a pie sector).
    pub width: i32,
    /// Rounded ends.
    pub rounded: bool,
    /// Fill the ring with this image, placed with its top-left corner at
    /// `center − (radius, radius)` (normally a `2·radius` square). Pixels outside the image are
    /// not drawn.
    pub image: Option<&'a ImagePixels<'a>>,
    /// Blend mode.
    pub blend_mode: BlendMode,
}

impl Default for ArcDsc<'_> {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            opa: Opa::COVER,
            width: 1,
            rounded: false,
            image: None,
            blend_mode: BlendMode::Normal,
        }
    }
}

/// A rounded end: a disc of radius `rad` (1/256 px) at `(cx, cy)` (1/256 px, relative to the
/// arc center).
#[derive(Clone, Copy, Debug)]
struct Cap {
    cx: i64,
    cy: i64,
    rad: i64,
}

impl Cap {
    fn new(a: Angle, mid256: i64, rad: i64) -> Self {
        Self {
            cx: mid256 * i64::from(cos(a)) / 32767,
            cy: mid256 * i64::from(sin(a)) / 32767,
            rad,
        }
    }

    /// Coverage of the pixel whose center is `(px, py)` (1/256 px, relative to the center).
    #[inline]
    fn cov(&self, px: i64, py: i64) -> u8 {
        let (dx, dy) = (px - self.cx, py - self.cy);
        let lim = self.rad + 256;
        if dx.abs() > lim || dy.abs() > lim {
            return 0;
        }
        let dist = i64::from(isqrt64((dx * dx + dy * dy) as u64));
        (self.rad + 128 - dist).clamp(0, 255) as u8
    }
}

impl Painter<'_> {
    /// Draws an arc from `start` clockwise to `end` (angles clockwise from 3 o'clock).
    /// `start == end` draws nothing; `|end − start| ≥ 360°` draws the full ring.
    pub fn arc(&mut self, center: Point, radius: i32, start: Angle, end: Angle, dsc: &ArcDsc<'_>) {
        if radius <= 0 || dsc.width <= 0 || dsc.opa.is_transparent() || start == end {
            return;
        }
        twine_core::trace!(target: "twine::render", "arc {} r {} {}..{}", center, radius, start, end);
        let full = (i64::from(end.0) - i64::from(start.0)).abs() >= 3600;
        let w = dsc.width.min(radius);
        let outer = Rect::new(
            center.x - radius,
            center.y - radius,
            center.x + radius,
            center.y + radius,
        );
        let ri = radius - w;
        let inner = outer.expand(-w);
        // Angle range [s, e) with e > s.
        let s = start.normalized().0;
        let mut e = end.normalized().0;
        if e <= s {
            e += 3600;
        }
        let mid256 = i64::from(2 * radius - w) * 128;
        let rad = i64::from(w) * 128;
        let caps = (dsc.rounded && !full)
            .then(|| [Cap::new(Angle(s), mid256, rad), Cap::new(Angle(e), mid256, rad)]);
        // Bounding box of the sector (end points, axis extremes inside the range, caps).
        let bbox = if full {
            outer
        } else {
            let mut b = Rect::new(center.x, center.y, center.x + 1, center.y + 1);
            let mut add = |a: i32, r: i64| {
                let x = i64::from(center.x) + r * i64::from(cos(Angle(a))) / 32767;
                let y = i64::from(center.y) + r * i64::from(sin(Angle(a))) / 32767;
                b = b.union(&Rect::new(x as i32 - 1, y as i32 - 1, x as i32 + 2, y as i32 + 2));
            };
            for a in [s, e] {
                add(a, i64::from(radius));
                add(a, i64::from(ri));
            }
            let mut q = (s / 900 + 1) * 900;
            while q < e {
                add(q, i64::from(radius));
                q += 900;
            }
            let pad = if caps.is_some() { w / 2 + 2 } else { 1 };
            b.expand(pad).intersection(&outer).unwrap_or(Rect::ZERO)
        };
        let Some(rows) = bbox.intersection(&self.clip()) else {
            return;
        };
        // Image source.
        let img = match dsc.image {
            Some(img) if !is_direct_format(img.format) => {
                warn_unsupported_image(img.format);
                return;
            }
            other => other,
        };
        let img_pos = Point::new(center.x - radius, center.y - radius);
        let angle_mask = (!full).then_some(Mask::Angle {
            center,
            start: Angle(s),
            end: Angle(e),
        });
        let circle = {
            let c = &mut self.caches_mut().circle;
            c.ensure(radius);
            c.ensure(ri);
            take(c)
        };
        let qo = circle.quarter(radius);
        let qi = circle.quarter(ri);
        let mut sc = self.take_scratch();
        for y in rows.y0..rows.y1 {
            let o = RowSpan::new(outer, &qo, y);
            let i = RowSpan::new(inner, &qi, y);
            let mut lo = o.aa_l0.max(rows.x0);
            let mut hi = o.aa_r1.min(rows.x1);
            let mut paint = Paint::Solid(dsc.color);
            if let Some(img) = img {
                let iy = y - img_pos.y;
                if iy < 0 || iy >= i32::from(img.h) {
                    continue;
                }
                lo = lo.max(img_pos.x);
                hi = hi.min(img_pos.x + i32::from(img.w));
                let row = img.row(iy as u16);
                let bpp = usize::from(img.format.bpp()) / 8;
                let skip = ((lo - img_pos.x).max(0) as usize * bpp).min(row.len());
                paint = Paint::Src {
                    src: Source::Pixels {
                        data: &row[skip..],
                        format: img.format,
                    },
                    x0: lo,
                };
            }
            if lo >= hi {
                continue;
            }
            // Skip the hole (where the inner disc is fully covered), unless caps reach into it.
            let pieces = if !i.is_empty() && i.full_l < i.full_r && caps.is_none() {
                [(lo, hi.min(i.full_l)), (lo.max(i.full_r), hi)]
            } else {
                [(lo, hi), (0, 0)]
            };
            let py = 2 * i64::from(y - center.y) + 1; // pixel center, half pixels
            for (a, b) in pieces {
                if a >= b {
                    continue;
                }
                let mut f = |x0: i32, buf: &mut [u8]| {
                    for (k, v) in buf.iter_mut().enumerate() {
                        let x = x0 + k as i32;
                        *v = o.cov(x).saturating_sub(i.cov(x));
                    }
                    if let Some(m) = &angle_mask {
                        m.apply(y, x0, buf);
                    }
                    if let Some(caps) = &caps {
                        for (k, v) in buf.iter_mut().enumerate() {
                            let px = (2 * i64::from(x0 + k as i32 - center.x) + 1) * 128;
                            let c = caps[0].cov(px, py * 128).max(caps[1].cov(px, py * 128));
                            *v = (*v).max(c);
                        }
                    }
                };
                self.span(&mut sc, y, a, b, &paint, dsc.opa, dsc.blend_mode, Some(&mut f));
            }
        }
        self.put_scratch(sc);
        self.caches_mut().circle = circle;
    }
}
