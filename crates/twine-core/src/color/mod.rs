//! Colors: the canonical [`Color`] (RGB888), opacity ([`Opa`]), the runtime format tag
//! ([`ColorFormat`]) and typed pixel formats ([`PixelFormat`]).
//!
//! Every blend uses [`mix_channel`]: `udiv255(fg·a + bg·(255 − a))` — floor division by 255,
//! exact at `a = 0` and `a = 255`. This formula is normative for deterministic rendering (P6).

mod color_format;
mod formats;
mod opa;

use core::fmt;

pub use color_format::ColorFormat;
pub use formats::{
    A8, Argb8888, I1, L8, PixelFormat, Rgb565, Rgb565Swapped, Rgb888, Xrgb8888, expand_alpha, unpack_bits,
};
pub use opa::Opa;

use crate::math::udiv255;

/// Blends one 8-bit channel: `udiv255(fg·a + bg·(255 − a))`.
///
/// ```
/// use twine_core::color::mix_channel;
/// assert_eq!(mix_channel(200, 100, 255), 200);
/// assert_eq!(mix_channel(200, 100, 0), 100);
/// assert_eq!(mix_channel(255, 0, 128), 128);
/// ```
#[inline]
#[must_use]
pub const fn mix_channel(fg: u8, bg: u8, a: u8) -> u8 {
    udiv255(fg as u32 * a as u32 + bg as u32 * (255 - a as u32)) as u8
}

