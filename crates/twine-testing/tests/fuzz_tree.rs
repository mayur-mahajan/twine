//! Random widget-tree mutations (R5.S08): any sequence of creations, deletions, moves, flag,
//! state, size, layout and scroll changes and render steps — on live and stale node ids — keeps
//! the tree's structural invariants and the `Limits` bounds, panics nowhere and raises only
//! `Capacity` faults. The cargo-fuzz target `tree_ops` runs the same interpreter.
//!
//! req: SAF-019

use proptest::prelude::*;
use twine_testing::fuzz::{TreeOp, decode_tree_ops, run_tree_ops};

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// req: SAF-019
    #[test]
    fn random_tree_mutations_keep_invariants(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = run_tree_ops(&decode_tree_ops(&bytes));
    }
}

/// The bounds are reached: a long chain hits `max_tree_depth` and many creations hit
/// `max_nodes`, both refused with a `Capacity` fault and nothing else.
///
/// req: SAF-008, SAF-019
#[test]
fn bounds_are_reached_and_refused() {
    let chain: Vec<TreeOp> = (0..12u8).map(|i| TreeOp::Create { parent: i }).collect();
    let run = run_tree_ops(&chain);
    assert!(run.capacity_faults >= 1, "the depth bound refuses deep nodes");
    let wide = vec![TreeOp::Create { parent: 0 }; 80];
    let run = run_tree_ops(&wide);
    // 48 nodes: the screen and 44 objects counted here, plus the bottom, top and system layers.
    assert_eq!(run.alive, 45, "the node bound holds");
    assert!(run.capacity_faults >= 1);
}

/// Operations on deleted nodes are ignored (P7).
///
/// req: SAF-019
#[test]
fn stale_ids_are_ignored() {
    let ops = [
        TreeOp::Create { parent: 0 },
        TreeOp::Create { parent: 1 },
        TreeOp::Delete { node: 1 },
        TreeOp::Move { node: 2, parent: 0 },
        TreeOp::Size {
            node: 2,
            w: 10,
            h: -3,
        },
        TreeOp::Flag {
            node: 1,
            flag: 0,
            on: true,
        },
        TreeOp::Step { ms: 16 },
    ];
    assert_eq!(run_tree_ops(&ops).alive, 1);
}
