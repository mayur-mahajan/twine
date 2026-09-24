//! The [`Theme`] trait.

use twine_core::Color;
use twine_engine::ThemeHook;
use twine_text::Font;

/// A theme: the engine's [`ThemeHook`] (styling nodes, default font) plus the fonts and
/// colors widgets and applications take from it (LVGL `lv_theme_get_font_*`,
/// `lv_theme_get_color_*`).
///
/// ```
/// use twine_theme::{DefaultTheme, Palette, Theme};
/// let t = DefaultTheme::light();
/// assert_eq!(t.color_secondary(), Palette::Red.main());
/// assert!(core::ptr::eq(t.font_small(), t.font_large()));
/// ```
pub trait Theme: ThemeHook {
    /// The small font (LVGL `font_small`).
    fn font_small(&self) -> &'static Font;
    /// The large font (LVGL `font_large`).
    fn font_large(&self) -> &'static Font;
    /// The primary color (buttons, sliders, focus outlines).
    fn color_primary(&self) -> Color;
    /// The secondary color (checked buttons, edit outlines).
    fn color_secondary(&self) -> Color;
}
