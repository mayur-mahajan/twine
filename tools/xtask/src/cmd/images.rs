//! `cargo xtask gen-assets [--check]` draws the sample images in `assets/images/`
//! procedurally. (Examples, tests and demos convert them while they build:
//! `twine::include_image!` or `twine_build::image::Image` in a build script.)
//!
//! `--check` regenerates everything in memory and fails if a file differs or is missing.

use std::path::Path;

use crate::util::{R, workspace_root};

/// Directory of the procedurally drawn source images.
pub const ASSETS_DIR: &str = "assets/images";

/// Writes `bytes` to `path` unless unchanged (or `check`); records stale paths.
fn update(path: &Path, bytes: &[u8], check: bool, stale: &mut Vec<String>) -> R {
    if std::fs::read(path).ok().as_deref() != Some(bytes) {
        if !check {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, bytes)?;
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
        update(&root.join(ASSETS_DIR).join(name), bytes, check, &mut stale)?;
    }
    finish("gen-assets", "gen-assets", &stale, files.len(), check)
}
