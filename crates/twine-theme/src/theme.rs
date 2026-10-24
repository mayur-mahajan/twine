//! The [`Theme`] trait and [`FontScale`].

use twine_engine::ThemeHook;
use twine_text::Font;

/// A theme: the engine's [`ThemeHook`] (styling nodes, default font, design elements per mode)
/// plus the other fonts applications take from it (LVGL `lv_theme_get_font_*`).
///
/// ```
/// use twine_theme::{Lvgl, Theme};
/// static THEME: Lvgl = Lvgl::new();
/// assert!(core::ptr::eq(THEME.font_small(), THEME.font_large()));
/// ```
pub trait Theme: ThemeHook {
    /// The small font (LVGL `font_small`).
    fn font_small(&self) -> &'static Font;
    /// The large font (LVGL `font_large`).
    fn font_large(&self) -> &'static Font;
}

/// The small, normal (body) and large font of a theme (LVGL's `font_small`, `font_normal`,
/// `font_large`): its `design::FONT_SMALL` / `FONT_BODY` / `FONT_LARGE` elements, given to a
/// theme's element set ([`LvglSet::fonts`](crate::LvglSet::fonts)).
///
/// ```
/// use twine_style::ThemeMode;
/// use twine_style::design::{self, ElementTable};
/// use twine_theme::{FontScale, Lvgl};
///
/// let f = &twine_assets::fonts::MONTSERRAT_14;
/// static LIGHT: ElementTable = Lvgl::SET
///     .fonts(FontScale::uniform(&twine_assets::fonts::MONTSERRAT_14))
///     .table(ThemeMode::Light);
/// assert!(core::ptr::eq(LIGHT.get(design::FONT_LARGE).unwrap(), f));
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

/// The font the built-in themes' default element sets start with: Montserrat 14 with the
/// `assets` feature, else the empty font (draws no text: give the set a font).
#[cfg(any(feature = "lvgl", feature = "mono", feature = "simple"))]
pub(crate) const DEFAULT_FONT: &Font = {
    #[cfg(feature = "assets")]
    {
        &twine_assets::fonts::MONTSERRAT_14
    }
    #[cfg(not(feature = "assets"))]
    {
        &twine_text::EMPTY_FONT
    }
};

/// The font of `design::FONT_<which>` in the first of `modes`' tables (the empty font if it
/// has none): what the built-in themes report as their fonts.
#[cfg(any(feature = "lvgl", feature = "mono", feature = "simple"))]
pub(crate) fn first_font(
    modes: &[(twine_style::ThemeMode, &'static twine_style::design::ElementTable)],
    which: twine_style::design::FontElement,
) -> &'static Font {
    modes
        .first()
        .and_then(|(_, t)| t.get(which))
        .unwrap_or(&twine_text::EMPTY_FONT)
}
