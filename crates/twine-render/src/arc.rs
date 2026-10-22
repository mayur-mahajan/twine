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
use crate::circle::MAX_RADIUS;
use crate::mask::Mask;
use crate::num::{sat32, span};
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
    /// `mid256` and `rad` are below `2^31` (radius ≤ `MAX_RADIUS = 2^22`, in 1/256 px).
    fn new(a: Angle, mid256: i64, rad: i64) -> Self {
        // SAFE-ARITH: `|mid256| < 2^31` and `|cos|, |sin| ≤ 32_767`: products below 2^46.
        Self {
            cx: mid256.wrapping_mul(i64::from(cos(a))) / 32767,
            cy: mid256.wrapping_mul(i64::from(sin(a))) / 32767,
            rad,
        }
    }

    /// Coverage of the pixel whose center is `(px, py)` (1/256 px, relative to the center;
    /// `|px|, |py| < 2^41`).
    #[inline]
    fn cov(&self, px: i64, py: i64) -> u8 {
        // SAFE-ARITH: `|px|, |py| < 2^41` and `|cx|, |cy|, rad < 2^31`; past the early return
        // `|dx|, |dy| ≤ rad + 256 < 2^31`, so each square is below 2^62 and their sum fits
        // in `u64`.
        let (dx, dy) = (px.wrapping_sub(self.cx), py.wrapping_sub(self.cy));
        let lim = self.rad.wrapping_add(256);
        if dx.abs() > lim || dy.abs() > lim {
            return 0;
        }
        let d2 = (dx.wrapping_mul(dx) as u64).wrapping_add(dy.wrapping_mul(dy) as u64);
        let dist = i64::from(isqrt64(d2));
        self.rad.wrapping_add(128).wrapping_sub(dist).clamp(0, 255) as u8
    }
}

impl Painter<'_> {
    /// Draws an arc from `start` clockwise to `end` (angles clockwise from 3 o'clock).
    /// `start == end` draws nothing; `|end − start| ≥ 360°` draws the full ring. Radii above
    /// 2^22 px are clamped to 2^22 px. Never panics.
    pub fn arc(&mut self, center: Point, radius: i32, start: Angle, end: Angle, dsc: &ArcDsc<'_>) {
        if radius <= 0 || dsc.width <= 0 || dsc.opa.is_transparent() || start == end {
            return;
        }
        twine_core::trace!(target: "twine::render", "arc {} r {} {}..{}", center, radius, start, end);
        let radius = radius.min(MAX_RADIUS);
        // SAFE-ARITH (whole function): `1 ≤ w ≤ radius ≤ 2^22`, angles are `i32`s.
        let full = i64::from(end.as_deci_deg())
            .wrapping_sub(i64::from(start.as_deci_deg()))
            .abs()
            >= 3600;
        let w = dsc.width.min(radius);
        let outer = Rect::new(
            center.x.saturating_sub(radius),
            center.y.saturating_sub(radius),
            center.x.saturating_add(radius),
            center.y.saturating_add(radius),
        );
        let ri = radius.wrapping_sub(w);
        let inner = outer.expand(w.wrapping_neg());
        // Angle range [s, e) with e > s.
        let s = start.normalized().as_deci_deg();
        let mut e = end.normalized().as_deci_deg();
        if e <= s {
            // Normalized angles are in `0..3600`.
            e = e.wrapping_add(3600);
        }
        let mid256 = i64::from(radius.wrapping_mul(2).wrapping_sub(w)).wrapping_mul(128);
        let rad = i64::from(w).wrapping_mul(128);
        let caps = (dsc.rounded && !full).then(|| {
            [
                Cap::new(Angle::deci_deg(s), mid256, rad),
                Cap::new(Angle::deci_deg(e), mid256, rad),
            ]
        });
        // Bounding box of the sector (end points, axis extremes inside the range, caps).
        let bbox = if full {
            outer
        } else {
            let mut b = Rect::new(
                center.x,
                center.y,
                center.x.saturating_add(1),
                center.y.saturating_add(1),
            );
            let mut add = |a: i32, r: i64| {
                // `r ≤ 2^22`: the offsets are below 2^37 / 32_767.
                let x = i64::from(center.x)
                    .wrapping_add(r.wrapping_mul(i64::from(cos(Angle::deci_deg(a)))) / 32767);
                let y = i64::from(center.y)
                    .wrapping_add(r.wrapping_mul(i64::from(sin(Angle::deci_deg(a)))) / 32767);
                let (x, y) = (sat32(x), sat32(y));
                b = b.union(&Rect::new(
                    x.saturating_sub(1),
                    y.saturating_sub(1),
                    x.saturating_add(2),
                    y.saturating_add(2),
                ));
            };
            for a in [s, e] {
                add(a, i64::from(radius));
                add(a, i64::from(ri));
            }
            // `s, e < 7200`.
            let mut q = (s / 900).wrapping_add(1).wrapping_mul(900);
            while q < e {
                add(q, i64::from(radius));
                q = q.wrapping_add(900);
            }
            let pad = if caps.is_some() {
                (w / 2).wrapping_add(2)
            } else {
                1
            };
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
        let img_pos = Point::new(center.x.saturating_sub(radius), center.y.saturating_sub(radius));
        let angle_mask = (!full).then_some(Mask::Angle {
            center,
            start: Angle::deci_deg(s),
            end: Angle::deci_deg(e),
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
                let Some(iy) = (y >= img_pos.y)
                    .then(|| u16::try_from(span(img_pos.y, y)).ok())
                    .flatten()
                    .filter(|&iy| iy < img.h)
                else {
                    continue;
                };
                lo = lo.max(img_pos.x);
                hi = hi.min(img_pos.x.saturating_add(i32::from(img.w)));
                let row = img.row(iy);
                let bpp = usize::from(img.format.bpp()) / 8;
                let skip = span(img_pos.x, lo).saturating_mul(bpp);
                paint = Paint::Src {
                    src: Source::Pixels {
                        data: row.get(skip..).unwrap_or_default(),
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
            // Pixel center, half pixels (`|py| < 2^34`).
            let py = i64::from(y)
                .wrapping_sub(i64::from(center.y))
                .wrapping_mul(2)
                .wrapping_add(1);
            for (a, b) in pieces {
                if a >= b {
                    continue;
                }
                let mut f = |x0: i32, buf: &mut [u8]| {
                    // The chunk ends before `b ≤ i32::MAX`: the bounded range covers it.
                    for (v, x) in buf.iter_mut().zip(x0..i32::MAX) {
                        *v = o.cov(x).saturating_sub(i.cov(x));
                    }
                    if let Some(m) = &angle_mask {
                        m.apply(y, x0, buf);
                    }
                    if let Some([c0, c1]) = &caps {
                        // Pixel centers in 1/256 px: `|px|, |py · 128| < 2^41` (`Cap::cov`).
                        let py = py.wrapping_mul(128);
                        for (v, x) in buf.iter_mut().zip(x0..i32::MAX) {
                            let px = i64::from(x)
                                .wrapping_sub(i64::from(center.x))
                                .wrapping_mul(2)
                                .wrapping_add(1)
                                .wrapping_mul(128);
                            *v = (*v).max(c0.cov(px, py).max(c1.cov(px, py)));
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
