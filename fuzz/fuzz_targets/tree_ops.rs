//! Fuzzes random widget-tree mutations (creation, deletion, moves, flags, states, sizes, layout, scrolling, render steps; live and stale ids): the tree invariants and bounds must hold and nothing may panic.
//! The interpreter is `twine_testing::fuzz::run_tree_ops` (the property test `twine-testing/tests/fuzz_*.rs` runs it on stable).
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_testing::fuzz::{decode_tree_ops, run_tree_ops};

fuzz_target!(|data: &[u8]| {
    let _ = run_tree_ops(&decode_tree_ops(data));
});
