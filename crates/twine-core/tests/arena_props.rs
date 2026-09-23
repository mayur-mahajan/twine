//! Model test: `Arena` against a `HashMap` with random insert/remove/get sequences.

use std::collections::HashMap;

use proptest::prelude::*;
use twine_core::{Arena, Id};

#[derive(Debug, Clone)]
enum Op {
    Insert(i32),
    /// Remove the n-th id ever issued (modulo count), possibly stale.
    Remove(usize),
    Get(usize),
    Set(usize, i32),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any::<i32>().prop_map(Op::Insert),
        2 => any::<usize>().prop_map(Op::Remove),
        2 => any::<usize>().prop_map(Op::Get),
        1 => (any::<usize>(), any::<i32>()).prop_map(|(i, v)| Op::Set(i, v)),
    ]
}

proptest! {
    #[test]
    fn arena_matches_hashmap_model(ops in prop::collection::vec(op(), 1..300)) {
        let mut arena = Arena::new();
        let mut model: HashMap<u32, i32> = HashMap::new();
        let mut issued: Vec<Id<i32>> = Vec::new();
        for op in ops {
            match op {
                Op::Insert(v) => {
                    let id = arena.insert(v).unwrap();
                    prop_assert!(!issued.contains(&id), "id {} reused", id);
                    issued.push(id);
                    model.insert(id.to_raw(), v);
                }
                Op::Remove(n) if !issued.is_empty() => {
                    let id = issued[n % issued.len()];
                    prop_assert_eq!(arena.remove(id), model.remove(&id.to_raw()));
                }
                Op::Get(n) if !issued.is_empty() => {
                    let id = issued[n % issued.len()];
                    prop_assert_eq!(arena.get(id), model.get(&id.to_raw()));
                    prop_assert_eq!(arena.contains(id), model.contains_key(&id.to_raw()));
                }
                Op::Set(n, v) if !issued.is_empty() => {
                    let id = issued[n % issued.len()];
                    if let Some(slot) = arena.get_mut(id) {
                        *slot = v;
                        model.insert(id.to_raw(), v);
                    } else {
                        prop_assert!(!model.contains_key(&id.to_raw()));
                    }
                }
                _ => {}
            }
            prop_assert_eq!(arena.len(), model.len());
        }
        let mut live: Vec<(u32, i32)> = arena.iter().map(|(id, v)| (id.to_raw(), *v)).collect();
        let mut expected: Vec<(u32, i32)> = model.into_iter().collect();
        live.sort_unstable();
        expected.sort_unstable();
        prop_assert_eq!(live, expected);
    }
}
