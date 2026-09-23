//! Typed, byte-aligned pixel formats ([`PixelFormat`]) and helpers for sub-byte formats.
//!
//! Multi-byte raw values are stored **little-endian** in memory (all supported targets are
//! little-endian; using an explicit byte order keeps rendering bit-identical everywhere).

use super::{Color, ColorFormat, Opa};

/// A byte-aligned pixel format the renderer is monomorphized over.
///
/// Normative semantics of [`PixelFormat::mix`]: identical to
/// `from_color(Color::mix(to_color(fg), to_color(bg), a))`. Implementations may use faster
/// code only if they produce the same results.
///
/// ```
/// use twine_core::{Color, Opa, PixelFormat, color::Rgb565};
/// let raw = Rgb565::from_color(Color::RED);
/// assert_eq!(raw, 0xF800);
/// let mut px = [0u8; 2];
/// Rgb565::write(&mut px, raw);
/// assert_eq!(px, [0x00, 0xF8]);
/// assert_eq!(Rgb565::mix(raw, Rgb565::from_color(Color::BLACK), Opa::COVER), raw);
/// ```
pub trait PixelFormat: Copy + 'static {
    /// The in-memory representation of one pixel.
    type Raw: Copy + Default + PartialEq + core::fmt::Debug + 'static;
    /// The runtime tag of this format.
    const FORMAT: ColorFormat;
    /// Bytes per pixel.
    const BYTES: usize;
    /// Converts a color to the raw pixel value.
    fn from_color(c: Color) -> Self::Raw;
    /// Converts a raw pixel value to a color.
    fn to_color(raw: Self::Raw) -> Color;
    /// Blends `fg` over `bg` with alpha `a` (see the trait docs for the normative semantics).
    fn mix(fg: Self::Raw, bg: Self::Raw, a: Opa) -> Self::Raw;
    /// Reads a pixel from the first [`Self::BYTES`](PixelFormat::BYTES) bytes of `bytes`.
    ///
    /// # Panics
    /// If `bytes` is shorter than `BYTES` (callers slice whole pixels).
    fn read(bytes: &[u8]) -> Self::Raw;
    /// Writes a pixel to the first `BYTES` bytes of `bytes`.
    ///
    /// # Panics
    /// If `bytes` is shorter than `BYTES`.
    fn write(bytes: &mut [u8], raw: Self::Raw);
}

/// Implements `mix` through the normative generic formula.
macro_rules! generic_mix {
    () => {
        #[inline]
        fn mix(fg: Self::Raw, bg: Self::Raw, a: Opa) -> Self::Raw {
            Self::from_color(Color::mix(Self::to_color(fg), Self::to_color(bg), a))
        }
    };
}

/// RGB565, little-endian in memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Rgb565;

impl PixelFormat for Rgb565 {
    type Raw = u16;
    const FORMAT: ColorFormat = ColorFormat::Rgb565;
    const BYTES: usize = 2;
    #[inline]
    fn from_color(c: Color) -> u16 {
        c.to_rgb565()
    }
    #[inline]
    fn to_color(raw: u16) -> Color {
        Color::from_rgb565(raw)
    }
    generic_mix!();
    #[inline]
    fn read(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[0], bytes[1]])
    }
    #[inline]
    fn write(bytes: &mut [u8], raw: u16) {
        bytes[..2].copy_from_slice(&raw.to_le_bytes());
    }
}

/// RGB565 with swapped bytes: `raw = rgb565.swap_bytes()`, so the **memory bytes are
/// big-endian** and an SPI panel receives the high byte first without a conversion pass.
///
/// ```
/// use twine_core::{Color, PixelFormat, color::Rgb565Swapped};
/// let mut px = [0u8; 2];
/// Rgb565Swapped::write(&mut px, Rgb565Swapped::from_color(Color::RED));
/// assert_eq!(px, [0xF8, 0x00]);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Rgb565Swapped;

