//! Pixel blits: [`Painter::blit`] (translation only) and [`Painter::blit_transformed`]
//! (inverse-mapped rotation / scale / skew with nearest or bilinear sampling), plus
//! [`BlitDsc`].
//!
//! `blit_transformed` inverts the transform once, then walks every destination row with a
//! 16.16 fixed-point source position that is stepped per pixel (no per-pixel matrix product).
//! Bilinear sampling interpolates the four neighbouring texels with 8-bit weights, weighting
//! colors by alpha; texels outside the image are transparent, which anti-aliases the edges.

use core::sync::atomic::AtomicBool;

use twine_core::{Color, ColorFormat, Opa, Point, Rect, Transform};

use crate::blend::{BlendMode, Source};
use crate::dispatch::warn_once;
use crate::image::ImageMasks;
use crate::image::read::{Texel, TexelOps, recolor, with_texel};
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

/// Bilinear blend of four packed texels with weights `fx`, `fy` in `0..=256`.
#[inline(always)]
fn bilinear(t: [u32; 4], fx: u32, fy: u32) -> u32 {
    let w = [(256 - fx) * (256 - fy), fx * (256 - fy), (256 - fx) * fy, fx * fy];
    let a: u32 = (0..4).map(|i| (t[i] >> 24) * w[i]).sum();
    if a == 0 {
        return 0;
    }
    let out_a = (a + 32768) >> 16;
    let ch = |s: u32| -> u32 {
        if t.iter().all(|v| v >> 24 == 255) {
            let v: u32 = (0..4).map(|i| ((t[i] >> s) & 0xFF) * w[i]).sum();
            (v + 32768) >> 16
        } else {
            let v: u64 = (0..4)
                .map(|i| u64::from((t[i] >> s) & 0xFF) * u64::from((t[i] >> 24) * w[i]))
                .sum();
            ((v + u64::from(a) / 2) / u64::from(a)).min(255) as u32
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
    for px in out[..n * 4].chunks_exact_mut(4) {
        let v = if aa {
            let (u, v) = (sx - 32768, sy - 32768);
            let (x0, y0) = (u >> 16, v >> 16);
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
                bilinear(
                    [get(x0, y0), get(x0 + 1, y0), get(x0, y0 + 1), get(x0 + 1, y0 + 1)],
                    fx,
                    fy,
                )
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
        sx += ax;
        sy += bx;
    }
}

/// Floor and ceiling divisions for `i64`.
#[inline]
fn div_floor(a: i64, b: i64) -> i64 {
    a.div_euclid(b) - i64::from(b < 0 && a.rem_euclid(b) != 0)
}

/// The sub-range `[i0, i1)` of `0..n` where `lo ≤ s0 + i·st < hi`.
fn axis_range(s0: i64, st: i64, lo: i64, hi: i64, n: i64) -> (i64, i64) {
    if st == 0 {
        return if s0 >= lo && s0 < hi { (0, n) } else { (0, 0) };
    }
    // Conservative bounds (one extra pixel each side; samples outside are transparent).
    let (a, b) = if st > 0 {
        (div_floor(lo - s0, st) - 1, div_floor(hi - s0, st) + 2)
    } else {
        (div_floor(hi - s0, st) - 1, div_floor(lo - s0, st) + 2)
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
        for y in area.y0..area.y1 {
            let iy = (y - pos.y) as usize;
            if direct {
                let row = src.row(iy as u16);
                let s = Source::Pixels {
                    data: &row[((area.x0 - pos.x) as usize * bpp).min(row.len())..],
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
                let chunk = sc.cov.len() as i32;
                let mut x = area.x0;
                while x < area.x1 {
                    let e = (x + chunk).min(area.x1);
                    let n = (e - x) as usize;
                    let mut span = core::mem::take(&mut sc.span);
                    let ops = TexelOps {
                        recolor: rc,
                        chroma: None,
                    };
                    with_texel!(src, T => convert_row::<T>(src, (x - pos.x) as usize, iy, n, ops, &mut span));
                    self.blend_masked_span(y, x, e, Source::Argb(&span[..n * 4]), None, opa, blend_mode);
                    sc.span = span;
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
            i64::from(inv.a.0),
            i64::from(inv.b.0),
            i64::from(inv.c.0),
            i64::from(inv.d.0),
        );
        let (tx, ty) = (i64::from(inv.tx.0), i64::from(inv.ty.0));
        let (w16, h16) = (i64::from(src.w) << 16, i64::from(src.h) << 16);
        // Samples are drawn where the source position is inside the image (bilinear: within
        // half a texel outside, where the edge fades out).
        let m = if dsc.antialias { 32768 } else { 0 };
        let mut sc = self.take_scratch();
        let chunk = sc.cov.len() as i64;
        for y in area.y0..area.y1 {
            // Source position of the destination pixel center (x0 + ½, y + ½).
            let (px, py) = (2 * i64::from(area.x0) + 1, 2 * i64::from(y) + 1);
            let sx0 = (a * px + c * py) / 2 + tx;
            let sy0 = (b * px + d * py) / 2 + ty;
            let n = i64::from(area.width());
            let (xa, xb) = axis_range(sx0, a, -m, w16 + m, n);
            let (ya, yb) = axis_range(sy0, b, -m, h16 + m, n);
            let (i0, i1) = (xa.max(ya), xb.min(yb));
            let mut i = i0;
            while i < i1 {
                let e = (i + chunk).min(i1);
                let k = (e - i) as usize;
                let x = area.x0 + i as i32;
                let visible = match masks {
                    Some(mk) => mk.fill(y, x, &mut sc.cov[..k]),
                    None => true,
                };
                if visible {
                    with_texel!(src, T => sample_row::<T>(
                        src, sx0 + a * i, sy0 + b * i, a, b, k, dsc.antialias, ops, &mut sc.span
                    ));
                    let cov = if masks.is_some() {
                        Some(&mut sc.cov[..k])
                    } else {
                        None
                    };
                    self.blend_masked_span(
                        y,
                        x,
                        x + k as i32,
                        Source::Argb(&sc.span[..k * 4]),
                        cov,
                        dsc.opa,
                        dsc.blend_mode,
                    );
                }
                i = e;
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
