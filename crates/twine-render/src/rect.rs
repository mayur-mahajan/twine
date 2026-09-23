//! Rectangles: [`RectDsc`], [`BorderSide`], [`Painter::rect`] (shadow, background with
//! gradient, border, outline — in LVGL order) and [`Painter::fill_gradient`].

use core::mem::take;
use core::ops::{BitAnd, BitOr, BitOrAssign};

use twine_core::math::udiv255;
use twine_core::{Color, Insets, Opa, Rect};

use crate::Painter;
use crate::blend::BlendMode;
use crate::circle::Quarter;
use crate::gradient::{GradSampler, Gradient};
use crate::painter::{Paint, Scratch};
use crate::rrect::{RowSpan, eff_radius};
use crate::shadow::ShadowDsc;

/// LVGL's "circle" radius: any radius at least this large means half of the shorter side.
pub const RADIUS_CIRCLE: i32 = 0x7FFF;

/// Which sides of a border are drawn (bit flags).
///
/// ```
/// use twine_render::BorderSide;
/// let s = BorderSide::LEFT | BorderSide::TOP;
/// assert!(s.contains(BorderSide::TOP) && !s.contains(BorderSide::RIGHT));
/// assert!(BorderSide::FULL.contains(s));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BorderSide(pub u8);

impl BorderSide {
    /// No side.
    pub const NONE: BorderSide = BorderSide(0x00);
    /// Bottom side.
    pub const BOTTOM: BorderSide = BorderSide(0x01);
    /// Top side.
    pub const TOP: BorderSide = BorderSide(0x02);
    /// Left side.
    pub const LEFT: BorderSide = BorderSide(0x04);
    /// Right side.
    pub const RIGHT: BorderSide = BorderSide(0x08);
    /// All four sides.
    pub const FULL: BorderSide = BorderSide(0x0F);
    /// Internal borders (between cells of button matrices and tables). The renderer treats it
    /// like [`FULL`](Self::FULL); widgets use it to decide which cell edges to draw.
    pub const INTERNAL: BorderSide = BorderSide(0x10);

    /// Whether every side of `other` is set.
    #[must_use]
    pub const fn contains(self, other: BorderSide) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no side is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for BorderSide {
    type Output = BorderSide;
    fn bitor(self, o: BorderSide) -> BorderSide {
        BorderSide(self.0 | o.0)
    }
}

impl BitOrAssign for BorderSide {
    fn bitor_assign(&mut self, o: BorderSide) {
        self.0 |= o.0;
    }
}

impl BitAnd for BorderSide {
    type Output = BorderSide;
    fn bitand(self, o: BorderSide) -> BorderSide {
        BorderSide(self.0 & o.0)
    }
}

/// Everything a styled rectangle can draw. The default draws nothing.
///
/// ```
/// use twine_core::{Color, Opa, Rect};
/// use twine_render::{RectDsc, RADIUS_CIRCLE};
/// let dsc = RectDsc { radius: RADIUS_CIRCLE, bg_color: Color::BLUE, bg_opa: Opa::COVER, ..RectDsc::default() };
/// assert_eq!(dsc.border_width, 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectDsc<'a> {
    /// Corner radius (clamped to half the shorter side; [`RADIUS_CIRCLE`] for a circle/pill).
    pub radius: i32,
    /// Background color (ignored when `bg_grad` is set).
    pub bg_color: Color,
    /// Background opacity.
    pub bg_opa: Opa,
    /// Background gradient.
    pub bg_grad: Option<&'a Gradient>,
    /// Border color.
    pub border_color: Color,
    /// Border width.
    pub border_width: i32,
    /// Border opacity.
    pub border_opa: Opa,
    /// Border sides.
    pub border_side: BorderSide,
    /// Draw the border after the children ([`Painter::rect_border_post`]) instead of in
    /// [`Painter::rect`].
    pub border_post: bool,
    /// Outline color.
    pub outline_color: Color,
    /// Outline width.
    pub outline_width: i32,
    /// Outline opacity.
    pub outline_opa: Opa,
    /// Gap between the area and the outline.
    pub outline_pad: i32,
    /// Box shadow.
    pub shadow: ShadowDsc,
}

impl Default for RectDsc<'_> {
    fn default() -> Self {
        Self {
            radius: 0,
            bg_color: Color::WHITE,
            bg_opa: Opa::TRANSP,
            bg_grad: None,
            border_color: Color::BLACK,
            border_width: 0,
            border_opa: Opa::TRANSP,
            border_side: BorderSide::FULL,
            border_post: false,
            outline_color: Color::BLACK,
            outline_width: 0,
            outline_opa: Opa::TRANSP,
            outline_pad: 0,
            shadow: ShadowDsc::default(),
        }
    }
}

