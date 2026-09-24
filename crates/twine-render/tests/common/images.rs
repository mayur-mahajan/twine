//! Test images: a 32 × 32 RGBA pattern converted to every `ColorFormat`.
#![allow(dead_code, clippy::unreadable_literal)]

use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::{ImagePixels, Painter};

/// Pattern size.
pub const N: u16 = 32;

/// The pattern as straight-alpha `(r, g, b, a)`: a color gradient, opaque in the top half,
/// fading out to the right in the bottom half, with a dark 1-pixel frame and a white dot.
pub fn pattern() -> Vec<[u8; 4]> {
    let mut v = Vec::new();
    for y in 0..N {
        for x in 0..N {
            let (xi, yi) = (u32::from(x), u32::from(y));
            let mut c = [(xi * 8) as u8, (yi * 8) as u8, (255 - xi * 4) as u8, 255];
            if y >= N / 2 {
                c[3] = (255 - xi * 8) as u8;
            }
            if x == 0 || y == 0 || x == N - 1 || y == N - 1 {
                c = [20, 20, 60, 255];
            }
            if (12..16).contains(&x) && (4..8).contains(&y) {
                c = [255, 255, 255, 255];
            }
            v.push(c);
        }
    }
    v
}

/// An encoded test image.
pub struct Encoded {
    pub format: ColorFormat,
    pub data: Vec<u8>,
    pub palette: Vec<u8>,
}

impl Encoded {
    pub fn pixels(&self) -> ImagePixels<'_> {
        ImagePixels::from_parts(
            self.format,
            N,
            N,
            self.format.min_stride(N),
            &self.data,
            (!self.palette.is_empty()).then_some(&self.palette[..]),
            None,
            false,
        )
        .expect("valid test image")
    }
}

fn lum(c: [u8; 4]) -> u8 {
    Color::new(c[0], c[1], c[2]).luminance()
}

fn set_bits(row: &mut [u8], bpp: u8, x: usize, v: u8) {
    let bit = x * usize::from(bpp);
    let shift = 8 - usize::from(bpp) - bit % 8;
    row[bit / 8] |= v << shift;
}

/// The pattern in `format`.
pub fn encode(format: ColorFormat) -> Encoded {
    let px = pattern();
    let n = usize::from(N);
    let stride = usize::from(format.min_stride(N));
    let mut data = vec![0u8; stride * n];
    let mut palette = Vec::new();
    let bpp = format.bpp();
    // Palettes: I1 black/white, I2 4 grays, I4 transparent + 15 grays, I8 RGB332.
    match format {
        ColorFormat::I1 => palette.extend_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]),
        ColorFormat::I2 => (0..4u8).for_each(|i| palette.extend_from_slice(&[i * 85, i * 85, i * 85, 255])),
        ColorFormat::I4 => {
            palette.extend_from_slice(&[0, 0, 0, 0]);
            (1..16u8).for_each(|i| palette.extend_from_slice(&[i * 17, i * 17, i * 17, 255]));
        }
        ColorFormat::I8 => (0..=255u8).for_each(|i| {
            let (r, g, b) = ((i >> 5) * 36, ((i >> 2) & 7) * 36, (i & 3) * 85);
            palette.extend_from_slice(&[b, g, r, 255]);
        }),
        _ => {}
    }
    let mut alpha = Vec::new();
    for (i, &c) in px.iter().enumerate() {
        let (x, y) = (i % n, i / n);
        let row = &mut data[y * stride..(y + 1) * stride];
        let col = Color::new(c[0], c[1], c[2]);
        match format {
            ColorFormat::L8 => row[x] = lum(c),
            ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8 => {
                let a = (u32::from(c[3]) * u32::from(255 - lum(c) / 2) / 255) as u8;
                set_bits(row, bpp, x, a >> (8 - bpp));
            }
            ColorFormat::I1 => set_bits(row, 1, x, u8::from(lum(c) >= 128)),
            ColorFormat::I2 => set_bits(row, 2, x, lum(c) >> 6),
            ColorFormat::I4 => set_bits(row, 4, x, if c[3] < 128 { 0 } else { (lum(c) / 17).max(1) }),
            ColorFormat::I8 => row[x] = (c[0] >> 5) << 5 | (c[1] >> 5) << 2 | c[2] >> 6,
            ColorFormat::Rgb565 | ColorFormat::Rgb565A8 => {
                row[x * 2..x * 2 + 2].copy_from_slice(&col.to_rgb565().to_le_bytes());
                alpha.push(c[3]);
            }
            ColorFormat::Rgb565Swapped => {
                row[x * 2..x * 2 + 2].copy_from_slice(&col.to_rgb565().to_be_bytes());
            }
            ColorFormat::Rgb888 => row[x * 3..x * 3 + 3].copy_from_slice(&[c[2], c[1], c[0]]),
            ColorFormat::Xrgb8888 => row[x * 4..x * 4 + 4].copy_from_slice(&[c[2], c[1], c[0], 0x55]),
            ColorFormat::Argb8888 => row[x * 4..x * 4 + 4].copy_from_slice(&[c[2], c[1], c[0], c[3]]),
            ColorFormat::Argb8888Premultiplied => {
                let m = |v: u8| ((u32::from(v) * u32::from(c[3]) + 127) / 255) as u8;
                row[x * 4..x * 4 + 4].copy_from_slice(&[m(c[2]), m(c[1]), m(c[0]), c[3]]);
            }
        }
    }
    if format == ColorFormat::Rgb565A8 {
        data.extend_from_slice(&alpha);
    }
    Encoded {
        format,
        data,
        palette,
    }
}

/// Fills `area` with an 8-pixel gray checkerboard.
pub fn checker(p: &mut Painter<'_>, area: Rect) {
    p.fill(area, Color::hex(0xDDDDDD), Opa::COVER);
    for y in (area.y0..area.y1).step_by(8) {
        for x in (area.x0..area.x1).step_by(8) {
            if ((x - area.x0) / 8 + (y - area.y0) / 8) % 2 == 1 {
                p.fill(
                    Rect::new(x, y, (x + 8).min(area.x1), (y + 8).min(area.y1)),
                    Color::hex(0x999999),
                    Opa::COVER,
                );
            }
        }
    }
}
