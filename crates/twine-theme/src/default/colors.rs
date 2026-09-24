//! The colors of LVGL's default theme (`lv_theme_default.c`, the `LIGHT_COLOR_*` and
//! `DARK_COLOR_*` macros).

use twine_core::Color;

use crate::Palette;

/// Screen background, light mode (`LIGHT_COLOR_SCR` = `lv_palette_lighten(GREY, 4)`).
pub const LIGHT_SCR: Color = Palette::Grey.lighten(4);
/// Card (container) background, light mode (`LIGHT_COLOR_CARD` = white).
pub const LIGHT_CARD: Color = Color::WHITE;
/// Text, light mode (`LIGHT_COLOR_TEXT` = `lv_palette_darken(GREY, 4)`).
pub const LIGHT_TEXT: Color = Palette::Grey.darken(4);
/// Borders and neutral buttons, light mode (`LIGHT_COLOR_GREY` = `lv_palette_lighten(GREY, 2)`).
pub const LIGHT_GREY: Color = Palette::Grey.lighten(2);
/// Screen background, dark mode (`DARK_COLOR_SCR` = `0x15171A`).
pub const DARK_SCR: Color = Color::hex(0x0015_171A);
/// Card background, dark mode (`DARK_COLOR_CARD` = `0x282B30`).
pub const DARK_CARD: Color = Color::hex(0x0028_2B30);
/// Text, dark mode (`DARK_COLOR_TEXT` = `lv_palette_lighten(GREY, 5)`).
pub const DARK_TEXT: Color = Palette::Grey.lighten(5);
/// Borders and neutral buttons, dark mode (`DARK_COLOR_GREY` = `0x2F3237`).
pub const DARK_GREY: Color = Color::hex(0x002F_3237);

#[cfg(test)]
#[allow(clippy::unreadable_literal)] // as LVGL writes them
mod tests {
    use super::*;

    #[test]
    fn colors_match_lvgl() {
        assert_eq!(LIGHT_SCR, Color::hex(0xF5F5F5));
        assert_eq!(LIGHT_TEXT, Color::hex(0x212121));
        assert_eq!(LIGHT_GREY, Color::hex(0xE0E0E0));
        assert_eq!(DARK_TEXT, Color::hex(0xFAFAFA));
    }
}
