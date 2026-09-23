//! Library behind the `twine` command-line tool: the font generator (and, in the image module,
//! the image converter). `cargo xtask` uses these functions in-process.
//!
//! ```no_run
//! use std::path::Path;
//! use twine_cli::font::{FontOptions, Symbols, SubpxArg, generate};
//!
//! let opts = FontOptions {
//!     ttf: "assets/fonts/Montserrat-Medium.ttf".into(),
//!     size: 14,
//!     bpp: 4,
//!     ranges: vec!["0x20-0x7E".into()],
//!     symbols: Symbols::None,
//!     symbols_ttf: Vec::new(),
//!     compress: false,
//!     prefilter: true,
//!     kerning: true,
//!     subpx: SubpxArg::None,
//!     name: "MY_FONT".into(),
//!     out: "src/my_font.rs".into(),
//! };
//! let g = generate(&opts, Path::new(".")).unwrap();
//! std::fs::write(&opts.out, g.source).unwrap();
//! ```

pub mod font;
pub mod image;
