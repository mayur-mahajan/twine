//! [`ImagePixels`]: decoded pixels handed to the blitters.

use twine_core::{ColorFormat, Rect};

use crate::RenderError;

/// A borrowed block of decoded pixels.
///
/// The main plane (`data`) holds `h` rows of `stride` bytes. Indexed formats read their
/// `Argb8888` palette from `palette`; `Rgb565A8` reads its alpha plane (`w` bytes per row) from
/// `alpha`, or from the bytes following the main plane when `alpha` is `None`.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_render::ImagePixels;
///
/// let data = [1u8, 2, 3, 4, 5, 6];
/// let img = ImagePixels::new(ColorFormat::L8, 3, 2, &data);
/// assert_eq!(img.row(1), &[4, 5, 6]);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImagePixels<'a> {
    /// Format of `data`.
    pub format: ColorFormat,
    /// Width in pixels.
    pub w: u16,
    /// Height in pixels.
    pub h: u16,
    /// Bytes between rows of `data`.
    pub stride: u16,
    /// Pixel rows.
    pub data: &'a [u8],
    /// Palette of indexed formats (`Argb8888` entries).
    pub palette: Option<&'a [u8]>,
    /// Alpha plane of `Rgb565A8` (`w` bytes per row).
    pub alpha: Option<&'a [u8]>,
    /// The color channels are premultiplied by alpha (`Argb8888`, `Rgb565A8`). Implied by
    /// [`ColorFormat::Argb8888Premultiplied`].
    pub premultiplied: bool,
}

impl<'a> ImagePixels<'a> {
    /// Packed rows of `format` (stride = `w · bpp / 8` rounded up).
    #[must_use]
    pub fn new(format: ColorFormat, w: u16, h: u16, data: &'a [u8]) -> Self {
        Self {
            format,
            w,
            h,
            stride: format.min_stride(w),
            data,
            palette: None,
            alpha: None,
            premultiplied: format == ColorFormat::Argb8888Premultiplied,
        }
    }

