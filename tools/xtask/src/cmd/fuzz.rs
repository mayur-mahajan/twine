//! `cargo xtask fuzz <target> [--time <s>]`: runs one cargo-fuzz target of `fuzz/` (nightly).

use std::process::Command;

use crate::util::{R, run as run_cmd, workspace_root};

/// The fuzz targets in `fuzz/fuzz_targets/`.
pub const TARGETS: &[&str] = &[
    "fuzz_qoi",
    "fuzz_png",
    "fuzz_jpeg",
    "fuzz_gif",
    "fuzz_bmp",
    "fuzz_rle",
    "fuzz_lz4",
    "svg_parse",
    "lottie_load",
];

/// Runs `target` for `secs` seconds.
pub fn run(target: &str, secs: u32) -> R {
    if !TARGETS.contains(&target) {
        return Err(format!("fuzz: unknown target `{target}`; targets: {}", TARGETS.join(", ")).into());
    }
    let has_fuzz = Command::new("cargo")
        .args(["fuzz", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_fuzz {
        return Err(
            "fuzz: cargo-fuzz is not installed (`cargo install cargo-fuzz`; needs a nightly toolchain)"
                .into(),
        );
    }
    run_cmd(
        Command::new("cargo")
            .current_dir(workspace_root().join("fuzz"))
            .args(["+nightly", "fuzz", "run", target, "--"])
            .arg(format!("-max_total_time={secs}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_has_a_source_file() {
        for t in TARGETS {
            assert!(
                workspace_root()
                    .join("fuzz/fuzz_targets")
                    .join(format!("{t}.rs"))
                    .is_file(),
                "{t}"
            );
        }
    }
}
