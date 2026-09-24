//! CJK, Hebrew and Arabic/Persian text: built-in fonts, bidi reordering and shaping.
#![allow(clippy::unreadable_literal, clippy::borrow_as_ptr)] // colors; font identity checks

use twine_assets::fonts::{DEJAVU_16_PERSIAN_HEBREW, SOURCE_HAN_SANS_SC_16_CJK};
use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::Painter;
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_text::{GlyphCache, TextAlign, TextDir, TextDsc, draw_text, shape};

fn frame(p: &mut Painter<'_>, r: Rect) {
    let c = Color::hex(0xC0C0C0);
    p.fill(Rect::new(r.x0 - 1, r.y0 - 1, r.x1 + 1, r.y0), c, Opa::COVER);
    p.fill(Rect::new(r.x0 - 1, r.y1, r.x1 + 1, r.y1 + 1), c, Opa::COVER);
}

fn render(w: u16, h: u16, f: impl FnOnce(&mut Painter<'_>, &mut GlyphCache)) -> RenderHarness {
    let mut harness = RenderHarness::new(w, h, ColorFormat::Rgb565);
    harness.clear(Color::WHITE);
    let mut cache = GlyphCache::default();
    harness.paint(|p| f(p, &mut cache));
    harness
}

#[test]
fn cjk_16_sample() {
    let h = render(240, 120, |p, c| {
        let d = TextDsc::new(&SOURCE_HAN_SANS_SC_16_CJK);
        draw_text(
            p,
            Rect::from_xywh(4, 4, 232, 110),
            "你好，世界！中文字体。\n日本語：こんにちは、カタカナ。\nMixed ASCII 123",
            &d,
            c,
        );
    });
    assert_render_snapshot!(h, "cjk_16_sample");
}

#[test]
fn hebrew_16_sample() {
    let h = render(240, 70, |p, c| {
        let mut d = TextDsc::new(&DEJAVU_16_PERSIAN_HEBREW);
        d.base_dir = TextDir::Rtl;
        d.align = TextAlign::Auto;
        let r = Rect::from_xywh(4, 4, 232, 20);
        frame(p, r);
        draw_text(p, r, "שלום עולם 123", &d, c);
        d.base_dir = TextDir::Auto;
        let r = Rect::from_xywh(4, 34, 232, 20);
        frame(p, r);
        draw_text(p, r, "(שלום) Twine!", &d, c);
    });
    assert_render_snapshot!(h, "hebrew_16_sample");
}

#[test]
fn rtl_label_snapshot() {
    // A wrapped RTL paragraph: lines right-aligned, each reordered separately.
    let h = render(160, 90, |p, c| {
        let mut d = TextDsc::new(&DEJAVU_16_PERSIAN_HEBREW);
        d.base_dir = TextDir::Rtl;
        d.align = TextAlign::Auto;
        let r = Rect::from_xywh(4, 4, 152, 82);
        frame(p, r);
        draw_text(
            p,
            r,
            "זהו טקסט ארוך בעברית עם מספר 42 ומילה English באמצע.",
            &d,
            c,
        );
    });
    assert_render_snapshot!(h, "rtl_label");
}

#[test]
fn arabic_label_snapshot() {
    let h = render(240, 70, |p, c| {
        let mut d = TextDsc::new(&DEJAVU_16_PERSIAN_HEBREW);
        d.base_dir = TextDir::Auto;
        d.align = TextAlign::Auto;
        // Shape the logical text first, then draw (bidi reorders it).
        let ar = shape("مرحبا بالعالم");
        draw_text(p, Rect::from_xywh(4, 4, 232, 20), &ar, &d, c);
        let fa = shape("سلام دنیا، گل پژوهش");
        draw_text(p, Rect::from_xywh(4, 34, 232, 20), &fa, &d, c);
    });
    assert_render_snapshot!(h, "arabic_label");
}
