//! Pixel blits: [`Painter::blit`] (translation only) and [`Painter::blit_transformed`]
//! (inverse-mapped rotation / scale / skew with nearest or bilinear sampling), plus
//! [`BlitDsc`].
//!
//! `blit_transformed` inverts the transform once, then walks every destination row with a
//! 16.16 fixed-point source position that is stepped per pixel (no per-pixel matrix product).
//! Bilinear sampling interpolates the four neighbouring texels with 8-bit weights, weighting
//! colors by alpha; texels outside the image are transparent, which anti-aliases the edges.

use core::num::NonZeroI64;
use core::sync::atomic::AtomicBool;

use twine_core::{Color, ColorFormat, Opa, Point, Rect, Transform};

use crate::blend::{BlendMode, Source};
use crate::dispatch::warn_once;
use crate::image::ImageMasks;
use crate::image::read::{Texel, TexelOps, recolor, with_texel};
use crate::num::{advance, sat32, sat64, span};
use crate::painter::Paint;
use crate::{AccelResult, ImagePixels, Painter};

/// Parameters of [`Painter::blit_transformed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlitDsc {
    /// Maps source pixel coordinates to screen coordinates (include the image position).
    pub transform: Transform,
    /// Overall opacity.
    pub opa: Opa,
    /// Bilinear filtering (else nearest neighbour).
    pub antialias: bool,
    /// Blend mode.
    pub blend_mode: BlendMode,
    /// Mixes sampled colors towards this color by this opacity. Alpha-only sources (`A1`…`A8`)
    /// are black unless recolored.
    pub recolor: Option<(Color, Opa)>,
}

impl Default for BlitDsc {
    fn default() -> Self {
        Self {
            transform: Transform::IDENTITY,
            opa: Opa::COVER,
            antialias: true,
            blend_mode: BlendMode::Normal,
            recolor: None,
        }
    }
}

/// `Σ f(t_i) · w_i` for 8-bit `f` values and bilinear weights summing to 2^16 (below 2^24).
#[inline(always)]
fn weighted(t: &[u32; 4], w: &[u32; 4], f: impl Fn(u32) -> u32) -> u32 {
    // SAFE-ARITH: `f(t_i) ≤ 255` and the weights sum to 2^16.
    t.iter()
        .zip(w)
        .fold(0u32, |s, (&v, &wi)| s.wrapping_add(f(v).wrapping_mul(wi)))
}

