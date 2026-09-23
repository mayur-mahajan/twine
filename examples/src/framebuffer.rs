//! Pixel access to raw framebuffers in the simulator's panel formats.

use twine_core::color::{Argb8888, I1, L8, Rgb565, Rgb565Swapped, Rgb888, Xrgb8888};
use twine_core::{Color, ColorFormat, PixelFormat};

/// A full-screen framebuffer of `width` pixels per row in `format`.
#[derive(Debug)]
pub struct Frame<'a> {
    /// The pixel bytes.
    pub data: &'a mut [u8],
    /// Pixel format.
    pub format: ColorFormat,
    /// Width in pixels.
    pub width: u32,
}

impl<'a> Frame<'a> {
    /// Wraps `data` (rows of `format.stride(width)` bytes).
    pub fn new(data: &'a mut [u8], format: ColorFormat, width: u32) -> Self {
        Self { data, format, width }
    }

    /// Height in whole rows.
    #[must_use]
    pub fn height(&self) -> u32 {
        let stride = self.format.stride(self.width).max(1) as usize;
        (self.data.len() / stride) as u32
    }

    /// Writes pixel `(x, y)`; out-of-range pixels and unsupported formats are ignored.
    ///
    /// `I1` stores `1` for colors with luminance ≥ 128.
    pub fn put(&mut self, x: u32, y: u32, c: Color) {
        if x >= self.width {
            return;
        }
        let stride = self.format.stride(self.width) as usize;
        let Some(row) = self.data.get_mut(y as usize * stride..(y as usize + 1) * stride) else {
            return;
        };
        let x = x as usize;
        match self.format {
            ColorFormat::Rgb565 => Rgb565::write(&mut row[x * 2..], Rgb565::from_color(c)),
            ColorFormat::Rgb565Swapped => {
                Rgb565Swapped::write(&mut row[x * 2..], Rgb565Swapped::from_color(c));
            }
            ColorFormat::Rgb888 => Rgb888::write(&mut row[x * 3..], Rgb888::from_color(c)),
            ColorFormat::Xrgb8888 => Xrgb8888::write(&mut row[x * 4..], Xrgb8888::from_color(c)),
            ColorFormat::Argb8888 => Argb8888::write(&mut row[x * 4..], Argb8888::from_color(c)),
            ColorFormat::L8 => L8::write(&mut row[x..], L8::from_color(c)),
            ColorFormat::I1 => I1::set(row, x, c.luminance() >= 128),
            _ => {}
        }
    }

    /// Fills the rectangle `[x0, x1) × [y0, y1)` (clipped to the frame).
    pub fn fill_rect(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, c: Color) {
        for y in y0..y1.min(self.height()) {
            for x in x0..x1.min(self.width) {
                self.put(x, y, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_writes_each_format() {
        let mut data = [0u8; 4];
        Frame::new(&mut data, ColorFormat::Rgb565, 2).put(1, 0, Color::RED);
        assert_eq!(data, [0, 0, 0x00, 0xF8]);
        let mut data = [0u8; 2];
        let mut f = Frame::new(&mut data, ColorFormat::I1, 9);
        f.put(8, 0, Color::WHITE);
        f.put(9, 0, Color::WHITE); // out of range
        assert_eq!(f.height(), 1);
        assert_eq!(data, [0, 0x80]);
    }
}
