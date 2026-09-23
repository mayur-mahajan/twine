//! [`DrawBuf`]: a typed view of pixel memory covering a screen rectangle.

use twine_core::{Color, ColorFormat, Rect};

use crate::RenderError;

/// Whether `format` can be the format of a [`DrawBuf`] (independent of enabled features).
#[must_use]
pub const fn is_draw_format(format: ColorFormat) -> bool {
    matches!(
        format,
        ColorFormat::Rgb565
            | ColorFormat::Rgb565Swapped
            | ColorFormat::Rgb888
            | ColorFormat::Xrgb8888
            | ColorFormat::Argb8888
            | ColorFormat::L8
            | ColorFormat::I1
    )
}

/// Pixel memory in one [`ColorFormat`] representing the screen rectangle `area`.
///
/// All drawing uses absolute screen coordinates; the buffer maps them to its memory. `stride`
/// is the distance between rows in bytes.
///
/// ```
/// use twine_core::{Color, ColorFormat, Rect};
/// use twine_render::DrawBuf;
///
/// let mut data = vec![0u8; 4 * 2 * 2];
/// let mut buf = DrawBuf::new_packed(&mut data, ColorFormat::Rgb565, Rect::from_xywh(10, 20, 4, 2)).unwrap();
/// buf.clear(Color::RED);
/// assert_eq!(buf.row(21, 12, 13), &[0x00, 0xF8]);
/// ```
#[derive(Debug)]
pub struct DrawBuf<'a> {
    data: &'a mut [u8],
    format: ColorFormat,
    stride: usize,
    area: Rect,
}

/// Bytes needed for `width` pixels of `format` (rounded up to whole bytes).
#[inline]
pub(crate) const fn row_bytes(format: ColorFormat, width: usize) -> usize {
    (width * format.bpp() as usize).div_ceil(8)
}

impl<'a> DrawBuf<'a> {
    /// Wraps `data` as the pixels of `area` (screen coordinates) with rows `stride` bytes apart.
    ///
    /// Errors: [`RenderError::UnsupportedFormat`] for formats that cannot be drawn into,
    /// [`RenderError::InvalidArea`] for an empty area, a stride shorter than a row, or an `I1`
    /// area whose `x0`/`x1` are not multiples of 8, and [`RenderError::BufferTooSmall`].
    pub fn new(
        data: &'a mut [u8],
        format: ColorFormat,
        stride: usize,
        area: Rect,
    ) -> Result<Self, RenderError> {
        if !is_draw_format(format) {
            return Err(RenderError::UnsupportedFormat(format));
        }
        if area.is_empty() {
            return Err(RenderError::InvalidArea);
        }
        if format == ColorFormat::I1 && (area.x0.rem_euclid(8) != 0 || area.x1.rem_euclid(8) != 0) {
            return Err(RenderError::InvalidArea);
        }
        let row = row_bytes(format, area.width() as usize);
        if stride < row {
            return Err(RenderError::InvalidArea);
        }
        let needed = stride * (area.height() as usize - 1) + row;
        if data.len() < needed {
            return Err(RenderError::BufferTooSmall {
                needed,
                got: data.len(),
            });
        }
        Ok(Self {
            data,
            format,
            stride,
            area,
        })
    }

    /// Like [`new`](Self::new) with `stride = width · bpp / 8` (rows packed).
    pub fn new_packed(data: &'a mut [u8], format: ColorFormat, area: Rect) -> Result<Self, RenderError> {
        let stride = row_bytes(format, area.width().max(0) as usize);
        Self::new(data, format, stride, area)
    }

    /// The pixel format.
    #[must_use]
    pub fn format(&self) -> ColorFormat {
        self.format
    }