impl PixelFormat for Rgb565Swapped {
    type Raw = u16;
    const FORMAT: ColorFormat = ColorFormat::Rgb565Swapped;
    const BYTES: usize = 2;
    #[inline]
    fn from_color(c: Color) -> u16 {
        c.to_rgb565().swap_bytes()
    }
    #[inline]
    fn to_color(raw: u16) -> Color {
        Color::from_rgb565(raw.swap_bytes())
    }
    generic_mix!();
    #[inline]
    fn read(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[0], bytes[1]])
    }
    #[inline]
    fn write(bytes: &mut [u8], raw: u16) {
        bytes[..2].copy_from_slice(&raw.to_le_bytes());
    }
}

/// 24-bit RGB stored as `[B, G, R]` (like LVGL).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Rgb888;

impl PixelFormat for Rgb888 {
    type Raw = [u8; 3];
    const FORMAT: ColorFormat = ColorFormat::Rgb888;
    const BYTES: usize = 3;
    #[inline]
    fn from_color(c: Color) -> [u8; 3] {
        [c.b, c.g, c.r]
    }
    #[inline]
    fn to_color(raw: [u8; 3]) -> Color {
        Color::new(raw[2], raw[1], raw[0])
    }
    generic_mix!();
    #[inline]
    fn read(bytes: &[u8]) -> [u8; 3] {
        [bytes[0], bytes[1], bytes[2]]
    }
    #[inline]
    fn write(bytes: &mut [u8], raw: [u8; 3]) {
        bytes[..3].copy_from_slice(&raw);
    }
}

/// 32-bit `0xFF_RR_GG_BB` (alpha byte ignored on read, written as `0xFF`), little-endian.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Xrgb8888;

impl PixelFormat for Xrgb8888 {
    type Raw = u32;
    const FORMAT: ColorFormat = ColorFormat::Xrgb8888;
    const BYTES: usize = 4;
    #[inline]
    fn from_color(c: Color) -> u32 {
        c.to_u32_argb(Opa::COVER)
    }
    #[inline]
    fn to_color(raw: u32) -> Color {
        Color::hex(raw)
    }
    generic_mix!();
    #[inline]
    fn read(bytes: &[u8]) -> u32 {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    #[inline]
    fn write(bytes: &mut [u8], raw: u32) {
        bytes[..4].copy_from_slice(&raw.to_le_bytes());
    }
}

/// 32-bit `0xAA_RR_GG_BB` with straight alpha, little-endian (memory `B, G, R, A`).
///
/// [`PixelFormat::from_color`] sets alpha to 255; use [`Argb8888::from_color_opa`] for other
/// opacities. [`PixelFormat::mix`] follows the normative generic formula (result alpha 255).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Argb8888;

impl Argb8888 {
    /// Raw value of `c` with opacity `opa` as alpha.
    #[inline]
    #[must_use]
    pub const fn from_color_opa(c: Color, opa: Opa) -> u32 {
        c.to_u32_argb(opa)
    }

