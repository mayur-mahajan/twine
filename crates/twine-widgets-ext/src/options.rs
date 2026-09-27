//! [`Options`]: the `'\n'`-separated option list of dropdowns and rollers (LVGL stores the
//! options as one string: memory-efficient, one allocation at most).

use alloc::string::String;

/// The options of a [`Dropdown`](crate::dropdown::Dropdown) or a
/// [`Roller`](crate::roller::Roller): one string, options separated by `'\n'`, in flash
/// ([`Static`](Options::Static): no allocation) or owned.
///
/// ```
/// use twine_widgets_ext::Options;
/// let o = Options::Static("Apple\nBanana\nCherry");
/// assert_eq!(o.count(), 3);
/// assert_eq!(o.get(1), Some("Banana"));
/// assert_eq!(o.index_of("Cherry"), Some(2));
/// assert_eq!(Options::Static("").count(), 0);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Options {
    /// A `'static` string (LVGL `set_options_static`).
    Static(&'static str),
    /// An owned copy (LVGL `set_options`); its capacity is reused by later option lists.
    Owned(String),
    /// `'static` options with Arabic/Persian letters, stored with their contextual forms
    /// (feature `arabic-shaping`, like LVGL's `LV_USE_ARABIC_PERSIAN_CHARS`).
    #[cfg(feature = "arabic-shaping")]
    Shaped {
        /// The options as given.
        source: &'static str,
        /// The options as drawn.
        shaped: String,
    },
}

impl Default for Options {
    fn default() -> Self {
        Options::Static("")
    }
}

impl Options {
    /// The whole string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Options::Static(s) => s,
            Options::Owned(s) => s,
            #[cfg(feature = "arabic-shaping")]
            Options::Shaped { shaped, .. } => shaped,
        }
    }

    /// `'static` options: stored in place, or shaped into an owned copy when they contain
    /// Arabic/Persian letters (feature `arabic-shaping`).
    pub(crate) fn from_static(s: &'static str) -> Options {
        #[cfg(feature = "arabic-shaping")]
        if twine_text::needs_shaping(s) {
            return Options::Shaped {
                source: s,
                shaped: twine_text::shape(s),
            };
        }
        Options::Static(s)
    }

    /// Whether these are the `'static` options `s` (same address).
    pub(crate) fn is_static(&self, s: &'static str) -> bool {
        match self {
            Options::Static(cur) => core::ptr::eq(*cur, s),
            #[cfg(feature = "arabic-shaping")]
            Options::Shaped { source, .. } => core::ptr::eq(*source, s),
            Options::Owned(_) => false,
        }
    }

    /// Whether setting `s` would store what is stored now (comparing the shaped form when
    /// `s` needs shaping).
    pub(crate) fn same_as(&self, s: &str) -> bool {
        #[cfg(feature = "arabic-shaping")]
        if twine_text::needs_shaping(s) {
            return self.as_str() == twine_text::shape(s);
        }
        self.as_str() == s
    }

    /// The number of options: `'\n'`s + 1, 0 for an empty string (LVGL
    /// `count_options_in_str`; LVGL's empty list is `NULL`).
    #[must_use]
    pub fn count(&self) -> u16 {
        count(self.as_str())
    }

    /// Option `i` (`None` past the end).
    #[must_use]
    pub fn get(&self, i: u16) -> Option<&str> {
        if self.as_str().is_empty() {
            return None;
        }
        self.as_str().split('\n').nth(usize::from(i))
    }

    /// The index of the option equal to `option` (LVGL `get_option_index`).
    #[must_use]
    pub fn index_of(&self, option: &str) -> Option<u16> {
        if self.as_str().is_empty() {
            return None;
        }
        self.as_str()
            .split('\n')
            .position(|o| o == option)
            .and_then(|i| u16::try_from(i).ok())
    }

    /// The length of the whole string in bytes (the storage size).
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_str().len()
    }

    /// Whether there are no options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// Replaces the options with a copy of `s`, reusing the owned buffer's capacity.
    pub(crate) fn set_owned(&mut self, s: &str) {
        if !matches!(self, Options::Owned(_)) {
            *self = Options::Owned(String::new());
        }
        if let Options::Owned(buf) = self {
            buf.clear();
            #[cfg(feature = "arabic-shaping")]
            if twine_text::needs_shaping(s) {
                twine_text::shape_into(s, buf);
                return;
            }
            buf.push_str(s);
        }
    }

    /// Inserts `option` before option `pos` (at the end when `pos` is past it; LVGL
    /// `lv_dropdown_add_option`). Converts static options to owned ones.
    pub(crate) fn insert(&mut self, option: &str, pos: u32) {
        let n = u32::from(self.count());
        if n == 0 {
            self.set_owned(option);
            return;
        }
        let mut s = String::with_capacity(self.len() + option.len() + 1);
        s.push_str(self.as_str());
        if pos >= n {
            s.push('\n');
            s.push_str(option);
        } else {
            // The byte after the `pos`-th '\n' (0 for the first option).
            let at = if pos == 0 {
                0
            } else {
                s.match_indices('\n')
                    .nth(pos as usize - 1)
                    .map_or(s.len(), |(i, _)| i + 1)
            };
            s.insert(at, '\n');
            s.insert_str(at, option);
        }
        *self = Options::Owned(s);
    }
}

/// The number of `'\n'`-separated options in `s` (0 for `""`).
pub(crate) fn count(s: &str) -> u16 {
    if s.is_empty() {
        return 0;
    }
    let n = s.bytes().filter(|&b| b == b'\n').count() + 1;
    u16::try_from(n).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "arabic-shaping")]
    #[test]
    fn arabic_options_are_shaped_and_idempotent() {
        static OPTS: &str = "English\nسلام";
        let o = Options::from_static(OPTS);
        assert_eq!(o.get(1), Some("\u{FEB3}\u{FEFC}\u{FEE1}"));
        assert!(o.is_static(OPTS));
        assert!(o.same_as(OPTS));
        let mut owned = Options::Static("");
        owned.set_owned("سلام");
        assert!(owned.same_as("سلام"));
        assert_eq!(Options::from_static("plain"), Options::Static("plain"));
    }

    #[test]
    fn insert_positions() {
        let mut o = Options::Static("a\nb");
        o.insert("x", 0);
        assert_eq!(o.as_str(), "x\na\nb");
        o.insert("y", 2);
        assert_eq!(o.as_str(), "x\na\ny\nb");
        o.insert("z", 99);
        assert_eq!(o.as_str(), "x\na\ny\nb\nz");
        let mut e = Options::default();
        e.insert("only", 5);
        assert_eq!((e.as_str(), e.count()), ("only", 1));
    }

    #[test]
    fn owned_reuses_capacity() {
        let mut o = Options::Owned(String::with_capacity(64));
        o.set_owned("a\nb\nc");
        let cap = match &o {
            Options::Owned(s) => s.capacity(),
            _ => 0,
        };
        o.set_owned("d\ne");
        assert!(matches!(&o, Options::Owned(s) if s.capacity() == cap));
        assert_eq!(o.get(1), Some("e"));
        assert_eq!(o.get(2), None);
    }
}
