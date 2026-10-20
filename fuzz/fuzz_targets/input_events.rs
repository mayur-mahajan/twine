//! Fuzzes random input-event sequences (pointer, keys, encoder, waits, pointer health) on a screen of widgets: the tree invariants and a live focus must hold and nothing may panic.
//! The interpreter is `twine_testing::fuzz::run_input_ops` (the property test `twine-testing/tests/fuzz_*.rs` runs it on stable).
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_testing::fuzz::{decode_input_ops, run_input_ops};

fuzz_target!(|data: &[u8]| {
    let _ = run_input_ops(&decode_input_ops(data));
});
