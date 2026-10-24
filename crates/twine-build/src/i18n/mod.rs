//! Compile-time translations: read translation files (Fluent `.ftl`, gettext `.po` or TOML),
//! check them, and generate the Rust catalog that `twine_extra::i18n` uses at run time — a
//! `Key` enum (one variant per message), a `Lang` enum (one variant per language) and static
//! message tables.
//!
//! ```
//! use twine_build::i18n::{Format, Translations};
//!
//! let catalog = Translations::new()
//!     .source("en", Format::Toml, r#"
//!         greeting = "Hello, {name}!"
//!         [items]
//!         one = "{count} item"
//!         other = "{count} items"
//!     "#)
//!     .source("de", Format::Fluent, "
//! greeting = Hallo, { $name }!
//! items = { $count ->
//!     [one] { $count } Ding
//!    *[other] { $count } Dinge
//! }
//! ")
//!     .default_language("en")
//!     .build()
//!     .unwrap();
//! assert_eq!(catalog.keys().collect::<Vec<_>>(), ["greeting", "items"]);
//! assert_eq!(catalog.preview("de", "items").unwrap(), "{count -> [one] {count} Ding | *[other] {count} Dinge}");
//! let rust = catalog.to_rust();
//! assert!(rust.contains("pub enum Key"));
//! ```
//!
//! In a build script, read a directory with one file per language (`i18n/en.ftl`,
//! `i18n/de.po`, `i18n/zh.toml`, … — the file stem is the language code) and write the catalog
//! into `OUT_DIR`:
//!
//! ```no_run
//! // In `fn main` of build.rs:
//! let catalog = twine_build::i18n::Translations::from_dir("i18n")
//!     .default_language("en")
//!     .build()
//!     .unwrap_or_else(|e| panic!("{e}"));
//! catalog.write_to_out_dir("i18n.rs").unwrap_or_else(|e| panic!("{e}"));
//! ```
//!
//! and include it in the application (in a module of its own):
//!
//! ```ignore
//! mod i18n {
//!     include!(concat!(env!("OUT_DIR"), "/i18n.rs"));
//! }
//! use i18n::{Key, Lang};
//! ```
//!
//! `twine i18n <dir> -o <file.rs>` (the `twine-cli` tool) does the same from the command
//! line.
//!
//! # What is checked
//!
//! Errors (the build fails, each with file and line where known): syntax errors and
//! unsupported constructs; a key or language whose Rust name collides with another; a key
//! defined twice; a message using an argument the default language's message does not have;
//! positional arguments that are not `0..n`; more than [`MAX_ARGS`] arguments; a plural message
//! without an argument to choose on; a language without a known plural rule (choose one with
//! [`plural_rules`](Translations::plural_rules)); a `.po` plural entry with a number of forms
//! that matches neither the language's categories nor its integer categories.
//!
//! Warnings ([`Catalog::warnings`]; errors with [`strict`](Translations::strict)): a key
//! missing in a language (its text falls back to the language's fallback, then to the default
//! language — the generated catalog has every text in every language, so nothing is looked up
//! or missing at run time); a key only present in a non-default language (ignored); a plural
//! variant the language never chooses (`[few]` in English); `#, fuzzy` `.po` entries (treated as
//! missing).
//!
//! # Keys, languages and arguments
//!
//! - Key identifiers (`greeting`, `items-count`, `settings.title`) become `Key` variants
//!   (`Greeting`, `ItemsCount`, `SettingsTitle`); keys are sorted by identifier, so the output
//!   does not depend on the order in the files.
//! - Language codes (`en`, `pt-BR`, `zh_Hant`) become `Lang` variants (`En`, `PtBr`, `ZhHant`);
//!   the default language comes first, then the others by code, unless
//!   [`languages`](Translations::languages) gives the order.
//! - Arguments are named (`{name}`, `{ $name }` in Fluent; `-` becomes `_`) or positional
//!   (`{0}`, `{1}` in TOML and `.po`), so translations can reorder them. A message's arguments
//!   are those of its default-language text; translations may use fewer.
//! - Base direction: right to left for Arabic, Hebrew, Persian, Urdu, Yiddish, Pashto, Sindhi,
//!   Uyghur, Sorani, Dhivehi and Syriac (and any tag with an Arabic, Hebrew, Thaana, Syriac, N'Ko
//!   or Adlam script subtag); override with [`base_dir`](Translations::base_dir).
//! - Plural rules: CLDR cardinal rules ([`RULES`]), chosen by language code.
//! - Number formats: CLDR symbols of the language's default numbering system ([`NUMBERS`]:
//!   separators, Indian grouping, minus sign, percent pattern, Arabic-Indic digits for `ar`,
//!   Extended Arabic-Indic for `fa`), emitted as `Language::NUMBER_FORMATS`; override with
//!   [`number_symbols`](Translations::number_symbols). A language without known symbols uses
//!   CLDR root (`1,234.5`) with a warning.

