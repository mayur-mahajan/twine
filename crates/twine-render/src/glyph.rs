//! Glyph blending for text: A8 coverage ([`Painter::glyph_a8`]) and subpixel (LCD) coverage
//! ([`Painter::glyph_lcd`]).

use core::mem::take;

use twine_core::color::mix_channel;
use twine_core::{Color, ColorFormat, Opa, PixelFormat, Rect};

use crate::blend::{BlendMode, Source};
use crate::dispatch::dispatch_format;
use crate::mask::MaskResult;
use crate::{AccelResult, Painter};

/// Channel order of the three coverage values per pixel of subpixel glyphs (the panel's
/// subpixel layout).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SubpxOrder {
    /// Red, green, blue (left to right, or top to bottom for vertical layouts).
    #[default]
    Rgb,
    /// Blue, green, red.
    Bgr,
}

#[inline]
fn scale(v: u8, opa: Opa) -> u8 {
    if opa.0 == 255 {
        v
    } else {
        twine_core::math::udiv255(u32::from(v) * u32::from(opa.0)) as u8
    }
}

/// Per-channel blend of one LCD row into pixels of format `F`.
fn lcd_row<F: PixelFormat>(
    row: &mut [u8],
    cov: &[u8],
    mask: Option<&[u8]>,
    color: Color,
    opa: Opa,
    order: SubpxOrder,
) {
    for (i, px) in row.chunks_exact_mut(F::BYTES).enumerate() {
        let c = &cov[i * 3..i * 3 + 3];
        let (mut cr, cg, mut cb) = (c[0], c[1], c[2]);
        if order == SubpxOrder::Bgr {
            core::mem::swap(&mut cr, &mut cb);
        }
        let m = mask.map_or(opa, |m| Opa(scale(m[i], opa)));
        let (ar, ag, ab) = (scale(cr, m), scale(cg, m), scale(cb, m));
        if ar == 0 && ag == 0 && ab == 0 {
            continue;
        }
        let dst = F::to_color(F::read(px));
        let out = Color::new(
            mix_channel(color.r, dst.r, ar),
            mix_channel(color.g, dst.g, ag),
            mix_channel(color.b, dst.b, ab),
        );
        F::write(px, F::from_color(out));
    }
}

