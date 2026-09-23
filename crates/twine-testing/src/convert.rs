//! Pixel format conversion for assertions and snapshots: any supported [`ColorFormat`] buffer →
//! 8-bit RGB.
//!
//! Supported formats: every byte-aligned [`PixelFormat`]
//! (`Rgb565`, `Rgb565Swapped`, `Rgb888`, `Xrgb8888`, `Argb8888`, `L8`) plus `A8` (shown as gray,
//! `0` = black) and `I1` (1-bit, MSB first, shown with the two `mono` colors). `Argb8888` is
//! composited over mid gray `#808080`, so transparency is visible in snapshots. Other formats
//! panic with a clear message — this is a test-only crate.

use twine_core::color::{Argb8888, I1, L8, Rgb565, Rgb565Swapped, Rgb888, Xrgb8888};
use twine_core::{Color, ColorFormat, Opa, PixelFormat};

/// The background `Argb8888` pixels are composited over.
pub const ALPHA_BACKGROUND: Color = Color::GRAY;

/// Whether `format` can be converted by this module.
#[must_use]
pub const fn is_supported(format: ColorFormat) -> bool {
    matches!(
        format,
        ColorFormat::Rgb565
            | ColorFormat::Rgb565Swapped
            | ColorFormat::Rgb888
            | ColorFormat::Xrgb8888
            | ColorFormat::Argb8888
            | ColorFormat::L8
            | ColorFormat::A8
            | ColorFormat::I1
    )
}

fn assert_supported(format: ColorFormat) {
    assert!(
        is_supported(format),
        "twine-testing: color format {format} cannot be converted to RGB (supported: RGB565, \
         RGB565_SWAPPED, RGB888, XRGB8888, ARGB8888, L8, A8, I1)"
    );
}

/// The color of pixel `(x, y)` of a buffer with row pitch `stride` bytes.
///
/// `mono` = (color of `0` bits, color of `1` bits) for `I1`.
///
/// # Panics
/// If `format` is unsupported or the pixel lies outside `buf`.
#[must_use]
pub fn pixel_color(
    buf: &[u8],
    format: ColorFormat,
    stride: u32,
    x: u32,
    y: u32,
    mono: (Color, Color),
) -> Color {
    assert_supported(format);
    let row_start = y as usize * stride as usize;
    let row = &buf[row_start..];
    let bpp = usize::from(format.bpp());
    if format == ColorFormat::I1 {
        return if I1::get(row, x as usize) { mono.1 } else { mono.0 };
    }
    let px = &row[x as usize * bpp / 8..];
    match format {
        ColorFormat::Rgb565 => Rgb565::to_color(Rgb565::read(px)),
        ColorFormat::Rgb565Swapped => Rgb565Swapped::to_color(Rgb565Swapped::read(px)),
        ColorFormat::Rgb888 => Rgb888::to_color(Rgb888::read(px)),
        ColorFormat::Xrgb8888 => Xrgb8888::to_color(Xrgb8888::read(px)),
        ColorFormat::Argb8888 => {
            let raw = Argb8888::read(px);
            Color::mix(Argb8888::to_color(raw), ALPHA_BACKGROUND, Argb8888::alpha(raw))
        }
        ColorFormat::L8 | ColorFormat::A8 => L8::to_color(px[0]),
        _ => unreachable!("checked by assert_supported"),
    }
}

/// Converts a `w × h` buffer (row pitch `stride` bytes) to tightly packed 8-bit RGB
/// (`w * h * 3` bytes).
///
/// ```
/// use twine_core::{Color, ColorFormat};
/// use twine_testing::convert::to_rgb888;
///
/// // Two RGB565 pixels: red, blue (little-endian).
/// let buf = [0x00, 0xF8, 0x1F, 0x00];
/// let rgb = to_rgb888(&buf, ColorFormat::Rgb565, 2, 1, 4, (Color::BLACK, Color::WHITE));
/// assert_eq!(rgb, [255, 0, 0, 0, 0, 255]);
/// ```
///
/// # Panics
/// If `format` is unsupported or `buf` is shorter than `stride * (h - 1)` + one row.
#[must_use]
pub fn to_rgb888(
    buf: &[u8],
    format: ColorFormat,
    w: u32,
    h: u32,
    stride: u32,
    mono: (Color, Color),
) -> Vec<u8> {
    assert_supported(format);
    let row_bytes = format.stride(w) as usize;
    if h > 0 {
        let need = (h as usize - 1) * stride as usize + row_bytes;
        assert!(
            buf.len() >= need,
            "twine-testing: {w}x{h} {format} buffer with stride {stride} needs {need} bytes, got {}",
            buf.len()
        );
    }
    let mut out = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        for x in 0..w {
            let c = pixel_color(buf, format, stride, x, y, mono);
            out.extend_from_slice(&[c.r, c.g, c.b]);
        }
    }
    out
}

