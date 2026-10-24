//! # twine-build
//!
//! Build-script helpers for Twine applications: assets are compiled on the host, in `build.rs`,
//! into Rust sources in `OUT_DIR`, so applications commit their sources (translation files,
//! TrueType fonts, PNG images) and never generated code. Nothing of this crate is linked into
//! the firmware.
//!
//! | Concept | Items |
//! |---------|-------|
//! | Compile-time translations | [`i18n::Translations`] (sources and options) → [`i18n::Catalog`] (checked; [`to_rust`](i18n::Catalog::to_rust), [`write_to_out_dir`](i18n::Catalog::write_to_out_dir), [`chars`](i18n::Catalog::chars), [`number_chars`](i18n::Catalog::number_chars)), [`i18n::Format`] (Fluent, gettext `.po`, TOML), [`i18n::Diagnostic`] / [`i18n::Error`], CLDR plural rules ([`i18n::RULES`]) |
//! | Bitmap fonts (feature `font`) | `font::Font` (TrueType/OpenType file, size, `font::Bpp`, characters, symbols, compression, kerning, subpixels, `font::Hinting`, text `font::Gamma`) → `font::GeneratedFont` (`write_to_out_dir`), `font::describe` |
//! | Glyph subsetting | [`chars::CharSource`] (typed ranges `'\u{20}'..='\u{7E}'`, strings, characters, translation catalogs with their numbers' characters), [`chars::rust_literal_chars`] (the string and character literals of Rust sources), [`chars::ranges`], `font::implied_chars` (the Arabic presentation forms and mirrored brackets a font must add) |
//! | Images (feature `image`) | `image::Image` (file or RGBA pixels, `ColorFormat`, `image::Compression`, dithering, premultiplied alpha, background) → `image::GeneratedImage` (`write_to_out_dir`, `to_tbin`) |
//! | Errors | [`AssetError`] (fonts and images) |
//!
//! The runtime side of the translations is `twine_extra::i18n` (feature `i18n`): the
//! generated `Key` and `Lang` enums implement its `MessageKey` and `Language` traits, and its
//! `tr!` macro shows a message in the current language. Generated fonts are
//! `twine::text::Font` statics and generated images `twine::image::Image` statics.
//!
//! ```toml
//! # Cargo.toml of the application
//! [build-dependencies]
//! twine-build = "0.1"
//! ```
//!
//! A build script that compiles the translations, a font holding exactly the characters they
//! (and the application's string literals) use, and an image:
//!
//! ```no_run
//! # #[cfg(all(feature = "font", feature = "image"))]
//! # fn main() {
//! // build.rs
//! use twine_build::font::{Bpp, Font};
//! use twine_build::i18n::Translations;
//! use twine_build::image::{ColorFormat, Image};
//!
//! let catalog = Translations::from_dir("i18n").default_language("en").build().unwrap();
//! catalog.write_to_out_dir("i18n.rs").unwrap();
//!
//! Font::from_ttf("assets/NotoSansSC-Regular.otf")
//!     .size(16)
//!     .bpp(Bpp::Four)
//!     .chars_from('\u{20}'..='\u{7E}')
//!     .chars_from(&catalog)
//!     .chars_from_rust_sources("src")
//!     .name("TEXT_FONT")
//!     .build()
//!     .unwrap()
//!     .write_to_out_dir("text_font.rs")
//!     .unwrap();
//!
//! Image::from_file("assets/logo.png")
//!     .format(ColorFormat::Rgb565)
//!     .name("LOGO")
//!     .build()
//!     .unwrap()
//!     .write_to_out_dir("logo.rs")
//!     .unwrap();
//! # }
//! # #[cfg(not(all(feature = "font", feature = "image")))]
//! # fn main() {}
//! ```
//!
//! and the application includes the generated statics:
//!
//! ```ignore
//! include!(concat!(env!("OUT_DIR"), "/text_font.rs")); // pub static TEXT_FONT: twine::text::Font
//! include!(concat!(env!("OUT_DIR"), "/logo.rs"));      // pub static LOGO: twine::image::Image
//! ```
//!
//! The command-line tool `twine` (crate `twine-cli`: `twine font`, `twine image`,
//! `twine i18n`) is a front end of the same builders, for files that are committed (the
//! built-in fonts of `twine-assets`). The `include_font!` and `include_image!` macros of the
//! `twine` facade (feature `include-assets`) run them at compile time without a build script.
#![forbid(unsafe_code)]

use std::fmt;
#[cfg(any(feature = "font", feature = "image"))]
use std::path::{Path, PathBuf};

pub mod chars;
#[cfg(feature = "font")]
pub mod font;
pub mod i18n;
#[cfg(feature = "image")]
pub mod image;
#[cfg(any(feature = "font", feature = "image"))]
mod sha256;