/// The inner rectangle of a border (LVGL rules: missing sides extend past the outer edge).
pub(crate) fn border_inner(area: Rect, radius: i32, width: i32, side: BorderSide) -> Rect {
    let side = if side.contains(BorderSide::INTERNAL) {
        BorderSide::FULL
    } else {
        side
    };
    let ext = width + radius;
    let d = |s: BorderSide| if side.contains(s) { width } else { -ext };
    area.inset(Insets::new(
        d(BorderSide::LEFT),
        d(BorderSide::TOP),
        d(BorderSide::RIGHT),
        d(BorderSide::BOTTOM),
    ))
}

impl Painter<'_> {
    /// Paints the ring between the rounded rectangles `(outer, ro)` and `(inner, ri)` (or the
    /// whole outer shape when `inner` is `None`) over the rows `rows`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ring(
        &mut self,
        sc: &mut Scratch,
        outer: Rect,
        ro: i32,
        inner: Option<(Rect, i32)>,
        paint: &Paint<'_>,
        opa: Opa,
        mode: BlendMode,
    ) {
        let Some(rows) = outer.intersection(&self.clip()) else {
            return;
        };
        let circle = {
            let c = &mut self.caches_mut().circle;
            c.ensure(ro);
            if let Some((_, ri)) = inner {
                c.ensure(ri);
            }
            take(c)
        };
        let qo = circle.quarter(ro);
        let qi = inner.map(|(r, ri)| (r, circle.quarter(ri)));
        for y in rows.y0..rows.y1 {
            let o = RowSpan::new(outer, &qo, y);
            let i = qi.as_ref().map(|(r, q)| RowSpan::new(*r, q, y));
            self.ring_row(sc, y, &o, i.as_ref(), paint, opa, mode);
        }
        self.caches_mut().circle = circle;
    }

    /// One row of [`ring`](Self::ring): splits the row at every coverage transition, fills the
    /// fully covered pieces directly and computes coverage only for anti-aliased pieces.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ring_row(
        &mut self,
        sc: &mut Scratch,
        y: i32,
        o: &RowSpan<'_>,
        i: Option<&RowSpan<'_>>,
        paint: &Paint<'_>,
        opa: Opa,
        mode: BlendMode,
    ) {
        let clip = self.clip();
        let (lo, hi) = (o.aa_l0.max(clip.x0), o.aa_r1.min(clip.x1));
        if lo >= hi {
            return;
        }
        let mut pts = [0i32; 10];
        let mut n = 0;
        let mut push = |v: i32| {
            if v > lo && v < hi {
                pts[n] = v;
                n += 1;
            }
        };
        push(o.full_l);
        push(o.full_r);
        if let Some(i) = i {
            if !i.is_empty() {
                push(i.aa_l0);
                push(i.full_l);
                push(i.full_r);
                push(i.aa_r1);
            }
        }
        pts[n] = lo;
        pts[n + 1] = hi;
        let pts = &mut pts[..n + 2];
        pts.sort_unstable();
        for w in 0..pts.len() - 1 {
            let (a, b) = (pts[w], pts[w + 1]);
            if a >= b {
                continue;
            }
            let oc = o.class(a);
            let ic = i.map_or(0, |i| i.class(a));
            if oc == 0 || ic == 2 {
                continue;
            }
            if oc == 2 && ic == 0 {
                self.span(sc, y, a, b, paint, opa, mode, None);
            } else {
                let mut f = |x0: i32, buf: &mut [u8]| {
                    for (k, v) in buf.iter_mut().enumerate() {
                        let x = x0 + k as i32;
                        *v = o.cov(x).saturating_sub(i.map_or(0, |i| i.cov(x)));
                    }
                };
                self.span(sc, y, a, b, paint, opa, mode, Some(&mut f));
            }
        }
    }

    /// Draws a rectangle: shadow, background (color or gradient), border (unless
    /// `border_post`), outline — in this order, like LVGL.
    pub fn rect(&mut self, area: Rect, dsc: &RectDsc<'_>) {
        if area.is_empty() {
            return;
        }
        twine_core::trace!(target: "twine::render", "rect {} r {}", area, dsc.radius);
        let r = eff_radius(area, dsc.radius);
        if dsc.shadow.is_visible() {
            self.shadow(area, r, dsc);
        }
        let border = dsc.border_width > 0 && !dsc.border_opa.is_transparent() && !dsc.border_side.is_empty();
        if !dsc.bg_opa.is_transparent() {
            // Under an opaque border only the inside needs a background (inset by width − 1 so
            // the anti-aliased inner edge of the border still has background below it).
            let (bg_area, bg_r) =
                if border && !dsc.border_post && dsc.border_opa.is_cover() && dsc.border_width > 1 {
                    let w = dsc.border_width - 1;
                    let a = border_inner(area, r, w, dsc.border_side);
                    (a.intersection(&area).unwrap_or(Rect::ZERO), (r - w).max(0))
                } else {
                    (area, r)
                };
            if !bg_area.is_empty() {
                self.bg(area, bg_area, eff_radius(bg_area, bg_r), dsc);
            }
        }
        if border && !dsc.border_post {
            self.border(
                area,
                r,
                dsc.border_width,
                dsc.border_side,
                dsc.border_color,
                dsc.border_opa,
            );
        }
        if dsc.outline_width > 0 && !dsc.outline_opa.is_transparent() {
            let ext = dsc.outline_pad + dsc.outline_width;
            let o = area.expand(ext);
            let ro = if r > 0 { r + ext } else { 0 };
            self.border(
                o,
                eff_radius(o, ro),
                dsc.outline_width,
                BorderSide::FULL,
                dsc.outline_color,
                dsc.outline_opa,
            );
        }
    }

    /// Draws only the border of `dsc` (for `border_post`: the engine calls this after the
    /// children).
    pub fn rect_border_post(&mut self, area: Rect, dsc: &RectDsc<'_>) {
        if dsc.border_width > 0 && !dsc.border_opa.is_transparent() && !area.is_empty() {
            let r = eff_radius(area, dsc.radius);
            self.border(
                area,
                r,
                dsc.border_width,
                dsc.border_side,
                dsc.border_color,
                dsc.border_opa,
            );
        }
    }

    /// Fills `area` with a gradient (geometry relative to `area`).
    pub fn fill_gradient(&mut self, area: Rect, grad: &Gradient, opa: Opa) {
        let dsc = RectDsc {
            bg_opa: opa,
            bg_grad: Some(grad),
            ..RectDsc::default()
        };
        if !area.is_empty() {
            self.bg(area, area, 0, &dsc);
        }
    }

    fn bg(&mut self, area: Rect, bg_area: Rect, r: i32, dsc: &RectDsc<'_>) {
        let mut sc = self.take_scratch();
        match dsc.bg_grad {
            None => {
                if r == 0 {
                    self.put_scratch(sc);
                    self.fill(bg_area, dsc.bg_color, dsc.bg_opa);
                    return;
                }
                self.ring(
                    &mut sc,
                    bg_area,
                    r,
                    None,
                    &Paint::Solid(dsc.bg_color),
                    dsc.bg_opa,
                    BlendMode::Normal,
                );
            }
            Some(g) => {
                let mut gc = take(&mut self.caches_mut().gradient);
                let idx = gc.lookup(g);
                let s = GradSampler::new(g, area);
                let paint = Paint::Grad {
                    s: &s,
                    map: gc.map(idx),
                    dither: g.dither && self.is_565(),
                };
                self.ring(&mut sc, bg_area, r, None, &paint, dsc.bg_opa, BlendMode::Normal);
                self.caches_mut().gradient = gc;
            }
        }
        self.put_scratch(sc);
    }

    pub(crate) fn border(
        &mut self,
        area: Rect,
        r: i32,
        width: i32,
        side: BorderSide,
        color: Color,
        opa: Opa,
    ) {
        let inner = border_inner(area, r, width, side);
        let ri = eff_radius(inner, (r - width).max(0));
        let mut sc = self.take_scratch();
        self.ring(
            &mut sc,
            area,
            r,
            Some((inner, ri)),
            &Paint::Solid(color),
            opa,
            BlendMode::Normal,
        );
        self.put_scratch(sc);
    }

    /// Draws the box shadow of `dsc` for the object `area` with effective radius `r`.
    fn shadow(&mut self, area: Rect, r: i32, dsc: &RectDsc<'_>) {
        let sd = dsc.shadow;
        let mut sc = self.take_scratch();
        let mut acc = take(&mut self.caches_mut().acc);
        let mut cache = take(&mut self.caches_mut().shadow);
        let g = cache.fit(area, r, &sd, acc.len());
        let paint = Paint::Solid(sd.color);
        if g.p == 0 {
            self.ring(
                &mut sc,
                g.shape,
                g.radius,
                None,
                &paint,
                sd.opa,
                BlendMode::Normal,
            );
        } else if let Some(rows) = g.bounds.intersection(&self.clip()) {
            let map = cache.corner(&g, &mut acc);
            // Pixels fully covered by an opaque background are skipped.
            let cover = dsc.bg_opa.is_cover() && dsc.bg_grad.is_none();
            let objq = if r > 0 {
                Quarter::Computed { r }
            } else {
                Quarter::Square
            };
            let (b, qw, qh) = (g.bounds, g.qw, g.qh);
            let (bw, bh) = (b.width(), b.height());
            let mxf = move |x: i32| -> usize {
                let xr = x - b.x0;
                (if xr < qw {
                    xr
                } else if xr >= bw - qw {
                    bw - 1 - xr
                } else {
                    qw - 1
                }) as usize
            };
            for y in rows.y0..rows.y1 {
                let yr = y - b.y0;
                let my = if yr < qh {
                    yr
                } else if yr >= bh - qh {
                    bh - 1 - yr
                } else {
                    qh - 1
                } as usize;
                let row = &map[my * qw as usize..(my + 1) * qw as usize];
                let (s0, s1) = if cover {
                    let rs = RowSpan::new(area, &objq, y);
                    if rs.full_l < rs.full_r {
                        (rs.full_l, rs.full_r)
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };
                let mid = (b.x0 + qw, b.x1 - qw);
                let pieces = [(b.x0, mid.0.min(b.x1)), (mid.0, mid.1), (mid.1.max(mid.0), b.x1)];
                for (k, &(a0, a1)) in pieces.iter().enumerate() {
                    for (p0, p1) in minus(a0, a1, s0, s1) {
                        if k == 1 {
                            let v = row[qw as usize - 1];
                            if v != 0 {
                                let o = Opa(udiv255(u32::from(v) * u32::from(sd.opa.0)) as u8);
                                self.span(&mut sc, y, p0, p1, &paint, o, BlendMode::Normal, None);
                            }
                        } else {
                            let mut f = |x0: i32, buf: &mut [u8]| {
                                for (i, v) in buf.iter_mut().enumerate() {
                                    *v = row[mxf(x0 + i as i32)];
                                }
                            };
                            self.span(
                                &mut sc,
                                y,
                                p0,
                                p1,
                                &paint,
                                sd.opa,
                                BlendMode::Normal,
                                Some(&mut f),
                            );
                        }
                    }
                }
            }
        }
        self.caches_mut().shadow = cache;
        self.caches_mut().acc = acc;
        self.put_scratch(sc);
    }
}

