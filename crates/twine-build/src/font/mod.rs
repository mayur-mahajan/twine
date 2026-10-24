//! Bitmap fonts from TrueType/OpenType files (feature `font`): a deterministic generator that
//! writes a `static` `twine::text::Font` as Rust source.
//!
//! [`Font`] is a builder: the source file, the pixel size, the bit depth ([`Bpp`]), the
//! characters ([`chars_from`](Font::chars_from) — typed ranges, strings, translation
//! catalogs — and the literals of the application's sources), optional Font Awesome symbols,
//! compression, kerning, subpixel rendering, [`Hinting`] and text [`Gamma`]. [`Font::build`] rasterizes the glyphs and
//! returns a [`GeneratedFont`], whose [`write_to_out_dir`](GeneratedFont::write_to_out_dir)
//! writes it for `include!` from a build script.
//!
//! ```no_run
//! // build.rs
//! use twine_build::font::{Bpp, Font};
//!
//! Font::from_ttf("assets/Montserrat-Medium.ttf")
//!     .size(16)
//!     .bpp(Bpp::Four)
//!     .chars_from('\u{20}'..='\u{7E}')
//!     .chars_from("°•")
//!     .name("BODY_FONT")
//!     .build()
//!     .unwrap_or_else(|e| panic!("{e}"))
//!     .write_to_out_dir("body_font.rs")
//!     .unwrap_or_else(|e| panic!("{e}"));
//! ```
//!
//! ```ignore
//! // src/main.rs
//! include!(concat!(env!("OUT_DIR"), "/body_font.rs")); // pub static BODY_FONT: twine::text::Font
//! ```
//!
//! # How a font is generated
//!
//! The outlines are grid-fitted with the `FreeType` auto-hinter (`skrifa`; [`Hinting::Full`] by
//! default, see [`Hinting`]) and rasterized (`ab_glyph_rasterizer`; unhinted glyphs with
//! `fontdue`), trimmed to their inked box and quantized to the bit depth
//! (`q = (v × max + 127) / 255`). The font carries text-gamma tables ([`Gamma`], default
//! γ = 1.2 dark-on-light / 1.8 light-on-dark) that map the stored levels to opacity when text
//! is drawn. Kerning comes from the font's `GPOS` pair
//! adjustments (or its legacy `kern` table), and bitmaps are optionally compressed with LVGL's
//! RLE scheme. Metrics: `line_height = ceil(ascent) + ceil(-descent)`,
//! `base_line = ceil(-descent)` (hhea), underline from the `post` table (fallback
//! `-(descent / 2)` and `max(1, size / 14)`), advances in 1/16 px. The output depends only on
//! the inputs, so regenerated files diff cleanly; its header records the source file's SHA-256,
//! the equivalent `twine font` command line, the hinting mode with the rasterizer versions and
//! the text gamma.
//!
//! Control characters are never glyphs: a range that includes U+007F skips it silently. A
//! character the font lacks is skipped with a warning ([`GeneratedFont::warnings`]).
//!
//! The forms the text pipeline draws in place of the characters asked for are added when the
//! font has them ([`implied_chars`]): the presentation forms and lam-alef ligatures of Arabic
//! and Persian letters (contextual shaping) and the partner of each mirrored bracket
//! (right-to-left runs). A font subset from translations therefore draws shaped Arabic.

#![allow(clippy::cast_precision_loss)] // font sizes and advances are small

mod emit;
mod gpos;
mod raster;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use twine_text::encode::{CmapData, FontData, KernData, encode_glyph};
pub use twine_text::{BitmapFormat, Bpp, Subpx};
use twine_text::{CmapKind, GlyphDsc};

use crate::chars::{CharSource, ranges, rust_source_chars};
use crate::{
    AssetError, is_ident, is_path, is_visibility, out_dir, sha256_hex, static_name, write_if_changed,
};
use emit::FontEmit;
pub use raster::Hinting;
use raster::{Cov, Face, HINTED_RASTERIZER, Rasterizer, UNHINTED_RASTERIZER, quantize};

/// The lam-alef ligatures the Arabic shaper draws for lam (U+0644) followed by an alef
/// variant: `(alef, isolated ligature)`; the final form is the next code point
/// (`twine::text::shape_into`).
const LAM_ALEF: [(char, char); 4] = [
    ('\u{0622}', '\u{FEF5}'),
    ('\u{0623}', '\u{FEF7}'),
    ('\u{0625}', '\u{FEF9}'),
    ('\u{0627}', '\u{FEFB}'),
];

/// The characters the text pipeline may draw for text made of `set`, besides `set` itself
/// (and including it): the contextual presentation forms of each Arabic and Persian letter and
/// the lam-alef ligatures (feature `arabic-shaping` of the runtime shapes text with
/// [`twine_text::shape_into`]), and the mirrored partner of each bracket (feature `bidi`
/// mirrors brackets in right-to-left runs, [`twine_text::mirror`]).
///
/// A font subset from the application's texts must contain them, or shaped Arabic and mirrored
/// brackets would be missing; [`Font::build`] adds those the font has.
///
/// ```
/// use std::collections::BTreeSet;
/// use twine_build::font::implied_chars;
///
/// // Beh (U+0628): isolated, final, initial and medial forms; "(" brings ")".
/// let set: BTreeSet<char> = "\u{0628}(".chars().collect();
/// let all: String = implied_chars(&set).into_iter().collect();
/// assert_eq!(all, "()\u{0628}\u{FE8F}\u{FE90}\u{FE91}\u{FE92}");
/// ```
#[must_use]
pub fn implied_chars(set: &BTreeSet<char>) -> BTreeSet<char> {
    let mut out = set.clone();
    for &c in set {
        out.insert(twine_text::mirror(c));
        let base = u32::from(c);
        if let Some(a) = twine_text::AP_CHARS_MAP
            .iter()
            .find(|a| base == 0x0622 + u32::from(a.offset))
        {
            let end = i32::from(a.end);
            for ofs in [0, a.beginning, a.middle, a.isolated] {
                if let Some(form) = u32::try_from(end + i32::from(ofs)).ok().and_then(char::from_u32) {
                    out.insert(form);
                }
            }
        }
    }
    if set.contains(&'\u{0644}') {
        for (alef, lig) in LAM_ALEF {
            if set.contains(&alef) {
                out.insert(lig);
                out.extend(char::from_u32(u32::from(lig) + 1));
            }
        }
    }
    out
}

