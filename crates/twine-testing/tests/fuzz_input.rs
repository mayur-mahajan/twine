//! Random input-event sequences (R5.S08): pointer, keypad and encoder events, waits and pointer
//! health changes on a screen of real widgets keep the tree invariants and a live focus, panic
//! nowhere and raise no fault except `InputDevice` after a health change. The cargo-fuzz target
//! `input_events` runs the same interpreter.
//!
//! req: SAF-020

use proptest::prelude::*;
use twine_testing::fuzz::{InputOp, decode_input_ops, run_input_ops};

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// req: SAF-020
    #[test]
    fn random_input_sequences_keep_invariants(bytes in proptest::collection::vec(any::<u8>(), 0..160)) {
        run_input_ops(&decode_input_ops(&bytes));
    }
}

/// A failing pointer in the middle of a drag: the press is processed as released, the fault is
/// the only one, and the UI settles.
///
/// req: SAF-020, SAF-004
#[test]
fn pointer_failing_mid_drag_settles() {
    run_input_ops(&[
        InputOp::Press { x: 30, y: 75 },
        InputOp::Move { x: 60, y: 75 },
        InputOp::PointerHealth { health: 2 },
        InputOp::Move { x: 90, y: 75 },
        InputOp::Wait { ms: 200 },
        InputOp::PointerHealth { health: 0 },
        InputOp::Release,
    ]);
}
