//! RGBA → every `ColorFormat`, in the memory layout of `twine_image::Image` (palette, main
//! plane, alpha plane).

use twine_core::ColorFormat;
use twine_image::{ImageFlags, ImageHeader};

use super::quantize::{Rgba, map_to_palette, median_cut};

/// Converted pixel data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Converted {
    /// Header (packed stride).
    pub header: ImageHeader,
    /// Uncompressed data.
    pub data: Vec<u8>,
}

/// 4 × 4 Bayer matrix (0..16).
const BAYER4: [[u16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Quantizes `v` to `levels - 1` steps: rounded, or ordered-dithered with threshold `t`
/// (0..16).
fn quant(v: u8, max: u16, t: Option<u16>) -> u16 {
    let v = u32::from(v);
    let max = u32::from(max);
    match t {
        None => ((v * max + 127) / 255) as u16,
        Some(t) => ((v * max * 16 + u32::from(t) * 255) / (255 * 16)).min(max) as u16,
    }
}

/// Luminance `(r·77 + g·150 + b·29) >> 8`.
#[must_use]
pub fn luminance(p: Rgba) -> u8 {
    ((u32::from(p[0]) * 77 + u32::from(p[1]) * 150 + u32::from(p[2]) * 29) >> 8) as u8
}

fn premul(p: Rgba) -> Rgba {
    let m = |c: u8| ((u32::from(c) * u32::from(p[3]) + 127) / 255) as u8;
    [m(p[0]), m(p[1]), m(p[2]), p[3]]
}

fn put_bits(row: &mut [u8], bpp: u8, x: usize, v: u8) {
    let bit = x * usize::from(bpp);
    row[bit / 8] |= v << (8 - usize::from(bpp) - bit % 8);
}

/// Converts `w × h` straight-alpha RGBA pixels to `format`.
///
/// - `A1`…`A8`: the alpha channel; `L8`: luminance (alpha ignored);
/// - `I1`…`I8`: median-cut palette (Floyd–Steinberg with `dither`);
/// - `Rgb565*`: rounded, or 4 × 4 ordered dithering with `dither`;
/// - `premultiply` premultiplies the colors of `Argb8888` and `Rgb565A8` and sets
///   [`ImageFlags::PREMULTIPLIED`] (`Argb8888Premultiplied` is always premultiplied).
///
/// # Errors
/// If `premultiply` is requested for a format without both color and alpha.
pub fn convert(
    rgba: &[u8],
    w: u16,
    h: u16,
    format: ColorFormat,
    dither: bool,
    premultiply: bool,
) -> Result<Converted, String> {
    let n = usize::from(w) * usize::from(h);
    let px: Vec<Rgba> = rgba
        .chunks_exact(4)
        .take(n)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    if px.len() != n {
        return Err(format!("expected {n} pixels, got {}", px.len()));
    }
    if premultiply
        && !matches!(
            format,
            ColorFormat::Argb8888 | ColorFormat::Rgb565A8 | ColorFormat::Argb8888Premultiplied
        )
    {
        return Err(format!(
            "--premultiply needs a format with color and alpha (not {format})"
        ));
    }
    let mut header = ImageHeader::new(format, w, h);
    if premultiply && format != ColorFormat::Argb8888Premultiplied {
        header.flags |= ImageFlags::PREMULTIPLIED;
    }
    let stride = usize::from(header.stride);
    let wu = usize::from(w);
    let mut data = vec![0u8; header.data_size()];
    let pal_len = format.palette_len() * 4;
    let (pal_bytes, rest) = data.split_at_mut(pal_len);
    let (plane, alpha_plane) = rest.split_at_mut(stride * usize::from(h));
    let bpp = format.bpp();
    let threshold = |i: usize| dither.then(|| BAYER4[(i / wu) % 4][(i % wu) % 4]);
    if format.is_indexed() {
        let palette = median_cut(&px, format.palette_len());
        for (i, p) in palette.iter().enumerate() {
            pal_bytes[i * 4..i * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        let idx = map_to_palette(&px, wu, &palette, dither);
        for (i, &k) in idx.iter().enumerate() {
            put_bits(&mut plane[(i / wu) * stride..], bpp, i % wu, k);
        }
        return Ok(Converted { header, data });
    }
    for (i, &p0) in px.iter().enumerate() {
        let (x, y) = (i % wu, i / wu);
        let row = &mut plane[y * stride..(y + 1) * stride];
        let p = if premultiply || format == ColorFormat::Argb8888Premultiplied {
            premul(p0)
        } else {
            p0
        };
        match format {
            ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8 => {
                let max = (1u16 << bpp) - 1;
                put_bits(row, bpp, x, quant(p[3], max, None) as u8);
            }
            ColorFormat::L8 => row[x] = luminance(p),
            ColorFormat::Rgb565 | ColorFormat::Rgb565Swapped | ColorFormat::Rgb565A8 => {
                let t = threshold(i);
                let v = (quant(p[0], 31, t) << 11) | (quant(p[1], 63, t) << 5) | quant(p[2], 31, t);
                let b = if format == ColorFormat::Rgb565Swapped {
                    v.to_be_bytes()
                } else {
                    v.to_le_bytes()
                };
                row[x * 2..x * 2 + 2].copy_from_slice(&b);
                if format == ColorFormat::Rgb565A8 {
                    alpha_plane[i] = p[3];
                }
            }
            ColorFormat::Rgb888 => row[x * 3..x * 3 + 3].copy_from_slice(&[p[2], p[1], p[0]]),
            ColorFormat::Xrgb8888 => row[x * 4..x * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], 0xFF]),
            ColorFormat::Argb8888 | ColorFormat::Argb8888Premultiplied => {
                row[x * 4..x * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
            ColorFormat::I1 | ColorFormat::I2 | ColorFormat::I4 | ColorFormat::I8 => {
                unreachable!("handled above")
            }
        }
    }
    Ok(Converted { header, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_rounds_and_dithers() {
        assert_eq!(quant(255, 31, None), 31);
        assert_eq!(quant(0, 31, None), 0);
        assert_eq!(quant(4, 31, None), 0);
        assert_eq!(quant(5, 31, None), 1);
        // Ordered dithering averages to the input over the 16 thresholds.
        let sum: u32 = (0..16).map(|t| u32::from(quant(100, 31, Some(t)))).sum();
        let avg = sum as f32 / 16.0 * 255.0 / 31.0;
        assert!((avg - 100.0).abs() < 8.0, "{avg}");
    }

    #[test]
    fn layouts() {
        let px = [255u8, 0, 0, 128, 0, 0, 255, 255];
        let c = convert(&px, 2, 1, ColorFormat::Rgb565A8, false, false).unwrap();
        assert_eq!(c.data, [0x00, 0xF8, 0x1F, 0x00, 128, 255]);
        let c = convert(&px, 2, 1, ColorFormat::A2, false, false).unwrap();
        assert_eq!(c.data, [0b1011_0000]);
        let c = convert(&px, 2, 1, ColorFormat::Argb8888, false, true).unwrap();
        assert_eq!(c.data, [0, 0, 128, 128, 255, 0, 0, 255]);
        assert!(c.header.flags.contains(ImageFlags::PREMULTIPLIED));
        let c = convert(&px, 2, 1, ColorFormat::I1, false, false).unwrap();
        // Palette sorted: blue (0,0,255,255) then red (255,0,0,128).
        assert_eq!(&c.data[..8], &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert_eq!(c.data[8], 0b1000_0000);
        assert!(convert(&px, 2, 1, ColorFormat::L8, false, true).is_err());
    }
}