/// Symbols that Font Awesome Free 5 does not contain, drawn from another glyph turned 90°
/// clockwise: `NEW_LINE` (U+F8A2, a custom glyph in LVGL's merged font) uses "level-down-alt"
/// (U+F3BE), which turned clockwise is a "↵" return arrow.
pub const SYMBOL_ALIASES: &[(char, char)] = &[('\u{F8A2}', '\u{F3BE}')];

/// The crate path the `twine font` command line uses unless told otherwise (`--crate-path`).
const CLI_CRATE_PATH: &str = "twine_text";

/// The default pixel size of [`Font`]: 14 px, the size of Twine's default font.
pub const DEFAULT_SIZE: u16 = 14;

/// The text gamma of a generated font: the strength `γ` of its coverage → opacity curve for
/// dark-on-light and for light-on-dark text, in hundredths (`120` = 1.2). The font stores one
/// 16-entry table per curve, `round(255 · (v / max)^(1 / γ))` (identity at both ends,
/// monotonic); the text renderer picks one per text run from the text and background colors
/// (`twine::text::gamma`), so it costs nothing per pixel.
///
/// `γ = 1` is LVGL's linear mapping; `γ > 1` makes partially covered pixels more opaque
/// (heavier text), `γ < 1` lighter. [`DEFAULT`](Gamma::DEFAULT) (1.2 / 1.8) and
/// [`LINEAR`](Gamma::LINEAR) reference shared tables of the runtime crate; other strengths
/// write their tables into the generated file (40 bytes).
///
/// ```
/// use twine_build::font::Gamma;
///
/// assert_eq!(Gamma::default(), Gamma::DEFAULT);
/// let g = Gamma::new(130, 200);
/// assert_eq!((g.dark_on_light(), g.light_on_dark()), (130, 200));
/// assert_eq!("1.2,1.8".parse::<Gamma>(), Ok(Gamma::DEFAULT));
/// assert_eq!("linear".parse::<Gamma>(), Ok(Gamma::LINEAR));
/// assert_eq!(Gamma::DEFAULT.tables(), twine_text::gamma::DEFAULT);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Gamma {
    dark_on_light: u16,
    light_on_dark: u16,
}

impl Default for Gamma {
    /// [`Gamma::DEFAULT`].
    fn default() -> Gamma {
        Gamma::DEFAULT
    }
}

impl Gamma {
    /// No gamma: LVGL's linear mapping (4 bpp `v × 17`) for both polarities.
    pub const LINEAR: Gamma = Gamma::new(100, 100);
    /// The default: γ = 1.2 for dark-on-light text (sRGB blending already makes it heavy) and
    /// 1.8 for light-on-dark text (thin without it). Values settled with the prototype
    /// comparison and the regenerated golden images (R6.S03).
    pub const DEFAULT: Gamma = Gamma::new(120, 180);
    /// The smallest strength [`Font::build`] accepts (0.5).
    pub const MIN: u16 = 50;
    /// The largest strength [`Font::build`] accepts (4.0).
    pub const MAX: u16 = 400;

    /// Curves of strength `dark_on_light / 100` and `light_on_dark / 100` (checked against
    /// [`MIN`](Gamma::MIN)..=[`MAX`](Gamma::MAX) by [`Font::build`]).
    #[must_use]
    pub const fn new(dark_on_light: u16, light_on_dark: u16) -> Gamma {
        Gamma {
            dark_on_light,
            light_on_dark,
        }
    }

    /// The dark-on-light strength in hundredths.
    #[must_use]
    pub const fn dark_on_light(self) -> u16 {
        self.dark_on_light
    }

    /// The light-on-dark strength in hundredths.
    #[must_use]
    pub const fn light_on_dark(self) -> u16 {
        self.light_on_dark
    }

    /// The 16-entry table of strength `centi / 100`: `round(255 · (v / 15)^(100 / centi))`.
    #[must_use]
    pub fn table(centi: u16) -> [u8; 16] {
        let mut t = [0u8; 16];
        let inv = 100.0 / f64::from(centi.max(1));
        for (v, e) in t.iter_mut().enumerate() {
            *e = (255.0 * (v as f64 / 15.0).powf(inv)).round().clamp(0.0, 255.0) as u8;
        }
        t
    }

    /// The runtime tables of these curves.
    #[must_use]
    pub fn tables(self) -> twine_text::TextGamma {
        twine_text::TextGamma {
            dark_on_light: twine_text::GammaRamp::new(Gamma::table(self.dark_on_light)),
            light_on_dark: twine_text::GammaRamp::new(Gamma::table(self.light_on_dark)),
        }
    }

