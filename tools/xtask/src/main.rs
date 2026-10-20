//! Build automation for the Twine workspace (`cargo xtask <command>`).
//!
//! Every check that CI runs is reachable from here; `cargo xtask ci` runs them all.

mod cmd;
mod util;

use clap::{Parser, Subcommand};

/// Twine workspace automation.
#[derive(Debug, Parser)]
#[command(name = "cargo xtask", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Available commands.
#[derive(Debug, Subcommand)]
enum Cmd {
    /// Run everything CI runs (fmt, clippy, the safety profile, tests, todo-check, layers, fonts,
    /// images, style-props, nostd, docs, bench build, miri, snapshots, firmware).
    Ci {
        /// Skip the slow stages (nostd, safety-exclusions, stack-budget, doc, bench-build, miri,
        /// firmware).
        #[arg(long)]
        quick: bool,
        /// Run only the named stage(s) (repeatable), e.g. `--only clippy --only clippy-all-features`.
        #[arg(long)]
        only: Vec<String>,
    },
    /// Build every `no_std` crate for every embedded target.
    Nostd,
    /// Fail on untracked work markers; only step-tagged NOTE markers are allowed.
    TodoCheck,
    /// Verify crate layering (the layer table lives in `tools/xtask/src/cmd/layers.rs`).
    Layers,
    /// Run the snapshot tests against the golden-image manifests (`tests/snapshots.toml`) and
    /// report the run; the flags change the manifests (see `xtask::cmd::snapshots`).
    Snapshots {
        /// Approve every rendered image as the reference (review the diffs first).
        #[arg(long, conflicts_with_all = ["propose", "accept", "check"])]
        update: bool,
        /// With `--update`: remove the entries no test used.
        #[arg(long, requires = "update")]
        prune: bool,
        /// Add images without an entry to the manifests' `[pending]` block (awaiting review).
        #[arg(long, conflicts_with_all = ["accept", "check"])]
        propose: bool,
        /// Approve every pending entry (no test run).
        #[arg(long, conflicts_with = "check")]
        accept: bool,
        /// Only check that every manifest parses and has no pending entry (no test run).
        #[arg(long)]
        check: bool,
    },
    /// Run the tests of crates under Miri (nightly) to detect undefined behaviour.
    Miri {
        /// Crates to check (default: twine-core).
        #[arg(name = "crate")]
        crates: Vec<String>,
    },
    /// Run a simulator example.
    Sim {
        /// Example name (a Cargo example of `twine`: `crates/twine/examples/<example>.rs` or
        /// `<example>/main.rs`; or `eg_simulator`).
        example: String,
        /// Build in release mode.
        #[arg(long)]
        release: bool,
        /// Run without a window (`TWINE_SIM_HEADLESS=1`); PNGs go to
        /// `target/twine-sim/headless/<example>/`.
        #[arg(long)]
        headless: bool,
        /// `.twinescript` to execute in headless mode (implies `--headless`).
        #[arg(long)]
        script: Option<std::path::PathBuf>,
        /// Arguments forwarded to the example (after `--`).
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run every simulator example headless (with
    /// `crates/twine/examples/scripts/<name>.twinescript` if present).
    SimSmoke,
    /// Build the example firmware in `firmware/` (all, or one) and print flash/RAM sizes.
    Firmware {
        /// Example directory name in `firmware/` (`all` or omitted: every example).
        example: Option<String>,
        /// Fail instead of skipping an example whose target or toolchain is missing.
        #[arg(long)]
        strict: bool,
        /// Build only this feature set: the defaults with each listed feature replacing the
        /// default of its group (`panel-*`/`oled-*`, `touch-*`, `demo-*`).
        #[arg(long)]
        features: Option<String>,
    },
    /// Per-crate line coverage with thresholds (needs cargo-llvm-cov); `--mcdc`: MC/DC of the
    /// core crates (nightly with llvm-tools; falls back to line coverage without it).
    Coverage {
        /// MC/DC, branch and line coverage of core, reactive, engine, render, layout and style.
        #[arg(long)]
        mcdc: bool,
    },
    /// Run the renderer benchmarks (criterion; `--iai`: instruction counts vs the baseline).
    Bench {
        /// Run the iai-callgrind benches (Linux + valgrind) and compare with the baseline.
        #[arg(long)]
        iai: bool,
        /// With `--iai`: save the results as the new baseline.
        #[arg(long)]
        save_baseline: bool,
    },
    /// Regenerate the built-in fonts from `assets/fonts/fonts.toml`.
    Fonts {
        /// Only verify that the generated files are up to date.
        #[arg(long)]
        check: bool,
    },
    /// Draw the sample images in `assets/images/` (the Twine logo, gallery samples).
    GenAssets {
        /// Only verify that the files are up to date.
        #[arg(long)]
        check: bool,
    },
    /// Convert the images of `assets/images/images.toml` into `crates/twine/examples/assets/`.
    Images {
        /// Only verify that the generated files are up to date.
        #[arg(long)]
        check: bool,
    },
    /// Write the style property table (`crates/twine-style/PROPERTIES.md`) from the property
    /// metadata.
    StyleProps {
        /// Only verify that the file is up to date.
        #[arg(long)]
        check: bool,
    },
    /// Run one fuzz target of `fuzz/` (needs cargo-fuzz and nightly).
    Fuzz {
        /// Target name, e.g. `fuzz_png`.
        target: String,
        /// Seconds to run.
        #[arg(long, default_value_t = 60)]
        time: u32,
    },
    /// Count the remaining sites of the safety profile's lints per crate and check them against
    /// the status table of `docs/safety/profile.md`.
    SafetyLints {
        /// Rewrite the status table (review the change).
        #[arg(long)]
        update: bool,
    },
    /// Write the safety profile's panic inventory (`docs/safety/panics.md`): every reachable
    /// panic site of the profile crates' library code.
    Panics {
        /// Only verify that the file is up to date and every explicit site is justified.
        #[arg(long)]
        check: bool,
    },
    /// Measure the stack the engine's recursive tree walks need per tree level (release build of
    /// `twine-engine/tests/stack_depth.rs`) and check the worst case at the reference depth
    /// against the budget (`docs/safety/profile.md` § 10.4).
    StackBudget {
        /// Rewrite the measured table in `docs/safety/profile.md` (review the change).
        #[arg(long)]
        update: bool,
    },
    /// Write the requirements → tests matrix `docs/safety/traceability.md` from
    /// `docs/safety/requirements.md` and the tests' `req:` doc lines.
    Traceability {
        /// Only verify that the matrix is up to date and every requirement is verified.
        #[arg(long)]
        check: bool,
    },
    /// Report the API evolution plan's progress from its §5 checklist and check it against the
    /// plan's steps (`docs/plan/api-evolution.md`; succeeds with a note when the plan is absent).
    Progress,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Ci { quick, only } => cmd::ci::run(quick, &only),
        Cmd::Nostd => cmd::nostd::run(),
        Cmd::TodoCheck => cmd::todo::run(),
        Cmd::Layers => cmd::layers::run(),
        Cmd::Snapshots {
            update,
            prune,
            propose,
            accept,
            check,
        } => cmd::snapshots::run(if update {
            cmd::snapshots::Action::Update { prune }
        } else if propose {
            cmd::snapshots::Action::Propose
        } else if accept {
            cmd::snapshots::Action::Accept
        } else if check {
            cmd::snapshots::Action::Verify
        } else {
            cmd::snapshots::Action::Check
        }),
        Cmd::Miri { crates } => cmd::miri::run(&crates),
        Cmd::Sim {
            example,
            release,
            headless,
            script,
            args,
        } => cmd::sim::run(
            &example,
            &cmd::sim::SimOptions {
                release,
                headless: headless || script.is_some(),
                script,
            },
            &args,
        ),
        Cmd::SimSmoke => cmd::sim::smoke(),
        Cmd::Firmware {
            example,
            strict,
            features,
        } => cmd::firmware::run(example.as_deref(), strict, features.as_deref()),
        Cmd::Coverage { mcdc } => cmd::coverage::run(mcdc),
        Cmd::Bench { iai, save_baseline } => cmd::bench::run(iai, save_baseline),
        Cmd::Fonts { check } => cmd::fonts::run(check),
        Cmd::GenAssets { check } => cmd::images::gen_assets(check),
        Cmd::Images { check } => cmd::images::run(check),
        Cmd::StyleProps { check } => cmd::style_props::run(check),
        Cmd::Fuzz { target, time } => cmd::fuzz::run(&target, time),
        Cmd::Progress => cmd::progress::run(),
        Cmd::Traceability { check } => cmd::traceability::run(check),
        Cmd::SafetyLints { update } => cmd::safety::run_lint_status(update),
        Cmd::Panics { check } => cmd::panics::run(check),
        Cmd::StackBudget { update } => cmd::stack::run(update),
    };
    if let Err(e) = result {
        eprintln!("\x1b[31merror:\x1b[0m {e}");
        std::process::exit(1);
    }
}