mod emit;
mod ftl;
mod model;
mod number;
mod plural;
mod po;
mod toml_src;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use model::{Body, Entry, Seg, Sel, camel, concat};
pub use number::{NUMBERS, NumberSymbols, ROOT as ROOT_NUMBERS, numbers_for};
pub use plural::{Category, PluralRules, RULES, rules_for, rules_named};

/// The most arguments a message may have (the runtime's `twine_extra::i18n::MAX_ARGS`).
pub const MAX_ARGS: usize = 8;

/// The source format of a translation file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// Fluent (`.ftl`): the subset that compiles to static tables (see the module docs of
    /// [`Translations`]); one select expression per message for plurals.
    Fluent,
    /// gettext (`.po`), ID-based: `msgid` is the key, `msgstr` the text with `{name}`
    /// placeholders; plural entries list the language's CLDR categories in order.
    Po,
    /// TOML (`.toml`): `key = "text"`, tables group keys, a table with `other` is a plural
    /// message.
    Toml,
}

impl Format {
    /// The format of a file extension (`"ftl"`, `"po"`, `"toml"`).
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext {
            "ftl" => Some(Format::Fluent),
            "po" => Some(Format::Po),
            "toml" => Some(Format::Toml),
            _ => None,
        }
    }
}

/// A base text direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    /// Left to right.
    Ltr,
    /// Right to left.
    Rtl,
}

/// One problem found while reading or checking translations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The file (or inline source name) it is about; empty for the configuration.
    pub file: String,
    /// The 1-based line, or 0 when unknown.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.file.is_empty(), self.line) {
            (true, _) => f.write_str(&self.message),
            (false, 0) => write!(f, "{}: {}", self.file, self.message),
            (false, l) => write!(f, "{}:{l}: {}", self.file, self.message),
        }
    }
}

/// The translations could not be built: every problem found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The problems, in the order found.
    pub diagnostics: Vec<Diagnostic>,
}

impl Error {
    fn config(message: impl Into<String>) -> Error {
        Error {
            diagnostics: vec![diag("", 0, message)],
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

fn diag(file: &str, line: usize, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        file: file.to_string(),
        line,
        message: message.into(),
    }
}

#[derive(Clone, Debug)]
enum Input {
    Dir(PathBuf),
    File(PathBuf),
    Text {
        lang: String,
        format: Format,
        text: String,
    },
}

/// The translation sources and options: a builder whose [`build`](Translations::build) reads,
/// checks and compiles them into a [`Catalog`]. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Translations {
    inputs: Vec<Input>,
    default: Option<String>,
    languages: Option<Vec<String>>,
    fonts: BTreeMap<String, String>,
    dirs: BTreeMap<String, Dir>,
    plural: BTreeMap<String, String>,
    numbers: BTreeMap<String, NumberSymbols>,
    fallbacks: BTreeMap<String, String>,
    strict: bool,
    runtime: String,
}

impl Default for Translations {
    fn default() -> Self {
        Translations::new()
    }
}

impl Translations {
    /// No sources yet; the generated code uses the runtime at `::twine_extra::i18n`.
    #[must_use]
    pub fn new() -> Translations {
        Translations {
            inputs: Vec::new(),
            default: None,
            languages: None,
            fonts: BTreeMap::new(),
            dirs: BTreeMap::new(),
            plural: BTreeMap::new(),
            numbers: BTreeMap::new(),
            fallbacks: BTreeMap::new(),
            strict: false,
            runtime: "::twine_extra::i18n".to_string(),
        }
    }

    /// The translation files of directory `dir`: every `<code>.ftl`, `<code>.po` and
    /// `<code>.toml` (other files are ignored; one file per language).
    #[must_use]
    pub fn from_dir(dir: impl Into<PathBuf>) -> Translations {
        Translations::new().dir(dir)
    }

    /// Adds the translation files of directory `dir` (see [`from_dir`](Self::from_dir)).
    #[must_use]
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Translations {
        self.inputs.push(Input::Dir(dir.into()));
        self
    }

    /// Adds one translation file; its stem is the language code (`i18n/de.po` → `de`), its
    /// extension the [`Format`].
    #[must_use]
    pub fn file(mut self, path: impl Into<PathBuf>) -> Translations {
        self.inputs.push(Input::File(path.into()));
        self
    }

    /// Adds the translations of language `lang` from text (tests, generated sources).
    #[must_use]
    pub fn source(mut self, lang: &str, format: Format, text: &str) -> Translations {
        self.inputs.push(Input::Text {
            lang: lang.to_string(),
            format,
            text: text.to_string(),
        });
        self
    }

