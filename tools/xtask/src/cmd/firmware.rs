//! `cargo xtask firmware [board]`: builds the firmware crates in `firmware/`.

use crate::util::R;

/// Builds firmware for `board` (all boards when `None`).
#[allow(clippy::unnecessary_wraps)] // uniform command signature
pub fn run(board: Option<&str>) -> R {
    // NOTE(P17.S06): implemented in that step (skips boards whose toolchain is missing, with a warning).
    let _ = board;
    println!("firmware: available from P17.S06");
    Ok(())
}
