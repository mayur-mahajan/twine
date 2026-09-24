//! [`Palette`]: LVGL's material design palette (`src/misc/lv_palette.c`), value for value.
// The color tables are copied verbatim from LVGL (`0xRRGGBB`).
#![allow(clippy::unreadable_literal)]

use twine_core::Color;

/// The 19 material colors of LVGL's `lv_palette_t`, in LVGL order.
///
/// ```
/// use twine_core::Color;
/// use twine_theme::Palette;
/// assert_eq!(Palette::Blue.main(), Color::hex(0x2196F3));
/// assert_eq!(Palette::Grey.lighten(4), Color::hex(0xF5F5F5));
/// assert_eq!(Palette::Grey.darken(4), Color::hex(0x212121));
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Palette {
    /// `LV_PALETTE_RED`.
    Red,
    /// `LV_PALETTE_PINK`.
    Pink,
    /// `LV_PALETTE_PURPLE`.
    Purple,
    /// `LV_PALETTE_DEEP_PURPLE`.
    DeepPurple,
    /// `LV_PALETTE_INDIGO`.
    Indigo,
    /// `LV_PALETTE_BLUE`.
    Blue,
    /// `LV_PALETTE_LIGHT_BLUE`.
    LightBlue,
    /// `LV_PALETTE_CYAN`.
    Cyan,
    /// `LV_PALETTE_TEAL`.
    Teal,
    /// `LV_PALETTE_GREEN`.
    Green,
    /// `LV_PALETTE_LIGHT_GREEN`.
    LightGreen,
    /// `LV_PALETTE_LIME`.
    Lime,
    /// `LV_PALETTE_YELLOW`.
    Yellow,
    /// `LV_PALETTE_AMBER`.
    Amber,
    /// `LV_PALETTE_ORANGE`.
    Orange,
    /// `LV_PALETTE_DEEP_ORANGE`.
    DeepOrange,
    /// `LV_PALETTE_BROWN`.
    Brown,
    /// `LV_PALETTE_BLUE_GREY`.
    BlueGrey,
    /// `LV_PALETTE_GREY`.
    Grey,
}

/// `palette_main` of `lv_palette.c`, in [`Palette`] order.
const MAIN: [u32; 19] = [
    0xF44336, // Red
    0xE91E63, // Pink
    0x9C27B0, // Purple
    0x673AB7, // DeepPurple
    0x3F51B5, // Indigo
    0x2196F3, // Blue
    0x03A9F4, // LightBlue
    0x00BCD4, // Cyan
    0x009688, // Teal
    0x4CAF50, // Green
    0x8BC34A, // LightGreen
    0xCDDC39, // Lime
    0xFFEB3B, // Yellow
    0xFFC107, // Amber
    0xFF9800, // Orange
    0xFF5722, // DeepOrange
    0x795548, // Brown
    0x607D8B, // BlueGrey
    0x9E9E9E, // Grey
];

/// `lv_palette_lighten` table of `lv_palette.c` (levels 1..=5).
const LIGHT: [[u32; 5]; 19] = [
    [0xEF5350, 0xE57373, 0xEF9A9A, 0xFFCDD2, 0xFFEBEE], // Red
    [0xEC407A, 0xF06292, 0xF48FB1, 0xF8BBD0, 0xFCE4EC], // Pink
    [0xAB47BC, 0xBA68C8, 0xCE93D8, 0xE1BEE7, 0xF3E5F5], // Purple
    [0x7E57C2, 0x9575CD, 0xB39DDB, 0xD1C4E9, 0xEDE7F6], // DeepPurple
    [0x5C6BC0, 0x7986CB, 0x9FA8DA, 0xC5CAE9, 0xE8EAF6], // Indigo
    [0x42A5F5, 0x64B5F6, 0x90CAF9, 0xBBDEFB, 0xE3F2FD], // Blue
    [0x29B6F6, 0x4FC3F7, 0x81D4FA, 0xB3E5FC, 0xE1F5FE], // LightBlue
    [0x26C6DA, 0x4DD0E1, 0x80DEEA, 0xB2EBF2, 0xE0F7FA], // Cyan
    [0x26A69A, 0x4DB6AC, 0x80CBC4, 0xB2DFDB, 0xE0F2F1], // Teal
    [0x66BB6A, 0x81C784, 0xA5D6A7, 0xC8E6C9, 0xE8F5E9], // Green
    [0x9CCC65, 0xAED581, 0xC5E1A5, 0xDCEDC8, 0xF1F8E9], // LightGreen
    [0xD4E157, 0xDCE775, 0xE6EE9C, 0xF0F4C3, 0xF9FBE7], // Lime
    [0xFFEE58, 0xFFF176, 0xFFF59D, 0xFFF9C4, 0xFFFDE7], // Yellow
    [0xFFCA28, 0xFFD54F, 0xFFE082, 0xFFECB3, 0xFFF8E1], // Amber
    [0xFFA726, 0xFFB74D, 0xFFCC80, 0xFFE0B2, 0xFFF3E0], // Orange
    [0xFF7043, 0xFF8A65, 0xFFAB91, 0xFFCCBC, 0xFBE9E7], // DeepOrange
    [0x8D6E63, 0xA1887F, 0xBCAAA4, 0xD7CCC8, 0xEFEBE9], // Brown
    [0x78909C, 0x90A4AE, 0xB0BEC5, 0xCFD8DC, 0xECEFF1], // BlueGrey
    [0xBDBDBD, 0xE0E0E0, 0xEEEEEE, 0xF5F5F5, 0xFAFAFA], // Grey
];

