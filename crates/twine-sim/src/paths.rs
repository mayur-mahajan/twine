//! Output locations of the simulator (`target/twine-sim/…`).

use std::path::{Path, PathBuf};

/// The cargo target directory: `CARGO_TARGET_DIR`, else the ancestor of the running executable
/// whose parent holds `Cargo.lock` (`<workspace>/target`), else `./target`.
#[must_use]
pub fn target_dir() -> PathBuf {
    if let Some(t) = std::env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(t);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.ancestors()
                .find(|a| a.parent().is_some_and(|p| p.join("Cargo.lock").is_file()))
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from("target"))
}

/// `<target>/twine-sim`: screenshots, recordings and headless output.
#[must_use]
pub fn sim_dir() -> PathBuf {
    target_dir().join("twine-sim")
}

/// The file stem of the running executable (the example name), or `"sim"`.
#[must_use]
pub fn exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "sim".into())
}
