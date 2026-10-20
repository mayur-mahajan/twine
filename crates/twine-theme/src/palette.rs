//! [`Palette`]: LVGL's material design palette (`src/misc/lv_palette.c`), value for value.
// The color tables are copied verbatim from LVGL (`0xRRGGBB`).
#![allow(clippy::unreadable_literal)]

use twine_core::Color;

/// The 19 material colors of LVGL's `lv_palette_t`, in LVGL order. Each has ten
/// [`Tone`]s: [`tone`](Self::tone) (LVGL `lv_palette_lighten` / `lv_palette_darken`),
/// [`main`](Self::main) for the middle one. A palette entry converts into its main
/// [`Color`] (`Color::from(Palette::Teal)`), so theme builders take either.
///
/// ```
/// use twine_core::Color;
/// use twine_theme::{Palette, Tone};
/// assert_eq!(Palette::Blue.main(), Color::hex(0x2196F3));
/// assert_eq!(Palette::Grey.tone(Tone::L4), Color::hex(0xF5F5F5));
/// assert_eq!(Palette::Grey.tone(Tone::D4), Color::hex(0x212121));
/// assert_eq!(Color::from(Palette::Blue), Palette::Blue.tone(Tone::Main));
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

/// The ten shades of every palette entry, lightest to darkest in [`Tone`] order: LVGL's
/// `lv_palette_lighten` levels 5..=1, `palette_main`, then `lv_palette_darken` levels 1..=4
/// (`lv_palette.c`). A `match` instead of tables indexed by the discriminant: every lookup is
/// in range by construction (no index to check, the safety profile's `indexing_slicing`).
const fn shades(p: Palette) -> [u32; 10] {
    match p {
        Palette::Red => [
            0xFFEBEE, 0xFFCDD2, 0xEF9A9A, 0xE57373, 0xEF5350, 0xF44336, 0xE53935, 0xD32F2F, 0xC62828,
            0xB71C1C,
        ],
        Palette::Pink => [
            0xFCE4EC, 0xF8BBD0, 0xF48FB1, 0xF06292, 0xEC407A, 0xE91E63, 0xD81B60, 0xC2185B, 0xAD1457,
            0x880E4F,
        ],
        Palette::Purple => [
            0xF3E5F5, 0xE1BEE7, 0xCE93D8, 0xBA68C8, 0xAB47BC, 0x9C27B0, 0x8E24AA, 0x7B1FA2, 0x6A1B9A,
            0x4A148C,
        ],
        Palette::DeepPurple => [
            0xEDE7F6, 0xD1C4E9, 0xB39DDB, 0x9575CD, 0x7E57C2, 0x673AB7, 0x5E35B1, 0x512DA8, 0x4527A0,
            0x311B92,
        ],
        Palette::Indigo => [
            0xE8EAF6, 0xC5CAE9, 0x9FA8DA, 0x7986CB, 0x5C6BC0, 0x3F51B5, 0x3949AB, 0x303F9F, 0x283593,
            0x1A237E,
        ],
        Palette::Blue => [
            0xE3F2FD, 0xBBDEFB, 0x90CAF9, 0x64B5F6, 0x42A5F5, 0x2196F3, 0x1E88E5, 0x1976D2, 0x1565C0,
            0x0D47A1,
        ],
        Palette::LightBlue => [
            0xE1F5FE, 0xB3E5FC, 0x81D4FA, 0x4FC3F7, 0x29B6F6, 0x03A9F4, 0x039BE5, 0x0288D1, 0x0277BD,
            0x01579B,
        ],
        Palette::Cyan => [
            0xE0F7FA, 0xB2EBF2, 0x80DEEA, 0x4DD0E1, 0x26C6DA, 0x00BCD4, 0x00ACC1, 0x0097A7, 0x00838F,
            0x006064,
        ],
        Palette::Teal => [
            0xE0F2F1, 0xB2DFDB, 0x80CBC4, 0x4DB6AC, 0x26A69A, 0x009688, 0x00897B, 0x00796B, 0x00695C,
            0x004D40,
        ],
        Palette::Green => [
            0xE8F5E9, 0xC8E6C9, 0xA5D6A7, 0x81C784, 0x66BB6A, 0x4CAF50, 0x43A047, 0x388E3C, 0x2E7D32,
            0x1B5E20,
        ],
        Palette::LightGreen => [
            0xF1F8E9, 0xDCEDC8, 0xC5E1A5, 0xAED581, 0x9CCC65, 0x8BC34A, 0x7CB342, 0x689F38, 0x558B2F,
            0x33691E,
        ],
        Palette::Lime => [
            0xF9FBE7, 0xF0F4C3, 0xE6EE9C, 0xDCE775, 0xD4E157, 0xCDDC39, 0xC0CA33, 0xAFB42B, 0x9E9D24,
            0x827717,
        ],
        Palette::Yellow => [
            0xFFFDE7, 0xFFF9C4, 0xFFF59D, 0xFFF176, 0xFFEE58, 0xFFEB3B, 0xFDD835, 0xFBC02D, 0xF9A825,
            0xF57F17,
        ],
        Palette::Amber => [
            0xFFF8E1, 0xFFECB3, 0xFFE082, 0xFFD54F, 0xFFCA28, 0xFFC107, 0xFFB300, 0xFFA000, 0xFF8F00,
            0xFF6F00,
        ],
        Palette::Orange => [
            0xFFF3E0, 0xFFE0B2, 0xFFCC80, 0xFFB74D, 0xFFA726, 0xFF9800, 0xFB8C00, 0xF57C00, 0xEF6C00,
            0xE65100,
        ],
        Palette::DeepOrange => [
            0xFBE9E7, 0xFFCCBC, 0xFFAB91, 0xFF8A65, 0xFF7043, 0xFF5722, 0xF4511E, 0xE64A19, 0xD84315,
            0xBF360C,
        ],
        Palette::Brown => [
            0xEFEBE9, 0xD7CCC8, 0xBCAAA4, 0xA1887F, 0x8D6E63, 0x795548, 0x6D4C41, 0x5D4037, 0x4E342E,
            0x3E2723,
        ],
        Palette::BlueGrey => [
            0xECEFF1, 0xCFD8DC, 0xB0BEC5, 0x90A4AE, 0x78909C, 0x607D8B, 0x546E7A, 0x455A64, 0x37474F,
            0x263238,
        ],
        Palette::Grey => [
            0xFAFAFA, 0xF5F5F5, 0xEEEEEE, 0xE0E0E0, 0xBDBDBD, 0x9E9E9E, 0x757575, 0x616161, 0x424242,
            0x212121,
        ],
    }
}

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
        Color::hex(shades(self)[5])
    }

    /// The color of `tone`: [`Tone::L1`]…[`Tone::L5`] are LVGL's `lv_palette_lighten(p, 1..=5)`,
    /// [`Tone::D1`]…[`Tone::D4`] `lv_palette_darken(p, 1..=4)`, [`Tone::Main`]
    /// `lv_palette_main(p)`. Every tone exists for every palette entry: no level can be out of
    /// range.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_theme::{Palette, Tone};
    /// assert_eq!(Palette::Blue.tone(Tone::L2), Color::hex(0x64B5F6));
    /// assert_eq!(Palette::Red.tone(Tone::D1), Color::hex(0xE53935));
    /// ```
    #[doc(alias = "lv_palette_lighten")]
    #[doc(alias = "lv_palette_darken")]
    #[doc(alias = "lighten")]
    #[doc(alias = "darken")]
    #[must_use]
    pub const fn tone(self, tone: Tone) -> Color {
        let s = shades(self);
        Color::hex(match tone {
            Tone::L5 => s[0],
            Tone::L4 => s[1],
            Tone::L3 => s[2],
            Tone::L2 => s[3],
            Tone::L1 => s[4],
            Tone::Main => s[5],
            Tone::D1 => s[6],
            Tone::D2 => s[7],
            Tone::D3 => s[8],
            Tone::D4 => s[9],
        })
    }
}

