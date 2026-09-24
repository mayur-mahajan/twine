//! Image fonts (LVGL `imgfont`), fallback chains and missing-glyph warnings.
#![allow(clippy::unreadable_literal, clippy::borrow_as_ptr)] // colors; font identity checks
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use std::sync::Mutex;

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, ColorFormat, Rect};
use twine_render::ImagePixels;
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_text::{Font, GlyphCache, ImageFontProvider, TextDsc, TextLayout, draw_text};

const S: usize = 14;

/// A 14 × 14 ARGB8888 "emoji": a filled circle of `rgb` with two eyes and a mouth.
const fn face(rgb: u32) -> [u8; S * S * 4] {
    let mut px = [0u8; S * S * 4];
    let mut y = 0;
    while y < S {
        let mut x = 0;
        while x < S {
            let (dx, dy) = (2 * x as i32 - 13, 2 * y as i32 - 13);
            let d2 = dx * dx + dy * dy;
            let eye = (y == 4 || y == 5) && (x == 4 || x == 9);
            let mouth = y == 9 && x >= 4 && x <= 9 || y == 8 && (x == 3 || x == 10);
            let (c, a) = if d2 > 13 * 13 {
                (0, 0)
            } else if eye || mouth {
                (0x0030_2010, 255)
            } else {
                (rgb, 255)
            };
            let i = (y * S + x) * 4;
            // Argb8888 bytes in memory: B, G, R, A.
            px[i] = (c & 0xFF) as u8;
            px[i + 1] = ((c >> 8) & 0xFF) as u8;
            px[i + 2] = ((c >> 16) & 0xFF) as u8;
            px[i + 3] = a;
            x += 1;
        }
        y += 1;
    }
    px
}

static SMILE_PX: [u8; S * S * 4] = face(0x00FF_CC33);
static HEART_PX: [u8; S * S * 4] = face(0x00E5_3935);

