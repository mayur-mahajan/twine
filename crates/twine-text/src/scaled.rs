//! [`ScaledFont`]: a font with bitmap variants for scaled (hi-DPI) displays.

use twine_core::DisplayScale;
use twine_render::ImagePixels;

use crate::cache::GlyphCache;
use crate::font::{Font, GlyphInfo, GlyphProvider};
use crate::gamma::CoverageMap;

/// A font that follows the display scale: the `base` font on 1× displays, and on a scaled
/// display the variant whose scale is nearest (a larger one on a tie) — the same face rendered
/// at `scale ×` the base size, e.g. Montserrat 28 as the 2× variant of Montserrat 14.
///
/// The [`Font`] of a scaled font ([`font`](Self::font)) has the base font's metrics and
/// glyphs, so it can be used anywhere a `&'static Font` goes (styles, themes, labels); the style
/// resolver swaps in the variant for the display of each node (`twine-style` feature
/// `display-scale`). Without that feature, or on a 1× display, the base font is drawn and the
/// variants cost only their flash.
///
/// Bitmap variants exist only at the sizes the application generated, so a fractional scale
/// (1.25×, 1.5×) uses the nearest one; for exact sizes at every scale use a scalable TrueType
/// font (`TtfFont::scalable`, feature `ttf`).
///
/// **Pay for what you use:** only the variants listed are linked; plain fonts (every built-in
/// font) have no variants.
///
/// ```
/// use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_28, MONTSERRAT_42};
/// use twine_core::DisplayScale;
/// use twine_text::{Font, ScaledFont};
///
/// /// Body text: Montserrat 14 at 1×, 28 at 2×, 42 at 3×.
/// pub static BODY: Font = {
///     static SET: ScaledFont = ScaledFont::new(
///         &MONTSERRAT_14,
///         &[(DisplayScale::X2, &MONTSERRAT_28), (DisplayScale::X3, &MONTSERRAT_42)],
///     );
///     SET.font()
/// };
///
/// assert!(core::ptr::eq(BODY.for_scale(DisplayScale::ONE), &MONTSERRAT_14));
/// assert!(core::ptr::eq(BODY.for_scale(DisplayScale::X2), &MONTSERRAT_28));
/// assert!(core::ptr::eq(BODY.for_scale(DisplayScale::pct(275)), &MONTSERRAT_42)); // nearest
/// assert!(core::ptr::eq(BODY.for_scale(DisplayScale::pct(125)), &MONTSERRAT_14));
/// assert!(core::ptr::eq(BODY.for_scale(DisplayScale::pct(150)), &MONTSERRAT_28)); // tie: larger
/// ```
pub struct ScaledFont {
    base: &'static Font,
    variants: &'static [(DisplayScale, &'static Font)],
}

impl core::fmt::Debug for ScaledFont {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScaledFont")
            .field("base", self.base)
            .field("variants", &self.variants.len())
            .finish()
    }
}

impl ScaledFont {
    /// `base` (the 1× font) with `variants`: each the same face rendered for a display scale.
    #[must_use]
    pub const fn new(base: &'static Font, variants: &'static [(DisplayScale, &'static Font)]) -> Self {
        Self { base, variants }
    }

    /// The font to put in styles: the base font's metrics, fallback and subpixel layout over
    /// this scaled font (see the type documentation for the `static` pattern).
    #[must_use]
    pub const fn font(&'static self) -> Font {
        Font {
            line_height: self.base.line_height,
            base_line: self.base.base_line,
            underline_position: self.base.underline_position,
            underline_thickness: self.base.underline_thickness,
            provider: self,
            fallback: self.base.fallback,
            subpx: self.base.subpx,
        }
    }

    /// The base (1×) font.
    #[must_use]
    pub const fn base(&self) -> &'static Font {
        self.base
    }

    /// The font for `scale`: the base or the variant nearest to it (the larger one on a tie).
    #[must_use]
    pub fn pick(&self, scale: DisplayScale) -> &'static Font {
        let dist = |s: DisplayScale| s.raw_256().abs_diff(scale.raw_256());
        let mut best = (DisplayScale::ONE, self.base);
        for &(s, f) in self.variants {
            let (d, b) = (dist(s), dist(best.0));
            if d < b || (d == b && s > best.0) {
                best = (s, f);
            }
        }
        best.1
    }
}

/// Glyphs come from the base font: a scaled font drawn as is (a 1× display, or a build without
/// display scaling) looks exactly like its base.
impl GlyphProvider for ScaledFont {
    fn glyph_info(&self, cp: char, next: Option<char>) -> Option<GlyphInfo> {
        self.base.provider.glyph_info(cp, next)
    }

    fn render_a8(&self, info: &GlyphInfo, map: CoverageMap, out: &mut [u8], cache: &mut GlyphCache) -> bool {
        self.base.provider.render_a8(info, map, out, cache)
    }