    /// The `twine font --gamma` form: `linear`, `default`, or `<dark-on-light>,<light-on-dark>`
    /// with two decimals (`1.30,2.00`).
    #[must_use]
    pub fn arg(self) -> String {
        if self == Gamma::LINEAR {
            "linear".into()
        } else if self == Gamma::DEFAULT {
            "default".into()
        } else {
            format!(
                "{},{}",
                centi_str(self.dark_on_light),
                centi_str(self.light_on_dark)
            )
        }
    }

    fn check(self) -> Result<(), AssetError> {
        for v in [self.dark_on_light, self.light_on_dark] {
            if !(Gamma::MIN..=Gamma::MAX).contains(&v) {
                return Err(format!("text gamma {} is outside 0.50..=4.00", centi_str(v)).into());
            }
        }
        Ok(())
    }
}

/// `120` → `"1.20"`.
fn centi_str(v: u16) -> String {
    format!("{}.{:02}", v / 100, v % 100)
}

impl std::str::FromStr for Gamma {
    type Err = String;

    /// Parses `linear`, `default` or `<dark-on-light>,<light-on-dark>` strengths with up to two
    /// decimals (`1.2,1.8`).
    fn from_str(s: &str) -> Result<Gamma, String> {
        match s {
            "linear" => return Ok(Gamma::LINEAR),
            "default" => return Ok(Gamma::DEFAULT),
            _ => {}
        }
        let err = || {
            format!(
                "gamma must be `linear`, `default` or `<dark-on-light>,<light-on-dark>` like `1.2,1.8` (got {s:?})"
            )
        };
        let (a, b) = s.split_once(',').ok_or_else(err)?;
        let centi = |t: &str| -> Option<u16> {
            let (int, frac) = t.trim().split_once('.').unwrap_or((t.trim(), ""));
            if frac.len() > 2 || int.is_empty() {
                return None;
            }
            let i: u16 = int.parse().ok()?;
            let f: u16 = if frac.is_empty() {
                0
            } else {
                format!("{frac:0<2}").parse().ok()?
            };
            i.checked_mul(100)?.checked_add(f)
        };
        Ok(Gamma::new(centi(a).ok_or_else(err)?, centi(b).ok_or_else(err)?))
    }
}

/// A bitmap font to generate: a builder over the source font file and the options. See the
/// [module docs](self).
///
/// Relative paths (the font, symbol fonts, character files, Rust sources) are read relative
/// to [`root`](Font::root) — by default the current directory, which in a build script is the
/// package's directory — and are written as given into the generated header.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct Font {
    ttf: PathBuf,
    root: PathBuf,
    size: u16,
    bpp: Bpp,
    chars: BTreeSet<char>,
    ranges: Vec<RangeInclusive<char>>,
    chars_files: Vec<PathBuf>,
    rust_sources: Vec<PathBuf>,
    symbol_fonts: Vec<PathBuf>,
    format: BitmapFormat,
    kerning: bool,
    subpx: Subpx,
    hinting: Hinting,
    gamma: Gamma,
    name: Option<String>,
    visibility: String,
    crate_path: String,
}

impl Font {
    /// A font generated from the TrueType or OpenType file `path` (`.ttf`, `.otf`), with the
    /// defaults: [`DEFAULT_SIZE`] px, [`Bpp::Four`], no characters yet, plain bitmaps, kerning
    /// on, no subpixels, [`Hinting::Full`], [`Gamma::DEFAULT`], `pub`, the static named after the file and size
    /// (`Montserrat-Medium.ttf` at 16 px → `MONTSERRAT_MEDIUM_16`), paths through the `twine`
    /// facade (`twine::text`).
    ///
    /// ```
    /// use twine_build::font::Font;
    /// let f = Font::from_ttf("assets/Montserrat-Medium.ttf").size(16);
    /// assert_eq!(f.static_name(), "MONTSERRAT_MEDIUM_16");
    /// ```
    #[doc(alias = "lv_font_conv")]
    pub fn from_ttf(path: impl Into<PathBuf>) -> Font {
        Font {
            ttf: path.into(),
            root: PathBuf::new(),
            size: DEFAULT_SIZE,
            bpp: Bpp::Four,
            chars: BTreeSet::new(),
            ranges: Vec::new(),
            chars_files: Vec::new(),
            rust_sources: Vec::new(),
            symbol_fonts: Vec::new(),
            format: BitmapFormat::Plain,
            kerning: true,
            subpx: Subpx::None,
            hinting: Hinting::Full,
            gamma: Gamma::DEFAULT,
            name: None,
            visibility: "pub".into(),
            crate_path: "twine::text".into(),
        }
    }

    /// The directory relative paths are read from (default: the current directory).
    pub fn root(mut self, dir: impl Into<PathBuf>) -> Font {
        self.root = dir.into();
        self
    }

    /// The pixel size (em height), 1 to 255 px (checked by [`build`](Font::build)).
    pub fn size(mut self, px: u16) -> Font {
        self.size = px;
        self
    }

    /// The bit depth of the coverage values (default [`Bpp::Four`]): more levels look smoother
    /// and cost more flash (a 4 bpp font is about twice the size of a 2 bpp one).
    pub fn bpp(mut self, bpp: Bpp) -> Font {
        self.bpp = bpp;
        self
    }