    /// Pixels described part by part, validated: the stride must hold a row, `data` must hold
    /// `h` rows, indexed formats need a palette of at least
    /// [`palette_len`](ColorFormat::palette_len) entries, and `Rgb565A8` needs `w · h` alpha
    /// bytes (in `alpha`, or after the main plane in `data`).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_render::{ImagePixels, RenderError};
    ///
    /// let data = [0u8; 2 * 2 + 2 * 2];
    /// let px = ImagePixels::from_parts(ColorFormat::Rgb565A8, 1, 2, 2, &data, None, None, false).unwrap();
    /// assert_eq!(px.alpha_row(1), &[0]);
    /// let short = ImagePixels::from_parts(ColorFormat::Rgb565A8, 1, 2, 2, &data[..5], None, None, false);
    /// assert_eq!(short, Err(RenderError::BufferTooSmall { needed: 6, got: 5 }));
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        format: ColorFormat,
        w: u16,
        h: u16,
        stride: u16,
        data: &'a [u8],
        palette: Option<&'a [u8]>,
        alpha: Option<&'a [u8]>,
        premultiplied: bool,
    ) -> Result<Self, RenderError> {
        if stride < format.min_stride(w) {
            return Err(RenderError::InvalidArea);
        }
        let plane = usize::from(stride) * usize::from(h);
        let alpha_len = usize::from(w) * usize::from(h);
        let needed = if format == ColorFormat::Rgb565A8 && alpha.is_none() {
            plane + alpha_len
        } else {
            plane
        };
        if data.len() < needed {
            return Err(RenderError::BufferTooSmall {
                needed,
                got: data.len(),
            });
        }
        if format.is_indexed() {
            let needed = format.palette_len() * 4;
            let got = palette.map_or(0, <[u8]>::len);
            if got < needed {
                return Err(RenderError::BufferTooSmall { needed, got });
            }
        }
        if let (ColorFormat::Rgb565A8, Some(a)) = (format, alpha) {
            if a.len() < alpha_len {
                return Err(RenderError::BufferTooSmall {
                    needed: alpha_len,
                    got: a.len(),
                });
            }
        }
        Ok(Self {
            format,
            w,
            h,
            stride,
            data,
            palette,
            alpha,
            premultiplied: premultiplied || format == ColorFormat::Argb8888Premultiplied,
        })
    }

    /// Bytes of row `y` (up to `stride` bytes; empty when out of range or the data is short).
    #[must_use]
    pub fn row(&self, y: u16) -> &'a [u8] {
        if y >= self.h {
            return &[];
        }
        let s = usize::from(y) * usize::from(self.stride);
        let e = (s + usize::from(self.stride)).min(self.data.len());
        self.data.get(s..e).unwrap_or(&[])
    }

    /// The `w` alpha bytes of row `y` of an `Rgb565A8` image (empty for other formats, out of
    /// range rows or short data).
    #[must_use]
    pub fn alpha_row(&self, y: u16) -> &'a [u8] {
        if self.format != ColorFormat::Rgb565A8 || y >= self.h {
            return &[];
        }
        let w = usize::from(self.w);
        let (plane, base) = match self.alpha {
            Some(a) => (a, 0),
            None => (self.data, usize::from(self.stride) * usize::from(self.h)),
        };
        let s = base + usize::from(y) * w;
        plane.get(s..s + w).unwrap_or(&[])
    }

    /// The image rectangle when its top-left corner is at `(x, y)`.
    #[must_use]
    pub fn rect_at(&self, x: i32, y: i32) -> Rect {
        Rect::from_xywh(x, y, i32::from(self.w), i32::from(self.h))
    }

    /// Texel `(x, y)` as straight-alpha packed `0xAARRGGBB` (every format: palettes, alpha
    /// planes and premultiplied pixels are resolved; alpha-only formats give black with the
    /// texel's alpha). Outside the image, or when the data is short, the result is `0`
    /// (transparent).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_render::ImagePixels;
    /// let px = ImagePixels::new(ColorFormat::L8, 2, 1, &[0x10, 0x80]);
    /// assert_eq!(px.texel(1, 0), 0xFF80_8080);
    /// assert_eq!(px.texel(2, 0), 0);
    /// ```
    #[must_use]
    pub fn texel(&self, x: u16, y: u16) -> u32 {
        if x >= self.w || y >= self.h {
            return 0;
        }
        crate::image::read::with_texel!(self, T => <T as crate::image::read::Texel>::get(self, usize::from(x), usize::from(y)))
    }

    /// Whether texels must be un-premultiplied before blending.
    #[must_use]
    pub(crate) fn is_premultiplied(&self) -> bool {
        self.premultiplied || self.format == ColorFormat::Argb8888Premultiplied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_parts_validates() {
        let d = [0u8; 64];
        let pal = [0u8; 16];
        assert_eq!(
            ImagePixels::from_parts(ColorFormat::Rgb565, 4, 2, 7, &d, None, None, false),
            Err(RenderError::InvalidArea)
        );
        assert!(ImagePixels::from_parts(ColorFormat::I2, 4, 2, 1, &d, Some(&pal), None, false).is_ok());
        assert_eq!(
            ImagePixels::from_parts(ColorFormat::I2, 4, 2, 1, &d, Some(&pal[..12]), None, false),
            Err(RenderError::BufferTooSmall { needed: 16, got: 12 })
        );
        assert_eq!(
            ImagePixels::from_parts(ColorFormat::I2, 4, 2, 1, &d, None, None, false),
            Err(RenderError::BufferTooSmall { needed: 16, got: 0 })
        );
        let a = [9u8; 8];
        let px =
            ImagePixels::from_parts(ColorFormat::Rgb565A8, 4, 2, 8, &d[..16], None, Some(&a), false).unwrap();
        assert_eq!(px.alpha_row(1), &[9; 4]);
        assert!(px.alpha_row(2).is_empty());
        assert!(
            ImagePixels::from_parts(ColorFormat::Argb8888Premultiplied, 1, 1, 4, &d, None, None, false)
                .unwrap()
                .premultiplied
        );
    }
}
