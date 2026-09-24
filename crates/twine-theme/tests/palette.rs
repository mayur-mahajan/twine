//! Palette values against LVGL `src/misc/lv_palette.c` and the DPI helper.
// The color tables are copied verbatim from LVGL (`0xRRGGBB`).
#![allow(clippy::unreadable_literal)]

use twine_core::Color;
use twine_theme::{Palette, dpx};

/// `(palette, main, lighten 1..=5, darken 1..=4)`: every row of `lv_palette.c`
/// (`lv_palette_main`, `lv_palette_lighten`, `lv_palette_darken`), 19 × 10 values.
const LVGL: [(Palette, u32, [u32; 5], [u32; 4]); 19] = [
    (
        Palette::Red,
        0xF44336,
        [0xEF5350, 0xE57373, 0xEF9A9A, 0xFFCDD2, 0xFFEBEE],
        [0xE53935, 0xD32F2F, 0xC62828, 0xB71C1C],
    ),
    (
        Palette::Pink,
        0xE91E63,
        [0xEC407A, 0xF06292, 0xF48FB1, 0xF8BBD0, 0xFCE4EC],
        [0xD81B60, 0xC2185B, 0xAD1457, 0x880E4F],
    ),
    (
        Palette::Purple,
        0x9C27B0,
        [0xAB47BC, 0xBA68C8, 0xCE93D8, 0xE1BEE7, 0xF3E5F5],
        [0x8E24AA, 0x7B1FA2, 0x6A1B9A, 0x4A148C],
    ),
    (
        Palette::DeepPurple,
        0x673AB7,
        [0x7E57C2, 0x9575CD, 0xB39DDB, 0xD1C4E9, 0xEDE7F6],
        [0x5E35B1, 0x512DA8, 0x4527A0, 0x311B92],
    ),
    (
        Palette::Indigo,
        0x3F51B5,
        [0x5C6BC0, 0x7986CB, 0x9FA8DA, 0xC5CAE9, 0xE8EAF6],
        [0x3949AB, 0x303F9F, 0x283593, 0x1A237E],
    ),
    (
        Palette::Blue,
        0x2196F3,
        [0x42A5F5, 0x64B5F6, 0x90CAF9, 0xBBDEFB, 0xE3F2FD],
        [0x1E88E5, 0x1976D2, 0x1565C0, 0x0D47A1],
    ),
    (
        Palette::LightBlue,
        0x03A9F4,
        [0x29B6F6, 0x4FC3F7, 0x81D4FA, 0xB3E5FC, 0xE1F5FE],
        [0x039BE5, 0x0288D1, 0x0277BD, 0x01579B],
    ),
    (
        Palette::Cyan,
        0x00BCD4,
        [0x26C6DA, 0x4DD0E1, 0x80DEEA, 0xB2EBF2, 0xE0F7FA],
        [0x00ACC1, 0x0097A7, 0x00838F, 0x006064],
    ),
    (
        Palette::Teal,
        0x009688,
        [0x26A69A, 0x4DB6AC, 0x80CBC4, 0xB2DFDB, 0xE0F2F1],
        [0x00897B, 0x00796B, 0x00695C, 0x004D40],
    ),
    (
        Palette::Green,
        0x4CAF50,
        [0x66BB6A, 0x81C784, 0xA5D6A7, 0xC8E6C9, 0xE8F5E9],
        [0x43A047, 0x388E3C, 0x2E7D32, 0x1B5E20],
    ),
    (
        Palette::LightGreen,
        0x8BC34A,
        [0x9CCC65, 0xAED581, 0xC5E1A5, 0xDCEDC8, 0xF1F8E9],
        [0x7CB342, 0x689F38, 0x558B2F, 0x33691E],
    ),
    (
        Palette::Lime,
        0xCDDC39,
        [0xD4E157, 0xDCE775, 0xE6EE9C, 0xF0F4C3, 0xF9FBE7],
        [0xC0CA33, 0xAFB42B, 0x9E9D24, 0x827717],
    ),
    (
        Palette::Yellow,
        0xFFEB3B,
        [0xFFEE58, 0xFFF176, 0xFFF59D, 0xFFF9C4, 0xFFFDE7],
        [0xFDD835, 0xFBC02D, 0xF9A825, 0xF57F17],
    ),
    (
        Palette::Amber,
        0xFFC107,
        [0xFFCA28, 0xFFD54F, 0xFFE082, 0xFFECB3, 0xFFF8E1],
        [0xFFB300, 0xFFA000, 0xFF8F00, 0xFF6F00],
    ),
    (
        Palette::Orange,
        0xFF9800,
        [0xFFA726, 0xFFB74D, 0xFFCC80, 0xFFE0B2, 0xFFF3E0],
        [0xFB8C00, 0xF57C00, 0xEF6C00, 0xE65100],
    ),
    (
        Palette::DeepOrange,
        0xFF5722,
        [0xFF7043, 0xFF8A65, 0xFFAB91, 0xFFCCBC, 0xFBE9E7],
        [0xF4511E, 0xE64A19, 0xD84315, 0xBF360C],
    ),
    (
        Palette::Brown,
        0x795548,
        [0x8D6E63, 0xA1887F, 0xBCAAA4, 0xD7CCC8, 0xEFEBE9],
        [0x6D4C41, 0x5D4037, 0x4E342E, 0x3E2723],
    ),
    (
        Palette::BlueGrey,
        0x607D8B,
        [0x78909C, 0x90A4AE, 0xB0BEC5, 0xCFD8DC, 0xECEFF1],
        [0x546E7A, 0x455A64, 0x37474F, 0x263238],
    ),
    (
        Palette::Grey,
        0x9E9E9E,
        [0xBDBDBD, 0xE0E0E0, 0xEEEEEE, 0xF5F5F5, 0xFAFAFA],
        [0x757575, 0x616161, 0x424242, 0x212121],
    ),
];

