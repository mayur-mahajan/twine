//! `cargo xtask fonts [--check]`: regenerates the built-in fonts of `twine-assets` from
//! `assets/fonts/fonts.toml` with the font generator of `twine-build` (called in-process).
//!
//! The text fields are parsed into typed values when the file is read: `bpp` into a
//! `twine_text::Bpp` (1, 2, 4 or 8), `ranges` into typed character ranges (the `twine font
//! --range` syntax), the optional `hinting` (`full` by default) into a
//! `twine_build::font::Hinting` and `gamma` (`default` by default) into a
//! `twine_build::font::Gamma`.
//!
//! `--check` regenerates every font in memory and fails if a file in
//! `crates/twine-assets/src/fonts/` differs (or is missing).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};
use twine_build::font::{BitmapFormat, Bpp, Font, Gamma, Hinting, Subpx};
use twine_cli::font::{CharRanges, DEFAULT_SYMBOLS_TTF, parse_range};

use crate::util::{R, warn, workspace_root};

/// Directory of the generated font files, relative to the workspace root.
pub const FONTS_DIR: &str = "crates/twine-assets/src/fonts";
/// The font list, relative to the workspace root.
pub const FONTS_TOML: &str = "assets/fonts/fonts.toml";

/// One `[[font]]` entry of `fonts.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontEntry {
    /// Static name.
    pub name: String,
    /// twine-assets feature.
    pub feature: String,
    /// Output file name.
    pub file: String,
    /// Source TTF.
    pub ttf: String,
    /// Pixel size.
    pub size: u16,
    /// Bits per pixel (1, 2, 4 or 8).
    #[serde(deserialize_with = "de_bpp")]
    pub bpp: Bpp,
    /// Code point ranges (`"0x20-0x7F"`, `"0xB0"`, `"chars:°•"`).
    #[serde(deserialize_with = "de_ranges")]
    pub ranges: Vec<CharRanges>,
    /// `"fa"` or `"none"`.
    pub symbols: String,
    /// RLE compression.
    pub compress: bool,
    /// Kerning.
    pub kerning: bool,
    /// `"none"` (default), `"hor"` or `"ver"`.
    #[serde(default)]
    pub subpx: Option<String>,
    /// Optional file listing extra characters (e.g. a common CJK character list).
    #[serde(default)]
    pub chars_file: Option<String>,
    /// `"full"` (default), `"light"` or `"none"`.
    #[serde(default, deserialize_with = "de_hinting")]
    pub hinting: Hinting,
    /// `"default"` (default), `"linear"` or `"<dark-on-light>,<light-on-dark>"`.
    #[serde(default, deserialize_with = "de_gamma")]
    pub gamma: Gamma,
}

fn de_hinting<'de, D: Deserializer<'de>>(d: D) -> Result<Hinting, D::Error> {
    String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
}

fn de_gamma<'de, D: Deserializer<'de>>(d: D) -> Result<Gamma, D::Error> {
    String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
}

fn de_bpp<'de, D: Deserializer<'de>>(d: D) -> Result<Bpp, D::Error> {
    let bits = u8::deserialize(d)?;
    Bpp::new(bits).ok_or_else(|| serde::de::Error::custom(format!("bpp must be 1, 2, 4 or 8 (got {bits})")))
}

fn de_ranges<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<CharRanges>, D::Error> {
    Vec::<String>::deserialize(d)?
        .iter()
        .map(|s| parse_range(s).map_err(serde::de::Error::custom))
        .collect()
}

#[derive(Debug, Deserialize)]
struct FontsToml {
    font: Vec<FontEntry>,
}

/// Parses `fonts.toml` text.
pub fn parse(text: &str) -> Result<Vec<FontEntry>, Box<dyn std::error::Error>> {
    let t: FontsToml = toml::from_str(text)?;
    Ok(t.font)
}

