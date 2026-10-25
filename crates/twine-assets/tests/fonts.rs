//! Every built-in font has the glyphs it promises.

use twine_assets::fonts::*;
use twine_text::{Font, GlyphCache, Polarity, Symbol, has_placeholder};

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
    ("8-smooth", &MONTSERRAT_8_SMOOTH),
    ("10-smooth", &MONTSERRAT_10_SMOOTH),
    ("12-smooth", &MONTSERRAT_12_SMOOTH),
    ("14-smooth", &MONTSERRAT_14_SMOOTH),
    ("16-smooth", &MONTSERRAT_16_SMOOTH),
    ("18-smooth", &MONTSERRAT_18_SMOOTH),
    ("20-smooth", &MONTSERRAT_20_SMOOTH),
    ("22-smooth", &MONTSERRAT_22_SMOOTH),
    ("24-smooth", &MONTSERRAT_24_SMOOTH),
    ("26-smooth", &MONTSERRAT_26_SMOOTH),
    ("28-smooth", &MONTSERRAT_28_SMOOTH),
    ("30-smooth", &MONTSERRAT_30_SMOOTH),
    ("32-smooth", &MONTSERRAT_32_SMOOTH),
    ("34-smooth", &MONTSERRAT_34_SMOOTH),
    ("36-smooth", &MONTSERRAT_36_SMOOTH),
    ("38-smooth", &MONTSERRAT_38_SMOOTH),
    ("40-smooth", &MONTSERRAT_40_SMOOTH),
    ("42-smooth", &MONTSERRAT_42_SMOOTH),
    ("44-smooth", &MONTSERRAT_44_SMOOTH),
    ("46-smooth", &MONTSERRAT_46_SMOOTH),
    ("48-smooth", &MONTSERRAT_48_SMOOTH),
];

/// The code point of every [`Symbol`] that draws something (all but [`Symbol::Dummy`]).
fn drawn_symbols() -> impl Iterator<Item = char> {
    Symbol::ALL
        .iter()
        .filter(|&&s| s != Symbol::Dummy)
        .map(|s| s.as_char())
}

/// Every `Symbol` variant has a glyph in the built-in (symbol-merged) fonts, so no symbol is
/// drawn as a missing-glyph box; `Symbol::Dummy` is invisible by design.
#[test]
fn every_symbol_variant_has_a_glyph() {
    let mut cache = GlyphCache::default();
    for s in Symbol::ALL.iter().copied().filter(|&s| s != Symbol::Dummy) {
        check_renders(s.name(), &MONTSERRAT_14, s.as_char(), &mut cache);
        let (_, info) = MONTSERRAT_14.glyph(s.as_char(), None).unwrap();
        assert!(info.box_w > 0 && info.box_h > 0, "{}: empty glyph", s.name());
    }
    assert!(!has_placeholder(Symbol::Dummy.as_char()));
}

fn check_renders(name: &str, font: &'static Font, c: char, cache: &mut GlyphCache) {
    let (f, info) = font
        .glyph(c, None)
        .unwrap_or_else(|| panic!("{name}: missing {c:?}"));
    let mut out = vec![0u8; usize::from(info.box_w) * usize::from(info.box_h)];
    assert!(
        f.provider
            .render_a8(&info, Polarity::DarkOnLight.into(), &mut out, cache),
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
        for c in drawn_symbols() {
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
        for c in drawn_symbols() {
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

/// The A8 opacity rows of `c` in `font`, light on dark.
fn rows(font: &'static Font, c: char) -> Vec<Vec<u8>> {
    let (f, info) = font.glyph(c, None).unwrap();
    let w = usize::from(info.box_w);
    let mut out = vec![0u8; w * usize::from(info.box_h)];
    assert!(f.provider.render_a8(
        &info,
        Polarity::LightOnDark.into(),
        &mut out,
        &mut GlyphCache::new(0)
    ));
    out.chunks_exact(w).map(<[u8]>::to_vec).collect()
}

/// The `-smooth` fonts (R6.S05) are unhinted with LVGL's linear coverage: the stem of `l` at
/// 14 px straddles two columns, none fully opaque, where the hinted font puts it on one full
/// column; and every opacity is a step of the linear `× 17` expansion.
#[test]
fn smooth_fonts_are_unhinted_and_linear() {
    let full_column = |rows: &[Vec<u8>]| {
        let middle = &rows[rows.len() / 2];
        middle.contains(&255)
    };
    assert!(full_column(&rows(&MONTSERRAT_14, 'l')), "hinted: one full column");
    let smooth = rows(&MONTSERRAT_14_SMOOTH, 'l');
    assert!(!full_column(&smooth), "unhinted: the stem is split: {smooth:?}");
    for (name, font) in MONTSERRAT.iter().filter(|(n, _)| n.ends_with("-smooth")) {
        for c in ['l', 'i', ',', '.', 'a'] {
            for row in rows(font, c) {
                assert!(
                    row.iter().all(|a| a % 17 == 0),
                    "{name} {c:?}: not linear: {row:?}"
                );
            }
        }
    }
    // Same metrics as the hinted font except the advances: the line box is the font's.
    assert_eq!(MONTSERRAT_14_SMOOTH.line_height, MONTSERRAT_14.line_height);
}
