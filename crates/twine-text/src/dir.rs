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

impl TextDir {
    /// The direction `text` is laid out in: [`TextDir::Auto`] detects it from the first strong
    /// character with the `bidi` feature and means left to right without it.
    ///
    /// ```
    /// use twine_text::TextDir;
    /// assert_eq!(TextDir::Rtl.resolved("abc"), TextDir::Rtl);
    /// assert_eq!(TextDir::Auto.resolved("abc"), TextDir::Ltr);
    /// ```
    #[must_use]
    pub fn resolved(self, text: &str) -> TextDir {
        #[cfg(feature = "bidi")]
        {
            crate::bidi::resolve_dir(self, text)
        }
        #[cfg(not(feature = "bidi"))]
        {
            let _ = text;
            match self {
                TextDir::Rtl => TextDir::Rtl,
                TextDir::Ltr | TextDir::Auto => TextDir::Ltr,
            }
        }
    }
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
