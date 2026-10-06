//! [`Options`]: the option list of dropdowns and rollers — a `'\n'`-separated string (LVGL's
//! format: in flash, or one owned allocation) or a `&'static` slice of options (zero-copy).

use alloc::string::String;

use twine_core::Size;
use twine_style::COORD_MAX;
use twine_text::{Font, TextLayout};

/// `'static` options handed to `set_options_static` of a
/// [`Dropdown`](crate::dropdown::Dropdown) or a [`Roller`](crate::roller::Roller): stored by
/// reference, never copied (unless they need Arabic shaping).
///
/// Converts from a `&'static str` of `'\n'`-separated options and from a `&'static` slice or
/// array of options.
///
/// ```
/// use twine_widgets_ext::StaticOptions;
///
/// static DAYS: [&str; 3] = ["Mon", "Tue", "Wed"];
/// assert_eq!(StaticOptions::from("Mon\nTue"), StaticOptions::Str("Mon\nTue"));
/// assert_eq!(StaticOptions::from(&DAYS), StaticOptions::Slice(&DAYS));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum StaticOptions {
    /// Options separated by `'\n'` (LVGL's format).
    Str(&'static str),
    /// One option per element (an option must not contain `'\n'`).
    Slice(&'static [&'static str]),
}

impl From<&'static str> for StaticOptions {
    fn from(s: &'static str) -> Self {
        StaticOptions::Str(s)
    }
}

impl From<&'static [&'static str]> for StaticOptions {
    fn from(s: &'static [&'static str]) -> Self {
        StaticOptions::Slice(s)
    }
}

impl<const N: usize> From<&'static [&'static str; N]> for StaticOptions {
    fn from(s: &'static [&'static str; N]) -> Self {
        StaticOptions::Slice(s)
    }
}

impl StaticOptions {
    /// Whether `self` and `other` are the same `'static` data (same address and length).
    fn same(self, other: StaticOptions) -> bool {
        match (self, other) {
            (StaticOptions::Str(a), StaticOptions::Str(b)) => core::ptr::eq(a, b),
            (StaticOptions::Slice(a), StaticOptions::Slice(b)) => core::ptr::eq(a, b),
            _ => false,
        }
    }
}

/// The options of a [`Dropdown`](crate::dropdown::Dropdown) or a
/// [`Roller`](crate::roller::Roller): `'\n'`-separated options in flash
/// ([`Static`](Options::Static)) or owned ([`Owned`](Options::Owned), one allocation reused by
/// later lists), or a `&'static` slice of options ([`Slice`](Options::Slice): zero-copy, e.g.
/// a table of translated names).
///
/// It compares equal to a `&str` of the same options joined with `'\n'`, and
/// [`Display`](core::fmt::Display) writes them so.
///
/// ```
/// use twine_widgets_ext::Options;
///
/// let o = Options::Static("Apple\nBanana\nCherry");
/// assert_eq!(o.count(), 3);
/// assert_eq!(o.get(1), Some("Banana"));
/// assert_eq!(o.index_of("Cherry"), Some(2));
/// assert_eq!(Options::Static("").count(), 0);
///
/// static FRUITS: [&str; 3] = ["Apple", "Banana", "Cherry"];
/// let s = Options::Slice(&FRUITS);
/// assert_eq!(s.get(2), Some("Cherry"));
/// assert_eq!(s, "Apple\nBanana\nCherry");
/// assert_eq!(s.iter().collect::<Vec<_>>(), o.iter().collect::<Vec<_>>());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Options {
    /// A `'static` string of `'\n'`-separated options (LVGL `set_options_static`).
    Static(&'static str),
    /// A `'static` slice, one option per element (stored by reference).
    Slice(&'static [&'static str]),
    /// An owned copy of `'\n'`-separated options (LVGL `set_options`); its capacity is reused
    /// by later option lists.
    Owned(String),
    /// `'static` options with Arabic/Persian letters, stored with their contextual forms
    /// (feature `arabic-shaping`, like LVGL's `LV_USE_ARABIC_PERSIAN_CHARS`).
    #[cfg(feature = "arabic-shaping")]
    Shaped {
        /// The options as given.
        source: StaticOptions,
        /// The options as drawn, `'\n'`-separated.
        shaped: String,
    },
}

impl Default for Options {
    fn default() -> Self {
        Options::Static("")
    }
}

/// The options of an [`Options`], in order ([`Options::iter`]).
#[derive(Clone, Debug)]
pub struct OptionsIter<'a>(IterInner<'a>);

#[derive(Clone, Debug)]
enum IterInner<'a> {
    Text(core::str::Split<'a, char>),
    Slice(core::slice::Iter<'a, &'static str>),
    Empty,
}

