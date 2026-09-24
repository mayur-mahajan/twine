//! [`TextDir`]: the base direction of a paragraph.

use crate::hit::TextAlign;

/// Base direction of text (LVGL `lv_base_dir_t` subset; widgets map the style's base
/// direction to it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TextDir {
    /// Left to right.
    #[default]
    Ltr,
    /// Right to left.
    Rtl,
    /// From the first strong character (feature `bidi`; left to right without it).
    Auto,
}

impl TextAlign {
    /// Resolves [`TextAlign::Auto`] for base direction `dir`: right for right-to-left text,
    /// left otherwise. Other alignments are returned unchanged.
    ///
    /// ```
    /// use twine_text::{TextAlign, TextDir};
    /// assert_eq!(TextAlign::Auto.resolve(TextDir::Rtl), TextAlign::Right);
    /// assert_eq!(TextAlign::Auto.resolve(TextDir::Ltr), TextAlign::Left);
    /// assert_eq!(TextAlign::Center.resolve(TextDir::Rtl), TextAlign::Center);
    /// ```
    #[must_use]
    pub const fn resolve(self, dir: TextDir) -> TextAlign {
        match (self, dir) {
            (TextAlign::Auto, TextDir::Rtl) => TextAlign::Right,
            (TextAlign::Auto, _) => TextAlign::Left,
            (a, _) => a,
        }
    }
}
