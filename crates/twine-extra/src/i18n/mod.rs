//! Compile-time translations (the role of LVGL's `lv_translation`): a catalog generated at
//! build time from translation files, a reactive current language, and [`tr!`](crate::tr)
//! texts that follow it.
//!
//! # The catalog
//!
//! `twine-build` (in `build.rs`) or `twine i18n` (the `twine-cli` tool) reads Fluent (`.ftl`),
//! gettext (`.po`) or TOML files — one per language — checks them and generates Rust:
//!
//! - `enum Key`: one variant per message (`Key::Greeting`), with `const fn args()` (the
//!   argument names [`tr!`](crate::tr) checks at compile time) and `const fn id()`;
//! - `enum Lang`: one variant per language (`Lang::De`), with `code()`, `from_code()`,
//!   `base_dir()` (right to left for Arabic, Hebrew, Persian, …), `font()` (the language's
//!   font, if the catalog names one), `ALL` and `DEFAULT`;
//! - one static message table per language, indexed by `Key` — every key in every language
//!   (missing translations were filled from fallback languages at build time), so nothing is
//!   searched, sorted or missing at run time.
//!
//! The enums implement [`MessageKey`] and [`Language`]; [`Message`], [`Segment`],
//! [`Plural`], [`Variant`] and [`Selector`] are the table's types. Include the generated
//! file in a module of its own:
//!
//! ```ignore
//! // build.rs: twine_build::i18n::Translations::from_dir("i18n").default_language("en")
//! //               .build()?.write_to_out_dir("i18n.rs")?;
//! mod i18n {
//!     include!(concat!(env!("OUT_DIR"), "/i18n.rs"));
//! }
//! ```
//!
//! # Showing texts
//!
//! ```
//! # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
//! use i18n::{Key, Lang};
//! use twine_extra::i18n::{Decimal, provide_i18n};
//! use twine_extra::tr;
//! use twine_view::prelude::*;
//!
//! fn app(cx: Scope) -> impl View {
//!     let i18n = provide_i18n(cx, Lang::En);
//!     let n = cx.signal(3);
//!     column((
//!         label(tr!(Key::Hello)),                         // a `&'static str`, never copied
//!         label(tr!(Key::Greeting, name = "Ada")),        // named arguments
//!         label(tr!(Key::Items, count = n.get())),        // plural: 0 / one / other …
//!         label(tr!(Key::Moved, "a.txt", "/tmp")),        // positional: {0}, {1}
//!         label(tr!(Key::Total, amount = Decimal::new(123_456, 2))), // 1,234.56 / 1.234,56 / ١٬٢٣٤٫٥٦
//!         button(label("Deutsch")).on_click(move || i18n.set_language(Lang::De)),
//!     ))
//!     .base_dir(move || i18n.base_dir())                  // right to left in Arabic
//! }
//! # let _ = app;
//! ```
//!
//! [`tr!`](crate::tr) checks at compile time that the arguments are exactly the key's (names,
//! positions, none missing or extra). Arguments are any `Display` values, formatted in place
//! into the label's text buffer; a plural message chooses its variant from the argument's
//! digits as displayed ([`PluralOperands::of`]), with the language's CLDR rule
//! ([`plural`]).
//!
//! # Numbers
//!
//! Integer and [`Decimal`] arguments of [`tr!`](crate::tr) are written with the current
//! language's [`NumberFormat`] — CLDR separators and grouping (Indian `12,34,567`), minus
//! sign, percent pattern and native digits (Arabic-Indic in Arabic, Extended Arabic-Indic in
//! Persian) — generated into the catalog for its languages only ([`Language::NUMBER_FORMATS`])
//! and linked only when a number is formatted. Formatting is integer-only and allocation-free
//! ([`number`]); [`I18n::number`] localizes a number outside a message. Plural messages choose
//! on the CLDR operands of the number as shown.
//!
//! # Cost
//!
//! Switching the language is allocation-free and re-runs only the text bindings: views are not
//! rebuilt, and labels whose text is the same in both languages are not redrawn. A text
//! lookup is two bounds-checked indexings (language row, key); a message without arguments is
//! a `&'static str` stored without copying. The tables live in flash; per message a
//! [`Message`] (three words) plus its strings.
//!
//! Code below [`provide_i18n`] reaches the handle with [`use_i18n`] (a `Result`) or
//! [`expect_i18n`] (panics outside), following the context convention of `Scope` (§ Context
//! values). Without a provider, [`tr!`](crate::tr) shows the default language and logs a
//! warning.