    /// The default language: it must define every key, every other language falls back to it,
    /// and it is `Lang::DEFAULT`. Without this option: the first of
    /// [`languages`](Self::languages), or the only language.
    #[must_use]
    pub fn default_language(mut self, code: &str) -> Translations {
        self.default = Some(code.to_string());
        self
    }

    /// The languages to compile, in `Lang` order (others are skipped). Without this option:
    /// every language found, the default first, then by code.
    #[must_use]
    pub fn languages<S: Into<String>>(mut self, codes: impl IntoIterator<Item = S>) -> Translations {
        self.languages = Some(codes.into_iter().map(Into::into).collect());
        self
    }

    /// The font of language `code`: `path` is a Rust path to a `&'static`-able
    /// `twine_text::Font` (`"crate::fonts::CJK"`, typically a font whose fallback chain covers
    /// the language's script), returned by `Lang::font` and `I18n::font`.
    #[must_use]
    pub fn font(mut self, code: &str, path: &str) -> Translations {
        self.fonts.insert(code.to_string(), path.to_string());
        self
    }

    /// Overrides the base direction of language `code` (see the module docs for the default).
    #[must_use]
    pub fn base_dir(mut self, code: &str, dir: Dir) -> Translations {
        self.dirs.insert(code.to_string(), dir);
        self
    }

    /// Chooses the plural rule of language `code` by [`PluralRules::name`] (for languages the
    /// generator does not know, or to override).
    #[must_use]
    pub fn plural_rules(mut self, code: &str, rules: &str) -> Translations {
        self.plural.insert(code.to_string(), rules.to_string());
        self
    }

    /// Sets how language `code` writes numbers (for languages the generator does not know,
    /// or to override [`NUMBERS`]).
    #[must_use]
    pub fn number_symbols(mut self, code: &str, symbols: NumberSymbols) -> Translations {
        self.numbers.insert(code.to_string(), symbols);
        self
    }

    /// Language `code` takes its missing texts from `fallback` first (then from the default
    /// language). Without this option a regional language (`de-AT`) falls back to its base
    /// language (`de`) when that is compiled too.
    #[must_use]
    pub fn fallback(mut self, code: &str, fallback: &str) -> Translations {
        self.fallbacks.insert(code.to_string(), fallback.to_string());
        self
    }

    /// Turns warnings (missing translations, unknown keys, never-chosen plural variants, fuzzy
    /// entries) into errors.
    #[must_use]
    pub fn strict(mut self, strict: bool) -> Translations {
        self.strict = strict;
        self
    }

    /// The Rust path of the runtime module the generated code uses (default
    /// `::twine_extra::i18n`).
    #[must_use]
    pub fn runtime_path(mut self, path: &str) -> Translations {
        self.runtime = path.to_string();
        self
    }

    /// Reads, checks and compiles the translations.
    ///
    /// # Errors
    /// Every problem found (see the module docs), with file and line where known.
    pub fn build(&self) -> Result<Catalog, Error> {
        Builder::default().run(self)
    }
}

/// One language of a [`Catalog`].
#[derive(Clone, Debug)]
struct LangOut {
    code: String,
    variant: String,
    file: String,
    dir: Dir,
    rules: &'static PluralRules,
    numbers: NumberSymbols,
    font: Option<String>,
}

/// One key of a [`Catalog`].
#[derive(Clone, Debug)]
struct KeyOut {
    id: String,
    variant: String,
    args: Vec<String>,
}

/// Checked, compiled translations: every key in every language (fallbacks applied), ready to
/// be written as Rust ([`to_rust`](Catalog::to_rust),
/// [`write_to_out_dir`](Catalog::write_to_out_dir)).
#[derive(Clone, Debug)]
pub struct Catalog {
    langs: Vec<LangOut>,
    /// Index of the default language in `langs`.
    default: usize,
    keys: Vec<KeyOut>,
    /// `[language][key]`, resolved (no `.po` forms left).
    messages: Vec<Vec<Body>>,
    warnings: Vec<Diagnostic>,
    /// Files and directories read (for `cargo:rerun-if-changed`).
    inputs: Vec<PathBuf>,
    runtime: String,
}

impl Catalog {
    /// The language codes, in `Lang` order (the default first unless
    /// [`Translations::languages`] says otherwise).
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.langs.iter().map(|l| l.code.as_str())
    }