impl<'a> Iterator for OptionsIter<'a> {
    type Item = &'a str;

    #[inline]
    fn next(&mut self) -> Option<&'a str> {
        match &mut self.0 {
            IterInner::Text(it) => it.next(),
            IterInner::Slice(it) => it.next().copied(),
            IterInner::Empty => None,
        }
    }
}

impl Options {
    /// The `'\n'`-separated text of the text variants (`None` for a slice).
    fn text(&self) -> Option<&str> {
        match self {
            Options::Static(s) => Some(s),
            Options::Owned(s) => Some(s),
            Options::Slice(_) => None,
            #[cfg(feature = "arabic-shaping")]
            Options::Shaped { shaped, .. } => Some(shaped),
        }
    }

    /// The options in order (none for an empty list). Allocation-free.
    #[must_use]
    pub fn iter(&self) -> OptionsIter<'_> {
        OptionsIter(match (self, self.text()) {
            (Options::Slice(s), _) => IterInner::Slice(s.iter()),
            (_, Some("") | None) => IterInner::Empty,
            (_, Some(t)) => IterInner::Text(t.split('\n')),
        })
    }

    /// `'static` options: stored in place, or shaped into an owned copy when they contain
    /// Arabic/Persian letters (feature `arabic-shaping`).
    pub(crate) fn from_static(s: StaticOptions) -> Options {
        #[cfg(feature = "arabic-shaping")]
        {
            let needs = match s {
                StaticOptions::Str(t) => twine_text::needs_shaping(t),
                StaticOptions::Slice(v) => v.iter().any(|t| twine_text::needs_shaping(t)),
            };
            if needs {
                let mut shaped = String::new();
                match s {
                    StaticOptions::Str(t) => twine_text::shape_into(t, &mut shaped),
                    StaticOptions::Slice(v) => {
                        for (i, t) in v.iter().enumerate() {
                            if i > 0 {
                                shaped.push('\n');
                            }
                            shaped.push_str(&twine_text::shape(t));
                        }
                    }
                }
                return Options::Shaped { source: s, shaped };
            }
        }
        match s {
            StaticOptions::Str(t) => Options::Static(t),
            StaticOptions::Slice(v) => Options::Slice(v),
        }
    }

    /// Whether these are the `'static` options `s` (same address).
    pub(crate) fn is_static(&self, s: StaticOptions) -> bool {
        match self {
            Options::Static(cur) => s.same(StaticOptions::Str(cur)),
            Options::Slice(cur) => s.same(StaticOptions::Slice(cur)),
            #[cfg(feature = "arabic-shaping")]
            Options::Shaped { source, .. } => source.same(s),
            Options::Owned(_) => false,
        }
    }

    /// Whether setting `s` would store what is stored now (comparing the shaped form when
    /// `s` needs shaping).
    pub(crate) fn same_as(&self, s: &str) -> bool {
        #[cfg(feature = "arabic-shaping")]
        if twine_text::needs_shaping(s) {
            return *self == twine_text::shape(s).as_str();
        }
        *self == s
    }

    /// The number of options: `'\n'`s + 1 (0 for an empty string; LVGL
    /// `count_options_in_str`, whose empty list is `NULL`), or the slice's length — at most
    /// `u16::MAX`.
    #[must_use]
    pub fn count(&self) -> u16 {
        match self.text() {
            Some(t) => count(t),
            None => u16::try_from(self.iter().count()).unwrap_or(u16::MAX),
        }
    }

    /// Option `i` (`None` past the end).
    #[must_use]
    pub fn get(&self, i: u16) -> Option<&str> {
        match self {
            Options::Slice(s) => s.get(usize::from(i)).copied(),
            _ => self.iter().nth(usize::from(i)),
        }
    }

    /// The index of the option equal to `option` (LVGL `get_option_index`).
    #[must_use]
    pub fn index_of(&self, option: &str) -> Option<u16> {
        self.iter()
            .position(|o| o == option)
            .and_then(|i| u16::try_from(i).ok())
    }

    /// The length in bytes of the options joined with `'\n'`.
    #[must_use]
    pub fn len(&self) -> usize {
        match self.text() {
            Some(t) => t.len(),
            None => self.iter().map(|o| o.len() + 1).sum::<usize>().saturating_sub(1),
        }
    }

    /// Whether there are no options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// The size of the options laid out one per line in `font`: the widest option, and
    /// `n × line_height + (n − 1) × line_space` (one empty line for no options), like a label
    /// of the joined text. Allocation-free.
    #[must_use]
    pub fn measure(&self, font: &'static Font, letter_space: i32, line_space: i32) -> Size {
        let mut n = 0i32;
        let mut w = 0;
        for o in self {
            let mut l = TextLayout::new(o, font);
            l.letter_space = letter_space;
            l.max_width = COORD_MAX;
            w = w.max(l.measure().w);
            n += 1;
        }
        let n = n.max(1);
        Size::new(w, n * i32::from(font.line_height) + (n - 1) * line_space)
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
        for (i, o) in self.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            if u32::try_from(i).is_ok_and(|i| i == pos) {
                s.push_str(option);
                s.push('\n');
            }
            s.push_str(o);
        }
        if pos >= n {
            s.push('\n');
            s.push_str(option);
        }
        *self = Options::Owned(s);
    }
}