pub mod number;
pub mod plural;

use core::fmt::{Display, Write};

use alloc::boxed::Box;
use twine_reactive::{MissingContext, Scope, Signal};
use twine_view::text::TextProp;

pub use number::{Decimal, NumberFormat};
pub use plural::{PluralCategory, PluralOperands, PluralRule};
pub use twine_view::prelude::{BaseDir, Font};

/// The most arguments a message may have (the generator refuses more).
pub const MAX_ARGS: usize = 8;

/// The value of one message argument.
///
/// [`tr!`](crate::tr) passes integers and [`Decimal`]s as [`Arg::Number`] (formatted with the
/// current language's [`NumberFormat`], plural operands from the digits shown) and everything
/// else as [`Arg::Text`] (its `Display`; a plural message parses its digits).
#[derive(Clone, Copy)]
pub enum Arg<'a> {
    /// A number, formatted per language: `formats` is the catalog's
    /// [`Language::NUMBER_FORMATS`] (indexed by language).
    Number {
        /// The value and its display options.
        value: Decimal,
        /// One format per language of the catalog.
        formats: &'static [NumberFormat],
    },
    /// Any other value, shown as it displays.
    Text(&'a dyn Display),
}

impl core::fmt::Debug for Arg<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Arg::Number { value, .. } => f.debug_tuple("Number").field(value).finish(),
            Arg::Text(t) => f.debug_tuple("Text").field(&format_args!("{t}")).finish(),
        }
    }
}

impl Arg<'_> {
    /// The CLDR plural operands of the value as shown in language `lang` (`None`: not a plain
    /// number).
    #[must_use]
    pub fn operands(&self) -> Option<PluralOperands> {
        match self {
            Arg::Number { value, .. } => value.operands(),
            Arg::Text(t) => PluralOperands::of(*t),
        }
    }

    /// Writes the value as shown in language number `lang` (an index into
    /// [`Language::ALL`]).
    ///
    /// # Errors
    /// Only those of `w`.
    pub fn write(&self, lang: usize, w: &mut dyn Write) -> core::fmt::Result {
        match self {
            Arg::Number { value, formats } => {
                value.write(formats.get(lang).unwrap_or(&NumberFormat::ROOT), w)
            }
            Arg::Text(t) => write!(w, "{t}"),
        }
    }
}

/// One piece of a message: literal text or an argument's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    /// Literal text.
    Text(&'static str),
    /// The argument at this index of the key's `args()`.
    Arg(u8),
}

/// What a plural [`Variant`] matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    /// Exactly this non-negative integer (`[0]` in Fluent, `"0" = …` in TOML); checked before
    /// the categories.
    Exact(u64),
    /// A CLDR category of the language's rule.
    Category(PluralCategory),
}

/// One variant of a [`Plural`] message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant {
    /// What it matches.
    pub selector: Selector,
    /// Its text.
    pub segments: &'static [Segment],
}

/// A message that chooses its text by the plural category of an argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plural {
    /// Index (in the key's `args()`) of the argument it chooses on.
    pub arg: u8,
    /// The variants, exact values first.
    pub variants: &'static [Variant],
    /// The text when no variant matches (or the argument is not a plain number).
    pub other: &'static [Segment],
}