/// `lv_palette_darken` table of `lv_palette.c` (levels 1..=4).
const DARK: [[u32; 4]; 19] = [
    [0xE53935, 0xD32F2F, 0xC62828, 0xB71C1C], // Red
    [0xD81B60, 0xC2185B, 0xAD1457, 0x880E4F], // Pink
    [0x8E24AA, 0x7B1FA2, 0x6A1B9A, 0x4A148C], // Purple
    [0x5E35B1, 0x512DA8, 0x4527A0, 0x311B92], // DeepPurple
    [0x3949AB, 0x303F9F, 0x283593, 0x1A237E], // Indigo
    [0x1E88E5, 0x1976D2, 0x1565C0, 0x0D47A1], // Blue
    [0x039BE5, 0x0288D1, 0x0277BD, 0x01579B], // LightBlue
    [0x00ACC1, 0x0097A7, 0x00838F, 0x006064], // Cyan
    [0x00897B, 0x00796B, 0x00695C, 0x004D40], // Teal
    [0x43A047, 0x388E3C, 0x2E7D32, 0x1B5E20], // Green
    [0x7CB342, 0x689F38, 0x558B2F, 0x33691E], // LightGreen
    [0xC0CA33, 0xAFB42B, 0x9E9D24, 0x827717], // Lime
    [0xFDD835, 0xFBC02D, 0xF9A825, 0xF57F17], // Yellow
    [0xFFB300, 0xFFA000, 0xFF8F00, 0xFF6F00], // Amber
    [0xFB8C00, 0xF57C00, 0xEF6C00, 0xE65100], // Orange
    [0xF4511E, 0xE64A19, 0xD84315, 0xBF360C], // DeepOrange
    [0x6D4C41, 0x5D4037, 0x4E342E, 0x3E2723], // Brown
    [0x546E7A, 0x455A64, 0x37474F, 0x263238], // BlueGrey
    [0x757575, 0x616161, 0x424242, 0x212121], // Grey
];

impl Palette {
    /// Every palette entry, in LVGL order.
    pub const ALL: [Palette; 19] = [
        Palette::Red,
        Palette::Pink,
        Palette::Purple,
        Palette::DeepPurple,
        Palette::Indigo,
        Palette::Blue,
        Palette::LightBlue,
        Palette::Cyan,
        Palette::Teal,
        Palette::Green,
        Palette::LightGreen,
        Palette::Lime,
        Palette::Yellow,
        Palette::Amber,
        Palette::Orange,
        Palette::DeepOrange,
        Palette::Brown,
        Palette::BlueGrey,
        Palette::Grey,
    ];

    /// The main color (LVGL `lv_palette_main`).
    #[must_use]
    pub const fn main(self) -> Color {
        Color::hex(MAIN[self as usize])
    }

    /// A lighter shade, `lvl` 1..=5 (LVGL `lv_palette_lighten`). Out-of-range levels are
    /// clamped silently (a `const fn` cannot log; see [`lighten_checked`](Self::lighten_checked)).
    #[must_use]
    pub const fn lighten(self, lvl: u8) -> Color {
        let i = if lvl < 1 {
            0
        } else if lvl > 5 {
            4
        } else {
            lvl as usize - 1
        };
        Color::hex(LIGHT[self as usize][i])
    }

    /// A darker shade, `lvl` 1..=4 (LVGL `lv_palette_darken`). Out-of-range levels are
    /// clamped silently (see [`darken_checked`](Self::darken_checked)).
    #[must_use]
    pub const fn darken(self, lvl: u8) -> Color {
        let i = if lvl < 1 {
            0
        } else if lvl > 4 {
            3
        } else {
            lvl as usize - 1
        };
        Color::hex(DARK[self as usize][i])
    }

    /// Like [`lighten`](Self::lighten), logging `warn!` for a level outside 1..=5.
    #[must_use]
    pub fn lighten_checked(self, lvl: u8) -> Color {
        if !(1..=5).contains(&lvl) {
            twine_core::warn!(target: "twine::style", "palette lighten level {} out of 1..=5, clamped", lvl);
        }
        self.lighten(lvl)
    }

    /// Like [`darken`](Self::darken), logging `warn!` for a level outside 1..=4.
    #[must_use]
    pub fn darken_checked(self, lvl: u8) -> Color {
        if !(1..=4).contains(&lvl) {
            twine_core::warn!(target: "twine::style", "palette darken level {} out of 1..=4, clamped", lvl);
        }
        self.darken(lvl)
    }
}