impl FontEntry {
    /// The generator of this entry (files are read relative to `root`, the workspace root).
    pub fn font(&self, root: &Path) -> Result<Font, Box<dyn std::error::Error>> {
        let subpx = match self.subpx.as_deref().unwrap_or("none") {
            "none" => Subpx::None,
            "hor" => Subpx::Hor,
            "ver" => Subpx::Ver,
            s => return Err(format!("{}: subpx must be none/hor/ver (got {s:?})", self.name).into()),
        };
        let mut f = Font::from_ttf(&self.ttf)
            .root(root)
            .size(self.size)
            .bpp(self.bpp)
            .format(if self.compress {
                BitmapFormat::Compressed
            } else {
                BitmapFormat::Plain
            })
            .kerning(self.kerning)
            .subpx(subpx)
            .hinting(self.hinting)
            .gamma(self.gamma)
            .name(&self.name)
            .crate_path("twine_text");
        for r in &self.ranges {
            f = f.chars_from(r.0.as_slice());
        }
        if let Some(file) = &self.chars_file {
            f = f.chars_from_file(file);
        }
        match self.symbols.as_str() {
            "fa" => f = f.symbol_fonts(DEFAULT_SYMBOLS_TTF.iter().copied()),
            "none" => {}
            s => return Err(format!("{}: symbols must be \"fa\" or \"none\" (got {s:?})", self.name).into()),
        }
        Ok(f)
    }

    /// The generated file, relative to the workspace root.
    pub fn out(&self) -> PathBuf {
        PathBuf::from(FONTS_DIR).join(&self.file)
    }
}

/// Regenerates (or with `check`, verifies) every font.
pub fn run(check: bool) -> R {
    let root = workspace_root();
    let entries = parse(&std::fs::read_to_string(root.join(FONTS_TOML))?)?;
    let mut stale = Vec::new();
    let mut table = Vec::new();
    for e in &entries {
        let g = e
            .font(&root)?
            .build()
            .map_err(|err| format!("{}: {err}", e.name))?;
        for w in &g.warnings {
            warn(&format!("{}: {w}", e.name));
        }
        let rel = e.out();
        let path = root.join(&rel);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() != Some(g.source.as_str()) {
            stale.push(rel.display().to_string());
            if !check {
                g.write(&path)?;
                println!("fonts: wrote {}", rel.display());
            }
        }
        table.push((
            e.feature.clone(),
            g.code_points.len() - 1,
            i32::from(g.data.line_height),
            g.flash_bytes,
        ));
    }
    println!("\n| feature | glyphs | line height | flash bytes |\n|---|---:|---:|---:|");
    for (f, n, lh, bytes) in &table {
        println!("| `{f}` | {n} | {lh} | {bytes} |");
    }
    if stale.is_empty() {
        println!("\nfonts: {} fonts up to date", entries.len());
        Ok(())
    } else if check {
        Err(format!(
            "fonts: out of date (run `cargo xtask fonts`): {}",
            stale.join(", ")
        )
        .into())
    } else {
        println!("\nfonts: {} of {} fonts regenerated", stale.len(), entries.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_toml_parses_and_has_unique_names() {
        let entries = parse(&std::fs::read_to_string(workspace_root().join(FONTS_TOML)).unwrap()).unwrap();
        assert!(entries.len() >= 23);
        let mut names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), entries.len());
        for e in &entries {
            assert_eq!(e.font(&workspace_root()).unwrap().static_name(), e.name);
        }
        let bad = parse(
            "[[font]]\nname = \"X\"\nfeature = \"x\"\nfile = \"x.rs\"\nttf = \"x.ttf\"\nsize = 8\nbpp = 3\nranges = []\nsymbols = \"none\"\ncompress = false\nkerning = true\n",
        );
        assert!(
            bad.unwrap_err().to_string().contains("bpp must be 1, 2, 4 or 8"),
            "bpp checked when parsed"
        );
    }
}
