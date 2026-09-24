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
    ("twine-drivers", "all"),
    ("twine-drivers", "all,async"),
    ("twine-drivers", "all,defmt,async"),
    ("twine-drivers", "all,log"),
    ("twine-drivers", "ili9341,xpt2046,async"),
    ("twine-drivers", "co5300,ft6x36,async"),
    ("twine-drivers", "ssd1306,i2c,encoder"),
    ("twine-view", "async"),
    ("twine-embassy", "defmt"),
    ("twine-embassy", "log"),
    ("twine-demos", "async"),
    ("twine-demos", "async,defmt"),
    ("twine-reactive", "log"),
    ("twine-reactive", "defmt"),
    ("twine-anim", "log"),
    ("twine-anim", "defmt"),
    ("twine-style", "log"),
    ("twine-style", "defmt"),
    ("twine-layout", "log"),
    ("twine-layout", "defmt"),
    (
        "twine-render",
        "log,color-rgb565,color-rgb565-swapped,color-rgb888,color-xrgb8888,color-argb8888,color-l8,color-i1",
    ),
    ("twine-render", "defmt,color-rgb565"),
    ("twine-text", "log"),
    ("twine-text", "defmt"),
    ("twine-text", "log,bidi,arabic-shaping"),
    ("twine-fs", "log,fs-fat"),
    ("twine-fs", "defmt,fs-fat"),
    ("twine-assets", "all-fonts"),
    (
        "twine-image",
        "log,img-qoi,img-png,img-jpeg,img-bmp,img-gif,img-lz4",
    ),
    ("twine-image", "defmt,img-qoi,img-lz4"),
    ("twine-vector", "log,svg"),
    ("twine-vector", "defmt,svg"),
    (
        "twine-engine",
        "log,debug-checks,perf-monitor,test-ids,color-rgb565,color-rgb565-swapped,color-l8,color-i1",
    ),
    ("twine-engine", "defmt,color-rgb565,perf-monitor"),
    ("twine-theme", "log"),
    ("twine-theme", "defmt"),
    ("twine-widgets", "log"),
    ("twine-widgets", "defmt"),
    ("twine-view", "log"),
    ("twine-view", "defmt"),
    ("twine-extra", "log,qrcode,barcode"),
    ("twine-extra", "defmt,qrcode,barcode"),
    ("twine-lottie", "log"),
    ("twine-lottie", "defmt"),
    ("twine-accel-stm32", "log"),
    ("twine-embedded-graphics", "drawtarget,log"),
    ("twine-embedded-graphics", "drawtarget,defmt"),
];

/// `(crate, features, target)` builds for features that only make sense on some targets
/// (runtime TrueType fonts use `f32` and `static_cell`, which needs CAS atomics: FPU targets).
pub const NOSTD_TARGET_FEATURE_SETS: &[(&str, &str, &str)] = &[
    ("twine-text", "ttf,bidi,arabic-shaping", "thumbv7em-none-eabihf"),
    ("twine-text", "ttf", "thumbv8m.main-none-eabihf"),
    // DMA2D register access for the supported STM32 chips (Cortex-M4F / M7).
    ("twine-accel-stm32", "stm32f429zi", "thumbv7em-none-eabihf"),
    ("twine-accel-stm32", "stm32f746ng,defmt", "thumbv7em-none-eabihf"),
    ("twine-accel-stm32", "stm32h743zi,log", "thumbv7em-none-eabihf"),
];

/// Targets without atomic compare-and-swap. The library crates leave `portable-atomic`'s
/// fallback to the application; these builds pick the single-core one.
pub const NO_CAS_TARGETS: &[&str] = &["thumbv6m-none-eabi", "riscv32imc-unknown-none-elf"];

/// A `cargo` command for building for `target` (with the `portable-atomic` single-core cfg on
/// targets without compare-and-swap).
fn target_cargo(target: &str) -> std::process::Command {
    let mut cmd = cargo();
    if NO_CAS_TARGETS.contains(&target) {
        let var = format!(
            "CARGO_TARGET_{}_RUSTFLAGS",
            target.replace(['-', '.'], "_").to_ascii_uppercase()
        );
        cmd.env(var, "--cfg portable_atomic_unsafe_assume_single_core");
    }
    cmd
}

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
        let mut cmd = target_cargo(t);
        cmd.args(["build", "--target", t]);
        for c in NOSTD_CRATES {
            cmd.args(["-p", c]);
        }
        run_cmd(&mut cmd)?;
        for (krate, features) in NOSTD_FEATURE_SETS {
            run_cmd(target_cargo(t).args(["build", "--target", t, "-p", krate, "--features", features]))?;
        }
    }
    for (krate, features, t) in NOSTD_TARGET_FEATURE_SETS {
        run_cmd(target_cargo(t).args(["build", "--target", t, "-p", krate, "--features", features]))?;
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
