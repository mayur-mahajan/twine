//! The [`Theme`] trait and [`FontScale`].

use twine_engine::ThemeHook;
use twine_text::Font;

/// A theme: the engine's [`ThemeHook`] (styling nodes, default font, design elements per mode)
/// plus the other fonts applications take from it (LVGL `lv_theme_get_font_*`).
///
/// ```
/// use twine_theme::{DefaultTheme, Theme};
/// let t = DefaultTheme::light();
/// assert!(core::ptr::eq(t.font_small(), t.font_large()));
/// ```
pub trait Theme: ThemeHook {
    /// The small font (LVGL `font_small`).
    fn font_small(&self) -> &'static Font;
    /// The large font (LVGL `font_large`).
    fn font_large(&self) -> &'static Font;
}

/// The small, normal (body) and large font of a theme (LVGL's `font_small`, `font_normal`,
/// `font_large`), also its `design::FONT_SMALL` / `FONT_BODY` / `FONT_LARGE` elements.
///
/// ```
/// use twine_theme::{DefaultTheme, FontScale};
/// let f = &twine_assets::fonts::MONTSERRAT_14;
/// let _t = DefaultTheme::builder()
///     .fonts(FontScale { small: f, normal: f, large: f })
///     .build();
/// assert!(core::ptr::eq(FontScale::uniform(f).large, f));
/// ```
#[derive(Clone, Copy)]
pub struct FontScale {
    /// Captions, markers (a checkbox's check mark).
    pub small: &'static Font,
    /// Body text: the display's default font.
    pub normal: &'static Font,
    /// Titles.
    pub large: &'static Font,
}

impl FontScale {
    /// `font` for all three sizes.
    #[must_use]
    pub const fn uniform(font: &'static Font) -> Self {
        Self {
            small: font,
            normal: font,
            large: font,
        }
    }
}

impl core::fmt::Debug for FontScale {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FontScale").finish_non_exhaustive()
    }
}

/// The font a theme builder starts with: Montserrat 14 with the `assets` feature, else the
/// empty font (draws no text: set the theme's fonts).
pub(crate) fn default_font() -> &'static Font {
    #[cfg(feature = "assets")]
    {
        &twine_assets::fonts::MONTSERRAT_14
    }
    #[cfg(not(feature = "assets"))]
    {
        &twine_text::EMPTY_FONT
    }
}

/// Warns when a theme is built with the empty default font (no `assets` feature, no font set).
pub(crate) fn check_font(theme: &str, font: &'static Font) {
    if cfg!(not(feature = "assets")) && core::ptr::eq(font, &raw const twine_text::EMPTY_FONT) {
        twine_core::warn!(target: "twine::style", "{} theme built without a font: text is not drawn", theme);
    }
}
