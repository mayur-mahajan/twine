//! The [`Theme`] trait.

use twine_engine::ThemeHook;
use twine_text::Font;

/// A theme: the engine's [`ThemeHook`] (styling nodes, default font, primary and secondary
/// colors) plus the other fonts applications take from it (LVGL `lv_theme_get_font_*`).
///
/// ```
/// use twine_theme::{DefaultTheme, Palette, Theme, ThemeHook};
/// let t = DefaultTheme::light();
/// assert_eq!(t.color_secondary(), Palette::Red.main());
/// assert!(core::ptr::eq(t.font_small(), t.font_large()));
/// ```
pub trait Theme: ThemeHook {
    /// The small font (LVGL `font_small`).
    fn font_small(&self) -> &'static Font;
    /// The large font (LVGL `font_large`).
    fn font_large(&self) -> &'static Font;
}
