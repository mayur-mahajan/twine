//! No allocation after warm-up: glyph decoding (plain and compressed) and text drawing.

use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_14_SUBPX, MONTSERRAT_28};
use twine_core::{ColorFormat, Rect};
use twine_testing::RenderHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_text::{Font, GlyphCache, TextDecor, TextDsc, draw_text};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

fn render_100(font: &'static Font, cache: &mut GlyphCache, out: &mut [u8]) {
    for c in (0x21u8..0x7E).map(char::from).cycle().take(100) {
        let (f, info) = font.glyph(c, None).unwrap();
        assert!(f.provider.render_a8(&info, out, cache));
    }
}

#[test]
fn no_alloc_after_warmup() {
    let mut out = vec![0u8; 255 * 255];
    for font in [&MONTSERRAT_14, &MONTSERRAT_28] {
        for budget in [0, 1024, 8192] {
            let mut cache = GlyphCache::new(budget);
            render_100(font, &mut cache, &mut out);
            let ((), s) = count_allocs(|| render_100(font, &mut cache, &mut out));
            assert_eq!(s.allocs + s.reallocs, 0, "budget {budget}: {s:?}");
        }
    }
}

#[test]
fn no_alloc_steady_state() {
    let text = "Steady state: wrapped, underlined, selected text…\nwith symbols \u{F00C} and ellipsis at the end of a long paragraph.";
    for font in [&MONTSERRAT_14, &MONTSERRAT_28, &MONTSERRAT_14_SUBPX] {
        let mut h = RenderHarness::new(200, 120, ColorFormat::Rgb565);
        let mut cache = GlyphCache::default();
        let mut d = TextDsc::new(font);
        d.decor = TextDecor::UNDERLINE | TextDecor::STRIKETHROUGH;
        d.sel_start = Some(3);
        d.sel_end = Some(20);
        d.ellipsis_lines = Some(3);
        let draw = |h: &mut RenderHarness, cache: &mut GlyphCache| {
            h.paint(|p| draw_text(p, Rect::from_xywh(0, 0, 200, 120), text, &d, cache));
        };
        draw(&mut h, &mut cache);
        let ((), s) = count_allocs(|| draw(&mut h, &mut cache));
        assert_eq!(s.allocs + s.reallocs, 0, "{s:?}");
    }
}
