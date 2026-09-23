//! Text benchmark scenarios shared by the criterion and iai-callgrind benches.

use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_28};
use twine_core::{ColorFormat, Rect};
use twine_render::{DrawBuf, Painter, RenderCaches};
use twine_text::{GlyphCache, TextDsc, TextLayout, draw_text};

/// 50 glyphs (with spaces) of ordinary text.
pub const TEXT_50: &str = "The quick brown fox jumps over the lazy dog 012345";

/// A ~1 KiB paragraph.
pub fn paragraph() -> String {
    "Twine lays out text with the same line breaking rules as LVGL, wrapping at spaces, commas \
     and hyphens, and splitting words that are longer than a line. "
        .repeat(8)
}

/// A 320 × 240 RGB565 target with caches.
pub struct Target {
    data: Vec<u8>,
    caches: RenderCaches,
    /// The glyph cache (default budget).
    pub glyphs: GlyphCache,
}

impl Target {
    /// A cleared target.
    pub fn new() -> Self {
        Self {
            data: vec![0xFF; 320 * 240 * 2],
            caches: RenderCaches::default(),
            glyphs: GlyphCache::default(),
        }
    }

    fn draw(&mut self, text: &str, dsc: &TextDsc) {
        let area = Rect::from_xywh(0, 0, 320, 240);
        let buf = DrawBuf::new_packed(&mut self.data, ColorFormat::Rgb565, area).expect("valid buffer");
        let mut p = Painter::new(buf, &mut self.caches);
        draw_text(
            &mut p,
            Rect::from_xywh(4, 4, 312, 40),
            text,
            dsc,
            &mut self.glyphs,
        );
    }

    /// 50 glyphs of Montserrat 14 (plain 4 bpp bitmaps).
    pub fn draw_50_glyphs_montserrat14(&mut self) {
        self.draw(TEXT_50, &TextDsc::new(&MONTSERRAT_14));
    }

    /// 50 glyphs of Montserrat 28 (compressed; served from the glyph cache when warm).
    pub fn glyph_cache_hit_render(&mut self) {
        self.draw("aaaaaaaaaa", &TextDsc::new(&MONTSERRAT_28));
    }
}

/// Lays out `text` in 200 px lines; returns the line count.
pub fn layout_wrap_200(text: &str) -> usize {
    let mut l = TextLayout::new(text, &MONTSERRAT_14);
    l.max_width = 200;
    l.line_count()
}

/// Measures a 20-character label.
pub fn measure_label_20() -> i32 {
    TextLayout::new("Temperature: 21.5 °C", &MONTSERRAT_14)
        .measure()
        .w
}
