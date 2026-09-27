//! Translations (LVGL `lv_translation`): a static table of texts per language, a reactive
//! current language, and [`tr!`](crate::tr) texts that follow it.
//!
//! ```
//! use twine_extra::i18n::{Translations, provide_i18n};
//! use twine_extra::{tr, translations};
//! use twine_view::prelude::*;
//!
//! static TEXTS: Translations = translations! {
//!     langs: ["en", "de"],
//!     "greeting" => ["Hello", "Hallo"],
//!     "items" => ["{} items", "{} Dinge"],
//! };
//!
//! fn app(cx: Scope) -> impl View {
//!     let i18n = provide_i18n(cx, &TEXTS, "en");
//!     let n = cx.signal(3);
//!     column((
//!         label(tr!("greeting")),              // follows the language, never copied
//!         label(tr!("items", n.get())),        // `{}` placeholders, formatted in place
//!         button(label("Deutsch")).on_click(move || { i18n.set_language("de"); }),
//!     ))
//! }
//! # let _ = app;
//! ```
//!
//! Switching the language re-runs only the text bindings: views are not rebuilt, and labels
//! whose text is the same in both languages are not redrawn. The table lives in flash
//! (`static`), is sorted by key (checked at compile time by [`translations!`](crate::translations))
//! and is searched by binary search. A missing key or translation shows the key itself and
//! logs one `warn!` (per key and language).

use core::cell::RefCell;
use core::fmt::{Display, Write};

use alloc::boxed::Box;
use twine_reactive::{Scope, Signal};
use twine_view::text::TextProp;

/// A translation table: `table[i] = (key, [text per language])`, sorted by key.
#[derive(Debug)]
pub struct Translations {
    /// Language codes, e.g. `["en", "de", "zh"]` (the column order of `table`).
    pub langs: &'static [&'static str],
    /// `(key, texts)` rows sorted by key (strictly ascending bytes); each row has one text per
    /// language.
    pub table: &'static [(&'static str, &'static [&'static str])],
}

impl Translations {
    /// Index of language `code` in [`langs`](Self::langs).
    #[must_use]
    pub fn lang_index(&self, code: &str) -> Option<u8> {
        self.langs
            .iter()
            .position(|l| *l == code)
            .and_then(|i| u8::try_from(i).ok())
    }

    /// The text of `key` in language `lang` (`None` when the key or the translation is
    /// missing; an empty string counts as missing).
    #[must_use]
    pub fn lookup(&self, key: &str, lang: u8) -> Option<&'static str> {
        let i = self
            .table
            .binary_search_by(|(k, _)| k.as_bytes().cmp(key.as_bytes()))
            .ok()?;
        self.table[i]
            .1
            .get(usize::from(lang))
            .copied()
            .filter(|s| !s.is_empty())
    }
}