    /// The screen rectangle the buffer represents.
    #[must_use]
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Bytes between rows.
    #[must_use]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The whole underlying memory.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.data
    }

    /// The whole underlying memory, mutably.
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.data
    }

    /// Byte range of pixels `[x0, x1)` of row `y` (absolute coordinates).
    #[inline]
    fn range(&self, y: i32, x0: i32, x1: i32) -> core::ops::Range<usize> {
        debug_assert!(
            y >= self.area.y0 && y < self.area.y1 && x0 >= self.area.x0 && x1 <= self.area.x1 && x0 <= x1,
            "row {y} [{x0}, {x1}) outside {}",
            self.area
        );
        let bpp = self.format.bpp() as usize;
        let start = (y - self.area.y0) as usize * self.stride;
        let bx0 = (x0 - self.area.x0) as usize * bpp / 8;
        let bx1 = ((x1 - self.area.x0) as usize * bpp).div_ceil(8);
        start + bx0..start + bx1
    }

    /// Bytes of pixels `[x0, x1)` of row `y`, in absolute coordinates that must lie inside
    /// [`area`](Self::area) (checked with `debug_assert!`). For `I1` the bytes containing the
    /// pixels are returned (`x0` rounded down, `x1` up to a multiple of 8).
    #[inline]
    pub fn row_mut(&mut self, y: i32, x0: i32, x1: i32) -> &mut [u8] {
        let r = self.range(y, x0, x1);
        &mut self.data[r]
    }

    /// Shared version of [`row_mut`](Self::row_mut).
    #[inline]
    #[must_use]
    pub fn row(&self, y: i32, x0: i32, x1: i32) -> &[u8] {
        let r = self.range(y, x0, x1);
        &self.data[r]
    }

    /// Fills the whole buffer with `color` (opaque). Formats whose `color-*` feature is disabled
    /// are left unchanged (with a warning).
    pub fn clear(&mut self, color: Color) {
        let area = self.area;
        for y in area.y0..area.y1 {
            crate::blend::blend_row(
                self,
                y,
                area.x0,
                area.x1,
                crate::Source::Solid(color),
                None,
                twine_core::Opa::COVER,
                crate::BlendMode::Normal,
            );
        }
    }

    /// A shorter-lived `DrawBuf` over the same memory.
    pub fn reborrow(&mut self) -> DrawBuf<'_> {
        DrawBuf {
            data: self.data,
            format: self.format,
            stride: self.stride,
            area: self.area,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawbuf_rejects_small_buffer() {
        let mut d = [0u8; 15];
        let area = Rect::from_xywh(0, 0, 4, 2);
        assert_eq!(
            DrawBuf::new_packed(&mut d, ColorFormat::Rgb565, area).unwrap_err(),
            RenderError::BufferTooSmall { needed: 16, got: 15 }
        );
        // The last row needs no padding up to the stride.
        let mut d = [0u8; 20];
        assert!(DrawBuf::new(&mut d, ColorFormat::Rgb565, 12, area).is_ok());
        assert_eq!(
            DrawBuf::new(&mut d, ColorFormat::Rgb565, 6, area).unwrap_err(),
            RenderError::InvalidArea
        );
        assert_eq!(
            DrawBuf::new_packed(&mut d, ColorFormat::A8, area).unwrap_err(),
            RenderError::UnsupportedFormat(ColorFormat::A8)
        );
        assert_eq!(
            DrawBuf::new_packed(&mut d, ColorFormat::L8, Rect::ZERO).unwrap_err(),
            RenderError::InvalidArea
        );
    }

    #[test]
    fn row_mut_maps_absolute_coords() {
        let mut d = [0u8; 3 * 4 * 3];
        let mut b = DrawBuf::new_packed(&mut d, ColorFormat::Rgb888, Rect::from_xywh(100, 50, 4, 3)).unwrap();
        assert_eq!(b.stride(), 12);
        b.row_mut(51, 101, 103).fill(7);
        assert_eq!(b.row(51, 100, 104), &[0, 0, 0, 7, 7, 7, 7, 7, 7, 0, 0, 0]);
        assert!(d[..12].iter().all(|&v| v == 0));
        assert_eq!(&d[12..24], &[0, 0, 0, 7, 7, 7, 7, 7, 7, 0, 0, 0]);
    }

    #[test]
    fn i1_requires_byte_aligned_x() {
        let mut d = [0u8; 8];
        assert_eq!(
            DrawBuf::new_packed(&mut d, ColorFormat::I1, Rect::from_xywh(4, 0, 8, 2)).unwrap_err(),
            RenderError::InvalidArea
        );
        assert_eq!(
            DrawBuf::new_packed(&mut d, ColorFormat::I1, Rect::from_xywh(0, 0, 12, 2)).unwrap_err(),
            RenderError::InvalidArea
        );
        let mut b = DrawBuf::new_packed(&mut d, ColorFormat::I1, Rect::from_xywh(8, 0, 16, 2)).unwrap();
        assert_eq!(b.stride(), 2);
        assert_eq!(b.row_mut(1, 17, 18).len(), 1);
    }
}