#[cfg(any(feature = "font", feature = "image"))]
pub use sha256::sha256_hex;

/// A font or image could not be generated: an invalid option, an unreadable or undecodable
/// input file, or an output that cannot be written.
///
/// ```
/// use twine_build::AssetError;
///
/// let e = AssetError::new("size must be 1..=255 px (got 0)");
/// assert_eq!(e.to_string(), "size must be 1..=255 px (got 0)");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetError {
    message: String,
}

impl AssetError {
    /// An error with a human-readable `message`.
    #[must_use]
    pub fn new(message: impl Into<String>) -> AssetError {
        AssetError {
            message: message.into(),
        }
    }

    /// An error about the file `path`.
    #[cfg(any(feature = "font", feature = "image"))]
    #[must_use]
    pub fn file(path: &Path, message: impl fmt::Display) -> AssetError {
        AssetError::new(format!("{}: {message}", path.display()))
    }

    /// The message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AssetError {}

impl From<String> for AssetError {
    fn from(message: String) -> AssetError {
        AssetError { message }
    }
}

impl From<&str> for AssetError {
    fn from(message: &str) -> AssetError {
        AssetError::new(message)
    }
}

/// `$OUT_DIR`, or an error naming the method that needs it.
#[cfg(any(feature = "font", feature = "image"))]
fn out_dir(method: &str) -> Result<PathBuf, AssetError> {
    std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| AssetError::new(format!("OUT_DIR is not set: call {method} from a build script")))
}

/// Writes `bytes` to `path` unless the file already holds them (so dependents are not rebuilt
/// needlessly), creating the parent directory.
#[cfg(any(feature = "font", feature = "image"))]
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), AssetError> {
    if std::fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| AssetError::file(dir, e))?;
    }
    std::fs::write(path, bytes).map_err(|e| AssetError::file(path, e))
}

/// A Rust identifier in `SCREAMING_SNAKE_CASE` made of `parts` (`["Montserrat-Medium", "16"]`
/// → `MONTSERRAT_MEDIUM_16`): letters and digits upper-cased, everything else one `_`, a
/// leading digit prefixed with `_`.
#[cfg(any(feature = "font", feature = "image"))]
fn static_name(parts: &[&str]) -> String {
    let mut s = String::new();
    for c in parts.join("_").chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_uppercase());
        } else if !s.is_empty() && !s.ends_with('_') {
            s.push('_');
        }
    }
    while s.ends_with('_') {
        s.pop();
    }
    if s.is_empty() {
        s.push_str("UNNAMED");
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// Whether `s` is a Rust identifier (ASCII; not `_`).
#[cfg(any(feature = "font", feature = "image"))]
fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    s != "_"
        && c.next().is_some_and(|f| f == '_' || f.is_ascii_alphabetic())
        && c.all(|x| x == '_' || x.is_ascii_alphanumeric())
}

/// Whether `s` is a Rust path (`twine::text`, `::twine_text`, `crate::fonts`).
#[cfg(any(feature = "font", feature = "image"))]
fn is_path(s: &str) -> bool {
    let s = s.strip_prefix("::").unwrap_or(s);
    !s.is_empty() && s.split("::").all(is_ident)
}

/// Whether `s` is a visibility the generated statics may carry (`pub`, `pub(crate)`,
/// `pub(super)`, `pub(in path)`, or empty for private).
#[cfg(any(feature = "font", feature = "image"))]
fn is_visibility(s: &str) -> bool {
    match s {
        "" | "pub" | "pub(crate)" | "pub(super)" | "pub(self)" => true,
        _ => s
            .strip_prefix("pub(in ")
            .and_then(|r| r.strip_suffix(')'))
            .is_some_and(is_path),
    }
}

#[cfg(all(test, any(feature = "font", feature = "image")))]
mod tests {
    use super::*;

    #[test]
    fn static_names() {
        assert_eq!(static_name(&["Montserrat-Medium", "16"]), "MONTSERRAT_MEDIUM_16");
        assert_eq!(static_name(&["8bit font"]), "_8BIT_FONT");
        assert_eq!(static_name(&["--"]), "UNNAMED");
        assert_eq!(static_name(&["logo"]), "LOGO");
    }

    #[test]
    fn identifiers_paths_and_visibilities() {
        assert!(is_ident("MONTSERRAT_14"));
        assert!(!is_ident("14"));
        assert!(!is_ident("A-B"));
        assert!(!is_ident("_"));
        assert!(is_path("twine::text"));
        assert!(is_path("::twine_text"));
        assert!(!is_path("twine::"));
        assert!(!is_path("twine text"));
        assert!(is_visibility("pub(crate)"));
        assert!(is_visibility(""));
        assert!(is_visibility("pub(in crate::ui)"));
        assert!(!is_visibility("pub(foo)"));
    }
}