    /// The key identifiers, in `Key` order (sorted).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().map(|k| k.id.as_str())
    }

    /// The warnings (see the module docs); empty with [`Translations::strict`].
    #[must_use]
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// The files and directories that were read.
    #[must_use]
    pub fn inputs(&self) -> &[PathBuf] {
        &self.inputs
    }

    /// Every character of every text in every language (arguments excluded): the glyphs the
    /// application's fonts need for its translations.
    ///
    /// ```
    /// use twine_build::i18n::{Format, Translations};
    ///
    /// let c = Translations::new().source("en", Format::Toml, "a = \"Hi {n}\"").build().unwrap();
    /// assert_eq!(c.chars().into_iter().collect::<String>(), " Hi");
    /// ```
    ///
    /// Numbers formatted into messages are not included: see
    /// [`number_chars`](Catalog::number_chars). As a [`CharSource`](crate::chars::CharSource)
    /// (`Font::chars_from(&catalog)`), the catalog gives both.
    #[must_use]
    pub fn chars(&self) -> BTreeSet<char> {
        self.messages
            .iter()
            .flatten()
            .flat_map(Body::segments)
            .filter_map(|s| match s {
                Seg::Text(t) => Some(t.chars()),
                Seg::Arg(_) => None,
            })
            .flatten()
            .collect()
    }

    /// The characters every language writes numbers with: its ten digits (Arabic-Indic in
    /// Arabic, Extended Arabic-Indic in Persian, …), decimal and grouping separators, minus
    /// sign and percent affixes ([`NumberSymbols`]) — the glyphs numeric `tr!` arguments and
    /// `I18n::number` draw. Formatting marks (U+061C, U+200E, U+200F) are left out: they have
    /// no glyph.
    ///
    /// ```
    /// use twine_build::i18n::{Format, Translations};
    ///
    /// let c = Translations::new()
    ///     .source("en", Format::Toml, "a = \"Hi\"")
    ///     .source("de", Format::Toml, "a = \"Hallo\"")
    ///     .default_language("en")
    ///     .build()
    ///     .unwrap();
    /// assert_eq!(c.number_chars().into_iter().collect::<String>(), "%,-.0123456789\u{A0}");
    /// ```
    #[must_use]
    pub fn number_chars(&self) -> BTreeSet<char> {
        let mut set = BTreeSet::new();
        for n in self.langs.iter().map(|l| &l.numbers) {
            set.extend((0..10).filter_map(|d| char::from_u32(u32::from(n.zero) + d)));
            for s in [n.decimal, n.group, n.minus, n.percent_prefix, n.percent_suffix] {
                set.extend(
                    s.chars()
                        .filter(|c| !matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}')),
                );
            }
        }
        set
    }

    /// A readable form of the message `key` in language `lang` (`None` when either is unknown):
    /// the text with `{argument}` placeholders, a plural message as
    /// `{count -> [one] … | *[other] …}`.
    #[must_use]
    pub fn preview(&self, lang: &str, key: &str) -> Option<String> {
        let l = self.langs.iter().position(|l| l.code == lang)?;
        let k = self.keys.iter().position(|k| k.id == key)?;
        Some(preview(&self.messages[l][k]))
    }

    /// The generated Rust source (see the crate docs of `twine_extra::i18n` for what it
    /// contains).
    #[must_use]
    pub fn to_rust(&self) -> String {
        emit::rust(self)
    }

    /// Writes the generated Rust source to `path` (only when it changed, so dependents are not
    /// rebuilt needlessly).
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        let src = self.to_rust();
        if std::fs::read_to_string(path).ok().as_deref() == Some(src.as_str()) {
            return Ok(());
        }
        std::fs::write(path, src)
    }

    /// The build-script convenience: writes the catalog to `$OUT_DIR/<file_name>`, prints
    /// `cargo:rerun-if-changed` for every input and `cargo:warning` for every warning, and
    /// returns the path written.
    ///
    /// # Errors
    /// When `OUT_DIR` is not set (not in a build script) or the file cannot be written.
    pub fn write_to_out_dir(&self, file_name: &str) -> Result<PathBuf, Error> {
        let out = std::env::var_os("OUT_DIR")
            .ok_or_else(|| Error::config("OUT_DIR is not set: call write_to_out_dir from a build script"))?;
        let path = Path::new(&out).join(file_name);
        self.write(&path)
            .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
        for input in &self.inputs {
            println!("cargo:rerun-if-changed={}", input.display());
        }
        for w in &self.warnings {
            println!("cargo:warning={w}");
        }
        Ok(path)
    }
}

fn preview_segs(segs: &[Seg]) -> String {
    segs.iter()
        .map(|s| match s {
            Seg::Text(t) => t.clone(),
            Seg::Arg(a) => format!("{{{a}}}"),
        })
        .collect()
}

