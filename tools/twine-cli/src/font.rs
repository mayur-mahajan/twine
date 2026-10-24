//! `twine font`: generates a Rust bitmap font from a TrueType/OpenType file — the command-line
//! form of `twine_build::font::Font` (which build scripts use directly).
//!
//! The text options are parsed into typed values once, when the command line is read: the
//! bit depth into a [`Bpp`] (1, 2, 4 or 8), each `--range` into typed character ranges
//! ([`parse_range`]), `--hinting` into a [`Hinting`] and `--gamma` into a [`Gamma`].
//!
//! ```no_run
//! use clap::Parser;
//! use twine_cli::font::{FontArgs, run};
//!
//! #[derive(Parser)]
//! struct Cli {
//!     #[command(flatten)]
//!     font: FontArgs,
//! }
//!
//! let cli = Cli::parse_from([
//!     "twine", "--ttf", "assets/fonts/Montserrat-Medium.ttf", "--size", "14",
//!     "--range", "0x20-0x7E", "--name", "MY_FONT", "--out", "src/my_font.rs",
//! ]);
//! run(&cli.font).unwrap();
//! ```

use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use twine_build::chars::ranges;
use twine_build::font::{BitmapFormat, Bpp, Font, Gamma, Hinting, Subpx, describe};

/// Result type of the command (errors are human-readable messages).
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Default Font Awesome files for `--symbols fa` (searched in order), relative to the working
/// directory: the solid set plus the brands set (USB and Bluetooth are brand icons).
pub const DEFAULT_SYMBOLS_TTF: &[&str] = &["assets/fonts/fa-solid-900.ttf", "assets/fonts/fa-brands-400.ttf"];

/// Which symbol set to merge (`--symbols`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Symbols {
    /// No symbols.
    #[default]
    None,
    /// The Font Awesome glyphs of every `twine_text::Symbol` (`Symbol::is_font_awesome`).
    Fa,
}

/// Subpixel mode argument (`--subpx`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum SubpxArg {
    /// Grayscale.
    #[default]
    None,
    /// Horizontal RGB subpixels.
    Hor,
    /// Vertical RGB subpixels.
    Ver,
}

impl From<SubpxArg> for Subpx {
    fn from(s: SubpxArg) -> Self {
        match s {
            SubpxArg::None => Subpx::None,
            SubpxArg::Hor => Subpx::Hor,
            SubpxArg::Ver => Subpx::Ver,
        }
    }
}

/// Parses a bit depth (`1`, `2`, `4` or `8`) into a [`Bpp`].
///
/// ```
/// use twine_cli::font::parse_bpp;
/// use twine_text::Bpp;
///
/// assert_eq!(parse_bpp("2"), Ok(Bpp::Two));
/// assert!(parse_bpp("3").is_err());
/// ```
///
/// # Errors
/// A message naming the accepted depths for anything else.
pub fn parse_bpp(s: &str) -> std::result::Result<Bpp, String> {
    s.parse::<u8>()
        .ok()
        .and_then(Bpp::new)
        .ok_or_else(|| format!("bpp must be 1, 2, 4 or 8 (got `{s}`)"))
}

/// The characters of one `--range` argument, as typed ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharRanges(pub Vec<RangeInclusive<char>>);

fn parse_num(s: &str) -> std::result::Result<u32, String> {
    let s = s.trim();
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        s.parse::<u32>()
    };
    v.map_err(|_| format!("invalid code point `{s}` (use decimal or 0x-prefixed hex)"))
}

const SURROGATES: RangeInclusive<u32> = 0xD800..=0xDFFF;

