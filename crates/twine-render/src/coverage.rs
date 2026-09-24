//! [`SpanSource`] and [`Painter::coverage_span`]: blending externally computed coverage rows
//! (the vector rasterizer's output) through the clip, the mask stack and [`blend_span`].
//!
//! [`blend_span`]: crate::blend_span

use twine_core::{Color, Opa};

use crate::Painter;
use crate::blend::{BlendMode, Source};

/// Fills `out` (straight-alpha `Argb8888` bytes, memory `B, G, R, A`, `out.len() / 4` pixels)
/// with the colors of row `y` starting at screen x `x0`.
pub type PixelFn<'s> = &'s dyn Fn(i32, i32, &mut [u8]);

/// What a coverage span is painted with.
#[derive(Clone, Copy)]
pub enum SpanSource<'s> {
    /// One color at an opacity.
    Solid(Color, Opa),
    /// Colors computed per pixel (gradients, image patterns) at an opacity. The function is
    /// called once per chunk of at most [`RenderCaches::max_span`](crate::RenderCaches::max_span)
    /// pixels, only for the part of the span inside the clip.
    Pixels(PixelFn<'s>, Opa),
}

impl core::fmt::Debug for SpanSource<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpanSource::Solid(c, o) => f.debug_tuple("Solid").field(c).field(o).finish(),
            SpanSource::Pixels(_, o) => f.debug_tuple("Pixels").field(&"<fn>").field(o).finish(),
        }
    }
}

impl Painter<'_> {
    /// Blends the coverage row `coverage` (one byte per pixel, 255 = fully covered) starting at
    /// screen `(x0, y)` with `src`, after clipping and applying the mask stack.
    ///
    /// This is the output stage of the vector rasterizer: it computes anti-aliased coverage
    /// itself and hands every row here. Drawing never allocates.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_render::{DrawBuf, Painter, RenderCaches, SpanSource};
    ///
    /// let mut caches = RenderCaches::default();
    /// let mut data = vec![0u8; 4];
    /// let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 4, 1)).unwrap();
    /// let mut p = Painter::new(buf, &mut caches);
    /// p.coverage_span(0, 1, &[255, 128], &SpanSource::Solid(Color::WHITE, Opa::COVER));
    /// drop(p); // the buffer is read after the painter (and any accelerator) is done
    /// assert_eq!(data, [0, 255, 128, 0]);
    /// ```
    pub fn coverage_span(&mut self, y: i32, x0: i32, coverage: &[u8], src: &SpanSource<'_>) {
        self.coverage_span_mode(y, x0, coverage, src, BlendMode::Normal);
    }

    /// [`coverage_span`](Self::coverage_span) with a blend mode.
    pub fn coverage_span_mode(
        &mut self,
        y: i32,
        x0: i32,
        coverage: &[u8],
        src: &SpanSource<'_>,
        mode: BlendMode,
    ) {
        let clip = self.clip();
        if y < clip.y0 || y >= clip.y1 || coverage.is_empty() {
            return;
        }
        let opa = match *src {
            SpanSource::Solid(_, o) | SpanSource::Pixels(_, o) => o,
        };
        if opa.is_transparent() {
            return;
        }
        let x1 = x0.saturating_add(i32::try_from(coverage.len()).unwrap_or(i32::MAX));
        let a0 = x0.max(clip.x0);
        let a1 = x1.min(clip.x1);
        if a0 >= a1 {
            return;
        }
        let mut sc = self.take_scratch();
        let chunk = sc.cov.len().max(1) as i32;
        let mut a = a0;
        while a < a1 {
            let b = (a + chunk).min(a1);
            let k = (b - a) as usize;
            let off = (a - x0) as usize;
            let cov = &coverage[off..off + k];
            // Skip chunks with no coverage at all (common at the ends of vector rows).
            if cov.iter().any(|&c| c != 0) {
                let buf = &mut sc.cov[..k];
                buf.copy_from_slice(cov);
                let s = match *src {
                    SpanSource::Solid(c, _) => Source::Solid(c),
                    SpanSource::Pixels(f, _) => {
                        let out = &mut sc.span[..k * 4];
                        f(y, a, out);
                        Source::Argb(out)
                    }
                };
                self.blend_masked_span(y, a, b, s, Some(buf), opa, mode);
            }
            a = b;
        }
        self.put_scratch(sc);
    }
}

#[cfg(test)]
mod tests {
    use twine_core::{ColorFormat, Rect};

    use super::*;
    use crate::{DrawBuf, RenderCaches, RenderConfig};

    #[test]
    fn clipped_and_chunked() {
        let mut caches = RenderCaches::new(&RenderConfig {
            max_span: 16,
            ..RenderConfig::default()
        });
        let mut data = [0u8; 40];
        {
            let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 40, 1)).unwrap();
            let mut p = Painter::new(buf, &mut caches);
            let cov: [u8; 50] = core::array::from_fn(|i| (i * 5) as u8);
            let f = |_y: i32, x0: i32, out: &mut [u8]| {
                for (i, px) in out.chunks_exact_mut(4).enumerate() {
                    let v = (x0 as usize + i) as u8;
                    px.copy_from_slice(&[v, v, v, 255]);
                }
            };
            p.with_clip(Rect::from_xywh(2, 0, 30, 1), |p| {
                p.coverage_span(0, -5, &cov, &SpanSource::Pixels(&f, Opa::COVER));
            });
            assert_eq!(p.touched_area(), Some(Rect::new(2, 0, 32, 1)));
        }
        assert_eq!(data[..2], [0, 0]);
        assert_eq!(data[32..], [0; 8]);
        // Pixel x = 20: coverage index 25 (125), color 20 → 20·125/255 ≈ 9.
        assert!(data[20].abs_diff(9) <= 1, "{}", data[20]);
    }
}