/// Whether `keys` are in strictly ascending byte order (used by
/// [`translations!`](crate::translations) at compile time).
///
/// ```
/// use twine_extra::i18n::keys_sorted;
/// assert!(keys_sorted(&["a", "ab", "b"]));
/// assert!(!keys_sorted(&["b", "a"]));
/// assert!(!keys_sorted(&["a", "a"]));
/// ```
#[must_use]
pub const fn keys_sorted(keys: &[&str]) -> bool {
    let mut i = 1;
    while i < keys.len() {
        if !str_less(keys[i - 1], keys[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// `a < b` byte-wise (a `const fn`).
const fn str_less(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut i = 0;
    while i < a.len() && i < b.len() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
        i += 1;
    }
    a.len() < b.len()
}

/// Whether every row has exactly `langs` texts (used by [`translations!`](crate::translations)).
#[must_use]
pub const fn rows_complete(langs: usize, rows: &[&[&str]]) -> bool {
    let mut i = 0;
    while i < rows.len() {
        if rows[i].len() != langs {
            return false;
        }
        i += 1;
    }
    true
}

/// A static [`Translations`] table, checked at compile time: keys must be string literals in
/// strictly ascending byte order and every row must have one text per language.
///
/// ```
/// use twine_extra::i18n::Translations;
/// use twine_extra::translations;
///
/// static T: Translations = translations! {
///     langs: ["en", "fr"],
///     "no" => ["No", "Non"],
///     "yes" => ["Yes", "Oui"],
/// };
/// assert_eq!(T.lookup("yes", 1), Some("Oui"));
/// ```
///
/// Unsorted keys do not compile:
///
/// ```compile_fail
/// use twine_extra::i18n::Translations;
/// use twine_extra::translations;
///
/// static T: Translations = translations! {
///     langs: ["en"],
///     "yes" => ["Yes"],
///     "no" => ["No"],
/// };
/// ```
#[macro_export]
macro_rules! translations {
    (langs: [$($lang:literal),* $(,)?], $($key:literal => [$($text:literal),* $(,)?]),* $(,)?) => {{
        const _: () = ::core::assert!(
            $crate::i18n::keys_sorted(&[$($key),*]),
            "translations!: keys must be in strictly ascending byte order"
        );
        const _: () = ::core::assert!(
            $crate::i18n::rows_complete([$($lang),*].len(), &[$(&[$($text),*]),*]),
            "translations!: every key needs one text per language"
        );
        $crate::i18n::Translations {
            langs: &[$($lang),*],
            table: &[$(($key, &[$($text),*])),*],
        }
    }};
}

/// The translations and the current language, provided as a context by [`provide_i18n`].
/// `Copy`: capture it in event handlers.
#[derive(Clone, Copy, Debug)]
pub struct I18n {
    lang: Signal<u8>,
    table: &'static Translations,
}

impl I18n {
    /// Switches to language `code`; every [`tr!`](crate::tr) text follows. Returns `false`
    /// (and logs a warning) for an unknown code.
    pub fn set_language(&self, code: &str) -> bool {
        let Some(i) = self.table.lang_index(code) else {
            twine_core::warn!(target: "twine::i18n", "unknown language {:?}", code);
            return false;
        };
        if self.lang.get_untracked() != i {
            twine_core::info!(target: "twine::i18n", "language: {}", code);
        }
        self.lang.set_if_changed(i);
        true
    }

    /// The current language code (tracked: a binding reading it re-runs on a switch).
    #[must_use]
    pub fn language(&self) -> &'static str {
        self.table
            .langs
            .get(usize::from(self.lang.get()))
            .copied()
            .unwrap_or("")
    }

    /// The index of the current language (tracked).
    #[must_use]
    pub fn language_index(&self) -> u8 {
        self.lang.get()
    }

    /// The translations.
    #[must_use]
    pub fn table(&self) -> &'static Translations {
        self.table
    }

    /// `key` in the current language (tracked); the key itself when missing (one `warn!` per
    /// key and language).
    #[must_use]
    pub fn tr(&self, key: &'static str) -> &'static str {
        let lang = self.lang.get();
        if let Some(s) = self.table.lookup(key, lang) {
            return s;
        }
        if first_warning(key, lang) {
            let code = self.table.langs.get(usize::from(lang)).copied().unwrap_or("?");
            twine_core::warn!(target: "twine::i18n", "missing translation of {:?} for {}", key, code);
        }
        key
    }
}

/// Provides the translations `table` with `initial` as the current language (the first
/// language when `initial` is unknown) to `cx` and its descendants, for [`tr!`](crate::tr)
/// and [`use_i18n`].
pub fn provide_i18n(cx: Scope, table: &'static Translations, initial: &str) -> I18n {
    let idx = table.lang_index(initial).unwrap_or_else(|| {
        twine_core::warn!(target: "twine::i18n", "unknown language {:?}, using the first", initial);
        0
    });
    let i18n = I18n {
        lang: cx.signal(idx),
        table,
    };
    cx.provide(i18n);
    i18n
}

/// The [`I18n`] provided to `cx` or an ancestor.
#[must_use]
pub fn use_i18n(cx: Scope) -> Option<I18n> {
    cx.use_context::<I18n>()
}

/// Keys (pointer, length, language) already warned about; later warnings pass through once the
/// set is full.
static WARNED: critical_section::Mutex<RefCell<heapless::Vec<(usize, usize, u8), 32>>> =
    critical_section::Mutex::new(RefCell::new(heapless::Vec::new()));

fn first_warning(key: &'static str, lang: u8) -> bool {
    let id = (key.as_ptr() as usize, key.len(), lang);
    critical_section::with(|cs| {
        let mut w = WARNED.borrow_ref_mut(cs);
        if w.contains(&id) {
            return false;
        }
        let _ = w.push(id);
        true
    })
}

fn no_i18n(key: &str) {
    twine_core::warn!(target: "twine::i18n", "tr!({:?}) outside provide_i18n: showing the key", key);
}

/// The text of [`tr!`](crate::tr) without arguments.
#[doc(hidden)]
#[must_use]
pub fn tr_text(key: &'static str) -> TextProp {
    TextProp::Scoped(Box::new(move |cx| {
        if let Some(i) = use_i18n(cx) {
            TextProp::StaticFn(Box::new(move || i.tr(key)))
        } else {
            no_i18n(key);
            TextProp::Static(key)
        }
    }))
}

/// The text of [`tr!`](crate::tr) with arguments: `args` writes the arguments' values with
/// [`write_placeholders`] into the translated format string.
#[doc(hidden)]
#[must_use]
pub fn tr_text_with(key: &'static str, args: impl Fn(&'static str, &mut dyn Write) + 'static) -> TextProp {
    TextProp::Scoped(Box::new(move |cx| {
        let i = use_i18n(cx);
        if i.is_none() {
            no_i18n(key);
        }
        TextProp::Write(Box::new(move |w| args(i.map_or(key, |i| i.tr(key)), w)))
    }))
}

/// Writes `fmt` with each `{}` replaced by the next of `args` (`{{` and `}}` are literal
/// braces; missing arguments leave the placeholder empty, extra ones are ignored).
///
/// ```
/// use twine_extra::i18n::write_placeholders;
/// let mut s = String::new();
/// write_placeholders(&mut s, "{} of {} {{ok}}", &[&3, &"ten"]);
/// assert_eq!(s, "3 of ten {ok}");
/// ```
pub fn write_placeholders(w: &mut dyn Write, fmt: &str, args: &[&dyn Display]) {
    let mut next = 0;
    let mut rest = fmt;
    while let Some(i) = rest.find(['{', '}']) {
        let _ = w.write_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("{{").or_else(|| tail.strip_prefix("}}")) {
            let _ = w.write_str(&tail[..1]);
            rest = after;
        } else if let Some(after) = tail.strip_prefix("{}") {
            if let Some(a) = args.get(next) {
                let _ = write!(w, "{a}");
            }
            next += 1;
            rest = after;
        } else {
            let _ = w.write_str(&tail[..1]);
            rest = &tail[1..];
        }
    }
    let _ = w.write_str(rest);
}

/// A translated text for a label (or any text property): `tr!("key")` follows the current
/// language of [`provide_i18n`] and stores the `'static` translation without copying;
/// `tr!("key", a, b)` fills the `{}` placeholders of the translation with the arguments
/// (re-evaluated when a signal they read changes, formatted in place). Without
/// `provide_i18n` in an enclosing scope the key is shown.
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::tr_text($key)
    };
    ($key:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::tr_text_with($key, move |__fmt: &'static str, __w: &mut dyn ::core::fmt::Write| {
            $crate::i18n::write_placeholders(__w, __fmt, &[$(&$arg as &dyn ::core::fmt::Display),+]);
        })
    };
}
