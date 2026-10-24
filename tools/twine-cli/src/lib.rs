//! Library behind the `twine` command-line tool: the font generator (module [`font`]), the
//! image converter (module [`image`]) and the translation compiler (module [`i18n`]) — the
//! command-line front ends of `twine_build::font::Font`, `twine_build::image::Image` and
//! `twine_build::i18n` (which build scripts use directly). `cargo xtask` uses the argument
//! parsers in-process.
//!
//! Text options are parsed into typed values when the command line is read: bit depths into
//! `twine_text::Bpp`, `--range` specs into typed character ranges, formats into
//! `ColorFormat`.
//!
//! ```
//! use twine_cli::font::parse_range;
//!
//! let digits = parse_range("0x30-0x39").unwrap();
//! assert_eq!(digits.0, ['0'..='9']);
//! ```
#![forbid(unsafe_code)]

pub mod font;
pub mod i18n;
pub mod image;