    /// Adds characters: typed ranges (`'\u{20}'..='\u{7E}'`), characters, strings, a
    /// translation [`Catalog`](crate::i18n::Catalog) (every character of every message, and
    /// the digits and signs of each language's numbers) or lists of those ([`CharSource`]).
    /// Call it as often as needed; the font holds the union. [`build`](Font::build) adds the
    /// forms shaping and mirroring draw for them ([`implied_chars`]).
    ///
    /// ```
    /// use twine_build::font::Font;
    /// use twine_build::i18n::{Format, Translations};
    ///
    /// let catalog = Translations::new()
    ///     .source("en", Format::Toml, "ok = \"OK\"")
    ///     .source("de", Format::Toml, "ok = \"Größe\"")
    ///     .default_language("en")
    ///     .build()
    ///     .unwrap();
    /// let f = Font::from_ttf("x.ttf").chars_from('0'..='9').chars_from(&catalog);
    /// // German writes percentages as `12\u{A0}%`.
    /// assert_eq!(f.chars().iter().collect::<String>(), "%,-.0123456789GKOer\u{A0}ßö");
    /// ```
    pub fn chars_from(mut self, source: impl CharSource) -> Font {
        let mut set = BTreeSet::new();
        source.add_chars(&mut set);
        self.ranges.extend(ranges(&set));
        self.chars.extend(set);
        self
    }

    /// Adds the characters of a UTF-8 text file (whitespace ignored), e.g. a list of common
    /// CJK characters. Read by [`build`](Font::build).
    pub fn chars_from_file(mut self, path: impl Into<PathBuf>) -> Font {
        self.chars_files.push(path.into());
        self
    }

    /// Adds the characters of the string and character literals of the Rust sources under
    /// `path` (a `.rs` file or a directory, searched recursively) — what the application can
    /// show from its own code. See [`rust_literal_chars`](crate::chars::rust_literal_chars).
    /// Read by [`build`](Font::build).
    pub fn chars_from_rust_sources(mut self, path: impl Into<PathBuf>) -> Font {
        self.rust_sources.push(path.into());
        self
    }

    /// Merges the glyphs of every Font Awesome `twine::text::Symbol` (`Symbol::is_font_awesome`)
    /// from these symbol fonts, searched in order (e.g. Font Awesome Free 5's
    /// `fa-solid-900.ttf`, then `fa-brands-400.ttf` for the USB and Bluetooth icons). Symbols
    /// are centered vertically on the font's line box. An empty list merges nothing.
    pub fn symbol_fonts<P: Into<PathBuf>>(mut self, paths: impl IntoIterator<Item = P>) -> Font {
        self.symbol_fonts = paths.into_iter().map(Into::into).collect();
        self
    }

    /// How bitmaps are stored (default [`BitmapFormat::Plain`]): plain bitmaps draw fastest,
    /// compressed ones (LVGL RLE, with or without the XOR row prefilter) take less flash and
    /// are decoded into the glyph cache.
    pub fn format(mut self, format: BitmapFormat) -> Font {
        self.format = format;
        self
    }

    /// Whether to store pair kerning (default `true`).
    pub fn kerning(mut self, kerning: bool) -> Font {
        self.kerning = kerning;
        self
    }

    /// Subpixel rendering (default [`Subpx::None`]): horizontal or vertical RGB subpixel glyphs,
    /// for displays whose subpixel order is known.
    pub fn subpx(mut self, subpx: Subpx) -> Font {
        self.subpx = subpx;
        self
    }

    /// Grid-fitting of the outlines (default [`Hinting::Full`]: crisp stems, whole-pixel
    /// advances); [`Hinting::None`] keeps the outlines and advances as designed. Host-only:
    /// the firmware is the same size either way.
    ///
    /// ```
    /// use twine_build::font::{Font, Hinting};
    /// let f = Font::from_ttf("a.ttf").chars_from('A').hinting(Hinting::Light).name("A");
    /// assert!(f.command_line().contains("--hinting light"));
    /// ```
    pub fn hinting(mut self, hinting: Hinting) -> Font {
        self.hinting = hinting;
        self
    }

    /// The text-gamma curves stored with the font (default [`Gamma::DEFAULT`];
    /// [`Gamma::LINEAR`] for LVGL's linear coverage mapping). Checked by
    /// [`build`](Font::build): each strength within 0.5..=4.0.
    ///
    /// ```
    /// use twine_build::font::{Font, Gamma};
    /// let f = Font::from_ttf("a.ttf").chars_from('A').gamma(Gamma::new(130, 200)).name("A");
    /// assert!(f.command_line().contains("--gamma 1.30,2.00"));
    /// ```
    pub fn gamma(mut self, gamma: Gamma) -> Font {
        self.gamma = gamma;
        self
    }

    /// The name of the generated static (a Rust identifier, checked by [`build`](Font::build));
    /// its data statics are named after it (`<NAME>_BITMAP`, `<NAME>_GLYPHS`, …).
    pub fn name(mut self, name: &str) -> Font {
        self.name = Some(name.to_string());
        self
    }

    /// The visibility of the generated static (default `pub`; `""` for private,
    /// `pub(crate)`, …).
    pub fn visibility(mut self, visibility: &str) -> Font {
        self.visibility = visibility.to_string();
        self
    }

    /// The path through which the generated code names the text crate (default
    /// `twine::text`, for applications using the `twine` facade; `twine_text` for code that
    /// depends on `twine-text` directly).
    pub fn crate_path(mut self, path: &str) -> Font {
        self.crate_path = path.to_string();
        self
    }

