//! `cargo xtask fonts`: regenerates the built-in fonts of `twine-assets`.

use crate::util::R;

/// Regenerates the fonts.
#[allow(clippy::unnecessary_wraps)] // uniform command signature
pub fn run() -> R {
    // NOTE(P05.S04): implemented in that step.
    println!("fonts: available from P05.S04");
    Ok(())
}