impl Painter<'_> {
    /// Blends an A8 coverage block in `color` at `opa`: `alpha` holds `area.width()` values per
    /// row with a row stride of `stride` bytes. Clipped, masked, never allocates.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_render::{DrawBuf, Painter, RenderCaches};
    ///
    /// let mut caches = RenderCaches::default();
    /// let mut px = vec![0u8; 4];
    /// let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, 4, 1)).unwrap();
    /// let mut p = Painter::new(buf, &mut caches);
    /// p.glyph_a8(Rect::from_xywh(1, 0, 2, 1), &[255, 128], 2, Color::WHITE, Opa::COVER);
    /// assert_eq!(px, [0, 255, 128, 0]);
    /// ```
    pub fn glyph_a8(&mut self, area: Rect, alpha: &[u8], stride: usize, color: Color, opa: Opa) {
        if opa.is_transparent() || area.is_empty() {
            return;
        }
        let w = area.width() as usize;
        if stride < w || alpha.len() < stride * (area.height() as usize - 1) + w {
            debug_assert!(false, "glyph_a8: coverage buffer too small");
            twine_core::warn!(target: "twine::render", "glyph_a8: coverage buffer too small; not drawn");
            return;
        }
        let Some(vis) = area.intersection(&self.clip()) else {
            return;
        };
        if opa.0 == 255
            && vis == area
            && !self.has_masks()
            && area.area() >= crate::ACCEL_MIN_PX
            && self
                .accel_op(area, |acc, buf| acc.blend_a8(buf, area, color, alpha, stride))
                .is_some_and(|r| r != AccelResult::Unsupported)
        {
            return;
        }
        let mut cov = take(&mut self.caches_mut().cov);
        let chunk = cov.len().max(1);
        for y in vis.y0..vis.y1 {
            let row = &alpha[(y - area.y0) as usize * stride..][..w];
            let mut x = vis.x0;
            while x < vis.x1 {
                let n = ((vis.x1 - x) as usize).min(chunk);
                let src = &row[(x - area.x0) as usize..][..n];
                if src.iter().all(|&v| v == 0) {
                    x += n as i32;
                    continue;
                }
                let c = &mut cov[..n];
                c.copy_from_slice(src);
                self.blend_masked_span(
                    y,
                    x,
                    x + n as i32,
                    Source::Solid(color),
                    Some(c),
                    opa,
                    BlendMode::Normal,
                );
                x += n as i32;
            }
        }
        self.caches_mut().cov = cov;
    }

    /// Blends a subpixel (LCD) glyph: `cov` holds 3 coverage values per destination pixel (in
    /// `order`), `area.width() × 3` per row, row stride `stride`. Each channel is mixed
    /// separately: `dst.c = mix(color.c, dst.c, cov_c × opa / 255)`. `L8`, `I1` and `Argb8888`
    /// (layer) buffers use the average of the three values as ordinary coverage.
    pub fn glyph_lcd(
        &mut self,
        area: Rect,
        cov: &[u8],
        stride: usize,
        color: Color,
        opa: Opa,
        order: SubpxOrder,
    ) {
        if opa.is_transparent() || area.is_empty() {
            return;
        }
        let w = area.width() as usize;
        if stride < 3 * w || cov.len() < stride * (area.height() as usize - 1) + 3 * w {
            debug_assert!(false, "glyph_lcd: coverage buffer too small");
            twine_core::warn!(target: "twine::render", "glyph_lcd: coverage buffer too small; not drawn");
            return;
        }
        let Some(vis) = area.intersection(&self.clip()) else {
            return;
        };
        let format = self.buf().format();
        let per_channel = matches!(
            format,
            ColorFormat::Rgb565 | ColorFormat::Rgb565Swapped | ColorFormat::Rgb888 | ColorFormat::Xrgb8888
        ) && crate::is_format_enabled(format);
        let mut mask = take(&mut self.caches_mut().mask);
        let chunk = mask.len().max(1);
        for y in vis.y0..vis.y1 {
            let row = &cov[(y - area.y0) as usize * stride..][..3 * w];
            let mut x = vis.x0;
            while x < vis.x1 {
                let n = ((vis.x1 - x) as usize).min(chunk);
                let off = (x - area.x0) as usize;
                let src = &row[off * 3..(off + n) * 3];
                if src.iter().all(|&v| v == 0) {
                    x += n as i32;
                    continue;
                }
                let m = &mut mask[..n];
                if per_channel {
                    m.fill(255);
                    let r = self.apply_masks(y, x, m);
                    if r != MaskResult::Transparent {
                        let mref = (r == MaskResult::Changed).then_some(&*m);
                        let buf = self.buf_mut();
                        let dst = buf.row_mut(y, x, x + n as i32);
                        dispatch_format!(format, F => lcd_row::<F>(dst, src, mref, color, opa, order));
                        self.mark_touched(Rect::new(x, y, x + n as i32, y + 1));
                    }
                } else {
                    for (i, v) in m.iter_mut().enumerate() {
                        let t = &src[i * 3..i * 3 + 3];
                        *v = ((u16::from(t[0]) + u16::from(t[1]) + u16::from(t[2]) + 1) / 3) as u8;
                    }
                    self.blend_masked_span(
                        y,
                        x,
                        x + n as i32,
                        Source::Solid(color),
                        Some(m),
                        opa,
                        BlendMode::Normal,
                    );
                }
                x += n as i32;
            }
        }
        self.caches_mut().mask = mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawBuf, RenderCaches};

    fn l8(w: i32, h: i32, f: impl FnOnce(&mut Painter<'_>)) -> alloc::vec::Vec<u8> {
        let mut caches = RenderCaches::default();
        let mut px = alloc::vec![0u8; (w * h) as usize];
        let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, w, h)).unwrap();
        f(&mut Painter::new(buf, &mut caches));
        px
    }

    #[test]
    fn glyph_a8_blends_coverage() {
        let px = l8(4, 2, |p| {
            p.glyph_a8(
                Rect::from_xywh(1, 0, 2, 2),
                &[255, 0, 99, 128, 64, 7],
                3,
                Color::WHITE,
                Opa::COVER,
            );
        });
        assert_eq!(px, [0, 255, 0, 0, 0, 128, 64, 0]);
        let px = l8(2, 1, |p| {
            p.glyph_a8(
                Rect::from_xywh(0, 0, 2, 1),
                &[255, 255],
                2,
                Color::WHITE,
                Opa(128),
            )
        });
        assert_eq!(px, [128, 128]);
    }

    #[test]
    fn glyph_a8_respects_clip() {
        let px = l8(4, 2, |p| {
            p.with_clip(Rect::from_xywh(0, 1, 2, 1), |p| {
                p.glyph_a8(
                    Rect::from_xywh(0, 0, 3, 2),
                    &[255; 6],
                    3,
                    Color::WHITE,
                    Opa::COVER,
                );
            });
        });
        assert_eq!(px, [0, 0, 0, 0, 255, 255, 0, 0]);
        // Partly outside the buffer.
        let px = l8(2, 1, |p| {
            p.glyph_a8(
                Rect::from_xywh(-1, 0, 2, 1),
                &[50, 60],
                2,
                Color::WHITE,
                Opa::COVER,
            )
        });
        assert_eq!(px, [60, 0]);
    }

    #[test]
    fn glyph_lcd_channel_blend_math() {
        let mut caches = RenderCaches::default();
        let mut px = alloc::vec![0u8; 2 * 3];
        let buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb888, Rect::from_xywh(0, 0, 2, 1)).unwrap();
        let mut p = Painter::new(buf, &mut caches);
        p.fill(Rect::from_xywh(0, 0, 2, 1), Color::new(0, 100, 200), Opa::COVER);
        // Pixel 0: R full, G half, B none. Pixel 1: nothing.
        p.glyph_lcd(
            Rect::from_xywh(0, 0, 2, 1),
            &[255, 128, 0, 0, 0, 0],
            6,
            Color::WHITE,
            Opa::COVER,
            SubpxOrder::Rgb,
        );
        drop(p);
        // Rgb888 memory order is B, G, R.
        assert_eq!(&px[..3], &[200, mix_channel(255, 100, 128), 255]);
        assert_eq!(&px[3..], &[200, 100, 0]);

        let mut px2 = alloc::vec![0u8; 3];
        let buf = DrawBuf::new_packed(&mut px2, ColorFormat::Rgb888, Rect::from_xywh(0, 0, 1, 1)).unwrap();
        let mut p = Painter::new(buf, &mut caches);
        p.glyph_lcd(
            Rect::from_xywh(0, 0, 1, 1),
            &[255, 0, 0],
            3,
            Color::WHITE,
            Opa(128),
            SubpxOrder::Bgr,
        );
        drop(p);
        // BGR: the first value is blue; opa 128 halves it.
        assert_eq!(px2, [128, 0, 0]);

        // L8 falls back to average coverage.
        let px = l8(1, 1, |p| {
            p.glyph_lcd(
                Rect::from_xywh(0, 0, 1, 1),
                &[255, 255, 0],
                3,
                Color::WHITE,
                Opa::COVER,
                SubpxOrder::Rgb,
            )
        });
        assert_eq!(px, [170]);
    }
}