    /// The characters added with [`chars_from`](Font::chars_from) so far (files and Rust
    /// sources are read by [`build`](Font::build)).
    #[must_use]
    pub fn chars(&self) -> &BTreeSet<char> {
        &self.chars
    }

    /// The name of the generated static: [`name`](Font::name), or the file stem and size.
    #[must_use]
    pub fn static_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            let stem = self
                .ttf
                .file_stem()
                .map_or_else(|| "font".into(), |s| s.to_string_lossy().into_owned());
            static_name(&[&stem, &self.size.to_string()])
        })
    }

    /// The `twine font` command line that generates the same font (recorded in the generated
    /// header; characters are written as code-point ranges, and `--crate-path` when it is not
    /// the command's default `twine_text`).
    ///
    /// ```
    /// use twine_build::font::{Bpp, Font};
    /// let f = Font::from_ttf("a.ttf").size(12).bpp(Bpp::Two).chars_from('A'..='Z').name("A");
    /// assert_eq!(
    ///     f.command_line(),
    ///     "twine font --ttf a.ttf --size 12 --bpp 2 --range 0x41-0x5A --name A --crate-path twine::text"
    /// );
    /// ```
    #[must_use]
    pub fn command_line(&self) -> String {
        let mut s = format!(
            "twine font --ttf {} --size {} --bpp {}",
            self.ttf.display(),
            self.size,
            self.bpp.bits()
        );
        for r in &self.ranges {
            let (a, b) = (u32::from(*r.start()), u32::from(*r.end()));
            if a == b {
                let _ = write!(s, " --range 0x{a:X}");
            } else {
                let _ = write!(s, " --range 0x{a:X}-0x{b:X}");
            }
        }
        for p in &self.chars_files {
            let _ = write!(s, " --chars-file {}", p.display());
        }
        for p in &self.rust_sources {
            let _ = write!(s, " --chars-from-rust {}", p.display());
        }
        if !self.symbol_fonts.is_empty() {
            s.push_str(" --symbols fa");
            for p in &self.symbol_fonts {
                let _ = write!(s, " --symbols-ttf {}", p.display());
            }
        }
        match self.format {
            BitmapFormat::Plain => {}
            BitmapFormat::Compressed => s.push_str(" --compress"),
            BitmapFormat::CompressedNoPrefilter => s.push_str(" --compress --no-prefilter"),
        }
        if !self.kerning {
            s.push_str(" --no-kerning");
        }
        match self.subpx {
            Subpx::None => {}
            Subpx::Hor => s.push_str(" --subpx hor"),
            Subpx::Ver => s.push_str(" --subpx ver"),
        }
        if self.hinting != Hinting::Full {
            let _ = write!(s, " --hinting {}", self.hinting);
        }
        if self.gamma != Gamma::DEFAULT {
            let _ = write!(s, " --gamma {}", self.gamma.arg());
        }
        let _ = write!(s, " --name {}", self.static_name());
        if self.crate_path != CLI_CRATE_PATH {
            let _ = write!(s, " --crate-path {}", self.crate_path);
        }
        s
    }

    /// Rasterizes the glyphs and generates the font.
    ///
    /// # Errors
    /// An invalid option (a size outside 1..=255, a name that is not a Rust identifier, an
    /// invalid crate path or visibility, a gamma strength outside 0.5..=4.0, no characters,
    /// more than 65 534 glyphs, a glyph too large for the bitmap format), an unreadable or
    /// unparsable input file, or a font the auto-hinter cannot hint.
    pub fn build(&self) -> Result<GeneratedFont, AssetError> {
        let name = self.static_name();
        if self.size == 0 || self.size > 255 {
            return Err(format!("size must be 1..=255 px (got {})", self.size).into());
        }
        if !is_ident(&name) {
            return Err(format!("name `{name}` is not a Rust identifier").into());
        }
        if !is_path(&self.crate_path) {
            return Err(format!("crate path `{}` is not a Rust path", self.crate_path).into());
        }
        if !is_visibility(&self.visibility) {
            return Err(format!("`{}` is not a visibility", self.visibility).into());
        }
        self.gamma.check()?;
        let mut warnings = Vec::new();
        let ttf_path = self.root.join(&self.ttf);
        let mut inputs = vec![ttf_path.clone()];
        let (main_bytes, main) = load_font(&ttf_path, self.size)?;

        let mut cps = self.chars.clone();
        for path in &self.chars_files {
            let full = self.root.join(path);
            let text = std::fs::read_to_string(&full).map_err(|e| AssetError::file(&full, e))?;
            cps.extend(text.chars().filter(|c| !c.is_whitespace()));
            inputs.push(full);
        }
        for path in &self.rust_sources {
            let full = self.root.join(path);
            let (set, files) = rust_source_chars(&full).map_err(|e| AssetError::new(e.to_string()))?;
            cps.extend(set);
            // The directory too: a new file in it reruns the build script.
            inputs.push(full);
            inputs.extend(files);
        }
        if cps.is_empty() && self.symbol_fonts.is_empty() {
            return Err("no characters: add some with `chars_from`".into());
        }
        // The forms the text pipeline draws instead of the characters asked for (taken
        // when the font has them, never warned about).
        let implied: BTreeSet<char> = implied_chars(&cps).difference(&cps).copied().collect();

        // Line metrics.
        let lm = main
            .horizontal_line_metrics(f32::from(self.size))
            .ok_or_else(|| AssetError::file(&ttf_path, "the font has no horizontal line metrics"))?;
        let ascent = lm.ascent.ceil() as i32;
        let descent = (-lm.descent).ceil() as i32;
        let line_height = ascent + descent;
        let kerning_src = gpos::Kerning::new(&main_bytes)
            .ok_or_else(|| AssetError::file(&ttf_path, "ttf-parser cannot parse the font"))?;
        let scale = f32::from(self.size) / f32::from(kerning_src.units_per_em());
        let (underline_position, underline_thickness) = match kerning_src.underline() {
            Some((pos, thick)) => (
                (f32::from(pos) * scale).round() as i32,
                ((f32::from(thick) * scale).round() as i32).max(1),
            ),
            None => (-(descent / 2), i32::from(self.size / 14).max(1)),
        };

        let raster = Rasterizer::new(self.size, self.subpx, self.hinting);
        let main_face = Face::new(&main_bytes, &main, &raster).map_err(|e| AssetError::file(&ttf_path, e))?;
        let mut glyphs: BTreeMap<char, Glyph> = BTreeMap::new();
        for &cp in &cps {
            if !main.has_glyph(cp) {
                if cp.is_control() {
                    continue; // e.g. U+007F in 0x20..=0x7F: never drawn, silently skipped
                }
                warnings.push(format!(
                    "U+{:04X} is not in {}; skipped",
                    u32::from(cp),
                    self.ttf.display()
                ));
                continue;
            }
            let (cov, adv16) = raster
                .glyph(&main_face, cp, false)
                .map_err(|e| AssetError::file(&ttf_path, e))?;
            glyphs.insert(cp, Glyph { cp, adv16, cov });
        }
        for &cp in implied.iter().filter(|&&cp| main.has_glyph(cp)) {
            let (cov, adv16) = raster
                .glyph(&main_face, cp, false)
                .map_err(|e| AssetError::file(&ttf_path, e))?;
            glyphs.insert(cp, Glyph { cp, adv16, cov });
        }
        let main_cps: Vec<char> = glyphs.keys().copied().collect();

        let mut symbol_hashes = Vec::new();
        if !self.symbol_fonts.is_empty() {
            let mut sym_files = Vec::new();
            for path in &self.symbol_fonts {
                let full = self.root.join(path);
                let (bytes, f) = load_font(&full, self.size)?;
                symbol_hashes.push((file_name(path), sha256_hex(&bytes)));
                sym_files.push((full.clone(), bytes, f));
                inputs.push(full);
            }
            let mut sym_fonts = Vec::new();
            for (full, bytes, f) in &sym_files {
                sym_fonts.push((
                    full,
                    Face::new(bytes, f, &raster).map_err(|e| AssetError::file(full, e))?,
                ));
            }
            for cp in twine_text::Symbol::ALL
                .iter()
                .filter(|s| s.is_font_awesome())
                .map(|s| s.as_char())
            {
                let alias = SYMBOL_ALIASES.iter().find(|(c, _)| *c == cp);
                let src = alias.map_or(cp, |&(_, a)| a);
                let Some((sym_path, sym)) = sym_fonts.iter().find(|(_, f)| f.fd.has_glyph(src)) else {
                    warnings.push(format!(
                        "symbol U+{:04X} is not in the symbol fonts; skipped",
                        u32::from(cp)
                    ));
                    continue;
                };
                let (cov, adv16) = raster
                    .glyph(sym, src, alias.is_some())
                    .map_err(|e| AssetError::file(sym_path, e))?;
                // Center the symbol vertically on the main font's line box
                // (`[-descent, ascent]` around the baseline).
                let h_px = if self.subpx == Subpx::Ver {
                    cov.h / 3
                } else {
                    cov.h
                };
                let target = (ascent - descent - h_px).div_euclid(2);
                let cov = if cov.h == 0 {
                    cov
                } else {
                    let unit = if self.subpx == Subpx::Ver { 3 } else { 1 };
                    let dy = target * unit - cov.y0;
                    cov.shifted_y(dy)
                };
                glyphs.insert(cp, Glyph { cp, adv16, cov });
            }
        }

        // Glyph ids: sorted by code point, id 0 reserved.
        let ordered: Vec<&Glyph> = glyphs.values().collect();
        if ordered.len() >= usize::from(u16::MAX) {
            return Err("too many glyphs (max 65 534)".into());
        }
        let id_of: BTreeMap<char, u16> = ordered
            .iter()
            .enumerate()
            .map(|(i, g)| (g.cp, (i + 1) as u16))
            .collect();

        // Bitmaps and descriptors.
        let mut bitmap = Vec::new();
        let mut dscs = vec![GlyphDsc::default()];
        for g in &ordered {
            let c = &g.cov;
            let sub_unit_y = if self.subpx == Subpx::Ver { 3 } else { 1 };
            let sub_unit_x = if self.subpx == Subpx::Hor { 3 } else { 1 };
            // DIV: `sub_unit_x` and `sub_unit_y` are 1 or 3 (set just above).
            let (ofs_x, ofs_y) = (c.x0.div_euclid(sub_unit_x), c.y0.div_euclid(sub_unit_y));
            if c.w > 255 || c.h > 255 || !(-128..=127).contains(&ofs_x) || !(-128..=127).contains(&ofs_y) {
                return Err(format!(
                    "glyph U+{:04X} is too large for the bitmap format ({}×{} at offset {ofs_x},{ofs_y})",
                    u32::from(g.cp),
                    c.w,
                    c.h
                )
                .into());
            }
            let values: Vec<u8> = c.data.iter().map(|&v| quantize(v, self.bpp)).collect();
            let bitmap_index = bitmap.len() as u32;
            bitmap.extend(encode_glyph(&values, c.w as usize, self.bpp, self.format));
            dscs.push(GlyphDsc {
                bitmap_index: if c.w == 0 { 0 } else { bitmap_index },
                adv_w: g.adv16,
                box_w: c.w as u8,
                box_h: c.h as u8,
                ofs_x: ofs_x as i8,
                ofs_y: ofs_y as i8,
            });
        }

        let cmaps = build_cmaps(&ordered.iter().map(|g| g.cp).collect::<Vec<_>>());

        // Kerning between main-font glyphs.
        let mut kern = KernData::None;
        if self.kerning && kerning_src.has_kerning() {
            let mut ids = Vec::new();
            let mut values = Vec::new();
            for &l in &main_cps {
                for &r in &main_cps {
                    let units = kerning_src.kern(l, r);
                    if units == 0 {
                        continue;
                    }
                    let v16 = (units as f32 * scale * 16.0).round() as i32;
                    if v16 == 0 {
                        continue;
                    }
                    let clamped = v16.clamp(-127, 127);
                    if clamped != v16 {
                        warnings.push(format!(
                            "kerning {l:?}→{r:?} of {v16}/16 px clamped to {clamped}/16 px"
                        ));
                    }
                    ids.push([id_of[&l], id_of[&r]]);
                    values.push(clamped as i8);
                }
            }
            if !ids.is_empty() {
                kern = KernData::Pairs { ids, values };
            }
        }

        let data = FontData {
            line_height: line_height as i16,
            base_line: descent as i16,
            underline_position: underline_position.clamp(-128, 127) as i8,
            underline_thickness: underline_thickness.clamp(0, 255) as u8,
            subpx: self.subpx,
            bpp: self.bpp,
            format: self.format,
            bitmap,
            glyphs: dscs,
            cmaps,
            kern,
            kern_scale: 16,
            gamma: self.gamma.tables(),
        };
        let mut code_points = vec!['\0'];
        code_points.extend(ordered.iter().map(|g| g.cp));
        let command = self.command_line();
        let (source, flash_bytes) = emit::render_rust(&FontEmit {
            name: &name,
            visibility: &self.visibility,
            crate_path: &self.crate_path,
            command: &command,
            size: self.size,
            hinting: self.hinting,
            rasterizer: if self.hinting == Hinting::None {
                UNHINTED_RASTERIZER
            } else {
                HINTED_RASTERIZER
            },
            gamma: self.gamma,
            source: (file_name(&self.ttf), sha256_hex(&main_bytes)),
            symbols: &symbol_hashes,
            data: &data,
            code_points: &code_points,
        });
        Ok(GeneratedFont {
            data,
            code_points,
            source,
            flash_bytes,
            warnings,
            inputs,
        })
    }
}