/// The main color of the palette entry (LVGL `lv_palette_main`).
impl From<Palette> for Color {
    fn from(p: Palette) -> Color {
        p.main()
    }
}

/// A tone of a [`Palette`] color, lightest to darkest (Material's 50…900 shades).
///
/// ```
/// use twine_theme::{Palette, Tone};
/// assert_eq!(Tone::ALL.len(), 10);
/// assert!(Palette::Grey.tone(Tone::L5).relative_luminance() > Palette::Grey.tone(Tone::D4).relative_luminance());
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Tone {
    /// Lightest (`lv_palette_lighten(p, 5)`, Material 50).
    L5,
    /// `lv_palette_lighten(p, 4)` (Material 100).
    L4,
    /// `lv_palette_lighten(p, 3)` (Material 200).
    L3,
    /// `lv_palette_lighten(p, 2)` (Material 300).
    L2,
    /// `lv_palette_lighten(p, 1)` (Material 400).
    L1,
    /// The main color (`lv_palette_main`, Material 500).
    Main,
    /// `lv_palette_darken(p, 1)` (Material 600).
    D1,
    /// `lv_palette_darken(p, 2)` (Material 700).
    D2,
    /// `lv_palette_darken(p, 3)` (Material 800).
    D3,
    /// Darkest (`lv_palette_darken(p, 4)`, Material 900).
    D4,
}

impl Tone {
    /// Every tone, lightest to darkest.
    pub const ALL: [Tone; 10] = [
        Tone::L5,
        Tone::L4,
        Tone::L3,
        Tone::L2,
        Tone::L1,
        Tone::Main,
        Tone::D1,
        Tone::D2,
        Tone::D3,
        Tone::D4,
    ];
}
