//! `cargo xtask gen-assets [--check]` draws the sample images in `assets/images/`
//! procedurally; `cargo xtask images [--check]` converts the images listed in
//! `assets/images/images.toml` into Rust sources in `examples/src/assets/` with the
//! `twine image` converter (called in-process).
//!
//! `--check` regenerates everything in memory and fails if a file differs or is missing.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Deserialize;
use twine_cli::image::{CompressArg, ImageOptions, generate, parse_format};

use crate::util::{R, workspace_root};

/// The image list, relative to the workspace root.
pub const IMAGES_TOML: &str = "assets/images/images.toml";
/// Directory of the procedurally drawn source images.
pub const ASSETS_DIR: &str = "assets/images";
/// Directory of the generated Rust sources.
pub const OUT_DIR: &str = "examples/src/assets";

/// One `[[image]]` entry of `images.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageEntry {
    /// Static name.
    pub name: String,
    /// Output file stem (`<file>.rs` + `<file>.bin`).
    pub file: String,
    /// Source image, relative to the workspace root.
    pub input: String,
    /// `twine image --format` value.
    pub format: String,
    /// `none` (default), `rle` or `lz4`.
    #[serde(default)]
    pub compress: Option<String>,
    /// Dithering.
    #[serde(default)]
    pub dither: bool,
    /// Premultiplied alpha.
    #[serde(default)]
    pub premultiply: bool,
}

#[derive(Debug, Deserialize)]
struct ImagesToml {
    image: Vec<ImageEntry>,
}

/// Parses `images.toml` text.
pub fn parse(text: &str) -> Result<Vec<ImageEntry>, Box<dyn std::error::Error>> {
    Ok(toml::from_str::<ImagesToml>(text)?.image)
}

impl ImageEntry {
    /// The converter options of this entry.
    pub fn options(&self) -> Result<ImageOptions, Box<dyn std::error::Error>> {
        let compress = match self.compress.as_deref().unwrap_or("none") {
            "none" => CompressArg::None,
            "rle" => CompressArg::Rle,
            "lz4" => CompressArg::Lz4,
            s => return Err(format!("{}: compress must be none/rle/lz4 (got {s:?})", self.name).into()),
        };
        Ok(ImageOptions {
            input: PathBuf::from(&self.input),
            format: parse_format(&self.format).map_err(|e| format!("{}: {e}", self.name))?,
            compress,
            dither: self.dither,
            premultiply: self.premultiply,
            name: self.name.clone(),
            out: PathBuf::from(OUT_DIR).join(format!("{}.rs", self.file)),
            crate_path: "twine_image".into(),
        })
    }
}

/// The generated `mod.rs` of the output directory.
fn module_file(entries: &[ImageEntry]) -> String {
    let mut s = String::from(
        "//! Images converted by `cargo xtask images` from `assets/images/images.toml`; do not edit.\n\n",
    );
    // Sorted like rustfmt orders module declarations.
    let mut sorted: Vec<&ImageEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.file.cmp(&b.file));
    for e in sorted {
        let _ = writeln!(s, "/// `{}`: `{}` as {}.", e.name, e.input, e.format);
        let _ = writeln!(s, "pub mod {};", e.file);
    }
    s
}

/// Writes `bytes` to `path` unless unchanged (or `check`); records stale paths.
fn update(path: PathBuf, bytes: &[u8], check: bool, stale: &mut Vec<String>) -> R {
    if std::fs::read(&path).ok().as_deref() != Some(bytes) {
        if !check {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, bytes)?;
            println!("images: wrote {}", path.display());
        }
        stale.push(path.display().to_string());
    }
    Ok(())
}

fn finish(what: &str, cmd: &str, stale: &[String], total: usize, check: bool) -> R {
    if stale.is_empty() {
        println!("{what}: {total} files up to date");
        Ok(())
    } else if check {
        Err(format!(
            "{what}: out of date (run `cargo xtask {cmd}`): {}",
            stale.join(", ")
        )
        .into())
    } else {
        println!("{what}: {} of {total} files regenerated", stale.len());
        Ok(())
    }
}

/// Regenerates (or with `check`, verifies) the procedurally drawn sample images.
pub fn gen_assets(check: bool) -> R {
    let root = workspace_root();
    let files = twine_cli::image::assets::all()?;
    let mut stale = Vec::new();
    for (name, bytes) in &files {
        update(root.join(ASSETS_DIR).join(name), bytes, check, &mut stale)?;
    }
    finish("gen-assets", "gen-assets", &stale, files.len(), check)
}

/// Regenerates (or with `check`, verifies) every converted image and the module file.
pub fn run(check: bool) -> R {
    let root = workspace_root();
    let entries = parse(&std::fs::read_to_string(root.join(IMAGES_TOML))?)?;
    let mut stale = Vec::new();
    println!("| image | format | compression | bytes |\n|---|---|---|---:|");
    for e in &entries {
        let opts = e.options()?;
        let g = generate(&opts, &root)
            .map_err(|err| format!("{}: {err} (run `cargo xtask gen-assets` first?)", e.name))?;
        let out = root.join(&opts.out);
        update(out.clone(), g.source.as_bytes(), check, &mut stale)?;
        update(out.with_file_name(&g.bin_name), &g.bin, check, &mut stale)?;
        println!(
            "| `{}` | {} | {} | {} |",
            e.name,
            e.format,
            opts.compress.name(),
            g.bin.len()
        );
    }
    update(
        root.join(OUT_DIR).join("mod.rs"),
        module_file(&entries).as_bytes(),
        check,
        &mut stale,
    )?;
    finish("images", "images", &stale, entries.len() * 2 + 1, check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_toml_parses_and_has_unique_names() {
        let entries = parse(&std::fs::read_to_string(workspace_root().join(IMAGES_TOML)).unwrap()).unwrap();
        let mut files: Vec<_> = entries.iter().map(|e| e.file.clone()).collect();
        files.sort();
        files.dedup();
        assert_eq!(files.len(), entries.len());
        for e in &entries {
            e.options().unwrap();
        }
        assert!(module_file(&entries).contains("pub mod logo_rgb565;"));
    }
}