/// Writes `c` as pixel `(x, y)` of a buffer in `format` (for filling test displays).
///
/// `I1` stores `1` when the luminance of `c` is at least 128; `A8` stores the luminance.
///
/// # Panics
/// If `format` is unsupported or the pixel lies outside `buf`.
pub fn write_pixel(buf: &mut [u8], format: ColorFormat, stride: u32, x: u32, y: u32, c: Color) {
    assert_supported(format);
    let row = &mut buf[y as usize * stride as usize..];
    if format == ColorFormat::I1 {
        I1::set(row, x as usize, c.luminance() >= 128);
        return;
    }
    let bpp = usize::from(format.bpp());
    let px = &mut row[x as usize * bpp / 8..];
    match format {
        ColorFormat::Rgb565 => Rgb565::write(px, Rgb565::from_color(c)),
        ColorFormat::Rgb565Swapped => Rgb565Swapped::write(px, Rgb565Swapped::from_color(c)),
        ColorFormat::Rgb888 => Rgb888::write(px, Rgb888::from_color(c)),
        ColorFormat::Xrgb8888 => Xrgb8888::write(px, Xrgb8888::from_color(c)),
        ColorFormat::Argb8888 => Argb8888::write(px, Argb8888::from_color_opa(c, Opa::COVER)),
        ColorFormat::L8 | ColorFormat::A8 => px[0] = c.luminance(),
        _ => unreachable!("checked by assert_supported"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: (Color, Color) = (Color::BLACK, Color::WHITE);

    #[test]
    fn roundtrip_every_supported_format() {
        let colors = [Color::RED, Color::GREEN, Color::BLUE, Color::WHITE, Color::BLACK];
        for f in ColorFormat::ALL.into_iter().filter(|f| is_supported(*f)) {
            let stride = f.stride(colors.len() as u32);
            let mut buf = vec![0u8; stride as usize];
            for (x, c) in colors.iter().enumerate() {
                write_pixel(&mut buf, f, stride, x as u32, 0, *c);
            }
            let rgb = to_rgb888(&buf, f, colors.len() as u32, 1, stride, MONO);
            for (x, c) in colors.iter().enumerate() {
                let got = Color::new(rgb[x * 3], rgb[x * 3 + 1], rgb[x * 3 + 2]);
                let expected = match f {
                    ColorFormat::L8 | ColorFormat::A8 => L8::to_color(c.luminance()),
                    ColorFormat::I1 => {
                        if c.luminance() >= 128 {
                            MONO.1
                        } else {
                            MONO.0
                        }
                    }
                    _ => *c,
                };
                assert_eq!(got, expected, "{f} pixel {x}");
            }
        }
    }

    #[test]
    fn argb_is_composited_over_gray() {
        let mut px = [0u8; 4];
        Argb8888::write(&mut px, Argb8888::from_color_opa(Color::WHITE, Opa::TRANSP));
        assert_eq!(
            to_rgb888(&px, ColorFormat::Argb8888, 1, 1, 4, MONO),
            [128, 128, 128]
        );
    }

    #[test]
    fn i1_uses_mono_colors_and_stride() {
        // 2 rows of 3 px, stride 1: row 0 = 101, row 1 = 010.
        let buf = [0b1010_0000, 0b0100_0000];
        let mono = (Color::hex(0x10_20_30), Color::hex(0xA0_B0_C0));
        let rgb = to_rgb888(&buf, ColorFormat::I1, 3, 2, 1, mono);
        assert_eq!(&rgb[0..3], &[0xA0, 0xB0, 0xC0]);
        assert_eq!(&rgb[3..6], &[0x10, 0x20, 0x30]);
        assert_eq!(&rgb[12..15], &[0xA0, 0xB0, 0xC0]);
    }

    #[test]
    #[should_panic(expected = "cannot be converted")]
    fn unsupported_format_panics() {
        let _ = to_rgb888(&[0; 4], ColorFormat::I4, 2, 1, 1, MONO);
    }
}