/// One message of one language in a generated catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// A text without arguments.
    Text(&'static str),
    /// A text with arguments.
    Format(&'static [Segment]),
    /// A plural choice.
    Plural(&'static Plural),
}

impl Message {
    /// The text when the message has no arguments, else `None`.
    #[must_use]
    pub const fn as_static(&self) -> Option<&'static str> {
        match self {
            Message::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Writes the message with `args` (the key's arguments in `args()` order) in language
    /// number `lang` (index into [`Language::ALL`], for number formats) with its plural
    /// `rule`. A missing argument writes nothing.
    ///
    /// ```
    /// use twine_extra::i18n::{Arg, Decimal, Message, NumberFormat, Segment, plural};
    ///
    /// static M: Message = Message::Format(&[Segment::Arg(1), Segment::Text(" / "), Segment::Arg(0)]);
    /// static FORMATS: [NumberFormat; 1] = [NumberFormat { group: ".", ..NumberFormat::ROOT }];
    /// let mut s = String::new();
    /// let n = Arg::Number { value: Decimal::from(1234), formats: &FORMATS };
    /// M.write(&mut s, &[n, Arg::Text(&"b")], 0, plural::en);
    /// assert_eq!(s, "b / 1.234");
    /// ```
    pub fn write(&self, w: &mut dyn Write, args: &[Arg<'_>], lang: usize, rule: PluralRule) {
        let segments: &'static [Segment] = match *self {
            Message::Text(s) => {
                let _ = w.write_str(s);
                return;
            }
            Message::Format(segs) => segs,
            Message::Plural(p) => p.select(args.get(usize::from(p.arg)), rule),
        };
        for seg in segments {
            let _ = match *seg {
                Segment::Text(t) => w.write_str(t),
                Segment::Arg(i) => match args.get(usize::from(i)) {
                    Some(a) => a.write(lang, w),
                    None => Ok(()),
                },
            };
        }
    }
}

impl Plural {
    /// The variant for the value `arg` (its plural operands as shown) under `rule`: an exact
    /// match, else the category's variant, else [`other`](Self::other).
    #[must_use]
    pub fn select(&self, arg: Option<&Arg<'_>>, rule: PluralRule) -> &'static [Segment] {
        let Some(op) = arg.and_then(Arg::operands) else {
            return self.other;
        };
        let category = rule(op);
        self.variants
            .iter()
            .find(|v| match v.selector {
                Selector::Exact(n) => op.is_exactly(n),
                Selector::Category(c) => c == category,
            })
            .map_or(self.other, |v| v.segments)
    }
}

/// A language of a generated catalog (the generated `Lang` enum implements it).
pub trait Language: Copy + PartialEq + 'static {
    /// The default language (the others fall back to it).
    const DEFAULT: Self;
    /// Every language, in declaration order.
    const ALL: &'static [Self];
    /// Position in [`ALL`](Self::ALL).
    fn index(self) -> usize;
    /// The language code (`"en"`, `"pt-BR"`).
    fn code(self) -> &'static str;
    /// The base text direction.
    fn base_dir(self) -> BaseDir;
    /// The language's font, if the catalog names one.
    fn font(self) -> Option<&'static Font>;
    /// The CLDR plural rule.
    fn plural_rule(self) -> PluralRule;
    /// How each language writes numbers, indexed by [`index`](Self::index) (CLDR data
    /// generated for the catalog's languages; linked only when a number is formatted).
    const NUMBER_FORMATS: &'static [NumberFormat];
    /// How this language writes numbers.
    fn number_format(self) -> &'static NumberFormat {
        Self::NUMBER_FORMATS
            .get(self.index())
            .unwrap_or(&NumberFormat::ROOT)
    }
    /// The messages, indexed by [`MessageKey::index`].
    fn messages(self) -> &'static [Message];
}

/// A message key of a generated catalog (the generated `Key` enum implements it).
pub trait MessageKey: Copy + 'static {
    /// The catalog's language type.
    type Lang: Language;
    /// Position in each language's [`messages`](Language::messages).
    fn index(self) -> usize;
    /// The identifier in the translation files (shown when a message is missing).
    fn id(self) -> &'static str;
}

/// The message `key` in language `lang` (`None` only for a catalog that is not generated).
fn message<K: MessageKey>(lang: K::Lang, key: K) -> Option<&'static Message> {
    lang.messages().get(key.index())
}

/// The current language of a catalog, provided as a context by [`provide_i18n`]. `Copy`:
/// capture it in event handlers.
///
/// ```
/// # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
/// use i18n::{Key, Lang};
/// use twine_extra::i18n::provide_i18n;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let i18n = provide_i18n(cx, Lang::En);
/// assert_eq!(i18n.text(Key::Hello), "Hello");
/// i18n.set_language(Lang::Ar);
/// assert_eq!(i18n.language(), Lang::Ar);
/// assert_eq!(i18n.base_dir(), twine_extra::i18n::BaseDir::Rtl);
/// ```
#[derive(Clone, Copy)]
pub struct I18n<L: Language> {
    lang: Signal<L>,
}

impl<L: Language> core::fmt::Debug for I18n<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("I18n")
            .field("language", &self.lang.get_untracked().code())
            .finish()
    }
}

