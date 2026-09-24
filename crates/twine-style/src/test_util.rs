//! Helpers for unit tests.

use twine_text::{Font, GlyphCache, GlyphInfo, GlyphProvider, Subpx};

struct NoGlyphs;

impl GlyphProvider for NoGlyphs {
    fn glyph_info(&self, _cp: char, _next: Option<char>) -> Option<GlyphInfo> {
        None
    }

    fn render_a8(&self, _info: &GlyphInfo, _out: &mut [u8], _cache: &mut GlyphCache) -> bool {
        false
    }
}

/// A distinct glyph-less font (for identity tests).
pub(crate) const fn font(line_height: i16) -> Font {
    Font {
        line_height,
        base_line: 0,
        underline_position: 0,
        underline_thickness: 0,
        provider: &NoGlyphs,
        fallback: None,
        subpx: Subpx::None,
    }
}
