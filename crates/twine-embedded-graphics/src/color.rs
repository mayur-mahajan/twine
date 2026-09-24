//! [`EgColor`]: the embedded-graphics colour types that map onto a twine [`ColorFormat`].

use embedded_graphics_core::pixelcolor::raw::RawU16;
use embedded_graphics_core::pixelcolor::{
    BinaryColor, Gray8, GrayColor, IntoStorage, PixelColor, Rgb565, Rgb888, RgbColor,
};
use twine_core::{Color, ColorFormat};

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Rgb565 {}
    impl Sealed for super::Rgb888 {}
    impl Sealed for super::Gray8 {}
    impl Sealed for super::BinaryColor {}
}

/// An embedded-graphics colour with an exact twine pixel format.
///
/// | embedded-graphics | twine format | bytes of pixel `x` in a row |
/// |-------------------|--------------|-----------------------------|
/// | `Rgb565` | `Rgb565` | `2x`, `2x + 1` (little-endian) |
/// | `Rgb888` | `Rgb888` | `3x` … `3x + 2` (B, G, R) |
/// | `Gray8` | `L8` | `x` |
/// | `BinaryColor` | `I1` | bit `7 − x % 8` of byte `x / 8` (`1` = `On`) |
///
/// Sealed: the list is fixed by the formats the twine renderer produces.
///
/// ```
/// use embedded_graphics_core::pixelcolor::{BinaryColor, Rgb565};
/// use twine_core::ColorFormat;
/// use twine_embedded_graphics::EgColor;
///
/// assert_eq!(Rgb565::FORMAT, ColorFormat::Rgb565);
/// assert_eq!(Rgb565::from_row(&[0x00, 0xF8], 0), Rgb565::new(31, 0, 0));
/// assert_eq!(BinaryColor::from_row(&[0b0100_0000], 1), BinaryColor::On);
/// ```
pub trait EgColor: PixelColor + sealed::Sealed {
    /// The matching twine pixel format.
    const FORMAT: ColorFormat;

    /// Pixel `x` of a row of [`FORMAT`](Self::FORMAT) pixels. `row` must hold pixel `x`.
    fn from_row(row: &[u8], x: usize) -> Self;

    /// The colour as a twine [`Color`] (`On` = white, `Off` = black).
    fn to_color(self) -> Color;
}

impl EgColor for Rgb565 {
    const FORMAT: ColorFormat = ColorFormat::Rgb565;

    #[inline]
    fn from_row(row: &[u8], x: usize) -> Self {
        Rgb565::from(RawU16::new(u16::from_le_bytes([row[2 * x], row[2 * x + 1]])))
    }

    #[inline]
    fn to_color(self) -> Color {
        Color::from_rgb565(self.into_storage())
    }
}

impl EgColor for Rgb888 {
    const FORMAT: ColorFormat = ColorFormat::Rgb888;

    #[inline]
    fn from_row(row: &[u8], x: usize) -> Self {
        Rgb888::new(row[3 * x + 2], row[3 * x + 1], row[3 * x])
    }

    #[inline]
    fn to_color(self) -> Color {
        Color::new(self.r(), self.g(), self.b())
    }
}

impl EgColor for Gray8 {
    const FORMAT: ColorFormat = ColorFormat::L8;

    #[inline]
    fn from_row(row: &[u8], x: usize) -> Self {
        Gray8::new(row[x])
    }

    #[inline]
    fn to_color(self) -> Color {
        let l = self.luma();
        Color::new(l, l, l)
    }
}

impl EgColor for BinaryColor {
    const FORMAT: ColorFormat = ColorFormat::I1;

    #[inline]
    fn from_row(row: &[u8], x: usize) -> Self {
        BinaryColor::from(row[x / 8] & (0x80 >> (x % 8)) != 0)
    }

    #[inline]
    fn to_color(self) -> Color {
        if self.is_on() { Color::WHITE } else { Color::BLACK }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_round_trips_through_twine_color() {
        for raw in [0x0000u16, 0xF800, 0x07E0, 0x001F, 0xFFFF, 0x1234] {
            let c = Rgb565::from_row(&raw.to_le_bytes(), 0);
            assert_eq!(c.into_storage(), raw);
            assert_eq!(c.to_color().to_rgb565(), raw);
        }
    }

    #[test]
    fn rgb888_is_stored_bgr() {
        let c = Rgb888::from_row(&[0, 0, 0, 0x30, 0x20, 0x10], 1);
        assert_eq!(c, Rgb888::new(0x10, 0x20, 0x30));
        assert_eq!(c.to_color(), Color::new(0x10, 0x20, 0x30));
    }

    #[test]
    fn gray8_is_l8() {
        assert_eq!(Gray8::from_row(&[1, 2, 3], 2), Gray8::new(3));
        assert_eq!(Gray8::new(9).to_color(), Color::new(9, 9, 9));
    }
}