fn preview(body: &Body) -> String {
    match body {
        Body::Pattern(s) => preview_segs(s),
        Body::Plural {
            selector,
            variants,
            other,
        } => {
            let mut s = format!("{{{selector} ->");
            for (sel, segs) in variants {
                let key = match sel {
                    Sel::Exact(n) => n.to_string(),
                    Sel::Cat(c) => c.name().to_string(),
                };
                let _ = write!(s, " [{key}] {} |", preview_segs(segs));
            }
            let _ = write!(s, " *[other] {}}}", preview_segs(other));
            s
        }
        Body::Forms { forms, .. } => forms
            .iter()
            .map(|f| preview_segs(f))
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// A parsed source file.
struct Source {
    lang: String,
    file: String,
    entries: BTreeMap<String, Entry>,
    nplurals: Option<usize>,
}

#[derive(Default)]
struct Builder {
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    inputs: Vec<PathBuf>,
}

/// Languages written right to left (base language subtags) and RTL script subtags.
const RTL_LANGS: &[&str] = &[
    "ar", "ars", "ckb", "dv", "fa", "he", "iw", "ji", "ps", "sd", "syr", "ug", "ur", "yi",
];
const RTL_SCRIPTS: &[&str] = &["adlm", "arab", "hebr", "nkoo", "syrc", "thaa"];
const LTR_SCRIPTS: &[&str] = &["cyrl", "latn"];

fn default_dir(code: &str) -> Dir {
    let tag = code.to_ascii_lowercase().replace('_', "-");
    let mut parts = tag.split('-');
    let base = parts.next().unwrap_or("");
    for p in parts {
        if RTL_SCRIPTS.contains(&p) {
            return Dir::Rtl;
        }
        if LTR_SCRIPTS.contains(&p) {
            return Dir::Ltr;
        }
    }
    if RTL_LANGS.contains(&base) {
        Dir::Rtl
    } else {
        Dir::Ltr
    }
}

fn valid_code(code: &str) -> bool {
    let mut parts = code.split(['-', '_']);
    let base = parts.next().unwrap_or("");
    (2..=3).contains(&base.len())
        && base.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| !p.is_empty() && p.len() <= 8 && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

impl Builder {
    fn err(&mut self, file: &str, line: usize, message: impl Into<String>) {
        self.errors.push(diag(file, line, message));
    }

    fn warn(&mut self, strict: bool, d: Diagnostic) {
        if strict {
            self.errors.push(d);
        } else {
            self.warnings.push(d);
        }
    }

    /// Reads the inputs: `(lang, display name, format, text)`.
    fn read(&mut self, t: &Translations) -> Vec<(String, String, Format, String)> {
        let mut out = Vec::new();
        let read_file = |b: &mut Builder, path: &Path, out: &mut Vec<_>| {
            let name = path.display().to_string();
            let (Some(stem), Some(format)) = (
                path.file_stem().and_then(|s| s.to_str()),
                path.extension()
                    .and_then(|e| e.to_str())
                    .and_then(Format::from_extension),
            ) else {
                b.err(
                    &name,
                    0,
                    "not a translation file (expected `<language>.ftl`, `.po` or `.toml`)",
                );
                return;
            };
            b.inputs.push(path.to_path_buf());
            match std::fs::read_to_string(path) {
                Ok(text) => out.push((stem.to_string(), name, format, text)),
                Err(e) => b.err(&name, 0, format!("cannot read: {e}")),
            }
        };
        for input in &t.inputs {
            match input {
                Input::Dir(dir) => {
                    self.inputs.push(dir.clone());
                    let entries = match std::fs::read_dir(dir) {
                        Ok(e) => e,
                        Err(e) => {
                            self.err(
                                &dir.display().to_string(),
                                0,
                                format!("cannot read the directory: {e}"),
                            );
                            continue;
                        }
                    };
                    let mut paths: Vec<PathBuf> = entries
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| {
                            p.is_file()
                                && p.extension()
                                    .and_then(|e| e.to_str())
                                    .and_then(Format::from_extension)
                                    .is_some()
                        })
                        .collect();
                    paths.sort();
                    for p in paths {
                        read_file(self, &p, &mut out);
                    }
                }
                Input::File(path) => read_file(self, path, &mut out),
                Input::Text { lang, format, text } => {
                    let ext = match format {
                        Format::Fluent => "ftl",
                        Format::Po => "po",
                        Format::Toml => "toml",
                    };
                    out.push((lang.clone(), format!("<{lang}.{ext}>"), *format, text.clone()));
                }
            }
        }
        out
    }

    fn parse(&mut self, lang: String, file: String, format: Format, text: &str) -> Option<Source> {
        if !valid_code(&lang) {
            self.err(
                &file,
                0,
                format!("`{lang}` is not a language code (such as `en`, `pt-BR`, `zh_Hant`)"),
            );
            return None;
        }
        let (list, nplurals) = match format {
            Format::Fluent => (ftl::parse(text), None),
            Format::Toml => (toml_src::parse(text), None),
            Format::Po => match po::parse(text) {
                Ok(p) => {
                    if !p.fuzzy.is_empty() {
                        self.warnings.push(diag(
                            &file,
                            0,
                            format!("fuzzy entries skipped: {}", p.fuzzy.join(", ")),
                        ));
                    }
                    (Ok(p.entries), p.nplurals)
                }
                Err(e) => (Err(e), None),
            },
        };
        let list = match list {
            Ok(l) => l,
            Err(errors) => {
                for (line, m) in errors {
                    self.err(&file, line, m);
                }
                return None;
            }
        };
        let mut entries = BTreeMap::new();
        for (id, e) in list {
            let line = e.line;
            if entries.insert(id.clone(), e).is_some() {
                self.err(&file, line, format!("`{id}` is defined twice"));
            }
        }
        Some(Source {
            lang,
            file,
            entries,
            nplurals,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn run(mut self, t: &Translations) -> Result<Catalog, Error> {
        // 1. Read and parse.
        let mut sources: Vec<Source> = Vec::new();
        for (lang, file, format, text) in self.read(t) {
            if let Some(prev) = sources.iter().find(|s| s.lang == lang) {
                let msg = format!("a second file for language `{lang}` (the first is {})", prev.file);
                self.err(&file, 0, msg);
                continue;
            }
            if let Some(s) = self.parse(lang, file, format, &text) {
                sources.push(s);
            }
        }
        if sources.is_empty() && self.errors.is_empty() {
            self.err("", 0, "no translation files found");
        }
        if !self.errors.is_empty() {
            return Err(Error {
                diagnostics: self.errors,
            });
        }

        // 2. Languages and the default.
        let default = match (&t.default, &t.languages) {
            (Some(d), _) => d.clone(),
            (None, Some(l)) if !l.is_empty() => l[0].clone(),
            (None, _) if sources.len() == 1 => sources[0].lang.clone(),
            _ => {
                return Err(Error::config(format!(
                    "several languages ({}): choose the default language",
                    sources
                        .iter()
                        .map(|s| s.lang.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };
        let order: Vec<String> = if let Some(list) = &t.languages {
            for code in list {
                if !sources.iter().any(|s| &s.lang == code) {
                    self.err(
                        "",
                        0,
                        format!("language `{code}` is listed but has no translation file"),
                    );
                }
            }
            if !list.contains(&default) {
                self.err(
                    "",
                    0,
                    format!("the default language `{default}` is not in the language list"),
                );
            }
            list.clone()
        } else {
            let mut v: Vec<String> = sources.iter().map(|s| s.lang.clone()).collect();
            v.sort();
            v.retain(|c| *c != default);
            v.insert(0, default.clone());
            v
        };
        if !sources.iter().any(|s| s.lang == default) {
            self.err(
                "",
                0,
                format!("the default language `{default}` has no translation file"),
            );
        }
        for (what, map) in [
            ("font", &t.fonts),
            ("plural rules", &t.plural),
            ("fallback", &t.fallbacks),
        ] {
            for code in map.keys() {
                if !order.contains(code) {
                    self.err("", 0, format!("{what} for `{code}`, which is not compiled"));
                }
            }
        }
        for code in t.dirs.keys().chain(t.numbers.keys()) {
            if !order.contains(code) {
                self.err(
                    "",
                    0,
                    format!("base direction or number symbols for `{code}`, which is not compiled"),
                );
            }
        }
        if !self.errors.is_empty() {
            return Err(Error {
                diagnostics: self.errors,
            });
        }
        sources.retain(|s| order.contains(&s.lang));
        sources.sort_by_key(|s| order.iter().position(|c| *c == s.lang));

        // 3. Per-language settings.
        let mut langs = Vec::new();
        let mut variants = BTreeMap::new();
        for s in &sources {
            let rules = match t.plural.get(&s.lang) {
                Some(name) => rules_named(name).ok_or_else(|| {
                    format!(
                        "unknown plural rules `{name}` (known: {})",
                        RULES.iter().map(|r| r.name).collect::<Vec<_>>().join(", ")
                    )
                }),
                None => rules_for(&s.lang).ok_or_else(|| {
                    format!(
                        "no CLDR plural rule is known for `{}`: choose one with `plural_rules(\"{}\", …)` ({})",
                        s.lang,
                        s.lang,
                        RULES.iter().map(|r| r.name).collect::<Vec<_>>().join(", ")
                    )
                }),
            };
            let rules = match rules {
                Ok(r) => r,
                Err(m) => {
                    self.err(&s.file, 0, m);
                    continue;
                }
            };
            let known = t.numbers.get(&s.lang).copied().or_else(|| numbers_for(&s.lang));
            let numbers = if let Some(n) = known {
                n
            } else {
                let d = diag(
                    &s.file,
                    0,
                    format!(
                        "no CLDR number symbols are known for `{}`: numbers use CLDR root \
                         (`1,234.5`); set them with `number_symbols(\"{}\", …)`",
                        s.lang, s.lang
                    ),
                );
                self.warn(t.strict, d);
                number::ROOT
            };
            let variant = match camel(&s.lang) {
                Ok(v) => v,
                Err(m) => {
                    self.err(&s.file, 0, m);
                    continue;
                }
            };
            if let Some(other) = variants.insert(variant.clone(), s.lang.clone()) {
                self.err(
                    &s.file,
                    0,
                    format!("`{}` and `{other}` both become `Lang::{variant}`", s.lang),
                );
            }
            langs.push(LangOut {
                code: s.lang.clone(),
                variant,
                file: s.file.clone(),
                dir: t
                    .dirs
                    .get(&s.lang)
                    .copied()
                    .unwrap_or_else(|| default_dir(&s.lang)),
                rules,
                numbers,
                font: t.fonts.get(&s.lang).cloned(),
            });
        }
        if !self.errors.is_empty() {
            return Err(Error {
                diagnostics: self.errors,
            });
        }

        // 4. `.po` plural forms → categories.
        for (s, l) in sources.iter_mut().zip(&langs) {
            for (id, e) in &mut s.entries {
                if let Body::Forms { selector, forms } = &e.body {
                    match resolve_forms(selector.as_deref(), forms, l.rules, &l.code, s.nplurals) {
                        Ok(b) => e.body = b,
                        Err(m) => self.errors.push(diag(&s.file, e.line, format!("`{id}`: {m}"))),
                    }
                }
            }
        }

        // 5. Keys (from the default language) and their arguments.
        let def_idx = sources.iter().position(|s| s.lang == default).unwrap_or(0);
        let def = &sources[def_idx];
        let mut keys = Vec::new();
        let mut key_variants = BTreeMap::new();
        for (id, e) in &def.entries {
            let args = e.body.args();
            let positional: Vec<u32> = args.iter().filter_map(|a| a.parse().ok()).collect();
            if positional.iter().enumerate().any(|(i, p)| *p as usize != i) {
                self.errors.push(diag(
                    &def.file,
                    e.line,
                    format!("`{id}`: positional arguments must be {{0}}, {{1}}, … without gaps"),
                ));
            }
            if args.len() > MAX_ARGS {
                self.errors.push(diag(
                    &def.file,
                    e.line,
                    format!(
                        "`{id}`: {} arguments, at most {MAX_ARGS} are supported",
                        args.len()
                    ),
                ));
            }
            match camel(id) {
                Ok(v) => {
                    if let Some(other) = key_variants.insert(v.clone(), id.clone()) {
                        self.errors.push(diag(
                            &def.file,
                            e.line,
                            format!("`{id}` and `{other}` both become `Key::{v}`"),
                        ));
                    }
                    keys.push(KeyOut {
                        id: id.clone(),
                        variant: v,
                        args,
                    });
                }
                Err(m) => self.errors.push(diag(&def.file, e.line, m)),
            }
        }
        if keys.len() > usize::from(u16::MAX) {
            self.err(&def.file, 0, "more than 65535 keys");
        }

        // 6. Check the other languages against the default.
        for (s, l) in sources.iter().zip(&langs) {
            for (id, e) in &s.entries {
                let Some(k) = keys.iter().find(|k| k.id == *id) else {
                    if s.lang != def.lang {
                        let d = diag(
                            &s.file,
                            e.line,
                            format!(
                                "`{id}` is not a key of the default language `{}` (ignored)",
                                def.lang
                            ),
                        );
                        self.warn(t.strict, d);
                    }
                    continue;
                };
                for a in e.body.args() {
                    if !k.args.contains(&a) {
                        self.errors.push(diag(
                            &s.file,
                            e.line,
                            format!(
                                "`{id}` uses the argument `{a}`, which the default language's message does not have ({})",
                                if k.args.is_empty() { "none".to_string() } else { k.args.join(", ") }
                            ),
                        ));
                    }
                }
                if let Body::Plural { variants, .. } = &e.body {
                    for (sel, _) in variants {
                        if let Sel::Cat(c) = sel {
                            if !l.rules.all.contains(c) {
                                let d = diag(
                                    &s.file,
                                    e.line,
                                    format!(
                                        "`{id}`: `{c}` is never chosen in `{}` (its categories: {})",
                                        l.code,
                                        l.rules
                                            .all
                                            .iter()
                                            .map(|c| c.name())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    ),
                                );
                                self.warn(t.strict, d);
                            }
                        }
                    }
                }
            }
        }
        // 7. Fallbacks: every key in every language.
        let mut messages = Vec::new();
        for (li, s) in sources.iter().enumerate() {
            let chain = fallback_chain(li, def_idx, &langs, &t.fallbacks);
            let mut row = Vec::new();
            let mut missing = Vec::new();
            for k in &keys {
                let found = chain
                    .iter()
                    .find_map(|&ci| sources[ci].entries.get(&k.id))
                    .map(|e| e.body.clone());
                if !s.entries.contains_key(&k.id) {
                    missing.push(k.id.clone());
                }
                // The default language has every key, so the chain always ends with a text.
                row.push(found.unwrap_or_else(|| Body::Pattern(Vec::new())));
            }
            if !missing.is_empty() {
                let to: Vec<&str> = chain[1..].iter().map(|&i| langs[i].code.as_str()).collect();
                let d = diag(
                    &s.file,
                    0,
                    format!(
                        "{} missing translation(s), shown in {}: {}",
                        missing.len(),
                        to.join(" or "),
                        missing.join(", ")
                    ),
                );
                self.warn(t.strict, d);
            }
            messages.push(row);
        }
        if !self.errors.is_empty() {
            return Err(Error {
                diagnostics: self.errors,
            });
        }
        Ok(Catalog {
            langs,
            default: def_idx,
            keys,
            messages,
            warnings: self.warnings,
            inputs: self.inputs,
            runtime: t.runtime.clone(),
        })
    }
}

/// Language `li` itself, its explicit fallbacks, its base language, then the default.
fn fallback_chain(
    li: usize,
    default: usize,
    langs: &[LangOut],
    fallbacks: &BTreeMap<String, String>,
) -> Vec<usize> {
    let index = |code: &str| langs.iter().position(|l| l.code == code);
    let mut chain = vec![li];
    let mut cur = li;
    loop {
        let code = &langs[cur].code;
        let next = fallbacks.get(code).and_then(|f| index(f)).or_else(|| {
            let base = code.split(['-', '_']).next().unwrap_or(code);
            (base != code).then(|| index(base)).flatten()
        });
        match next {
            Some(n) if !chain.contains(&n) => {
                chain.push(n);
                cur = n;
            }
            _ => break,
        }
    }
    if !chain.contains(&default) {
        chain.push(default);
    }
    chain
}

/// Maps `.po` plural forms to the language's categories.
fn resolve_forms(
    selector: Option<&str>,
    forms: &[Vec<Seg>],
    rules: &PluralRules,
    code: &str,
    nplurals: Option<usize>,
) -> Result<Body, String> {
    let cats = if forms.len() == rules.all.len() {
        rules.all
    } else if forms.len() == rules.integer.len() {
        rules.integer
    } else {
        let names = |c: &[Category]| c.iter().map(|c| c.name()).collect::<Vec<_>>().join(", ");
        return Err(format!(
            "{} plural forms, but `{code}` has {} ({}) or {} for integers ({})",
            forms.len(),
            rules.all.len(),
            names(rules.all),
            rules.integer.len(),
            names(rules.integer),
        ));
    };
    if let Some(n) = nplurals {
        if n != forms.len() {
            return Err(format!(
                "{} plural forms, but the header says nplurals={n}",
                forms.len()
            ));
        }
    }
    let mut variants = Vec::new();
    let mut other = None;
    for (c, f) in cats.iter().zip(forms) {
        if *c == Category::Other {
            other = Some(f.clone());
        } else {
            variants.push((Sel::Cat(*c), f.clone()));
        }
    }
    // Integer-only forms without `other` (e.g. Russian: one, few, many): decimals take the
    // last form.
    let other = other.unwrap_or_else(|| forms.last().cloned().unwrap_or_default());
    let probe = Body::Plural {
        selector: String::new(),
        variants: variants.clone(),
        other: concat(&[&other]),
    };
    let args: Vec<String> = probe.args().into_iter().filter(|a| !a.is_empty()).collect();
    let selector = match (selector, args.as_slice()) {
        (Some(s), _) => s.to_string(),
        (None, [one]) => one.clone(),
        (None, []) => return Err("a plural entry needs an argument to choose on".to_string()),
        (None, args) => {
            return Err(format!(
                "the forms use several arguments ({}): name the one to choose on with `#. selector: …`",
                args.join(", ")
            ));
        }
    };
    Ok(Body::Plural {
        selector,
        variants,
        other,
    })
}

#[cfg(test)]
mod tests;
