//! Text drawing snapshots (Rgb565, 240×160) and drawing invariants.
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure
#![allow(clippy::unreadable_literal)] // colors read better as 0xRRGGBB

use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_14_SUBPX, MONTSERRAT_20, MONTSERRAT_28, UNSCII_8};
use twine_core::{Color, ColorFormat, Opa, Point, Rect};
use twine_render::Painter;
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_text::{GlyphCache, TextAlign, TextDecor, TextDsc, draw_text, symbols};

const W: u16 = 240;
const H: u16 = 160;

type Scene = fn(&mut Painter<'_>, &mut GlyphCache);

fn frame(p: &mut Painter<'_>, r: Rect) {
    let c = Color::hex(0xC0C0C0);
    p.fill(Rect::new(r.x0 - 1, r.y0 - 1, r.x1 + 1, r.y0), c, Opa::COVER);
    p.fill(Rect::new(r.x0 - 1, r.y1, r.x1 + 1, r.y1 + 1), c, Opa::COVER);
    p.fill(Rect::new(r.x0 - 1, r.y0, r.x0, r.y1), c, Opa::COVER);
    p.fill(Rect::new(r.x1, r.y0, r.x1 + 1, r.y1), c, Opa::COVER);
}

fn basic(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_14);
    for (i, align) in [TextAlign::Left, TextAlign::Center, TextAlign::Right]
        .into_iter()
        .enumerate()
    {
        let r = Rect::from_xywh(10, 8 + i as i32 * 26, 220, 20);
        frame(p, r);
        d.align = align;
        draw_text(p, r, "AVA Typography g j 0123 °", &d, c);
    }
    let mut d = TextDsc::new(&MONTSERRAT_28);
    d.color = Color::hex(0x1565C0);
    draw_text(p, Rect::from_xywh(10, 90, 220, 36), "Kerning: AV To", &d, c);
    let mut d = TextDsc::new(&UNSCII_8);
    d.color = Color::hex(0x2E7D32);
    draw_text(
        p,
        Rect::from_xywh(10, 135, 220, 10),
        "unscii 8: pixel font 0123",
        &d,
        c,
    );
}

const PARA: &str = "Twine lays out text with LVGL's line breaking rules: words wrap at spaces and hyphens, long-words-are-split, and\nnewlines force breaks.";

fn wrap_and_line_space(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_14);
    let r = Rect::from_xywh(6, 6, 110, 148);
    frame(p, r);
    draw_text(p, r, PARA, &d, c);
    d.line_space = 6;
    d.align = TextAlign::Center;
    let r = Rect::from_xywh(124, 6, 110, 148);
    frame(p, r);
    draw_text(p, r, PARA, &d, c);
}

fn letter_space(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_20);
    for (i, ls) in [-1, 0, 2, 5].into_iter().enumerate() {
        d.letter_space = ls;
        draw_text(
            p,
            Rect::from_xywh(8, 8 + i as i32 * 36, 230, 30),
            "Spacing WAVE",
            &d,
            c,
        );
    }
}

fn underline_strike(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_20);
    d.decor = TextDecor::UNDERLINE;
    draw_text(p, Rect::from_xywh(8, 8, 224, 30), "Underlined text", &d, c);
    d.decor = TextDecor::STRIKETHROUGH;
    d.color = Color::hex(0xC62828);
    draw_text(p, Rect::from_xywh(8, 48, 224, 30), "Struck through", &d, c);
    d.decor = TextDecor::UNDERLINE | TextDecor::STRIKETHROUGH;
    d.color = Color::BLACK;
    let mut d14 = d;
    d14.font = &MONTSERRAT_14;
    draw_text(
        p,
        Rect::from_xywh(8, 88, 150, 60),
        "Both decorations on wrapped lines",
        &d14,
        c,
    );
}