impl<L: Language> I18n<L> {
    /// Switches to `lang`; every [`tr!`](crate::tr) text follows (only bindings re-run, no
    /// allocation). Idempotent: the same language changes nothing.
    pub fn set_language(&self, lang: L) {
        if self.lang.get_untracked() != lang {
            twine_core::info!(target: "twine::i18n", "language: {}", lang.code());
        }
        self.lang.set_if_changed(lang);
    }

    /// The current language (tracked: a binding reading it re-runs on a switch).
    #[must_use]
    pub fn language(&self) -> L {
        self.lang.get()
    }

    /// The current language without tracking.
    #[must_use]
    pub fn language_untracked(&self) -> L {
        self.lang.get_untracked()
    }

    /// The current language's base direction (tracked): `.base_dir(move || i18n.base_dir())`.
    #[must_use]
    pub fn base_dir(&self) -> BaseDir {
        self.language().base_dir()
    }

    /// The current language's font, if the catalog names one (tracked).
    #[must_use]
    pub fn font(&self) -> Option<&'static Font> {
        self.language().font()
    }

    /// `value` localized in the current language (tracked): a `Display` for `text!` or
    /// `format!` (`label(text!("{}", i18n.number(n.get())))`).
    #[must_use]
    pub fn number(&self, value: impl Into<Decimal>) -> number::Localized<'static> {
        value.into().with(self.language().number_format())
    }

    /// The text of `key` (a message without arguments) in the current language (tracked).
    /// A key with arguments gives its identifier and logs a warning: show those with
    /// [`tr!`](crate::tr).
    #[must_use]
    pub fn text<K: MessageKey<Lang = L>>(&self, key: K) -> &'static str {
        static_text(self.language(), key)
    }
}

fn static_text<K: MessageKey>(lang: K::Lang, key: K) -> &'static str {
    match message(lang, key).and_then(Message::as_static) {
        Some(s) => s,
        None => not_static(key.id()),
    }
}

#[cold]
#[inline(never)]
fn not_static(id: &'static str) -> &'static str {
    twine_core::warn!(target: "twine::i18n", "{:?} has arguments: show it with tr!", id);
    id
}

/// Provides language `initial` of catalog `L` to `cx` and its descendants, for
/// [`tr!`](crate::tr), [`use_i18n`] and [`expect_i18n`].
#[must_use = "the handle switches the language"]
pub fn provide_i18n<L: Language>(cx: Scope, initial: L) -> I18n<L> {
    let i18n = I18n {
        lang: cx.signal(initial),
    };
    cx.provide(i18n);
    i18n
}

/// The [`I18n`] of catalog `L` provided to `cx` or an ancestor by [`provide_i18n`], or
/// [`MissingContext`] naming it when there is none — the non-panicking form (`.ok()` for code
/// that works with and without translations; see [`expect_i18n`]). A scope-chain walk without
/// allocation (`Scope` § Context values): look it up once while building.
///
/// Never panics.
///
/// # Errors
/// [`MissingContext`] naming `I18n<L>` when no ancestor provides one.
///
/// ```
/// # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
/// use twine_extra::i18n::use_i18n;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// assert!(use_i18n::<i18n::Lang>(cx).is_err()); // no `provide_i18n` above
/// ```
pub fn use_i18n<L: Language>(cx: Scope) -> Result<I18n<L>, MissingContext> {
    cx.use_context::<I18n<L>>()
}

/// The [`I18n`] of catalog `L` provided to `cx` or an ancestor by [`provide_i18n`]: the
/// panicking form of [`use_i18n`], a thin `#[track_caller]` wrapper for application code
/// below `provide_i18n` (e.g. a language picker).
///
/// `twine-extra` is not part of the `safety` profile yet, so this function exists in every
/// build; profile code uses [`use_i18n`] and handles the error.
///
/// # Panics
///
/// If no ancestor of `cx` called [`provide_i18n`] for `L`, with `"context
/// twine_extra::i18n::I18n<…> not provided"` at the caller's location ([`MissingContext`]).
///
/// ```should_panic
/// # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
/// use twine_extra::i18n::expect_i18n;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let _ = expect_i18n::<i18n::Lang>(cx); // panics: no `provide_i18n` above
/// ```
#[must_use]
#[track_caller]
pub fn expect_i18n<L: Language>(cx: Scope) -> I18n<L> {
    match use_i18n(cx) {
        Ok(i18n) => i18n,
        Err(e) => missing(e),
    }
}

/// The panic path of [`expect_i18n`] (cold, out of line). Not `MissingContext::panic`, which
/// the `safety` profile removes: this crate is not a profile crate and builds in safety builds
/// too.
#[cold]
#[inline(never)]
#[track_caller]
fn missing(e: MissingContext) -> ! {
    panic!("{e}")
}

/// The language a [`tr!`](crate::tr) text built in `cx` follows: the provided handle, or
/// `None` (the default language, with a warning) outside [`provide_i18n`].
fn lookup<K: MessageKey>(cx: Scope, key: K) -> Option<I18n<K::Lang>> {
    let found = use_i18n::<K::Lang>(cx).ok();
    if found.is_none() {
        no_i18n(key.id());
    }
    found
}

#[cold]
#[inline(never)]
fn no_i18n(id: &'static str) {
    twine_core::warn!(target: "twine::i18n", "tr!({}) outside provide_i18n: showing the default language", id);
}

// ---- `tr!` support ----------------------------------------------------------------------

/// How the arguments of one [`tr!`](crate::tr) map to a key's arguments: entry `i` is the
/// position (in the `tr!` call) of the key's argument `i`. Computed at compile time by
/// [`bind_args`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct ArgMap {
    to_given: [u8; MAX_ARGS],
    len: u8,
}

