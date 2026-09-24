//! Runtime TrueType fonts (feature `ttf`).
#![allow(clippy::unreadable_literal, clippy::borrow_as_ptr)] // colors; font identity checks
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use static_cell::StaticCell;
use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, ColorFormat, Rect};
use twine_fs::{MemoryFs, Vfs};
use twine_testing::snapshot::{Tolerance, assert_rgb_snapshot};
use twine_testing::{RenderHarness, snapshot_config};
use twine_text::{FontFileSource, GlyphCache, TextDsc, TextLayout, TtfError, TtfFont, draw_text};

static MONTSERRAT_TTF: &[u8] = include_bytes!("../../../assets/fonts/Montserrat-Medium.ttf");

#[test]
fn ttf_metrics_match_bitmap_font_roughly() {
    let f = TtfFont::new(MONTSERRAT_TTF, 14).unwrap();
    assert!(
        (f.line_height - MONTSERRAT_14.line_height).abs() <= 1,
        "{} vs {}",
        f.line_height,
        MONTSERRAT_14.line_height
    );
    assert!((f.base_line - MONTSERRAT_14.base_line).abs() <= 1);
    // Advances agree within a pixel per glyph (hinting-free outlines vs. the generator).
    let text = "Hello Twine 0123";
    let ttf_w = TextLayout::new(text, f).measure().w;
    let bmp_w = TextLayout::new(text, &MONTSERRAT_14).measure().w;
    assert!((ttf_w - bmp_w).abs() <= text.len() as i32, "{ttf_w} vs {bmp_w}");
    assert!(f.underline_thickness >= 1);
}

#[test]
fn ttf_invalid_input_is_an_error() {
    assert!(matches!(TtfFont::new(b"not a font", 14), Err(TtfError::Parse(_))));
    assert_eq!(
        TtfFont::new(MONTSERRAT_TTF, 0).unwrap_err(),
        TtfError::InvalidSize(0)
    );
    assert_eq!(
        TtfFont::new(MONTSERRAT_TTF, 999).unwrap_err(),
        TtfError::InvalidSize(999)
    );
}

#[test]
fn ttf_new_in_static_cell_once() {
    static CELL: StaticCell<TtfFont> = StaticCell::new();
    let f = TtfFont::new_in(&CELL, MONTSERRAT_TTF, 20).unwrap();
    assert!(f.line_height > 20);
    assert_eq!(
        TtfFont::new_in(&CELL, MONTSERRAT_TTF, 20).unwrap_err(),
        TtfError::AlreadyInitialized
    );
}

#[test]
fn ttf_from_file() {
    static FILES: &[(&str, &[u8])] = &[("fonts/montserrat.ttf", MONTSERRAT_TTF)];
    /// The adapter an application (or the engine) provides over its file system.
    struct VfsFonts(Vfs);
    impl FontFileSource for VfsFonts {
        fn read_all(&mut self, path: &str, out: &mut Vec<u8>) -> Result<(), TtfError> {
            *out = self.0.read_to_vec(path).map_err(|_| TtfError::File)?;
            Ok(())
        }
    }
    let mut vfs = Vfs::new();
    vfs.mount('A', Box::new(MemoryFs::new(FILES))).unwrap();
    let mut files = VfsFonts(vfs);
    let f = TtfFont::from_file(&mut files, "A:/fonts/montserrat.ttf", 17).unwrap();
    assert!(f.glyph('A', None).is_some());
    assert_eq!(
        TtfFont::from_file(&mut files, "A:/missing.ttf", 17).unwrap_err(),
        TtfError::File
    );
}

#[test]
fn ttf_render_snapshot_tolerant() {
    let mut h = RenderHarness::new(240, 120, ColorFormat::Rgb565);
    h.clear(Color::WHITE);
    let mut cache = GlyphCache::new(16 * 1024);
    let mut y = 4;
    for px in [13u16, 17, 23, 31] {
        let f = TtfFont::new(MONTSERRAT_TTF, px).unwrap();
        let mut d = TextDsc::new(f);
        d.color = Color::hex(0x1565C0);
        let text = format!("TTF {px} px AVg");
        h.paint(|p| draw_text(p, Rect::from_xywh(6, y, 230, 40), &text, &d, &mut cache));
        y += i32::from(f.line_height) + 2;
    }
    // Rasterization uses floats: allow small platform differences.
    assert_rgb_snapshot(
        &snapshot_config!(),
        "ttf_montserrat_sizes",
        240,
        120,
        &h.rgb888(),
        Tolerance::new(60, 24),
    );
}

#[test]
fn ttf_cache_hit_on_second_draw() {
    let f = TtfFont::new(MONTSERRAT_TTF, 18).unwrap();
    let d = TextDsc::new(f);
    let mut h = RenderHarness::new(120, 30, ColorFormat::Rgb565);
    let mut cache = GlyphCache::new(8 * 1024);
    h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 120, 30), "abca", &d, &mut cache));
    let s = cache.stats();
    assert_eq!((s.misses, s.hits), (3, 1), "'a' is rasterized once");
    let first = h.data().to_vec();
    h.clear(Color::WHITE);
    h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 120, 30), "abca", &d, &mut cache));
    let s = cache.stats();
    assert_eq!((s.misses, s.hits), (3, 5), "second draw only hits");
    assert!(h.data() == first.as_slice());
    // Without a cache budget the output is identical (rasterized per draw).
    let mut c0 = GlyphCache::new(0);
    h.clear(Color::WHITE);
    h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 120, 30), "abca", &d, &mut c0));
    assert!(h.data() == first.as_slice());
}

#[test]
fn ttf_missing_glyph_falls_back() {
    let f = TtfFont::new_with_fallback(MONTSERRAT_TTF, 16, Some(&MONTSERRAT_14)).unwrap();
    // The TTF has no Font Awesome symbols; the built-in font does.
    let ok = twine_text::symbols::OK.chars().next().unwrap();
    let (from, _) = f.glyph(ok, None).unwrap();
    assert!(core::ptr::eq(from, &MONTSERRAT_14));
    let (from, g) = f.glyph('A', None).unwrap();
    assert!(core::ptr::eq(from, f));
    assert!(g.box_w > 0);
    // Without a fallback the character is missing (placeholder advance).
    let plain = TtfFont::new(MONTSERRAT_TTF, 16).unwrap();
    assert!(plain.glyph(ok, None).is_none());
    assert_eq!(plain.advance_px(ok, None), plain.placeholder_width() + 2);
}