fn selection(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_14);
    d.sel_start = Some(20);
    d.sel_end = Some(6);
    let r = Rect::from_xywh(8, 8, 224, 60);
    draw_text(p, r, "Select a range of this text across lines", &d, c);
    let mut d = TextDsc::new(&MONTSERRAT_20);
    d.sel_start = Some(0);
    d.sel_end = Some(3);
    d.sel_bg_color = Color::hex(0xFFCA28);
    d.sel_color = Color::BLACK;
    draw_text(p, Rect::from_xywh(8, 80, 224, 30), "Highlighted", &d, c);
}

fn ellipsis(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_14);
    for (i, n) in [1usize, 2, 3].into_iter().enumerate() {
        d.ellipsis_lines = Some(n);
        let r = Rect::from_xywh(8 + i as i32 * 78, 8, 72, 140);
        frame(p, r);
        draw_text(p, r, PARA, &d, c);
    }
}

fn opa_50(p: &mut Painter<'_>, c: &mut GlyphCache) {
    p.fill(Rect::from_xywh(0, 60, 240, 40), Color::hex(0x1565C0), Opa::COVER);
    let mut d = TextDsc::new(&MONTSERRAT_28);
    d.opa = Opa(128);
    draw_text(p, Rect::from_xywh(8, 20, 230, 40), "Half opaque", &d, c);
    d.color = Color::WHITE;
    draw_text(p, Rect::from_xywh(8, 64, 230, 40), "On blue 50 %", &d, c);
    let mut d = TextDsc::new(&MONTSERRAT_14);
    d.color = Color::hex(0x6A1B9A);
    let s = format!(
        "{} {} {} {} {}",
        symbols::OK,
        symbols::WIFI,
        symbols::BATTERY_3,
        symbols::HOME,
        symbols::SETTINGS
    );
    draw_text(p, Rect::from_xywh(8, 110, 230, 20), &s, &d, c);
    d.ofs = Point::new(-20, 0);
    draw_text(
        p,
        Rect::from_xywh(8, 132, 100, 20),
        "Scrolled by 20 px offset",
        &d,
        c,
    );
}

fn subpx_hor(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let text = "Subpixel text: Hamburgefonstiv 0123";
    for (i, (font, bg, fg)) in [
        (&MONTSERRAT_14, Color::WHITE, Color::BLACK),
        (&MONTSERRAT_14_SUBPX, Color::WHITE, Color::BLACK),
        (&MONTSERRAT_14, Color::hex(0x202020), Color::WHITE),
        (&MONTSERRAT_14_SUBPX, Color::hex(0x202020), Color::WHITE),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::from_xywh(0, 4 + i as i32 * 26, 240, 24);
        p.fill(r, bg, Opa::COVER);
        let mut d = TextDsc::new(font);
        d.color = fg;
        draw_text(p, Rect::from_xywh(6, r.y0 + 3, 230, 20), text, &d, c);
    }
    let mut d = TextDsc::new(&MONTSERRAT_14_SUBPX);
    d.subpx_order = twine_render::SubpxOrder::Bgr;
    d.color = Color::hex(0xC62828);
    draw_text(
        p,
        Rect::from_xywh(6, 112, 230, 20),
        "BGR panel order, colored",
        &d,
        c,
    );
}

const SCENES: &[(&str, Scene)] = &[
    ("text_basic_left_center_right", basic),
    ("text_wrap_and_line_space", wrap_and_line_space),
    ("text_letter_space", letter_space),
    ("text_underline_strike", underline_strike),
    ("text_selection", selection),
    ("text_ellipsis", ellipsis),
    ("text_opa_50", opa_50),
    ("text_subpx_hor", subpx_hor),
];

fn render(scene: Scene) -> RenderHarness {
    let mut h = RenderHarness::new(W, H, ColorFormat::Rgb565);
    let mut c = GlyphCache::default();
    h.paint(|p| scene(p, &mut c));
    h
}

fn snap(name: &str) {
    let scene = SCENES.iter().find(|(n, _)| *n == name).unwrap().1;
    assert_render_snapshot!(render(scene), name);
}

#[test]
fn snap_text_basic_left_center_right() {
    snap("text_basic_left_center_right");
}

#[test]
fn snap_text_wrap_and_line_space() {
    snap("text_wrap_and_line_space");
}