/// An error message built at compile time (for `tr!`'s argument check).
struct ConstMsg {
    buf: [u8; 256],
    len: usize,
}

impl ConstMsg {
    const fn new() -> ConstMsg {
        ConstMsg {
            buf: [0; 256],
            len: 0,
        }
    }

    const fn push(mut self, s: &str) -> ConstMsg {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() && self.len < self.buf.len() {
            self.buf[self.len] = b[i];
            self.len += 1;
            i += 1;
        }
        self
    }

    const fn push_list(mut self, names: &[&str]) -> ConstMsg {
        if names.is_empty() {
            return self.push("none");
        }
        let mut i = 0;
        while i < names.len() {
            if i > 0 {
                self = self.push(", ");
            }
            self = self.push(names[i]);
            i += 1;
        }
        self
    }

    const fn fail(&self) -> ! {
        let (bytes, _) = self.buf.split_at(self.len);
        match core::str::from_utf8(bytes) {
            Ok(s) => panic!("{}", s),
            Err(_) => panic!("tr!: the arguments do not match the key"),
        }
    }
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether `name` is the decimal number `n` (a positional argument's name).
const fn is_position(name: &str, n: usize) -> bool {
    let b = name.as_bytes();
    if b.is_empty() || (b.len() > 1 && b[0] == b'0') {
        return false;
    }
    let mut v = 0usize;
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return false;
        }
        v = v * 10 + (b[i] - b'0') as usize;
        i += 1;
    }
    v == n
}

/// The decimal digits of a small number (positions are < [`MAX_ARGS`]).
const DIGITS: [&str; MAX_ARGS] = ["0", "1", "2", "3", "4", "5", "6", "7"];

/// Checks the arguments `given` to [`tr!`](crate::tr) (names; `""` for a positional
/// argument) against the key's `expected` arguments and maps them, failing compilation with a
/// message naming the problem. Evaluated in a `const` by the macro.
#[doc(hidden)]
#[must_use]
pub const fn bind_args(key: &str, expected: &[&str], given: &[&str]) -> ArgMap {
    let mut map = ArgMap {
        to_given: [u8::MAX; MAX_ARGS],
        len: 0,
    };
    if given.len() > MAX_ARGS || expected.len() > MAX_ARGS {
        ConstMsg::new()
            .push("tr!: too many arguments for `")
            .push(key)
            .push("`")
            .fail();
    }
    map.len = expected.len() as u8;
    let mut positional = 0;
    let mut g = 0;
    while g < given.len() {
        let name = given[g];
        let mut slot = 0;
        let mut found = false;
        while slot < expected.len() {
            let hit = if name.is_empty() {
                is_position(expected[slot], positional)
            } else {
                str_eq(expected[slot], name)
            };
            if hit {
                found = true;
                break;
            }
            slot += 1;
        }
        if !found {
            let shown = if name.is_empty() {
                if positional < DIGITS.len() {
                    DIGITS[positional]
                } else {
                    "?"
                }
            } else {
                name
            };
            ConstMsg::new()
                .push("tr!: `")
                .push(key)
                .push("` has no argument `")
                .push(shown)
                .push("` (its arguments: ")
                .push_list(expected)
                .push(")")
                .fail();
        }
        if map.to_given[slot] != u8::MAX {
            ConstMsg::new()
                .push("tr!: argument `")
                .push(expected[slot])
                .push("` of `")
                .push(key)
                .push("` given twice")
                .fail();
        }
        map.to_given[slot] = g as u8;
        if name.is_empty() {
            positional += 1;
        }
        g += 1;
    }
    let mut slot = 0;
    while slot < expected.len() {
        if map.to_given[slot] == u8::MAX {
            ConstMsg::new()
                .push("tr!: `")
                .push(key)
                .push("` needs the argument `")
                .push(expected[slot])
                .push("` (its arguments: ")
                .push_list(expected)
                .push(")")
                .fail();
        }
        slot += 1;
    }
    map
}