/// Parses one `--range` spec — `0x20-0x7F`, `0xB0`, `32-126` or `chars:°•` — into typed
/// character ranges (a range over the surrogate gap skips it).
///
/// ```
/// use twine_cli::font::{CharRanges, parse_range};
///
/// assert_eq!(parse_range("0x41-0x43"), Ok(CharRanges(vec!['A'..='C'])));
/// assert_eq!(parse_range("chars:°•"), Ok(CharRanges(vec!['°'..='°', '•'..='•'])));
/// assert!(parse_range("0x7F-0x20").is_err());
/// ```
///
/// # Errors
/// A message for a malformed spec, a code point beyond U+10FFFF, a reversed range, a range of
/// more than 65 536 code points or one that holds only surrogates.
pub fn parse_range(spec: &str) -> std::result::Result<CharRanges, String> {
    if let Some(chars) = spec.strip_prefix("chars:") {
        if chars.is_empty() {
            return Err("`chars:` range lists no characters".into());
        }
        let set: BTreeSet<char> = chars.chars().collect();
        return Ok(CharRanges(ranges(&set)));
    }
    let (a, b) = if let Some((a, b)) = spec.split_once('-') {
        (parse_num(a)?, parse_num(b)?)
    } else {
        let v = parse_num(spec)?;
        (v, v)
    };
    if a > b {
        return Err(format!("range `{spec}` is reversed"));
    }
    if b - a > 0x10000 {
        return Err(format!("range `{spec}` is larger than 65 536 code points"));
    }
    // A bound inside the surrogate gap moves to the nearest scalar value inside the range.
    let start = if SURROGATES.contains(&a) { 0xE000 } else { a };
    let end = if SURROGATES.contains(&b) { 0xD7FF } else { b };
    match (char::from_u32(start), char::from_u32(end)) {
        (Some(s), Some(e)) if s <= e => Ok(CharRanges(std::iter::once(s..=e).collect())),
        (Some(_), Some(_)) => Err(format!("range `{spec}` holds only surrogates")),
        _ => Err(format!("range `{spec}` goes beyond U+10FFFF")),
    }
}

/// Parses `--hinting` (`none`, `light` or `full`).
///
/// # Errors
/// Any other text.
///
/// ```
/// use twine_build::font::Hinting;
/// assert_eq!(twine_cli::font::parse_hinting("none"), Ok(Hinting::None));
/// assert!(twine_cli::font::parse_hinting("strong").is_err());
/// ```
pub fn parse_hinting(s: &str) -> std::result::Result<Hinting, String> {
    s.parse()
}

/// Parses `--gamma` (`default`, `linear` or `<dark-on-light>,<light-on-dark>`).
///
/// # Errors
/// Any other text.
///
/// ```
/// use twine_build::font::Gamma;
/// assert_eq!(twine_cli::font::parse_gamma("1.3,2"), Ok(Gamma::new(130, 200)));
/// assert!(twine_cli::font::parse_gamma("1.3").is_err());
/// ```
pub fn parse_gamma(s: &str) -> std::result::Result<Gamma, String> {
    s.parse()
}

/// Command-line arguments of `twine font`.
#[derive(Clone, Debug, clap::Args)]
#[command(about = "Generate a Rust bitmap font from a TTF file")]
pub struct FontArgs {
    /// Source TrueType or OpenType font.
    #[arg(long, required_unless_present = "dump")]
    pub ttf: Option<PathBuf>,
    /// Pixel size (em height).
    #[arg(long)]
    pub size: u16,
    /// Bits per pixel: 1, 2, 4 or 8.
    #[arg(long, default_value = "4", value_parser = parse_bpp)]
    pub bpp: Bpp,
    /// Code points: `0x20-0x7F`, `0xB0`, `32-126` or `chars:°•` (repeatable).
    #[arg(long = "range", value_parser = parse_range)]
    pub ranges: Vec<CharRanges>,
    /// UTF-8 text files whose characters are added (repeatable; whitespace ignored), e.g. a
    /// list of common CJK characters.
    #[arg(long = "chars-file")]
    pub chars_files: Vec<PathBuf>,
    /// Rust sources (a `.rs` file or a directory) whose string and character literals are
    /// added (repeatable): glyph subsetting to what the application shows.
    #[arg(long = "chars-from-rust")]
    pub chars_from_rust: Vec<PathBuf>,
    /// Merge the built-in symbol set.
    #[arg(long, value_enum, default_value_t = Symbols::None)]
    pub symbols: Symbols,
    /// Symbol font files, searched in order (repeatable; default
    /// `assets/fonts/fa-solid-900.ttf` and `assets/fonts/fa-brands-400.ttf`).
    #[arg(long)]
    pub symbols_ttf: Vec<PathBuf>,
    /// RLE-compress the glyph bitmaps.
    #[arg(long)]
    pub compress: bool,
    /// With `--compress`: skip the XOR row prefilter.
    #[arg(long)]
    pub no_prefilter: bool,
    /// Do not emit kerning.
    #[arg(long)]
    pub no_kerning: bool,
    /// Subpixel rendering.
    #[arg(long, value_enum, default_value_t = SubpxArg::None)]
    pub subpx: SubpxArg,
    /// Grid-fitting of the outlines: `full` (default: crisp stems, whole-pixel advances),
    /// `light` (vertical only, unhinted advances) or `none` (also for `--dump`).
    #[arg(long, default_value = "full", value_parser = parse_hinting)]
    pub hinting: Hinting,
    /// Text gamma: `default` (1.2 dark-on-light, 1.8 light-on-dark), `linear` (LVGL's
    /// mapping) or `<dark-on-light>,<light-on-dark>` strengths such as `1.3,2.0`.
    #[arg(long, default_value = "default", value_parser = parse_gamma)]
    pub gamma: Gamma,
    /// Name of the generated static (e.g. `MONTSERRAT_14`).
    #[arg(long, required_unless_present = "dump")]
    pub name: Option<String>,
    /// Path of the crate that defines `Font` in the generated code (`twine::text` for
    /// applications using the `twine` facade).
    #[arg(long, default_value = "twine_text")]
    pub crate_path: String,
    /// Output `.rs` file.
    #[arg(long, required_unless_present = "dump")]
    pub out: Option<PathBuf>,
    /// Print glyph metrics of this TTF instead of generating (debug aid).
    #[arg(long, conflicts_with_all = ["ttf", "out", "name"])]
    pub dump: Option<PathBuf>,
    /// With `--dump`: the characters to print.
    #[arg(long, default_value = "Ag")]
    pub chars: String,
}

