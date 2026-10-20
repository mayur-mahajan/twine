//! Fuzzes random reactive operation sequences (scopes, signals, memos, effects, writes, batches, reads; live and disposed handles) against a model of the runtime.
//! The interpreter is `twine_testing::fuzz::run_reactive_ops` (the property test `twine-testing/tests/fuzz_*.rs` runs it on stable).
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_testing::fuzz::{decode_reactive_ops, run_reactive_ops};

fuzz_target!(|data: &[u8]| {
    let _ = run_reactive_ops(&decode_reactive_ops(data));
});