    /// The alpha of a raw value.
    #[inline]
    #[must_use]
    pub const fn alpha(raw: u32) -> Opa {
        Opa((raw >> 24) as u8)
    }
}

impl PixelFormat for Argb8888 {
    type Raw = u32;
    const FORMAT: ColorFormat = ColorFormat::Argb8888;
    const BYTES: usize = 4;
    #[inline]
    fn from_color(c: Color) -> u32 {
        c.to_u32_argb(Opa::COVER)
    }
    #[inline]
    fn to_color(raw: u32) -> Color {
        Color::hex(raw)
    }
    generic_mix!();
    #[inline]
    fn read(bytes: &[u8]) -> u32 {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    #[inline]
    fn write(bytes: &mut [u8], raw: u32) {
        bytes[..4].copy_from_slice(&raw.to_le_bytes());
    }
}

/// 8-bit luminance; converts to gray.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct L8;

impl PixelFormat for L8 {
    type Raw = u8;
    const FORMAT: ColorFormat = ColorFormat::L8;
    const BYTES: usize = 1;
    #[inline]
    fn from_color(c: Color) -> u8 {
        c.luminance()
    }
    #[inline]
    fn to_color(raw: u8) -> Color {
        Color::new(raw, raw, raw)
    }
    generic_mix!();
    #[inline]
    fn read(bytes: &[u8]) -> u8 {
        bytes[0]
    }
    #[inline]
    fn write(bytes: &mut [u8], raw: u8) {
        bytes[0] = raw;
    }
}

/// 8-bit alpha-only format (not a [`PixelFormat`]; used for masks and glyphs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct A8;

impl A8 {
    /// The alpha of pixel `x` in `row`; out of range → transparent.
    #[inline]
    #[must_use]
    pub fn get(row: &[u8], x: usize) -> Opa {
        Opa(row.get(x).copied().unwrap_or(0))
    }
}

/// 1-bit format, most significant bit first (like LVGL; not a [`PixelFormat`]).
///
/// ```
/// use twine_core::color::I1;
/// let mut row = [0u8; 2];
/// I1::set(&mut row, 0, true);
/// I1::set(&mut row, 9, true);
/// assert_eq!(row, [0x80, 0x40]);
/// assert!(I1::get(&row, 9) && !I1::get(&row, 8));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct I1;

impl I1 {
    /// Bit `x` of `row` (MSB first); out of range → `false`.
    #[inline]
    #[must_use]
    pub fn get(row: &[u8], x: usize) -> bool {
        row.get(x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) != 0)
    }

