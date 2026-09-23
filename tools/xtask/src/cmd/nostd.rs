//! `cargo xtask nostd`: builds every `no_std` crate for every embedded target (P4, P11).

use crate::util::{R, cargo, has_target, run as run_cmd};

/// Every layered crate except the std-only ones (`twine-sim`, `twine-testing`).
pub const NOSTD_CRATES: &[&str] = &[
    "twine-core",
    "twine-hal",
    "twine-reactive",
    "twine-anim",
    "twine-render",
    "twine-text",
    "twine-image",
    "twine-vector",
    "twine-fs",
    "twine-style",
    "twine-layout",
    "twine-engine",
    "twine-theme",
    "twine-widgets",
    "twine-widgets-ext",
    "twine-view",
    "twine-extra",
    "twine-lottie",
    "twine-assets",
    "twine",
    "twine-drivers",
    "twine-embassy",
    "twine-accel-stm32",
    "twine-embedded-graphics",
    "twine-demos",
];

/// Embedded targets from `rust-toolchain.toml`.
pub const TARGETS: &[&str] = &[
    "thumbv6m-none-eabi",
    "thumbv7em-none-eabihf",
    "thumbv8m.main-none-eabihf",
    "riscv32imc-unknown-none-elf",
];

/// Extra `(crate, features)` builds performed per target after the default-feature build.
pub const NOSTD_FEATURE_SETS: &[(&str, &str)] = &[
    ("twine-core", "log"),
    ("twine-core", "defmt"),
    ("twine-hal", "async"),
    ("twine-hal", "defmt"),
    ("twine-reactive", "log"),
    ("twine-reactive", "defmt"),
];

/// Runs all builds; fails on the first error or on a missing target.
pub fn run() -> R {
    let missing: Vec<_> = TARGETS.iter().filter(|t| !has_target(t)).collect();
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        return Err(format!(
            "nostd: missing rustup target(s): {list}\n  hint: `rustup target add {list}` \
             (rust-toolchain.toml normally installs them)"
        )
        .into());
    }
    for t in TARGETS {
        let mut cmd = cargo();
        cmd.args(["build", "--target", t]);
        for c in NOSTD_CRATES {
            cmd.args(["-p", c]);
        }
        run_cmd(&mut cmd)?;
        for (krate, features) in NOSTD_FEATURE_SETS {
            run_cmd(cargo().args(["build", "--target", t, "-p", krate, "--features", features]))?;
        }
    }
    println!(
        "nostd: {} crates built for {} targets",
        NOSTD_CRATES.len(),
        TARGETS.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::layers::LAYERS;

    #[test]
    fn nostd_crates_are_layered_crates_minus_std_ones() {
        let expected: Vec<_> = LAYERS
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| !matches!(*n, "twine-sim" | "twine-testing"))
            .collect();
        let mut a = expected.clone();
        let mut b = NOSTD_CRATES.to_vec();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
    }
}