#[test]
fn snap_text_letter_space() {
    snap("text_letter_space");
}

#[test]
fn snap_text_underline_strike() {
    snap("text_underline_strike");
}

#[test]
fn snap_text_selection() {
    snap("text_selection");
}

#[test]
fn snap_text_ellipsis() {
    snap("text_ellipsis");
}

#[test]
fn snap_text_opa_50() {
    snap("text_opa_50");
}

#[test]
fn snap_text_subpx_hor() {
    snap("text_subpx_hor");
}

/// Renders every scene in 3 horizontal chunks (separate clips) and in strips of 7 rows (separate
/// buffers, like the engine's partial buffers); both must equal the single-pass render.
#[test]
fn snap_text_clip_partial_chunk() {
    for (name, scene) in SCENES {
        let full = render(*scene);
        let mut chunks = RenderHarness::new(W, H, ColorFormat::Rgb565);
        let mut c = GlyphCache::default();
        chunks.paint(|p| {
            for (y0, y1) in [(0, 53), (53, 101), (101, 160)] {
                p.with_clip(Rect::new(0, y0, i32::from(W), y1), |p| scene(p, &mut c));
            }
        });
        assert!(
            chunks.data() == full.data(),
            "{name}: clip chunks differ from the full render"
        );
        let mut strips = RenderHarness::new(W, H, ColorFormat::Rgb565);
        strips.paint_chunked(7, |p| scene(p, &mut c));
        assert!(
            strips.data() == full.data(),
            "{name}: 7-row strips differ from the full render"
        );
    }
}

#[test]
fn missing_glyph_draws_box_and_warns_once() {
    let mut h = RenderHarness::new(40, 20, ColorFormat::Rgb565);
    let mut c = GlyphCache::default();
    let d = TextDsc::new(&MONTSERRAT_14);
    assert!(c.first_warning((0, 0)));
    h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 40, 20), "中", &d, &mut c));
    // Box: 9 px wide (line height 18 / 2), ascent tall, 1 px outline.
    let px = |x: usize, y: usize| h.rgb888()[(y * 40 + x) * 3];
    let asc = MONTSERRAT_14.ascent() as usize;
    assert_eq!(px(0, 0), 0);
    assert_eq!(px(8, asc - 1), 0);
    assert_eq!(px(4, 4), 255, "hollow");
    assert_eq!(px(9, 0), 255);
    // The warning key is now recorded: a second attempt reports "already warned".
    assert!(!c.first_warning((MONTSERRAT_14.provider_key(), u32::from('中'))));
}

#[test]
fn draw_skips_invisible_lines() {
    let text = "line\n".repeat(200);
    let d = TextDsc::new(&MONTSERRAT_14);
    let mut h = RenderHarness::new(100, 40, ColorFormat::Rgb565);
    let mut c = GlyphCache::default();
    h.paint(|p| {
        p.with_clip(Rect::from_xywh(0, 18, 100, 18), |p| {
            draw_text(p, Rect::from_xywh(0, 0, 100, 5000), &text, &d, &mut c);
        });
    });
    // Only line 1 (y 18..36) is visible: 4 glyphs ("line"); lines 0 and 2 touch the clip only
    // with their descender/ascender area at most.
    let renders = c.stats().glyph_renders;
    assert!(renders <= 12, "{renders} glyphs rendered for 200 lines");
    assert!(renders >= 4);
}

#[test]
fn compressed_font_uses_cache() {
    let d = TextDsc::new(&MONTSERRAT_28);
    let mut h = RenderHarness::new(200, 40, ColorFormat::Rgb565);
    let mut c = GlyphCache::default();
    h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 200, 40), "aaaa", &d, &mut c));
    let s = c.stats();
    assert_eq!((s.misses, s.hits), (1, 3));
    // Without a cache budget the result is identical.
    let mut h2 = RenderHarness::new(200, 40, ColorFormat::Rgb565);
    let mut c0 = GlyphCache::new(0);
    h2.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 200, 40), "aaaa", &d, &mut c0));
    assert!(h.data() == h2.data());
}