    fn render_rows(
        &self,
        info: &GlyphInfo,
        map: CoverageMap,
        cache: &mut GlyphCache,
        sink: &mut dyn FnMut(usize, &[u8]),
    ) -> bool {
        self.base.provider.render_rows(info, map, cache, sink)
    }

    fn glyph_image(&self, cp: char, next: Option<char>) -> Option<&'static ImagePixels<'static>> {
        self.base.provider.glyph_image(cp, next)
    }

    fn font_for_scale(&self, scale: DisplayScale) -> Option<&'static Font> {
        Some(self.pick(scale))
    }
}

/// Declares a `static` [`Font`] that follows the display scale, in one line (the
/// [`ScaledFont`] / `TtfScaledFont` and the font over it are
/// generated):
///
/// - `ttf(base, data, px)` — `base` (a bitmap font of `px` px) at 1×, the TrueType face `data`
///   rasterized at `px × scale` elsewhere (feature `ttf`); optionally `.fallback(data)` — the
///   face `data` at the same base and size — or `.fallback(ttf(base, data, px))` for the
///   characters the face lacks (e.g. the symbol font).
/// - `variants(base, [scale => font, …])` — bitmap renderings for chosen scales
///   ([`ScaledFont`]).
///
/// ```
/// use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_28};
/// use twine_core::DisplayScale;
///
/// static MONTSERRAT: &[u8] = include_bytes!("../../../assets/fonts/Montserrat-Medium.ttf");
/// static FONT_AWESOME: &[u8] = include_bytes!("../../../assets/fonts/fa-solid-900.ttf");
///
/// twine_text::scaled_font! {
///     /// Body text, sharp at every scale, with Font Awesome symbols.
///     pub static BODY = ttf(MONTSERRAT_14, MONTSERRAT, 14).fallback(FONT_AWESOME);
/// }
/// twine_text::scaled_font! {
///     /// Body text from bitmaps only (no run-time rasterization).
///     static BODY_BITMAPS = variants(MONTSERRAT_14, [DisplayScale::X2 => MONTSERRAT_28]);
/// }
///
/// assert!(core::ptr::eq(BODY.for_scale(DisplayScale::ONE), &MONTSERRAT_14));
/// assert!(core::ptr::eq(BODY_BITMAPS.for_scale(DisplayScale::X2), &MONTSERRAT_28));
/// assert!(BODY.for_scale(DisplayScale::X2).line_height > MONTSERRAT_14.line_height);
/// ```
#[macro_export]
macro_rules! scaled_font {
    (
        $(#[$meta:meta])*
        $vis:vis static $name:ident = ttf($base:expr, $data:expr, $px:expr)
            . fallback ( ttf ( $fb_base:expr, $fb_data:expr, $fb_px:expr ) ) ;
    ) => {
        $(#[$meta])*
        $vis static $name: $crate::Font = {
            static FALLBACK: $crate::TtfScaledFont = $crate::TtfScaledFont::new(&$fb_base, $fb_data, $fb_px);
            static FALLBACK_FONT: $crate::Font = FALLBACK.font();
            static SCALED: $crate::TtfScaledFont =
                $crate::TtfScaledFont::new(&$base, $data, $px).fallback(&FALLBACK_FONT);
            SCALED.font()
        };
    };
    (
        $(#[$meta:meta])*
        $vis:vis static $name:ident = ttf($base:expr, $data:expr, $px:expr) . fallback ( $fb_data:expr ) ;
    ) => {
        $crate::scaled_font! {
            $(#[$meta])*
            $vis static $name = ttf($base, $data, $px).fallback(ttf($base, $fb_data, $px));
        }
    };
    (
        $(#[$meta:meta])*
        $vis:vis static $name:ident = ttf($base:expr, $data:expr, $px:expr) ;
    ) => {
        $(#[$meta])*
        $vis static $name: $crate::Font = {
            static SCALED: $crate::TtfScaledFont = $crate::TtfScaledFont::new(&$base, $data, $px);
            SCALED.font()
        };
    };
    (
        $(#[$meta:meta])*
        $vis:vis static $name:ident = variants($base:expr, [ $( $scale:expr => $font:expr ),* $(,)? ]) ;
    ) => {
        $(#[$meta])*
        $vis static $name: $crate::Font = {
            static SCALED: $crate::ScaledFont = $crate::ScaledFont::new(&$base, &[ $( ($scale, &$font) ),* ]);
            SCALED.font()
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_font::TEST_FONT;

    #[test]
    fn delegates_glyphs_to_the_base() {
        static SET: ScaledFont = ScaledFont::new(&TEST_FONT, &[]);
        static F: Font = SET.font();
        assert_eq!(F.line_height, TEST_FONT.line_height);
        let a = F.glyph('A', None).map(|(_, g)| g.adv_w);
        assert_eq!(a, TEST_FONT.glyph('A', None).map(|(_, g)| g.adv_w));
        assert!(core::ptr::eq(
            F.for_scale(DisplayScale::X2),
            core::ptr::from_ref(&TEST_FONT)
        ));
    }
}
