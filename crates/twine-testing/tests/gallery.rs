//! The renderer's reference drawings (`twine_testing::gallery`).

use twine_core::{ColorFormat, Rect};
use twine_testing::RenderHarness;
use twine_testing::gallery::checkerboard;

/// A square size below 1 px is taken as 1 px: it was a divisor (row parity) and the loop step,
/// so 0 divided by zero and a negative size never ended (R5.S10).
/// req: SAF-033
#[test]
fn regression_checkerboard_zero_size_does_not_panic() {
    for size in [0, -3, 1] {
        let mut h = RenderHarness::new(8, 8, ColorFormat::Rgb565);
        h.paint(|p| checkerboard(p, Rect::from_xywh(0, 0, 8, 8), size));
    }
}