/// A 24-bit RGB color (the canonical color type; pixel formats convert from/to it).
///
/// ```
/// use twine_core::{Color, Opa};
/// let c = Color::hex(0x3366CC);
/// assert_eq!(c, Color::new(0x33, 0x66, 0xCC));
/// assert_eq!(Color::hex3(0x36C), c);
/// assert_eq!(Color::mix(Color::WHITE, Color::BLACK, Opa::COVER), Color::WHITE);
/// assert_eq!(c.to_string(), "#3366CC");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// `#FFFFFF`
    pub const WHITE: Color = Color::hex(0xFF_FF_FF);
    /// `#000000`
    pub const BLACK: Color = Color::hex(0x00_00_00);
    /// `#FF0000`
    pub const RED: Color = Color::hex(0xFF_00_00);
    /// `#00FF00`
    pub const GREEN: Color = Color::hex(0x00_FF_00);
    /// `#0000FF`
    pub const BLUE: Color = Color::hex(0x00_00_FF);
    /// `#FFFF00`
    pub const YELLOW: Color = Color::hex(0xFF_FF_00);
    /// `#00FFFF`
    pub const CYAN: Color = Color::hex(0x00_FF_FF);
    /// `#FF00FF`
    pub const MAGENTA: Color = Color::hex(0xFF_00_FF);
    /// `#808080`
    pub const GRAY: Color = Color::hex(0x80_80_80);

    /// Creates a color from its channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b }
    }

    /// From `0xRRGGBB` (higher bits are ignored).
    #[must_use]
    pub const fn hex(v: u32) -> Color {
        Color::new((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// From `0xRGB`; each nibble is expanded ×17 (`0xF` → `0xFF`).
    #[must_use]
    pub const fn hex3(v: u16) -> Color {
        Color::new(
            ((v >> 8) & 0xF) as u8 * 17,
            ((v >> 4) & 0xF) as u8 * 17,
            (v & 0xF) as u8 * 17,
        )
    }

    /// Packs to RGB565 (truncating the low bits).
    #[must_use]
    pub const fn to_rgb565(self) -> u16 {
        ((self.r as u16 >> 3) << 11) | ((self.g as u16 >> 2) << 5) | (self.b as u16 >> 3)
    }

    /// Expands RGB565 with bit replication (`r5 → (r5 << 3) | (r5 >> 2)`), so that
    /// `from_rgb565(v).to_rgb565() == v` for every `v`.
    #[must_use]
    pub const fn from_rgb565(v: u16) -> Color {
        let r = ((v >> 11) & 0x1F) as u8;
        let g = ((v >> 5) & 0x3F) as u8;
        let b = (v & 0x1F) as u8;
        Color::new((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))
    }

    /// Perceived brightness `(r·77 + g·151 + b·28) >> 8` (white → 255).
    #[must_use]
    pub const fn luminance(self) -> u8 {
        ((self.r as u32 * 77 + self.g as u32 * 151 + self.b as u32 * 28) >> 8) as u8
    }

    /// Blends `fg` over `bg` with alpha `a`, per channel with [`mix_channel`].
    #[must_use]
    pub const fn mix(fg: Color, bg: Color, a: Opa) -> Color {
        Color::new(
            mix_channel(fg.r, bg.r, a.0),
            mix_channel(fg.g, bg.g, a.0),
            mix_channel(fg.b, bg.b, a.0),
        )
    }

    /// Mixes towards white by `lvl`.
    #[must_use]
    pub const fn lighten(self, lvl: Opa) -> Color {
        Color::mix(Color::WHITE, self, lvl)
    }

    /// Mixes towards black by `lvl`.
    #[must_use]
    pub const fn darken(self, lvl: Opa) -> Color {
        Color::mix(Color::BLACK, self, lvl)
    }

    /// `0xAARRGGBB` with the given opacity as alpha.
    #[must_use]
    pub const fn to_u32_argb(self, opa: Opa) -> u32 {
        ((opa.0 as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Per-channel linear interpolation from `a` to `b` with `t` in `0..=1024`.
    #[must_use]
    pub const fn lerp(a: Color, b: Color, t: u16) -> Color {
        const fn ch(a: u8, b: u8, t: u16) -> u8 {
            let v = crate::math::lerp_i32(a as i32, b as i32, t);
            if v < 0 {
                0
            } else if v > 255 {
                255
            } else {
                v as u8
            }
        }
        Color::new(ch(a.r, b.r, t), ch(a.g, b.g, t), ch(a.b, b.b, t))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_endpoints_exact() {
        for fg in 0..=255u8 {
            for bg in 0..=255u8 {
                assert_eq!(mix_channel(fg, bg, 255), fg);
                assert_eq!(mix_channel(fg, bg, 0), bg);
            }
        }
    }

    #[test]
    fn mix_is_monotonic_in_alpha() {
        for (fg, bg) in [(255u8, 0u8), (0, 255), (200, 13), (13, 200), (77, 77)] {
            let mut prev = mix_channel(fg, bg, 0);
            for a in 1..=255u8 {
                let v = mix_channel(fg, bg, a);
                if fg >= bg {
                    assert!(v >= prev, "{fg} {bg} {a}");
                } else {
                    assert!(v <= prev, "{fg} {bg} {a}");
                }
                prev = v;
            }
        }
    }

    #[test]
    fn rgb565_roundtrip_of_565_colors_is_exact() {
        for v in 0..=u16::MAX {
            assert_eq!(Color::from_rgb565(v).to_rgb565(), v);
        }
        assert_eq!(Color::WHITE.to_rgb565(), 0xFFFF);
        assert_eq!(Color::RED.to_rgb565(), 0xF800);
        assert_eq!(Color::from_rgb565(0xFFFF), Color::WHITE);
    }

    #[test]
    fn luminance_white_is_255() {
        assert_eq!(Color::WHITE.luminance(), 255);
        assert_eq!(Color::BLACK.luminance(), 0);
        assert_eq!(Color::GREEN.luminance(), 150);
    }

    #[test]
    fn helpers() {
        assert_eq!(Color::hex3(0xF80), Color::new(255, 136, 0));
        assert_eq!(Color::BLACK.lighten(Opa::COVER), Color::WHITE);
        assert_eq!(Color::WHITE.darken(Opa::COVER), Color::BLACK);
        assert_eq!(Color::WHITE.darken(Opa::TRANSP), Color::WHITE);
        assert_eq!(Color::hex(0x12_34_56).to_u32_argb(Opa(0x80)), 0x8012_3456);
        assert_eq!(
            Color::lerp(Color::BLACK, Color::WHITE, 512),
            Color::new(128, 128, 128)
        );
        assert_eq!(Color::lerp(Color::RED, Color::BLUE, 0), Color::RED);
        assert_eq!(Color::lerp(Color::RED, Color::BLUE, 1024), Color::BLUE);
        assert_eq!(Color::lerp(Color::BLACK, Color::WHITE, 4096), Color::WHITE);
        assert_eq!(alloc::format!("{}", Color::GRAY), "#808080");
    }
}