impl<'a> IntoIterator for &'a Options {
    type Item = &'a str;
    type IntoIter = OptionsIter<'a>;

    /// The options in order ([`Options::iter`]).
    fn into_iter(self) -> OptionsIter<'a> {
        self.iter()
    }
}

/// The options joined with `'\n'`.
impl core::fmt::Display for Options {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(t) = self.text() {
            return f.write_str(t);
        }
        for (i, o) in self.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            f.write_str(o)?;
        }
        Ok(())
    }
}

/// Equal to the options joined with `'\n'`.
impl PartialEq<str> for Options {
    fn eq(&self, other: &str) -> bool {
        if let Some(t) = self.text() {
            return t == other;
        }
        let mut theirs = if other.is_empty() {
            None
        } else {
            Some(other.split('\n'))
        };
        let mut ours = self.iter();
        loop {
            match (ours.next(), theirs.as_mut().and_then(Iterator::next)) {
                (None, None) => return true,
                (Some(a), Some(b)) if a == b => {}
                _ => return false,
            }
        }
    }
}

/// Equal to the options joined with `'\n'`.
impl PartialEq<&str> for Options {
    fn eq(&self, other: &&str) -> bool {
        *self == **other
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
    use alloc::string::ToString;

    use super::*;

    #[cfg(feature = "arabic-shaping")]
    #[test]
    fn arabic_options_are_shaped_and_idempotent() {
        static OPTS: &str = "English\nسلام";
        static SLICE: [&str; 2] = ["English", "سلام"];
        let o = Options::from_static(StaticOptions::Str(OPTS));
        assert_eq!(o.get(1), Some("\u{FEB3}\u{FEFC}\u{FEE1}"));
        assert!(o.is_static(StaticOptions::Str(OPTS)));
        assert!(o.same_as(OPTS));
        let mut owned = Options::Static("");
        owned.set_owned("سلام");
        assert!(owned.same_as("سلام"));
        assert_eq!(
            Options::from_static(StaticOptions::Str("plain")),
            Options::Static("plain")
        );
        let s = Options::from_static(StaticOptions::Slice(&SLICE));
        assert_eq!(s.get(1), Some("\u{FEB3}\u{FEFC}\u{FEE1}"));
        assert!(s.is_static(StaticOptions::Slice(&SLICE)));
    }

    #[test]
    fn insert_positions() {
        static S: [&str; 2] = ["p", "q"];
        let mut o = Options::Static("a\nb");
        o.insert("x", 0);
        assert_eq!(o, "x\na\nb");
        o.insert("y", 2);
        assert_eq!(o, "x\na\ny\nb");
        o.insert("z", 99);
        assert_eq!(o, "x\na\ny\nb\nz");
        let mut e = Options::default();
        e.insert("only", 5);
        assert_eq!((e.to_string().as_str(), e.count()), ("only", 1));
        let mut s = Options::Slice(&S);
        s.insert("m", 1);
        assert_eq!(s, Options::Owned("p\nm\nq".into()));
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

    #[test]
    fn slice_options_match_their_joined_text() {
        static S: [&str; 3] = ["Mon", "", "Wed"];
        let s = Options::Slice(&S);
        let t = Options::Static("Mon\n\nWed");
        assert_eq!(
            (s.count(), s.len(), s.is_empty()),
            (t.count(), t.len(), t.is_empty())
        );
        assert_eq!(s.index_of("Wed"), Some(2));
        assert_eq!(s.get(1), Some(""));
        assert_eq!(s, "Mon\n\nWed");
        assert_ne!(s, "Mon\n\nWed\nX");
        assert_ne!(s, "Mon");
        assert_eq!(s.to_string(), "Mon\n\nWed");
        assert!(s.same_as("Mon\n\nWed"));
        let empty = Options::Slice(&[]);
        assert!(empty.is_empty() && empty.to_string().is_empty() && empty.count() == 0);
        assert_eq!(empty.len(), 0);
        assert!(s.is_static(StaticOptions::Slice(&S)));
        assert!(!s.is_static(StaticOptions::Str("Mon\n\nWed")));
    }
}