impl ArgMap {
    /// Calls `f` with `given` reordered into the key's argument order.
    #[inline]
    fn with_ordered(&self, given: &[Arg<'_>], f: impl FnOnce(&[Arg<'_>])) {
        let mut ordered: [Arg<'_>; MAX_ARGS] = [Arg::Text(&""); MAX_ARGS];
        let n = usize::from(self.len).min(MAX_ARGS);
        for (slot, out) in ordered.iter_mut().enumerate().take(n) {
            if let Some(a) = self.to_given.get(slot).and_then(|g| given.get(usize::from(*g))) {
                *out = *a;
            }
        }
        f(ordered.get(..n).unwrap_or(&[]));
    }
}

/// The text of [`tr!`](crate::tr) without arguments.
#[doc(hidden)]
#[must_use]
pub fn tr_text<K: MessageKey>(key: K) -> TextProp {
    TextProp::Scoped(Box::new(move |cx| match lookup(cx, key) {
        Some(i) => TextProp::StaticFn(Box::new(move || i.text(key))),
        None => TextProp::Static(static_text(K::Lang::DEFAULT, key)),
    }))
}

/// The text of [`tr!`](crate::tr) with arguments: `args` calls its callback with the
/// arguments in the order of the `tr!` call; `map` (from [`bind_args`]) reorders them.
#[doc(hidden)]
#[must_use]
pub fn tr_text_args<K: MessageKey>(
    key: K,
    map: ArgMap,
    args: impl Fn(&mut dyn FnMut(&[Arg<'_>])) + 'static,
) -> TextProp {
    TextProp::Scoped(Box::new(move |cx| {
        let i18n = lookup(cx, key);
        TextProp::Write(Box::new(move |w| {
            let lang = i18n.map_or(K::Lang::DEFAULT, |i| i.language());
            let Some(msg) = message(lang, key) else { return };
            let rule = lang.plural_rule();
            let index = lang.index();
            args(&mut |given| map.with_ordered(given, |ordered| msg.write(w, ordered, index, rule)));
        }))
    }))
}

/// A [`tr!`](crate::tr) argument before its kind is chosen: `(&ArgTag(&value, key)).__tr_arg()`
/// picks [`ArgNumber`] for integers and [`Decimal`] (a method on `&ArgTag`, found first) and
/// [`ArgText`] for any other `Display` (autoref specialization; resolved at compile time).
#[doc(hidden)]
pub struct ArgTag<'a, T: ?Sized, K>(pub &'a T, pub K);

impl<T: ?Sized, K: Copy> Clone for ArgTag<'_, T, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized, K: Copy> Copy for ArgTag<'_, T, K> {}

/// [`ArgTag`] of a number: formatted per language.
#[doc(hidden)]
pub trait ArgNumber<'a> {
    /// The argument.
    fn __tr_arg(self) -> Arg<'a>;
}

impl<'a, T: Copy + Into<Decimal>, K: MessageKey> ArgNumber<'a> for &ArgTag<'a, T, K> {
    #[inline]
    fn __tr_arg(self) -> Arg<'a> {
        Arg::Number {
            value: (*self.0).into(),
            formats: <K::Lang as Language>::NUMBER_FORMATS,
        }
    }
}

/// [`ArgTag`] of any other `Display` value.
#[doc(hidden)]
pub trait ArgText<'a> {
    /// The argument.
    fn __tr_arg(self) -> Arg<'a>;
}

