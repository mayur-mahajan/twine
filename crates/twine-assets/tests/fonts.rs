//! Every built-in font has the glyphs it promises.

use twine_assets::fonts::*;
use twine_text::{Font, GlyphCache, symbols};

const MONTSERRAT: &[(&str, &Font)] = &[
    ("8", &MONTSERRAT_8),
    ("10", &MONTSERRAT_10),
    ("12", &MONTSERRAT_12),
    ("14", &MONTSERRAT_14),
    ("16", &MONTSERRAT_16),
    ("18", &MONTSERRAT_18),
    ("20", &MONTSERRAT_20),
    ("22", &MONTSERRAT_22),
    ("24", &MONTSERRAT_24),
    ("26", &MONTSERRAT_26),
    ("28", &MONTSERRAT_28),
    ("30", &MONTSERRAT_30),
    ("32", &MONTSERRAT_32),
    ("34", &MONTSERRAT_34),
    ("36", &MONTSERRAT_36),
    ("38", &MONTSERRAT_38),
    ("40", &MONTSERRAT_40),
    ("42", &MONTSERRAT_42),
    ("44", &MONTSERRAT_44),
    ("46", &MONTSERRAT_46),
    ("48", &MONTSERRAT_48),
    ("14-subpx", &MONTSERRAT_14_SUBPX),
];

fn check_renders(name: &str, font: &'static Font, c: char, cache: &mut GlyphCache) {
    let (f, info) = font
        .glyph(c, None)
        .unwrap_or_else(|| panic!("{name}: missing {c:?}"));
    let mut out = vec![0u8; usize::from(info.box_w) * usize::from(info.box_h)];
    assert!(
        f.provider.render_a8(&info, &mut out, cache),
        "{name}: {c:?} does not decode"
    );
}

#[test]
fn every_font_has_ascii_and_symbols() {
    let mut cache = GlyphCache::default();
    for (name, font) in MONTSERRAT {
        assert!(font.line_height > 0);
        for c in (0x20u8..0x7F).map(char::from).chain(['°', '•']) {
            check_renders(name, font, c, &mut cache);
        }
        for &c in symbols::ALL {
            check_renders(name, font, c, &mut cache);
        }
    }
    for (name, font) in [("unscii 8", &UNSCII_8), ("unscii 16", &UNSCII_16)] {
        assert!(font.line_height > 0);
        for c in (0x20u8..0x7F).map(char::from) {
            check_renders(name, font, c, &mut cache);
        }
    }
}

#[test]
fn montserrat_14_metrics_regression_guard() {
    // Values of the committed generated file; a change means the generator or source changed.
    assert_eq!(MONTSERRAT_14.line_height, 18);
    assert_eq!(MONTSERRAT_14.base_line, 4);
    assert_eq!(UNSCII_8.line_height, 8);
    assert_eq!(UNSCII_16.line_height, 16);
}

#[test]
fn cjk_font_contains_common_chars() {
    let mut cache = GlyphCache::default();
    // 50 samples spread over the character list, plus kana and CJK punctuation.
    let list: Vec<char> = include_str!("../../../assets/fonts/cjk_common_lvgl.txt")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let sample: Vec<char> = list
        .iter()
        .step_by(list.len() / 50)
        .copied()
        .chain("你好世界中文あいうえおアイウエオ日本語、。「」（）".chars())
        .collect();
    assert!(sample.len() >= 50);
    for (name, font) in [
        ("sc 14", &SOURCE_HAN_SANS_SC_14_CJK),
        ("sc 16", &SOURCE_HAN_SANS_SC_16_CJK),
    ] {
        for c in sample.iter().copied().chain((0x20u8..0x7F).map(char::from)) {
            check_renders(name, font, c, &mut cache);
        }
        for &c in symbols::ALL {
            check_renders(name, font, c, &mut cache);
        }
    }
    assert_eq!(SOURCE_HAN_SANS_SC_16_CJK.line_height, 24);
}

#[test]
fn persian_font_has_presentation_forms() {
    let mut cache = GlyphCache::default();
    let f = &DEJAVU_16_PERSIAN_HEBREW;
    // Hebrew letters, Arabic and Persian base letters.
    for c in "שלום אבגד سلام پچژگکی".chars() {
        check_renders("dejavu", f, c, &mut cache);
    }
    // Presentation forms produced by the shaper: seen initial, lam-alef final, meem isolated,
    // gaf medial, Persian yeh final, lam-alef isolated.
    for cp in [0xFEB3u32, 0xFEFC, 0xFEE1, 0xFB95, 0xFBFD, 0xFEFB, 0xFE91, 0xFE90] {
        check_renders("dejavu", f, char::from_u32(cp).unwrap(), &mut cache);
    }
}
