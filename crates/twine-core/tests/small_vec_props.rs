//! Model test: `SmallVec` behaves like `Vec` for random operation sequences.

use proptest::prelude::*;
use twine_core::SmallVec;

#[derive(Debug, Clone)]
enum Op {
    Push(u8),
    Pop,
    Insert(usize, u8),
    Remove(usize),
    SwapRemove(usize),
    RetainEven,
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => any::<u8>().prop_map(Op::Push),
        2 => Just(Op::Pop),
        2 => (0usize..12, any::<u8>()).prop_map(|(i, v)| Op::Insert(i, v)),
        2 => (0usize..12).prop_map(Op::Remove),
        1 => (0usize..12).prop_map(Op::SwapRemove),
        1 => Just(Op::RetainEven),
        1 => Just(Op::Clear),
    ]
}

proptest! {
    #[test]
    fn behaves_like_vec(ops in prop::collection::vec(op(), 0..200)) {
        let mut sv: SmallVec<u8, 4> = SmallVec::new();
        let mut v: Vec<u8> = Vec::new();
        for op in ops {
            match op {
                Op::Push(x) => {
                    sv.push(x);
                    v.push(x);
                }
                Op::Pop => prop_assert_eq!(sv.pop(), v.pop()),
                Op::Insert(i, x) => {
                    sv.insert(i, x);
                    if i <= v.len() {
                        v.insert(i, x);
                    }
                }
                Op::Remove(i) => {
                    let got = sv.remove(i);
                    let exp = if i < v.len() { v.remove(i) } else { 0 };
                    prop_assert_eq!(got, exp);
                }
                Op::SwapRemove(i) => {
                    let got = sv.swap_remove(i);
                    let exp = if i < v.len() { v.swap_remove(i) } else { 0 };
                    prop_assert_eq!(got, exp);
                }
                Op::RetainEven => {
                    sv.retain(|x| x % 2 == 0);
                    v.retain(|x| x % 2 == 0);
                }
                Op::Clear => {
                    sv.clear();
                    v.clear();
                }
            }
            prop_assert_eq!(sv.as_slice(), v.as_slice());
            prop_assert_eq!(sv.len(), v.len());
        }
    }
}