impl<'a, T: Display, K: Copy> ArgText<'a> for ArgTag<'a, T, K> {
    #[inline]
    fn __tr_arg(self) -> Arg<'a> {
        Arg::Text(self.0)
    }
}

/// A translated text for a label (or any text property), in the current language of
/// [`provide_i18n`]:
///
/// - `tr!(Key::Hello)` — a message without arguments, stored as `&'static str` (no copy);
/// - `tr!(Key::Greeting, name = user)` — named arguments (`{name}` / `{ $name }`);
/// - `tr!(Key::Moved, from, to)` — positional arguments (`{0}`, `{1}`), before named ones.
///
/// The key must be a constant (`Key::…`); the arguments are checked against its `args()` **at
/// compile time** — an unknown, duplicate or missing argument does not compile. Arguments are
/// evaluated (and formatted in place, without allocation once the label's buffer is large
/// enough) whenever a signal they read or the language changes: integers and [`Decimal`]s
/// are written with the current language's [`NumberFormat`] (`1,234.5` / `1.234,5` /
/// `١٬٢٣٤٫٥`), any other `Display` value as it displays. A plural message chooses its
/// variant on the CLDR operands of the argument as shown (`1` vs `1.0`). Without
/// `provide_i18n` in an enclosing scope the default language is shown (and a warning logged).
///
/// ```
/// # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
/// use i18n::Key;
/// use twine_extra::tr;
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let n = cx.signal(2);
/// let _ = label(tr!(Key::Items, count = n.get()));
/// let _ = label(tr!(Key::Moved, "a.txt", "/tmp"));
/// ```
///
/// A wrong argument is a compile error naming it:
///
/// ```compile_fail
/// # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
/// use i18n::Key;
/// // error: tr!: `greeting` has no argument `nme` (its arguments: name)
/// let _ = twine_extra::tr!(Key::Greeting, nme = "Ada");
/// ```
///
/// ```compile_fail
/// # mod i18n { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n/catalog.rs")); }
/// use i18n::Key;
/// // error: tr!: `items` needs the argument `count` (its arguments: count)
/// let _ = twine_extra::tr!(Key::Items);
/// ```
#[macro_export]
macro_rules! tr {
    ($key:expr $(,)?) => {{
        const _: $crate::i18n::ArgMap = $crate::i18n::bind_args($key.id(), $key.args(), &[]);
        $crate::i18n::tr_text($key)
    }};
    ($key:expr, $($rest:tt)+) => {
        $crate::__tr_args!([$key] [] [] $($rest)+)
    };
}

/// Implementation of [`tr!`](crate::tr): collects `name = value` and positional arguments.
#[doc(hidden)]
#[macro_export]
macro_rules! __tr_args {
    ([$key:expr] [$($n:expr,)*] [$($v:expr,)*] $name:ident = $val:expr, $($rest:tt)*) => {
        $crate::__tr_args!([$key] [$($n,)* ::core::stringify!($name),] [$($v,)* $val,] $($rest)*)
    };
    ([$key:expr] [$($n:expr,)*] [$($v:expr,)*] $name:ident = $val:expr) => {
        $crate::__tr_args!([$key] [$($n,)* ::core::stringify!($name),] [$($v,)* $val,])
    };
    ([$key:expr] [$($n:expr,)*] [$($v:expr,)*] $val:expr, $($rest:tt)*) => {
        $crate::__tr_args!([$key] [$($n,)* "",] [$($v,)* $val,] $($rest)*)
    };
    ([$key:expr] [$($n:expr,)*] [$($v:expr,)*] $val:expr) => {
        $crate::__tr_args!([$key] [$($n,)* "",] [$($v,)* $val,])
    };
    ([$key:expr] [$($n:expr,)*] [$($v:expr,)*]) => {{
        const __TR_MAP: $crate::i18n::ArgMap = $crate::i18n::bind_args($key.id(), $key.args(), &[$($n),*]);
        $crate::i18n::tr_text_args(
            $key,
            __TR_MAP,
            move |__tr_f: &mut dyn ::core::ops::FnMut(&[$crate::i18n::Arg<'_>])| {
                #[allow(unused_imports)]
                use $crate::i18n::{ArgNumber as _, ArgText as _};
                __tr_f(&[$((&$crate::i18n::ArgTag(&$v, $key)).__tr_arg()),*])
            },
        )
    }};
}
