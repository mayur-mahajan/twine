//! `cargo xtask fonts [--check]`: regenerates the built-in fonts of `twine-assets` from
//! `assets/fonts/fonts.toml` with the `twine font` generator (called in-process).
//!
//! `--check` regenerates every font in memory and fails if a file in
//! `crates/twine-assets/src/fonts/` differs (or is missing).

use std::path::PathBuf;

use serde::Deserialize;
use twine_cli::font::{FontOptions, SubpxArg, Symbols, generate};

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
    /// Bits per pixel.
    pub bpp: u8,
    /// Code point ranges.
    pub ranges: Vec<String>,
    /// `"fa"` or `"none"`.
    pub symbols: String,
    /// RLE compression.
    pub compress: bool,
    /// Kerning.
    pub kerning: bool,
    /// `"none"` (default), `"hor"` or `"ver"`.
    #[serde(default)]
    pub subpx: Option<String>,
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
    /// The generator options of this entry.
    pub fn options(&self) -> Result<FontOptions, Box<dyn std::error::Error>> {
        let symbols = match self.symbols.as_str() {
            "fa" => Symbols::Fa,
            "none" => Symbols::None,
            s => return Err(format!("{}: symbols must be \"fa\" or \"none\" (got {s:?})", self.name).into()),
        };
        let subpx = match self.subpx.as_deref().unwrap_or("none") {
            "none" => SubpxArg::None,
            "hor" => SubpxArg::Hor,
            "ver" => SubpxArg::Ver,
            s => return Err(format!("{}: subpx must be none/hor/ver (got {s:?})", self.name).into()),
        };
        Ok(FontOptions {
            ttf: PathBuf::from(&self.ttf),
            size: self.size,
            bpp: self.bpp,
            ranges: self.ranges.clone(),
            symbols,
            symbols_ttf: Vec::new(),
            compress: self.compress,
            prefilter: true,
            kerning: self.kerning,
            subpx,
            name: self.name.clone(),
            out: PathBuf::from(FONTS_DIR).join(&self.file),
        })
    }
}

/// Regenerates (or with `check`, verifies) every font.
pub fn run(check: bool) -> R {
    let root = workspace_root();
    let entries = parse(&std::fs::read_to_string(root.join(FONTS_TOML))?)?;
    let mut stale = Vec::new();
    let mut table = Vec::new();
    for e in &entries {
        let opts = e.options()?;
        let g = generate(&opts, &root).map_err(|err| format!("{}: {err}", e.name))?;
        for w in &g.warnings {
            warn(&format!("{}: {w}", e.name));
        }
        let path = root.join(&opts.out);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() != Some(g.source.as_str()) {
            stale.push(opts.out.display().to_string());
            if !check {
                std::fs::create_dir_all(path.parent().unwrap_or(&root))?;
                std::fs::write(&path, &g.source)?;
                println!("fonts: wrote {}", opts.out.display());
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
            e.options().unwrap();
        }
    }
}
