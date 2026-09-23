//! Conversion of the emulated panel memory to RGB for the window and PNGs.
//!
//! Deliberately a copy of the logic in `twine-testing` (not a dependency) to keep the simulator
//! independent of the test crate. Supported panel formats: `Rgb565`, `Rgb565Swapped` (big-endian
//! bytes), `Rgb888`, `Xrgb8888`, `Argb8888` (alpha ignored, like a real panel), `L8` and `I1`
//! (MSB first, shown with the configured mono colors). Other formats render black.

use twine_core::color::{I1, Rgb565, Rgb565Swapped, Rgb888, Xrgb8888};
use twine_core::{Color, ColorFormat, PixelFormat};

/// The color of pixel `(x, y)` of a panel buffer with row pitch `stride` bytes.
///
/// Pixels outside `buf` and unsupported formats give black.
#[must_use]
pub fn pixel(
    buf: &[u8],
    format: ColorFormat,
    stride: usize,
    x: usize,
    y: usize,
    mono: (Color, Color),
) -> Color {
    let Some(row) = buf.get(y * stride..) else {
        return Color::BLACK;
    };
    if format == ColorFormat::I1 {
        return if I1::get(row, x) { mono.1 } else { mono.0 };
    }
    let bytes = usize::from(format.bpp()) / 8;
    let Some(px) = row.get(x * bytes..x * bytes + bytes) else {
        return Color::BLACK;
    };
    match format {
        ColorFormat::Rgb565 => Rgb565::to_color(Rgb565::read(px)),
        ColorFormat::Rgb565Swapped => Rgb565Swapped::to_color(Rgb565Swapped::read(px)),
        ColorFormat::Rgb888 => Rgb888::to_color(Rgb888::read(px)),
        ColorFormat::Xrgb8888 | ColorFormat::Argb8888 => Xrgb8888::to_color(Xrgb8888::read(px)),
        ColorFormat::L8 => Color::new(px[0], px[0], px[0]),
        _ => Color::BLACK,
    }
}

/// Converts a `w × h` panel buffer to tightly packed 8-bit RGB.
#[must_use]
pub fn to_rgb888(
    buf: &[u8],
    format: ColorFormat,
    w: usize,
    h: usize,
    stride: usize,
    mono: (Color, Color),
) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let c = pixel(buf, format, stride, x, y, mono);
            out.extend_from_slice(&[c.r, c.g, c.b]);
        }
    }
    out
}

/// Converts a `w × h` panel buffer to `0x00RRGGBB` words (reusing `out`'s allocation).
pub fn to_xrgb(
    buf: &[u8],
    format: ColorFormat,
    w: usize,
    h: usize,
    stride: usize,
    mono: (Color, Color),
    out: &mut Vec<u32>,
) {
    out.clear();
    out.reserve(w * h);
    for y in 0..h {
        for x in 0..w {
            let c = pixel(buf, format, stride, x, y, mono);
            out.push((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: (Color, Color) = (Color::BLACK, Color::WHITE);

    #[test]
    fn formats_decode() {
        assert_eq!(
            pixel(&[0x00, 0xF8], ColorFormat::Rgb565, 2, 0, 0, MONO),
            Color::RED
        );
        assert_eq!(
            pixel(&[0xF8, 0x00], ColorFormat::Rgb565Swapped, 2, 0, 0, MONO),
            Color::RED
        );
        assert_eq!(
            pixel(&[0xFF, 0, 0], ColorFormat::Rgb888, 3, 0, 0, MONO),
            Color::BLUE
        );
        assert_eq!(
            pixel(&[0, 0xFF, 0, 0x00], ColorFormat::Argb8888, 4, 0, 0, MONO),
            Color::GREEN
        );
        assert_eq!(
            pixel(&[0x40], ColorFormat::L8, 1, 0, 0, MONO),
            Color::hex(0x40_40_40)
        );
        assert_eq!(pixel(&[0x40], ColorFormat::I1, 1, 1, 0, MONO), Color::WHITE);
        assert_eq!(pixel(&[0x40], ColorFormat::I4, 1, 0, 0, MONO), Color::BLACK);
        assert_eq!(pixel(&[0x40], ColorFormat::L8, 1, 5, 5, MONO), Color::BLACK);
    }

    #[test]
    fn xrgb_words() {
        let mut out = vec![1, 2, 3];
        to_xrgb(
            &[0x00, 0xF8, 0x1F, 0x00],
            ColorFormat::Rgb565,
            2,
            1,
            4,
            MONO,
            &mut out,
        );
        assert_eq!(out, [0x00FF_0000, 0x0000_00FF]);
        assert_eq!(
            to_rgb888(&[0x00, 0xF8], ColorFormat::Rgb565, 1, 1, 2, MONO),
            [255, 0, 0]
        );
    }
}
