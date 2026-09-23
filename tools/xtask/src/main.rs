//! Build automation for the Twine workspace (`cargo xtask <command>`).
//!
//! Every check that CI runs is reachable from here; `cargo xtask ci` runs them all. See
//! `docs/design/10-simulator-testing.md` §5 and `docs/plan/README.md`.

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
    /// Run everything CI runs (fmt, clippy, tests, todo-check, layers, nostd, docs, miri, snapshots, firmware).
    Ci {
        /// Skip the slow stages (nostd, doc, miri, firmware).
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
    /// Verify crate layering against `docs/design/01-architecture.md` §2.
    Layers,
    /// Run snapshot tests.
    Snapshots {
        /// Rewrite reference snapshots (review the diffs first).
        #[arg(long)]
        update: bool,
    },
    /// Run the tests of crates under Miri (nightly) to detect undefined behaviour.
    Miri {
        /// Crates to check (default: twine-core).
        #[arg(name = "crate")]
        crates: Vec<String>,
    },
    /// Run a simulator example.
    Sim {
        /// Example name (`examples/src/bin/<example>.rs`).
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
    /// Run every simulator example headless (with `examples/scripts/<name>.twinescript` if present).
    SimSmoke,
    /// Build firmware crates (all boards, or one).
    Firmware {
        /// Board crate name in `firmware/`.
        board: Option<String>,
    },
    /// Per-crate line coverage with thresholds (needs cargo-llvm-cov).
    Coverage,
    /// Run benchmarks.
    Bench {
        /// Save the results as the new baseline.
        #[arg(long)]
        save_baseline: bool,
    },
    /// Regenerate the built-in fonts.
    Fonts,
    /// Regenerate `docs/plan/PROGRESS.md` from the phase files.
    Progress,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Ci { quick, only } => cmd::ci::run(quick, &only),
        Cmd::Nostd => cmd::nostd::run(),
        Cmd::TodoCheck => cmd::todo::run(),
        Cmd::Layers => cmd::layers::run(),
        Cmd::Snapshots { update } => cmd::snapshots::run(update),
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
        Cmd::Firmware { board } => cmd::firmware::run(board.as_deref()),
        Cmd::Coverage => cmd::coverage::run(),
        Cmd::Bench { save_baseline } => cmd::bench::run(save_baseline),
        Cmd::Fonts => cmd::fonts::run(),
        Cmd::Progress => cmd::progress::run(),
    };
    if let Err(e) = result {
        eprintln!("\x1b[31merror:\x1b[0m {e}");
        std::process::exit(1);
    }
}
