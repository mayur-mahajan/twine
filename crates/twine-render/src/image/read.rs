//! Per-format texel readers: every [`ColorFormat`] → packed straight-alpha `0xAARRGGBB`.
//!
//! Indexed formats read their palette, alpha-only formats produce black with the pixel's alpha
//! (callers recolor them), `Rgb565A8` reads both planes, premultiplied pixels are
//! un-premultiplied and `L8` becomes gray.

use twine_core::color::{
    Argb8888, PixelFormat, Rgb565, expand_alpha, unpack_bits,
};
use twine_core::{Color, ColorFormat, Opa};

use crate::ImagePixels;

#[inline(always)]
pub(crate) const fn pack(c: Color, a: u8) -> u32 {
    ((a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// Reads packed `0xAARRGGBB` texels of one image format.
pub(crate) trait Texel {
    /// Texel `(x, y)`; `0` (transparent) when the data is short.
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32;
}

pub(crate) struct Px<F>(core::marker::PhantomData<F>);
impl<F: PixelFormat> Texel for Px<F> {
    #[inline(always)]
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32 {
        let o = y * usize::from(img.stride) + x * F::BYTES;
        match img.data.get(o..o + F::BYTES) {
            Some(b) => {
                if F::FORMAT == ColorFormat::Argb8888 {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    pack(F::to_color(F::read(b)), 255)
                }
            }
            None => 0,
        }
    }
}

pub(crate) struct Alpha;
impl Texel for Alpha {
    #[inline(always)]
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32 {
        let bpp = img.format.bpp();
        let row = img.row(y as u16);
        u32::from(expand_alpha(unpack_bits(row, bpp, x), bpp)) << 24
    }
}

pub(crate) struct Indexed;
impl Texel for Indexed {
    #[inline(always)]
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32 {
        let idx = usize::from(unpack_bits(img.row(y as u16), img.format.bpp(), x));
        img.palette
            .and_then(|p| p.get(idx * 4..idx * 4 + 4))
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

pub(crate) struct Rgb565A8;
impl Texel for Rgb565A8 {
    #[inline(always)]
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32 {
        let c = Px::<Rgb565>::get(img, x, y) & 0x00FF_FFFF;
        let a = img.alpha_row(y as u16).get(x).copied().unwrap_or(0);
        c | (u32::from(a) << 24)
    }
}

/// Un-premultiplies a packed texel.
#[inline(always)]
pub(crate) fn unpremultiply(v: u32) -> u32 {
    let a = v >> 24;
    if a == 0 || a == 255 {
        return v;
    }
    let un = |s: u32| ((((v >> s) & 0xFF) * 255 + a / 2) / a).min(255) << s;
    (a << 24) | un(16) | un(8) | un(0)
}

pub(crate) struct Premul;
impl Texel for Premul {
    #[inline(always)]
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32 {
        unpremultiply(Px::<Argb8888>::get(img, x, y))
    }
}

pub(crate) struct Rgb565A8Premul;
impl Texel for Rgb565A8Premul {
    #[inline(always)]
    fn get(img: &ImagePixels<'_>, x: usize, y: usize) -> u32 {
        unpremultiply(Rgb565A8::get(img, x, y))
    }
}

/// Calls `$body` with `$T` bound to the texel reader of the image `$img`.
macro_rules! with_texel {
    ($img:expr, $T:ident => $body:expr) => {{
        use $crate::image::read::*;
        use twine_core::color as tc;
        match ($img.format, $img.is_premultiplied()) {
            (ColorFormat::Rgb565, _) => {
                type $T = Px<tc::Rgb565>;
                $body
            }
            (ColorFormat::Rgb565Swapped, _) => {
                type $T = Px<tc::Rgb565Swapped>;
                $body
            }
            (ColorFormat::Rgb888, _) => {
                type $T = Px<tc::Rgb888>;
                $body
            }
            (ColorFormat::Xrgb8888, _) => {
                type $T = Px<tc::Xrgb8888>;
                $body
            }
            (ColorFormat::Argb8888, false) => {
                type $T = Px<tc::Argb8888>;
                $body
            }
            (ColorFormat::Argb8888 | ColorFormat::Argb8888Premultiplied, _) => {
                type $T = Premul;
                $body
            }
            (ColorFormat::L8, _) => {
                type $T = Px<tc::L8>;
                $body
            }
            (ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8, _) => {
                type $T = Alpha;
                $body
            }
            (ColorFormat::I1 | ColorFormat::I2 | ColorFormat::I4 | ColorFormat::I8, _) => {
                type $T = Indexed;
                $body
            }
            (ColorFormat::Rgb565A8, false) => {
                type $T = Rgb565A8;
                $body
            }
            (ColorFormat::Rgb565A8, true) => {
                type $T = Rgb565A8Premul;
                $body
            }
        }
    }};
}
pub(crate) use with_texel;

/// Applies a recolor to a packed texel (alpha kept).
#[inline(always)]
pub(crate) fn recolor(v: u32, rc: Option<(Color, Opa)>) -> u32 {
    match rc {
        None => v,
        Some((c, o)) => {
            let m = Color::mix(c, Color::hex(v), o);
            pack(m, (v >> 24) as u8)
        }
    }
}

/// Color transformations applied to every texel before blending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TexelOps {
    /// `(color, opa)`: mix the texel color towards `color`.
    pub recolor: Option<(Color, Opa)>,
    /// Texels whose color (`0xRRGGBB`) equals this become transparent.
    pub chroma: Option<u32>,
}

impl TexelOps {
    /// Raw texel with the chroma key applied (before interpolation).
    #[inline(always)]
    pub fn key(&self, v: u32) -> u32 {
        match self.chroma {
            Some(k) if v & 0x00FF_FFFF == k => 0,
            _ => v,
        }
    }
}

/// Converts `n` pixels of row `y` starting at column `x` to `Argb8888` bytes in `out`,
/// applying `ops` (chroma key, then recolor).
pub(crate) fn convert_row<T: Texel>(
    img: &ImagePixels<'_>,
    x: usize,
    y: usize,
    n: usize,
    ops: TexelOps,
    out: &mut [u8],
) {
    for (i, px) in out[..n * 4].chunks_exact_mut(4).enumerate() {
        let v = recolor(ops.key(T::get(img, x + i, y)), ops.recolor);
        px.copy_from_slice(&v.to_le_bytes());
    }
}

/// Reads `w` pixels of row `y` of `px`, starting at column `x0`, as straight-alpha `Argb8888`
/// bytes (`B, G, R, A`) into `out` (at least `4 · w` bytes; shorter `out` reads fewer pixels).
///
/// Every [`ColorFormat`] is supported: indexed pixels go through the palette, alpha-only
/// formats give black with the pixel's alpha, `Rgb565A8` combines both planes, premultiplied
/// pixels are un-premultiplied and `L8` becomes gray. Pixels outside the image are transparent.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_render::{ImagePixels, read_row_argb};
///
/// let px = ImagePixels::new(ColorFormat::A8, 2, 1, &[0x40, 0xFF]);
/// let mut out = [0u8; 8];
/// read_row_argb(&px, 0, 0, 2, &mut out);
/// assert_eq!(out, [0, 0, 0, 0x40, 0, 0, 0, 0xFF]);
/// ```
pub fn read_row_argb(px: &ImagePixels<'_>, y: u16, x0: u16, w: u16, out: &mut [u8]) {
    let n = usize::from(w).min(out.len() / 4);
    if y >= px.h {
        out[..n * 4].fill(0);
        return;
    }
    let inside = usize::from(px.w.saturating_sub(x0)).min(n);
    with_texel!(px, T => convert_row::<T>(px, usize::from(x0), usize::from(y), inside, TexelOps::default(), out));
    out[inside * 4..n * 4].fill(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_reads_expected_texel() {
        // A red-ish opaque pixel in every opaque format.
        let c = Color::new(255, 0, 0);
        let mut out = [0u8; 4];
        let cases: [(ColorFormat, &[u8]); 6] = [
            (ColorFormat::Rgb565, &[0x00, 0xF8]),
            (ColorFormat::Rgb565Swapped, &[0xF8, 0x00]),
            (ColorFormat::Rgb888, &[0, 0, 255]),
            (ColorFormat::Xrgb8888, &[0, 0, 255, 0]),
            (ColorFormat::Argb8888, &[0, 0, 255, 255]),
            (ColorFormat::Argb8888Premultiplied, &[0, 0, 255, 255]),
        ];
        for (f, d) in cases {
            read_row_argb(&ImagePixels::new(f, 1, 1, d), 0, 0, 1, &mut out);
            assert_eq!(out, [c.b, c.g, c.r, 255], "{f}");
        }
        read_row_argb(&ImagePixels::new(ColorFormat::L8, 1, 1, &[77]), 0, 0, 1, &mut out);
        assert_eq!(out, [77, 77, 77, 255]);
        // Premultiplied half-transparent white.
        read_row_argb(
            &ImagePixels::new(ColorFormat::Argb8888Premultiplied, 1, 1, &[128, 128, 128, 128]),
            0,
            0,
            1,
            &mut out,
        );
        assert_eq!(out, [255, 255, 255, 128]);
        // Indexed with palette.
        let pal = [1, 2, 3, 4, 5, 6, 7, 8];
        let px = ImagePixels {
            palette: Some(&pal),
            ..ImagePixels::new(ColorFormat::I1, 2, 1, &[0b0100_0000])
        };
        let mut two = [0u8; 8];
        read_row_argb(&px, 0, 0, 2, &mut two);
        assert_eq!(two, [1, 2, 3, 4, 5, 6, 7, 8]);
        // Out of range columns are transparent.
        let mut two = [9u8; 8];
        read_row_argb(&px, 0, 1, 2, &mut two);
        assert_eq!(two, [5, 6, 7, 8, 0, 0, 0, 0]);
    }

    #[test]
    fn chroma_key_then_recolor() {
        let ops = TexelOps {
            recolor: Some((Color::BLUE, Opa::COVER)),
            chroma: Some(0x00FF_0000),
        };
        assert_eq!(ops.key(0xFFFF_0000), 0);
        assert_eq!(recolor(ops.key(0xFF00_FF00), ops.recolor), 0xFF00_00FF);
    }
}