/// `[a0, a1)` minus `[s0, s1)` as up to two intervals (empty ones have `start >= end`).
fn minus(a0: i32, a1: i32, s0: i32, s1: i32) -> impl Iterator<Item = (i32, i32)> {
    let (l, r) = if s0 >= s1 || s1 <= a0 || s0 >= a1 {
        ((a0, a1), (0, 0))
    } else {
        ((a0, s0.min(a1)), (s1.max(a0), a1))
    };
    [l, r].into_iter().filter(|(x, y)| x < y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_inner_radius_clamps_to_zero() {
        let area = Rect::from_xywh(0, 0, 100, 50);
        let inner = border_inner(area, 5, 8, BorderSide::FULL);
        assert_eq!(inner, Rect::new(8, 8, 92, 42));
        assert_eq!(eff_radius(inner, 0), 0);
        // A missing side extends the inner rectangle past the outer edge.
        let inner = border_inner(area, 5, 8, BorderSide::TOP);
        assert!(inner.x0 < 0 && inner.x1 > 100 && inner.y0 == 8 && inner.y1 > 50);
        // Width ≥ half the size: empty inner rectangle (filled shape).
        assert!(border_inner(area, 0, 30, BorderSide::FULL).is_empty());
    }

    #[test]
    fn minus_intervals() {
        let v: alloc::vec::Vec<_> = minus(0, 10, 3, 5).collect();
        assert_eq!(v, [(0, 3), (5, 10)]);
        let v: alloc::vec::Vec<_> = minus(0, 10, 20, 30).collect();
        assert_eq!(v, [(0, 10)]);
        assert_eq!(minus(4, 6, 0, 10).count(), 0);
    }
}