#[test]
fn palette_main_matches_lvgl() {
    assert_eq!(Palette::ALL.len(), 19);
    for (p, main, _, _) in LVGL {
        assert_eq!(p.main(), Color::hex(main), "{p:?}");
    }
}

#[test]
fn palette_all_19x10_values_match_lvgl() {
    for (i, (p, main, light, dark)) in LVGL.iter().enumerate() {
        assert_eq!(Palette::ALL[i], *p, "LVGL order");
        assert_eq!(p.main(), Color::hex(*main));
        for (lvl, v) in light.iter().enumerate() {
            assert_eq!(
                p.lighten(lvl as u8 + 1),
                Color::hex(*v),
                "{p:?} lighten {}",
                lvl + 1
            );
        }
        for (lvl, v) in dark.iter().enumerate() {
            assert_eq!(
                p.darken(lvl as u8 + 1),
                Color::hex(*v),
                "{p:?} darken {}",
                lvl + 1
            );
        }
    }
}

#[test]
fn lighten_darken_tables_match_lvgl() {
    // lv_palette_lighten: {LV_COLOR_MAKE(0xEF, 0x53, 0x50), ...} (RED, level 1)
    assert_eq!(Palette::Red.lighten(1), Color::hex(0xEF5350));
    // lv_palette_lighten: GREY row, level 2 = LV_COLOR_MAKE(0xE0, 0xE0, 0xE0) (LIGHT_COLOR_GREY)
    assert_eq!(Palette::Grey.lighten(2), Color::hex(0xE0E0E0));
    // lv_palette_lighten: GREY row, level 4 = LV_COLOR_MAKE(0xF5, 0xF5, 0xF5) (LIGHT_COLOR_SCR)
    assert_eq!(Palette::Grey.lighten(4), Color::hex(0xF5F5F5));
    // lv_palette_lighten: GREY row, level 5 = LV_COLOR_MAKE(0xFA, 0xFA, 0xFA) (DARK_COLOR_TEXT)
    assert_eq!(Palette::Grey.lighten(5), Color::hex(0xFAFAFA));
    // lv_palette_lighten: BLUE row, level 3 = LV_COLOR_MAKE(0x90, 0xCA, 0xF9)
    assert_eq!(Palette::Blue.lighten(3), Color::hex(0x90CAF9));
    // lv_palette_lighten: DEEP_ORANGE row, level 5 = LV_COLOR_MAKE(0xFB, 0xE9, 0xE7)
    assert_eq!(Palette::DeepOrange.lighten(5), Color::hex(0xFBE9E7));
    // lv_palette_darken: GREY row, level 4 = LV_COLOR_MAKE(0x21, 0x21, 0x21) (LIGHT_COLOR_TEXT)
    assert_eq!(Palette::Grey.darken(4), Color::hex(0x212121));
    // lv_palette_darken: GREY row, level 2 = LV_COLOR_MAKE(0x61, 0x61, 0x61) (dark scrollbar)
    assert_eq!(Palette::Grey.darken(2), Color::hex(0x616161));
    // lv_palette_darken: TEAL row, level 4 = LV_COLOR_MAKE(0x00, 0x4D, 0x40)
    assert_eq!(Palette::Teal.darken(4), Color::hex(0x004D40));
    // lv_palette_darken: AMBER row, level 1 = LV_COLOR_MAKE(0xFF, 0xB3, 0x00)
    assert_eq!(Palette::Amber.darken(1), Color::hex(0xFFB300));
}

#[test]
fn out_of_range_levels_clamp() {
    assert_eq!(Palette::Red.lighten(0), Palette::Red.lighten(1));
    assert_eq!(Palette::Red.lighten(9), Palette::Red.lighten(5));
    assert_eq!(Palette::Red.darken(0), Palette::Red.darken(1));
    assert_eq!(Palette::Red.darken(7), Palette::Red.darken(4));
    assert_eq!(Palette::Red.lighten_checked(9), Palette::Red.lighten(5));
    assert_eq!(Palette::Red.darken_checked(0), Palette::Red.darken(1));
}

#[test]
fn dpx_rounds_like_lvgl() {
    assert_eq!(dpx(1, 130), 1);
    assert_eq!(dpx(10, 160), 10);
    assert_eq!(dpx(10, 320), 20);
    assert_eq!(dpx(-5, 160), -5);
    assert_eq!(dpx(0, 130), 0);
    // LV_DPX_CALC(130, 12) = (130 * 12 + 80) / 160 = 10
    assert_eq!(dpx(12, 130), 10);
    assert_eq!(dpx(12, 260), 20);
}