/// A generated font: the in-memory description and its Rust source.
#[derive(Clone, Debug)]
pub struct GeneratedFont {
    /// The font data (a live font for tests: [`FontData::leak`]).
    pub data: FontData,
    /// Code point of every glyph id (index 0 = reserved, `'\0'`).
    pub code_points: Vec<char>,
    /// The Rust source: the data statics and `<visibility> static <NAME>: <crate>::Font`.
    pub source: String,
    /// Bytes of static data (bitmaps, descriptors, cmaps, kerning, the font structs).
    pub flash_bytes: usize,
    /// Non-fatal problems: characters missing from the font, clamped kerning values.
    pub warnings: Vec<String>,
    /// Every file read (the font, symbol fonts, character files, Rust sources).
    pub inputs: Vec<PathBuf>,
}

impl GeneratedFont {
    /// Writes the Rust source to `path` (only when it changed, so dependents are not rebuilt
    /// needlessly; the parent directory is created).
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), AssetError> {
        write_if_changed(path.as_ref(), self.source.as_bytes())
    }

    /// The build-script convenience: writes the source to `$OUT_DIR/<file_name>`, prints
    /// `cargo:rerun-if-changed` for every input and `cargo:warning` for every warning, and
    /// returns the path written. Include it with
    /// `include!(concat!(env!("OUT_DIR"), "/<file_name>"))`.
    ///
    /// # Errors
    /// When `OUT_DIR` is not set (not in a build script) or the file cannot be written.
    pub fn write_to_out_dir(&self, file_name: &str) -> Result<PathBuf, AssetError> {
        let path = out_dir("GeneratedFont::write_to_out_dir")?.join(file_name);
        self.write(&path)?;
        for input in &self.inputs {
            println!("cargo:rerun-if-changed={}", input.display());
        }
        for w in &self.warnings {
            println!("cargo:warning={w}");
        }
        Ok(path)
    }
}