    /// Sets bit `x` of `row` (MSB first); out of range is ignored with a warning.
    #[inline]
    pub fn set(row: &mut [u8], x: usize, v: bool) {
        let Some(b) = row.get_mut(x / 8) else {
            crate::warn!(target: "twine::core", "I1::set: x = {} out of range", x);
            return;
        };
        let mask = 0x80u8 >> (x % 8);
        if v {
            *b |= mask;
        } else {
            *b &= !mask;
        }
    }
}

/// Raw value of pixel `x` in a row of `bpp`-bit pixels (1, 2, 4 packed MSB first; 8 = bytes).
/// Other depths and out-of-range pixels return 0.
///
/// ```
/// use twine_core::color::unpack_bits;
/// assert_eq!(unpack_bits(&[0b1011_0001], 2, 1), 0b11);
/// assert_eq!(unpack_bits(&[0xA5], 4, 1), 0x5);
/// ```
#[inline]
#[must_use]
pub fn unpack_bits(row: &[u8], bpp: u8, x: usize) -> u8 {
    match bpp {
        1 | 2 | 4 => {
            let per_byte = 8 / usize::from(bpp);
            let Some(&byte) = row.get(x / per_byte) else {
                return 0;
            };
            let shift = 8 - usize::from(bpp) * (x % per_byte + 1);
            (byte >> shift) & ((1u8 << bpp) - 1)
        }
        8 => row.get(x).copied().unwrap_or(0),
        _ => 0,
    }
}

/// Expands a `bpp`-bit alpha to 8 bits (1 → 0/255, 2 → ×85, 4 → ×17, 8 → unchanged).
#[inline]
#[must_use]
pub const fn expand_alpha(v: u8, bpp: u8) -> u8 {
    match bpp {
        1 => {
            if v & 1 != 0 {
                255
            } else {
                0
            }
        }
        2 => (v & 0x3) * 85,
        4 => (v & 0xF) * 17,
        _ => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::XorShift32;

    #[test]
    fn rgb565_swapped_memory_order_is_big_endian() {
        let mut px = [0u8; 2];
        Rgb565Swapped::write(&mut px, Rgb565Swapped::from_color(Color::RED));
        assert_eq!(px, [0xF8, 0x00]);
        assert_eq!(Rgb565Swapped::to_color(Rgb565Swapped::read(&px)), Color::RED);
        Rgb565::write(&mut px, Rgb565::from_color(Color::RED));
        assert_eq!(px, [0x00, 0xF8]);
    }

    #[test]
    fn rgb888_memory_order_is_bgr() {
        let mut px = [0u8; 3];
        Rgb888::write(&mut px, Rgb888::from_color(Color::new(1, 2, 3)));
        assert_eq!(px, [3, 2, 1]);
        assert_eq!(Rgb888::to_color(Rgb888::read(&px)), Color::new(1, 2, 3));
        let mut px = [0u8; 4];
        Xrgb8888::write(&mut px, Xrgb8888::from_color(Color::new(1, 2, 3)));
        assert_eq!(px, [3, 2, 1, 0xFF]);
    }

    #[test]
    fn argb8888_alpha_roundtrip() {
        for a in 0..=255u8 {
            let raw = Argb8888::from_color_opa(Color::new(10, 20, 30), Opa(a));
            assert_eq!(Argb8888::alpha(raw), Opa(a));
            let mut px = [0u8; 4];
            Argb8888::write(&mut px, raw);
            assert_eq!(px, [30, 20, 10, a]);
            assert_eq!(Argb8888::read(&px), raw);
            assert_eq!(Argb8888::to_color(raw), Color::new(10, 20, 30));
        }
        assert_eq!(Argb8888::alpha(Argb8888::from_color(Color::BLACK)), Opa::COVER);
    }

    fn check_mix<F: PixelFormat>(rng: &mut XorShift32, random: impl Fn(u32) -> F::Raw) {
        let alphas = [0u8, 1, 127, 254, 255];
        for i in 0..10_000 {
            let fg = random(rng.next_u32());
            let bg = random(rng.next_u32());
            let a = if i < alphas.len() * 20 {
                Opa(alphas[i % alphas.len()])
            } else {
                rng.next_opa()
            };
            let expected = F::from_color(Color::mix(F::to_color(fg), F::to_color(bg), a));
            assert_eq!(F::mix(fg, bg, a), expected, "{:?} {fg:?} {bg:?} {a:?}", F::FORMAT);
        }
    }

    #[test]
    fn pixel_mix_matches_generic_formula() {
        let mut rng = XorShift32::new(0x1234_5678);
        check_mix::<Rgb565>(&mut rng, |v| v as u16);
        check_mix::<Rgb565Swapped>(&mut rng, |v| v as u16);
        check_mix::<Rgb888>(&mut rng, |v| [v as u8, (v >> 8) as u8, (v >> 16) as u8]);
        check_mix::<Xrgb8888>(&mut rng, |v| v | 0xFF00_0000);
        check_mix::<Argb8888>(&mut rng, |v| v);
        check_mix::<L8>(&mut rng, |v| v as u8);
    }

    #[test]
    fn unpack_bits_msb_first() {
        let row = [0b1000_0001u8, 0b0110_1100];
        let bits: [u8; 16] = core::array::from_fn(|x| unpack_bits(&row, 1, x));
        assert_eq!(bits, [1, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 0, 1, 1, 0, 0]);
        let twos: [u8; 8] = core::array::from_fn(|x| unpack_bits(&row, 2, x));
        assert_eq!(twos, [0b10, 0, 0, 0b01, 0b01, 0b10, 0b11, 0]);
        let fours: [u8; 4] = core::array::from_fn(|x| unpack_bits(&row, 4, x));
        assert_eq!(fours, [0x8, 0x1, 0x6, 0xC]);
        assert_eq!(unpack_bits(&row, 8, 1), 0b0110_1100);
        assert_eq!(unpack_bits(&row, 4, 4), 0);
        assert_eq!(unpack_bits(&row, 3, 0), 0);
        assert_eq!(expand_alpha(1, 1), 255);
        assert_eq!(expand_alpha(2, 2), 170);
        assert_eq!(expand_alpha(0xF, 4), 255);
        assert_eq!(expand_alpha(77, 8), 77);
        assert_eq!(A8::get(&[1, 2], 1), Opa(2));
        assert_eq!(A8::get(&[1, 2], 5), Opa::TRANSP);
        let mut r = [0xFFu8];
        I1::set(&mut r, 3, false);
        assert_eq!(r, [0b1110_1111]);
        I1::set(&mut r, 99, true);
        assert!(!I1::get(&r, 99));
    }
}
