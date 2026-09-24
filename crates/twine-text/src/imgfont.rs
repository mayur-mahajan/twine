//! [`ImageFontProvider`]: glyphs drawn from images (LVGL `lv_imgfont`), e.g. emoji.

use twine_render::ImagePixels;

use crate::cache::GlyphCache;
use crate::font::{Font, GlyphInfo, GlyphProvider, Subpx};

/// Looks up the image of `cp` (with the following character `next`, for sequences such as
/// emoji with variation selectors) and its vertical offset: how far the image's bottom edge
/// lies **above** the baseline (negative = below).
pub type ImageGlyphLookup = fn(cp: char, next: Option<char>) -> Option<(&'static ImagePixels<'static>, i16)>;

/// A [`GlyphProvider`] whose glyphs are images (LVGL `lv_imgfont`).
///
/// Every glyph is as wide as its image (advance = image width, no kerning) and is drawn with
/// the text's opacity; text color and selection color do not apply. Characters the lookup
/// does not know are missing, so an image font is usually the fallback of a regular font (or
/// has one): `Font { fallback: Some(&MONTSERRAT_14), .. }`.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_render::ImagePixels;
/// use twine_text::{Font, ImageFontProvider, TextLayout};
///
/// static HEART_PX: [u8; 16 * 16 * 4] = [0xFF; 16 * 16 * 4];
/// static HEART: ImagePixels<'static> = ImagePixels {
///     format: ColorFormat::Argb8888, w: 16, h: 16, stride: 64, data: &HEART_PX,
///     palette: None, alpha: None, premultiplied: false,
/// };
/// fn lookup(cp: char, _next: Option<char>) -> Option<(&'static ImagePixels<'static>, i16)> {
///     (cp == '♥').then_some((&HEART, -2))
/// }
/// static EMOJI_PROVIDER: ImageFontProvider = ImageFontProvider { lookup, line_height: 18 };
/// static EMOJI: Font = EMOJI_PROVIDER.font(None);
///
/// assert_eq!(TextLayout::new("♥♥", &EMOJI).measure().w, 32);
/// ```
#[derive(Clone, Copy)]
pub struct ImageFontProvider {
    /// Character → image lookup.
    pub lookup: ImageGlyphLookup,
    /// Line height of the font built by [`font`](Self::font).
    pub line_height: i16,
}

impl core::fmt::Debug for ImageFontProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageFontProvider")
            .field("line_height", &self.line_height)
            .finish_non_exhaustive()
    }
}

impl ImageFontProvider {
    /// A [`Font`] over this provider: `line_height` tall with the baseline at the bottom
    /// (image offsets are relative to it), without decorations metrics.
    #[must_use]
    pub const fn font(&'static self, fallback: Option<&'static Font>) -> Font {
        Font {
            line_height: self.line_height,
            base_line: 0,
            underline_position: 0,
            underline_thickness: 1,
            provider: self,
            fallback,
            subpx: Subpx::None,
        }
    }
}

impl GlyphProvider for ImageFontProvider {
    fn glyph_info(&self, cp: char, next: Option<char>) -> Option<GlyphInfo> {
        let (img, ofs_y) = (self.lookup)(cp, next)?;
        Some(GlyphInfo {
            adv_w: img.w.saturating_mul(16),
            box_w: img.w,
            box_h: img.h,
            ofs_x: 0,
            ofs_y,
            bpp: 0,
            id: u32::from(cp),
            is_placeholder: false,
        })
    }

    /// Image glyphs have no coverage bitmap: always `false`.
    fn render_a8(&self, _info: &GlyphInfo, _out: &mut [u8], _cache: &mut GlyphCache) -> bool {
        false
    }

    fn render_rows(
        &self,
        _info: &GlyphInfo,
        _cache: &mut GlyphCache,
        _sink: &mut dyn FnMut(usize, &[u8]),
    ) -> bool {
        false
    }

    fn glyph_image(&self, cp: char, next: Option<char>) -> Option<&'static ImagePixels<'static>> {
        (self.lookup)(cp, next).map(|(img, _)| img)
    }
}
