//! `cargo xtask nostd`: builds every `no_std` crate for every embedded target (P4, P11), and
//! the facade in the safety profile for every target ([`NOSTD_SAFETY_SETS`]).

use crate::cmd::layers::LAYERS;
use crate::util::{R, cargo_ci, has_target, run as run_cmd};

/// Layered crates that only ever build for the host: the std-only simulator and test kit, and
/// the proc-macro crate (proc macros run inside the compiler, never on a target).
pub const HOST_CRATES: &[&str] = &["twine-sim", "twine-testing", "twine-macros"];

/// The crates `nostd` builds: every crate of the layer table ([`LAYERS`]) except the
/// [`HOST_CRATES`], so a new layered crate is built for the embedded targets by default.
pub fn nostd_crates() -> impl Iterator<Item = &'static str> {
    LAYERS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !HOST_CRATES.contains(n))
}

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
    ("twine-hal", "platform-embassy"),
    ("twine-drivers", "all"),
    ("twine-drivers", "all,async"),
    ("twine-drivers", "all,defmt,async"),
    ("twine-drivers", "all,log"),
    ("twine-drivers", "ili9341,xpt2046,async"),
    ("twine-drivers", "co5300,ft6x36,async"),
    ("twine-drivers", "ssd1306,i2c,encoder"),
    ("twine-view", "async"),
    // Facade panel features: the driver and its colour feature together (R3.S06).
    ("twine", "log,drivers-ili9341,drivers-ssd1306,drivers-ili9488"),
    ("twine", "async,drivers-co5300"),
    // Facade features forwarded to every member (R3.S10): firmware-like sets with `defmt` /
    // `log`, the embassy run loop, touch and bus drivers, and each font bundle.
    (
        "twine",
        "defmt,embassy,drivers-st7789,drivers-ft6x36,drivers-spi-dma,fonts-latin-small",
    ),
    (
        "twine",
        "log,embassy,drivers-ssd1306,drivers-i2c,drivers-xpt2046,fonts-mono",
    ),
    ("twine", "fonts-latin-medium"),
    ("twine", "fonts-latin-all"),
    ("twine", "fonts-rtl,fonts-cjk"),
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
    ("twine-assets", "fonts-all"),
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
/// (platform implementations for one CPU architecture; runtime TrueType fonts use `f32` and
/// `static_cell`, which needs CAS atomics: FPU targets).
pub const NOSTD_TARGET_FEATURE_SETS: &[(&str, &str, &str)] = &[
    // Architecture-specific platforms (R3.S01), on every target of their architecture.
    ("twine-hal", "platform-cortex-m", "thumbv6m-none-eabi"),
    ("twine-hal", "platform-cortex-m", "thumbv7em-none-eabihf"),
    ("twine-hal", "platform-cortex-m", "thumbv8m.main-none-eabihf"),
    ("twine-hal", "platform-riscv", "riscv32imc-unknown-none-elf"),
    // The same through the facade (R3.S10).
    ("twine", "platform-cortex-m,defmt", "thumbv6m-none-eabi"),
    ("twine", "platform-riscv,log", "riscv32imc-unknown-none-elf"),
    ("twine-text", "ttf,bidi,arabic-shaping", "thumbv7em-none-eabihf"),
    ("twine-text", "ttf", "thumbv8m.main-none-eabihf"),
    // DMA2D register access for the supported STM32 chips (Cortex-M4F / M7).
    ("twine-accel-stm32", "stm32f429zi", "thumbv7em-none-eabihf"),
    ("twine-accel-stm32", "stm32f746ng,defmt", "thumbv7em-none-eabihf"),
    ("twine-accel-stm32", "stm32h743zi,log", "thumbv7em-none-eabihf"),
];

/// Facade feature sets of the safety profile (`docs/safety/profile.md`), built for **every**
/// target with `--no-default-features` (the default `perf-monitor` is excluded by the profile):
/// a minimal firmware with `log`, and a full one with `defmt`, embassy, drivers, vector
/// graphics, SVG, the decoders and fonts the profile keeps, and right-to-left text.
pub const NOSTD_SAFETY_SETS: &[&str] = &[
    "safety,log,color-rgb565,img-qoi",
    "safety,defmt,embassy,drivers-st7789,drivers-ft6x36,drivers-spi-dma,drivers-ssd1306,\
     drivers-i2c,color-rgb565-swapped,color-i1,vector,svg,img-qoi,img-png,img-gif,img-bmp,\
     img-lz4,bidi,arabic-shaping,fonts-latin-small,fonts-mono",
];

/// Targets without atomic compare-and-swap. The library crates leave `portable-atomic`'s
/// fallback to the application; these builds pick the single-core one.
pub const NO_CAS_TARGETS: &[&str] = &["thumbv6m-none-eabi", "riscv32imc-unknown-none-elf"];

/// A `cargo` command for building for `target` (with the `portable-atomic` single-core cfg on
/// targets without compare-and-swap).
fn target_cargo(target: &str) -> std::process::Command {
    let mut cmd = cargo_ci();
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
        for c in nostd_crates() {
            cmd.args(["-p", c]);
        }
        run_cmd(&mut cmd)?;
        for (krate, features) in NOSTD_FEATURE_SETS {
            run_cmd(target_cargo(t).args(["build", "--target", t, "-p", krate, "--features", features]))?;
        }
        for features in NOSTD_SAFETY_SETS {
            run_cmd(target_cargo(t).args([
                "build",
                "--target",
                t,
                "-p",
                "twine",
                "--no-default-features",
                "--features",
                features,
            ]))?;
        }
    }
    for (krate, features, t) in NOSTD_TARGET_FEATURE_SETS {
        run_cmd(target_cargo(t).args(["build", "--target", t, "-p", krate, "--features", features]))?;
    }
    println!(
        "nostd: {} crates built for {} targets (and the safety profile: {} feature sets)",
        nostd_crates().count(),
        TARGETS.len(),
        NOSTD_SAFETY_SETS.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_crates_are_layered_crates() {
        for h in HOST_CRATES {
            assert!(
                LAYERS.iter().any(|(n, _)| n == h),
                "{h} is not in the layer table"
            );
        }
    }

    #[test]
    fn safety_sets_select_the_profile_without_excluded_parts() {
        for set in NOSTD_SAFETY_SETS {
            let features: Vec<&str> = set.split(',').collect();
            assert!(features.contains(&crate::cmd::safety::FEATURE), "{set}");
            for excluded in crate::cmd::safety::EXCLUDED_FEATURES {
                assert!(!features.contains(excluded), "{set} enables {excluded}");
            }
        }
    }

    #[test]
    fn nostd_crates_exclude_host_crates() {
        let crates: Vec<_> = nostd_crates().collect();
        assert_eq!(crates.len(), LAYERS.len() - HOST_CRATES.len());
        assert!(crates.contains(&"twine-core") && crates.contains(&"twine"));
        assert!(!crates.iter().any(|c| HOST_CRATES.contains(c)));
    }
}
