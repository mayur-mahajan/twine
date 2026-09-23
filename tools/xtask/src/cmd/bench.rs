//! `cargo xtask bench [--save-baseline]`: runs the benchmarks.

use crate::util::R;

/// Runs the benchmarks, optionally saving a new baseline.
#[allow(clippy::unnecessary_wraps)] // uniform command signature
pub fn run(save_baseline: bool) -> R {
    // NOTE(P03.S10): implemented in that step.
    let _ = save_baseline;
    println!("bench: available from P03.S10");
    Ok(())
}
