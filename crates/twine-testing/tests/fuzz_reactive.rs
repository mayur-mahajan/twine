//! Random reactive operation sequences (R5.S08): scopes created and disposed, signals, memos and
//! effects created in live and disposed scopes, writes, batches and reads on live and disposed
//! handles — checked against a model after every operation (live counts, no pending effects,
//! memo values, effect runs, `Disposed` errors), and the runtime returns to its starting counts
//! when everything is disposed. The cargo-fuzz target `reactive_ops` runs the same interpreter.
//!
//! req: SAF-021

use proptest::prelude::*;
use twine_testing::fuzz::{ReactiveOp, decode_reactive_ops, run_reactive_ops};

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// req: SAF-021
    #[test]
    fn random_reactive_sequences_match_the_model(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        run_reactive_ops(&decode_reactive_ops(&bytes));
    }
}

/// Handles of a disposed scope answer `Disposed`; their effects never run again.
///
/// req: SAF-021, SAF-007
#[test]
fn disposed_scope_handles_are_dead() {
    run_reactive_ops(&[
        ReactiveOp::Scope { parent: 0 },
        ReactiveOp::Signal { scope: 1, value: 3 },
        ReactiveOp::Signal { scope: 0, value: 4 },
        ReactiveOp::Memo { scope: 1, a: 0, b: 1 },
        ReactiveOp::Effect { scope: 1, signal: 0 },
        ReactiveOp::Read { memo: 0 },
        ReactiveOp::Dispose { scope: 1 },
        ReactiveOp::Set { signal: 0, value: 9 },
        ReactiveOp::Read { memo: 0 },
        ReactiveOp::Batch {
            first: 0,
            count: 3,
            value: 1,
        },
        ReactiveOp::Signal { scope: 1, value: 1 },
    ]);
}