/// The font builder of parsed arguments (not for `--dump`); relative paths are read from the
/// working directory.
///
/// # Errors
/// A message naming the missing required option (`--ttf`, `--name`).
pub fn font_from_args(a: &FontArgs) -> Result<Font> {
    let mut f = Font::from_ttf(a.ttf.clone().ok_or("--ttf is required")?)
        .size(a.size)
        .bpp(a.bpp)
        .kerning(!a.no_kerning)
        .subpx(a.subpx.into())
        .hinting(a.hinting)
        .gamma(a.gamma)
        .format(match (a.compress, a.no_prefilter) {
            (false, _) => BitmapFormat::Plain,
            (true, false) => BitmapFormat::Compressed,
            (true, true) => BitmapFormat::CompressedNoPrefilter,
        })
        .name(a.name.as_deref().ok_or("--name is required")?)
        .crate_path(&a.crate_path);
    for r in &a.ranges {
        f = f.chars_from(r.0.as_slice());
    }
    for p in &a.chars_files {
        f = f.chars_from_file(p);
    }
    for p in &a.chars_from_rust {
        f = f.chars_from_rust_sources(p);
    }
    if a.symbols == Symbols::Fa {
        if a.symbols_ttf.is_empty() {
            f = f.symbol_fonts(DEFAULT_SYMBOLS_TTF.iter().copied());
        } else {
            f = f.symbol_fonts(a.symbols_ttf.iter().cloned());
        }
    }
    Ok(f)
}

/// Runs `twine font` with parsed arguments (writes the output file or prints the dump).
///
/// # Errors
/// The errors of [`font_from_args`], `Font::build` and `describe`, or an I/O error writing
/// the output.
pub fn run(args: &FontArgs) -> Result<()> {
    if let Some(ttf) = &args.dump {
        print!("{}", describe(ttf, args.size, args.hinting, &args.chars)?);
        return Ok(());
    }
    let out = args.out.clone().ok_or("--out is required")?;
    let font = font_from_args(args)?;
    let g = font.build()?;
    for w in &g.warnings {
        eprintln!("warning: {w}");
    }
    g.write(&out)?;
    eprintln!(
        "{}: {} glyphs, {} bytes of font data → {}",
        font.static_name(),
        g.code_points.len() - 1,
        g.flash_bytes,
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(r: RangeInclusive<char>) -> Vec<RangeInclusive<char>> {
        std::iter::once(r).collect()
    }

    #[test]
    fn range_forms() {
        assert_eq!(parse_range("32-34").unwrap().0, one(' '..='"'));
        assert_eq!(parse_range("0xB0").unwrap().0, one('°'..='°'));
        assert!(parse_range("0x7F-0x20").is_err());
        assert!(parse_range("zz").is_err());
        assert!(parse_range("chars:").is_err());
        assert!(parse_range("0x0-0x10001").is_err());
        assert!(parse_range("0x110000").is_err());
        let gap = parse_range("0xD7FF-0xE000").unwrap().0;
        assert_eq!(gap[0].clone().count(), 2, "surrogates skipped");
        assert_eq!(
            parse_range("0xD900-0xE001").unwrap().0,
            one('\u{E000}'..='\u{E001}')
        );
        assert!(parse_range("0xD800-0xDFFF").is_err());
        assert_eq!(parse_range("chars:BAB").unwrap().0, one('A'..='B'));
    }
}