/// Bilinear blend of four packed texels with weights `fx`, `fy` in `0..=256`.
///
/// Out of line: it does not depend on the texel format, so the sampler of every format
/// shares one copy (one call per interpolated sample) instead of inlining it into each of
/// them (flash).
#[inline(never)]
fn bilinear(t: [u32; 4], fx: u32, fy: u32) -> u32 {
    // SAFE-ARITH: `fx, fy ≤ 256`, so the four weights are at most 2^16 and sum to 2^16; a
    // weighted sum of 8-bit values is at most `255 · 2^16 < 2^24`, and with an alpha factor
    // at most `255 · 255 · 2^16 < 2^32` (in `u64` anyway).
    let (ix, iy) = (256u32.wrapping_sub(fx), 256u32.wrapping_sub(fy));
    let w = [
        ix.wrapping_mul(iy),
        fx.wrapping_mul(iy),
        ix.wrapping_mul(fy),
        fx.wrapping_mul(fy),
    ];
    let a = weighted(&t, &w, |v| v >> 24);
    if a == 0 {
        return 0;
    }
    // SAFE-ARITH: `a < 2^24`, see above.
    let out_a = a.wrapping_add(32768) >> 16;
    let opaque = t.iter().all(|v| v >> 24 == 255);
    let ch = |s: u32| -> u32 {
        if opaque {
            // SHIFT: `s` is one of the literal channel offsets 16, 8, 0 passed to `ch` below.
            weighted(&t, &w, |v| (v >> s) & 0xFF).wrapping_add(32768) >> 16
        } else {
            let v = t.iter().zip(&w).fold(0u64, |sum, (&v, &wi)| {
                // SHIFT: `s` is one of the literal channel offsets 16, 8, 0 passed to `ch` below.
                let c = u64::from((v >> s) & 0xFF);
                sum.wrapping_add(c.wrapping_mul(u64::from((v >> 24).wrapping_mul(wi))))
            });
            // `a != 0` (returned above): the division's fallback is never taken.
            (v.wrapping_add(u64::from(a) / 2)
                .checked_div(u64::from(a))
                .unwrap_or(0))
            .min(255) as u32
        }
    };
    (out_a.min(255) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Fills `out` with `n` transformed samples; `sx`, `sy` are the 16.16 source coordinates of
/// the first destination pixel center, stepped by `(ax, bx)` per pixel.
#[allow(clippy::too_many_arguments)]
fn sample_row<T: Texel>(
    img: &ImagePixels<'_>,
    mut sx: i64,
    mut sy: i64,
    ax: i64,
    bx: i64,
    n: usize,
    aa: bool,
    ops: TexelOps,
    out: &mut [u8],
) {
    let (w, h) = (i64::from(img.w), i64::from(img.h));
    // SAFE-ARITH: `|sx|, |sy| < 2^51` for the samples of a row (see `blit_transformed_with`),
    // so the half-texel offsets and `x0 + 1`, `y0 + 1` cannot overflow, and neither can the
    // steps (the positions stay below 2^51 + 2^47).
    for px in out.chunks_exact_mut(4).take(n) {
        let v = if aa {
            let (u, v) = (sx.wrapping_sub(32768), sy.wrapping_sub(32768));
            let (x0, y0) = (u >> 16, v >> 16);
            let (x1, y1) = (x0.wrapping_add(1), y0.wrapping_add(1));
            let (fx, fy) = (((u >> 8) & 0xFF) as u32, ((v >> 8) & 0xFF) as u32);
            let get = |x: i64, y: i64| -> u32 {
                if x < 0 || y < 0 || x >= w || y >= h {
                    0
                } else {
                    ops.key(T::get(img, x as usize, y as usize))
                }
            };
            if fx == 0 && fy == 0 {
                get(x0, y0)
            } else {
                bilinear([get(x0, y0), get(x1, y0), get(x0, y1), get(x1, y1)], fx, fy)
            }
        } else {
            let (x, y) = (sx >> 16, sy >> 16);
            if x < 0 || y < 0 || x >= w || y >= h {
                0
            } else {
                ops.key(T::get(img, x as usize, y as usize))
            }
        };
        px.copy_from_slice(&recolor(v, ops.recolor).to_le_bytes());
        sx = sx.wrapping_add(ax);
        sy = sy.wrapping_add(bx);
    }
}

/// `⌊a / b⌋` (`b` is non-zero by its type; callers pass `|a| < 2^53`).
#[inline]
fn div_floor(a: i64, b: NonZeroI64) -> i64 {
    // `b != 0` and `a > i64::MIN`: the fallbacks are never taken.
    let q = a.checked_div_euclid(b.get()).unwrap_or(0);
    let r = a.checked_rem_euclid(b.get()).unwrap_or(0);
    // SAFE-ARITH: `|q| ≤ |a| < 2^53`.
    q.wrapping_sub(i64::from(b.get() < 0 && r != 0))
}

/// The sub-range `[i0, i1)` of `0..n` where `lo ≤ s0 + i·st < hi` (`|s0| ≤ 2^50`, `|lo|,
/// |hi| < 2^34`).
fn axis_range(s0: i64, st: i64, lo: i64, hi: i64, n: i64) -> (i64, i64) {
    let Some(st) = NonZeroI64::new(st) else {
        return if s0 >= lo && s0 < hi { (0, n) } else { (0, 0) };
    };
    // Conservative bounds (one extra pixel each side; samples outside are transparent).
    // SAFE-ARITH: the differences are below 2^51 and the quotients no larger.
    let (dl, dh) = (lo.wrapping_sub(s0), hi.wrapping_sub(s0));
    let (a, b) = if st.get() > 0 {
        (
            div_floor(dl, st).wrapping_sub(1),
            div_floor(dh, st).wrapping_add(2),
        )
    } else {
        (
            div_floor(dh, st).wrapping_sub(1),
            div_floor(dl, st).wrapping_add(2),
        )
    };
    (a.clamp(0, n), b.clamp(0, n))
}

/// Whether `format` can be blended directly as a [`Source::Pixels`].
pub(crate) fn is_direct(format: ColorFormat) -> bool {
    matches!(
        format,
        ColorFormat::Rgb565
            | ColorFormat::Rgb565Swapped
            | ColorFormat::Rgb888
            | ColorFormat::Xrgb8888
            | ColorFormat::Argb8888
            | ColorFormat::L8
    )
}

impl Painter<'_> {
    /// Draws `src` with its top-left corner at `pos` (no transform). Same-format opaque copies
    /// use `copy_from_slice`; unmasked, fully visible normal blits try the accelerator first.
    pub fn blit(&mut self, pos: Point, src: &ImagePixels<'_>, opa: Opa, blend_mode: BlendMode) {
        self.blit_recolor(pos, src, opa, blend_mode, None);
    }

    /// [`blit`](Self::blit) with an optional recolor.
    pub fn blit_recolor(
        &mut self,
        pos: Point,
        src: &ImagePixels<'_>,
        opa: Opa,
        blend_mode: BlendMode,
        rc: Option<(Color, Opa)>,
    ) {
        if opa.is_transparent() || src.w == 0 || src.h == 0 {
            return;
        }
        let full = src.rect_at(pos.x, pos.y);
        let Some(area) = full.intersection(&self.clip()) else {
            return;
        };
        if area == full && !self.has_masks() && blend_mode == BlendMode::Normal && rc.is_none() {
            if let Some(r) = self.accel_blit(full, src, opa) {
                if r {
                    return;
                }
            }
        }
        let mut sc = self.take_scratch();
        let direct = is_direct(src.format) && rc.is_none() && !src.is_premultiplied();
        let bpp = usize::from(src.format.bpp()) / 8;
        // `area` lies inside the image rectangle at `pos`: offsets below 2^16.
        let ix0 = span(pos.x, area.x0);
        for y in area.y0..area.y1 {
            let iy = span(pos.y, y);
            if direct {
                let row = src.row(u16::try_from(iy).unwrap_or(u16::MAX));
                let s = Source::Pixels {
                    data: row.get(ix0.saturating_mul(bpp)..).unwrap_or_default(),
                    format: src.format,
                };
                self.span(
                    &mut sc,
                    y,
                    area.x0,
                    area.x1,
                    &Paint::Src { src: s, x0: area.x0 },
                    opa,
                    blend_mode,
                    None,
                );
            } else {
                let chunk = sc.cov.len().max(1);
                let mut x = area.x0;
                while x < area.x1 {
                    let e = advance(x, chunk, area.x1);
                    let n = span(x, e);
                    let mut buf = core::mem::take(&mut sc.span);
                    let ops = TexelOps {
                        recolor: rc,
                        chroma: None,
                    };
                    with_texel!(src, T => convert_row::<T>(src, span(pos.x, x), iy, n, ops, &mut buf));
                    // SAFE-ARITH: `n ≤ chunk ≤ max_span ≤ 65_535`. SAFE-INDEX: the span scratch
                    // holds `4 · max_span` bytes; the fallback is never taken.
                    let argb = buf.get(..n.wrapping_mul(4)).unwrap_or_default();
                    self.blend_masked_span(y, x, e, Source::Argb(argb), None, opa, blend_mode);
                    sc.span = buf;
                    x = e;
                }
            }
        }
        self.put_scratch(sc);
    }

    /// Draws `src` mapped by `dsc.transform` (source pixel coordinates → screen). A singular
    /// transform draws nothing.
    pub fn blit_transformed(&mut self, src: &ImagePixels<'_>, dsc: &BlitDsc) {
        if dsc.opa.is_transparent() || src.w == 0 || src.h == 0 {
            return;
        }
        let t = dsc.transform;
        if t.is_translation_only() && t.tx.frac() == 0 && t.ty.frac() == 0 {
            let pos = Point::new(t.tx.to_int_floor(), t.ty.to_int_floor());
            return self.blit_recolor(pos, src, dsc.opa, dsc.blend_mode, dsc.recolor);
        }
        let ops = TexelOps {
            recolor: dsc.recolor,
            chroma: None,
        };
        self.blit_transformed_with(src, dsc, ops, None);
    }

    /// The inverse-mapping sampler behind [`blit_transformed`](Self::blit_transformed) and
    /// transformed [`image`](Self::image)s: `ops` recolors / keys texels, `masks` adds
    /// destination-space coverage (clip radius, bitmap mask).
    pub(crate) fn blit_transformed_with(
        &mut self,
        src: &ImagePixels<'_>,
        dsc: &BlitDsc,
        ops: TexelOps,
        masks: Option<&ImageMasks<'_>>,
    ) {
        let t = dsc.transform;
        let Some(inv) = t.invert() else {
            twine_core::debug!(target: "twine::render", "blit_transformed: singular transform, nothing drawn");
            return;
        };
        let img = Rect::from_xywh(0, 0, i32::from(src.w), i32::from(src.h));
        let Some(mut area) = t.map_rect_bounds(img).expand(1).intersection(&self.clip()) else {
            return;
        };
        if let Some(m) = masks {
            match m.bounds().and_then(|b| b.intersection(&area)) {
                Some(a) => area = a,
                None => return,
            }
        }
        let (a, b, c, d) = (
            i64::from(inv.a.raw()),
            i64::from(inv.b.raw()),
            i64::from(inv.c.raw()),
            i64::from(inv.d.raw()),
        );
        let (tx, ty) = (i64::from(inv.tx.raw()), i64::from(inv.ty.raw()));
        let (w16, h16) = (i64::from(src.w) << 16, i64::from(src.h) << 16);
        // Samples are drawn where the source position is inside the image (bilinear: within
        // half a texel outside, where the edge fades out).
        let m: i64 = if dsc.antialias { 32768 } else { 0 };
        let mut sc = self.take_scratch();
        let chunk = sc.cov.len().max(1);
        // Range argument: the inverse coefficients are `i32`s (`< 2^31`) and the doubled pixel
        // centers below 2^33, so the products are formed in `i128`; a row's start position is
        // clamped to `±2^50` (16.16, i.e. 2^34 texels: farther starts stay outside the image
        // for the whole row, as `axis_range` finds), and in-image samples are within `2^33` of
        // the image, so `sx0 + a·i` for `i` in the visible range stays below 2^51.
        let lim = 1i128 << 50;
        let row_start = |k1: i64, px: i64, k2: i64, py: i64, t: i64| -> i64 {
            let v = (i128::from(k1).wrapping_mul(i128::from(px)))
                .wrapping_add(i128::from(k2).wrapping_mul(i128::from(py)))
                / 2;
            sat64(v.wrapping_add(i128::from(t)).clamp(lim.wrapping_neg(), lim))
        };
        let n = i64::from(area.width());
        for y in area.y0..area.y1 {
            // Source position of the destination pixel center (x0 + ½, y + ½).
            // SAFE-ARITH: doubled `i32`s plus one fit in `i64`.
            let (px, py) = (
                i64::from(area.x0).wrapping_mul(2).wrapping_add(1),
                i64::from(y).wrapping_mul(2).wrapping_add(1),
            );
            let sx0 = row_start(a, px, c, py, tx);
            let sy0 = row_start(b, px, d, py, ty);
            // SAFE-ARITH: `m ≤ 2^15` and `w16, h16 ≤ 2^32`.
            let (xa, xb) = axis_range(sx0, a, m.wrapping_neg(), w16.wrapping_add(m), n);
            let (ya, yb) = axis_range(sy0, b, m.wrapping_neg(), h16.wrapping_add(m), n);
            let (i0, i1) = (xa.max(ya), xb.min(yb));
            // `0 ≤ i0 ≤ i1 ≤ n = area.width()`: the screen columns `area.x0 + i` fit in
            // `i32`.
            let (xs, xe) = (
                sat32(i64::from(area.x0).wrapping_add(i0)),
                sat32(i64::from(area.x0).wrapping_add(i1)),
            );
            let mut x = xs;
            while x < xe {
                let e = advance(x, chunk, xe);
                let k = span(x, e);
                // SAFE-ARITH: `i0 ≤ i < i1 ≤ 2^32` and `|a|, |b| < 2^31`: see above for the
                // sum.
                let i = i64::from(x).wrapping_sub(i64::from(area.x0));
                // SAFE-INDEX: `k ≤ chunk` (the scratch sizes); the fallbacks are never taken.
                let Some(cov) = sc.cov.get_mut(..k) else {
                    break;
                };
                let visible = match masks {
                    Some(mk) => mk.fill(y, x, cov),
                    None => true,
                };
                if visible {
                    with_texel!(src, T => sample_row::<T>(
                        src,
                        sx0.wrapping_add(a.wrapping_mul(i)),
                        sy0.wrapping_add(b.wrapping_mul(i)),
                        a,
                        b,
                        k,
                        dsc.antialias,
                        ops,
                        &mut sc.span
                    ));
                    let cov = if masks.is_some() { Some(cov) } else { None };
                    // SAFE-ARITH: `k ≤ max_span ≤ 65_535`.
                    let argb = sc.span.get(..k.wrapping_mul(4)).unwrap_or_default();
                    self.blend_masked_span(y, x, e, Source::Argb(argb), cov, dsc.opa, dsc.blend_mode);
                }
                x = e;
            }
        }
        self.put_scratch(sc);
    }
}

static WARN_FORMAT: AtomicBool = AtomicBool::new(false);

/// Warns once about an image format the arc/blit direct path cannot read.
pub(crate) fn warn_unsupported_image(format: ColorFormat) {
    if warn_once(&WARN_FORMAT) {
        twine_core::warn!(target: "twine::render", "image format {} not supported here; nothing drawn", format);
    }
}

/// Whether `format` is readable by [`Source::Pixels`].
pub(crate) fn is_direct_format(format: ColorFormat) -> bool {
    is_direct(format)
}

impl Painter<'_> {
    /// Tries `DrawAccel::blit`; `Some(true)` when done/queued.
    pub(crate) fn accel_blit(&mut self, area: Rect, src: &ImagePixels<'_>, opa: Opa) -> Option<bool> {
        let r = self.accel_op(area, |acc, buf| acc.blit(buf, area, src, opa))?;
        Some(r != AccelResult::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::read::pack;

    #[test]
    fn axis_range_bounds() {
        // s = 10 + 2i in [0, 20): i in [0, 5)
        let (a, b) = axis_range(10, 2, 0, 20, 100);
        assert!(a == 0 && (5..=7).contains(&b));
        let (a, b) = axis_range(10, 0, 0, 20, 100);
        assert_eq!((a, b), (0, 100));
        let (a, b) = axis_range(30, 0, 0, 20, 100);
        assert_eq!((a, b), (0, 0));
        let (a, b) = axis_range(30, -2, 0, 20, 100);
        assert!(a <= 6 && b >= 15);
    }

    #[test]
    fn bilinear_weights() {
        let t = [
            pack(Color::WHITE, 255),
            pack(Color::BLACK, 255),
            pack(Color::WHITE, 255),
            pack(Color::BLACK, 255),
        ];
        let v = bilinear(t, 128, 0);
        assert_eq!(v >> 24, 255);
        assert_eq!((v >> 16) & 0xFF, 128);
        // A transparent neighbour does not darken the color.
        let t = [pack(Color::WHITE, 255), 0, pack(Color::WHITE, 255), 0];
        let v = bilinear(t, 128, 0);
        assert_eq!(v & 0xFF_FFFF, 0xFF_FFFF);
        assert_eq!(v >> 24, 128);
    }
}