/// One rasterized glyph before encoding.
struct Glyph {
    cp: char,
    adv16: u16,
    cov: Cov,
}

fn load_font(path: &Path, size: u16) -> Result<(Vec<u8>, fontdue::Font), AssetError> {
    let bytes = std::fs::read(path).map_err(|e| AssetError::file(path, format!("cannot read: {e}")))?;
    let font = fontdue::Font::from_bytes(
        bytes.as_slice(),
        fontdue::FontSettings {
            scale: f32::from(size),
            ..fontdue::FontSettings::default()
        },
    )
    .map_err(|e| AssetError::file(path, format!("cannot parse: {e}")))?;
    Ok((bytes, font))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Cmaps for glyphs sorted by code point with ids `1..`: runs of ≥ 3 consecutive code points
/// become `Format0Tiny`; the code points between runs are grouped into `SparseTiny` cmaps (a
/// group never spans a run, so its glyph ids stay consecutive, and never exceeds 65 535 code
/// points).
///
/// ```
/// use twine_build::font::build_cmaps;
/// let cmaps = build_cmaps(&['A', 'B', 'C', '°']);
/// assert_eq!(cmaps.len(), 2);
/// assert_eq!((cmaps[0].range_start, cmaps[0].range_length), (0x41, 3));
/// ```
#[must_use]
pub fn build_cmaps(cps: &[char]) -> Vec<CmapData> {
    let mut cmaps = Vec::new();
    let mut sparse: Vec<(u32, u16)> = Vec::new(); // (code point, glyph id)
    let flush = |sparse: &mut Vec<(u32, u16)>, cmaps: &mut Vec<CmapData>| {
        if let Some(&(start, id)) = sparse.first() {
            let last = sparse.last().map_or(start, |p| p.0);
            cmaps.push(CmapData {
                range_start: start,
                range_length: (last - start + 1) as u16,
                glyph_id_start: id,
                unicode_list: sparse.iter().map(|&(c, _)| (c - start) as u16).collect(),
                ofs_u8: Vec::new(),
                ofs_u16: Vec::new(),
                kind: CmapKind::SparseTiny,
            });
        }
        sparse.clear();
    };
    let mut i = 0;
    while i < cps.len() {
        let mut j = i + 1;
        while j < cps.len() && u32::from(cps[j]) == u32::from(cps[j - 1]) + 1 && j - i < usize::from(u16::MAX)
        {
            j += 1;
        }
        let id = (i + 1) as u16;
        if j - i >= 3 {
            flush(&mut sparse, &mut cmaps);
            cmaps.push(CmapData {
                range_start: u32::from(cps[i]),
                range_length: (j - i) as u16,
                glyph_id_start: id,
                unicode_list: Vec::new(),
                ofs_u8: Vec::new(),
                ofs_u16: Vec::new(),
                kind: CmapKind::Format0Tiny,
            });
        } else {
            for (k, &ch) in cps.iter().enumerate().take(j).skip(i) {
                let cp = u32::from(ch);
                if sparse
                    .first()
                    .is_some_and(|&(s, _)| cp - s > u32::from(u16::MAX) - 1)
                {
                    flush(&mut sparse, &mut cmaps);
                }
                sparse.push((cp, (k + 1) as u16));
            }
        }
        i = j;
    }
    flush(&mut sparse, &mut cmaps);
    cmaps
}

/// Glyph metrics of `chars` in the font file `ttf` at `size` px with `hinting`, with each
/// glyph's coverage as ASCII art (`twine font --dump`, a debugging aid): line metrics,
/// unhinted advances, the generated advance and inked box, glyph indices and the kerning to the
/// previous character.
///
/// ```no_run
/// use twine_build::font::{Hinting, describe};
/// let text = describe("assets/fonts/Montserrat-Medium.ttf".as_ref(), 14, Hinting::Full, "Ag").unwrap();
/// print!("{text}");
/// ```
///
/// # Errors
/// When the font file cannot be read or parsed, or the auto-hinter fails.
pub fn describe(ttf: &Path, size: u16, hinting: Hinting, chars: &str) -> Result<String, AssetError> {
    const RAMP: &[u8] = b" .:-=+*#%@";
    let (bytes, font) = load_font(ttf, size)?;
    let raster = Rasterizer::new(size, Subpx::None, hinting);
    let face = Face::new(&bytes, &font, &raster).map_err(|e| AssetError::file(ttf, e))?;
    let mut out = String::new();
    if let Some(lm) = font.horizontal_line_metrics(f32::from(size)) {
        let _ = writeln!(
            out,
            "{} at {size} px, {hinting} hinting: ascent {:.2}, descent {:.2}, line gap {:.2}",
            ttf.display(),
            lm.ascent,
            lm.descent,
            lm.line_gap
        );
    }
    let k = gpos::Kerning::new(&bytes);
    let mut prev: Option<char> = None;
    for c in chars.chars() {
        let m = font.metrics(c, f32::from(size));
        let (cov, adv16) = raster
            .glyph(&face, c, false)
            .map_err(|e| AssetError::file(ttf, e))?;
        let _ = write!(
            out,
            "U+{:04X} {c:?}: advance {:.3} px (generated {}/16 px), box {}×{} at ({}, {}), glyph index {}",
            u32::from(c),
            m.advance_width,
            adv16,
            cov.w,
            cov.h,
            cov.x0,
            cov.y0,
            font.lookup_glyph_index(c)
        );
        if let (Some(p), Some(k)) = (prev, &k) {
            let units = k.kern(p, c);
            if units != 0 {
                let _ = write!(
                    out,
                    ", kerning after {p:?}: {units} units = {:.3} px",
                    units as f32 * f32::from(size) / f32::from(k.units_per_em())
                );
            }
        }
        out.push('\n');
        // Coverage as ASCII art (` .:-=+*#%@` by coverage).
        for row in cov.data.chunks(cov.w.max(1) as usize) {
            out.push_str("    |");
            out.extend(
                row.iter()
                    .map(|&v| char::from(RAMP[usize::from(v) * (RAMP.len() - 1) / 255])),
            );
            out.push_str("|\n");
        }
        prev = Some(c);
    }
    Ok(out)
}