const fn pixels(data: &'static [u8]) -> ImagePixels<'static> {
    ImagePixels {
        format: ColorFormat::Argb8888,
        w: S as u16,
        h: S as u16,
        stride: (S * 4) as u16,
        data,
        palette: None,
        alpha: None,
        premultiplied: false,
    }
}

static SMILE: ImagePixels<'static> = pixels(&SMILE_PX);
static HEART: ImagePixels<'static> = pixels(&HEART_PX);

fn lookup(cp: char, _next: Option<char>) -> Option<(&'static ImagePixels<'static>, i16)> {
    match cp {
        '😀' => Some((&SMILE, -2)),
        '❤' => Some((&HEART, -2)),
        _ => None,
    }
}

static EMOJI_PROVIDER: ImageFontProvider = ImageFontProvider {
    lookup,
    line_height: 18,
};
/// Montserrat with the emoji image font as fallback.
static TEXT_WITH_EMOJI: Font = Font {
    fallback: Some(&EMOJI),
    ..twine_text_font_copy()
};
static EMOJI: Font = EMOJI_PROVIDER.font(None);

/// `MONTSERRAT_14`'s metrics and provider (a `Font` is not `Copy`, so rebuild it).
const fn twine_text_font_copy() -> Font {
    Font {
        line_height: MONTSERRAT_14.line_height,
        base_line: MONTSERRAT_14.base_line,
        underline_position: MONTSERRAT_14.underline_position,
        underline_thickness: MONTSERRAT_14.underline_thickness,
        provider: MONTSERRAT_14.provider,
        fallback: None,
        subpx: MONTSERRAT_14.subpx,
    }
}

#[test]
fn imgfont_draws_image_glyph_snapshot() {
    let mut h = RenderHarness::new(200, 44, ColorFormat::Rgb565);
    h.clear(Color::WHITE);
    let mut cache = GlyphCache::default();
    let mut d = TextDsc::new(&TEXT_WITH_EMOJI);
    d.color = Color::hex(0x1A237E);
    h.paint(|p| {
        draw_text(
            p,
            Rect::from_xywh(4, 4, 192, 18),
            "Emoji 😀 via imgfont ❤",
            &d,
            &mut cache,
        );
        d.opa = twine_core::Opa(128);
        draw_text(p, Rect::from_xywh(4, 24, 192, 18), "Half 😀❤😀", &d, &mut cache);
    });
    assert_render_snapshot!(h, "imgfont_emoji");
}

#[test]
fn imgfont_layout_includes_image_advances() {
    let with = TextLayout::new("a😀b", &TEXT_WITH_EMOJI).measure().w;
    let without = TextLayout::new("ab", &TEXT_WITH_EMOJI).measure().w;
    assert_eq!(with - without, S as i32, "image glyph advance = image width");
    let (from, g) = TEXT_WITH_EMOJI.glyph('😀', None).unwrap();
    assert!(core::ptr::eq(from, &EMOJI));
    assert_eq!((g.bpp, g.box_w, g.box_h, g.ofs_y), (0, S as u16, S as u16, -2));
}

#[test]
fn fallback_chain_used_for_missing_glyph() {
    // 'a' from Montserrat, '❤' from the image font two levels down.
    static TOP: Font = Font {
        fallback: Some(&TEXT_WITH_EMOJI),
        ..twine_text_font_copy()
    };
    let (f, _) = TOP.glyph('a', None).unwrap();
    assert!(core::ptr::eq(f, &TOP));
    let (f, _) = TOP.glyph('❤', None).unwrap();
    assert!(core::ptr::eq(f, &EMOJI));
    assert!(TOP.glyph('中', None).is_none());
}

static CYCLE_A: Font = Font {
    fallback: Some(&CYCLE_B),
    ..twine_text_font_copy()
};
static CYCLE_B: Font = Font {
    fallback: Some(&CYCLE_A),
    ..twine_text_font_copy()
};

#[test]
fn fallback_cycle_does_not_loop() {
    assert!(CYCLE_A.glyph('中', None).is_none());
    assert_eq!(CYCLE_A.advance_px('中', None), CYCLE_A.placeholder_width() + 2);
    assert!(CYCLE_B.glyph('b', None).is_some());
    // Layout and drawing terminate too.
    assert!(TextLayout::new("中中", &CYCLE_A).measure().w > 0);
    let mut h = RenderHarness::new(40, 20, ColorFormat::Rgb565);
    let mut cache = GlyphCache::default();
    h.paint(|p| {
        draw_text(
            p,
            Rect::from_xywh(0, 0, 40, 20),
            "中",
            &TextDsc::new(&CYCLE_A),
            &mut cache,
        );
    });
}

struct Capture;
static LOGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record<'_>) {
        if r.target() == "twine::text" && r.level() <= log::Level::Warn {
            LOGS.lock().unwrap().push(r.args().to_string());
        }
    }
    fn flush(&self) {}
}

#[test]
fn missing_glyph_warns_once() {
    static LOGGER: Capture = Capture;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let mut h = RenderHarness::new(200, 20, ColorFormat::Rgb565);
    let mut cache = GlyphCache::default();
    let d = TextDsc::new(&TEXT_WITH_EMOJI);
    for _ in 0..3 {
        h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 200, 20), "アア イ 😀", &d, &mut cache));
    }
    let missing: Vec<_> = LOGS
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.contains("missing glyph U+30A"))
        .cloned()
        .collect();
    assert_eq!(missing, ["missing glyph U+30A2", "missing glyph U+30A4"]);
    // After the warning set is full, logging stops (one "suppressed" notice).
    for cp in 0x5000..0x5040u32 {
        let s = char::from_u32(cp).unwrap().to_string();
        h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 200, 20), &s, &d, &mut cache));
    }
    let logs = LOGS.lock().unwrap();
    let n = logs.iter().filter(|m| m.contains("missing glyph U+50")).count();
    assert_eq!(
        n,
        twine_text::WARN_CAP - 2,
        "the set holds U+30A2, U+30A4 and 30 more"
    );
    assert_eq!(logs.iter().filter(|m| m.contains("suppressed")).count(), 1);
}
