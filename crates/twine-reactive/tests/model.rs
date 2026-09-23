//! P07.S08: model-based test of the reactive graph against a naive recompute-everything model.
//!
//! Random programs of up to 60 operations run over up to 8 signals, 8 memos (each the sum of a
//! random subset of earlier signals/memos) and 8 initial effects (each recording the values it
//! reads). After every top-level operation the test asserts that
//! - every memo equals the model,
//! - every live effect ran at most once per flush; it ran if one of its signal dependencies was
//!   written or one of its memo dependencies changed value, and otherwise not (memo
//!   short-circuit) — except that a memo read *inside* a batch may already have propagated an
//!   intermediate change, which allows one extra run; its last recorded values equal the model,
//! - disposed effects never run again.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use proptest::prelude::*;
use proptest::sample::Index;
use twine_reactive::{EffectId, Memo, Signal, batch, create_root};

#[derive(Debug, Clone)]
enum Step {
    Set(Index, i64),
    ReadMemo(Index),
    DisposeEffect(Index),
}

#[derive(Debug, Clone)]
enum Op {
    Step(Step),
    Batch(Vec<Step>),
    CreateEffect(Vec<Index>),
}

#[derive(Debug, Clone)]
struct Program {
    signals: Vec<i64>,
    memo_deps: Vec<Vec<Index>>,
    effect_deps: Vec<Vec<Index>>,
    ops: Vec<Op>,
}

fn value() -> impl Strategy<Value = i64> {
    // A small range so that writes of an unchanged value and unchanged memo sums are common.
    0i64..4
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (any::<Index>(), value()).prop_map(|(i, v)| Step::Set(i, v)),
        1 => any::<Index>().prop_map(Step::ReadMemo),
        1 => any::<Index>().prop_map(Step::DisposeEffect),
    ]
}

fn deps() -> impl Strategy<Value = Vec<Index>> {
    prop::collection::vec(any::<Index>(), 1..=3)
}

fn program() -> impl Strategy<Value = Program> {
    let op = prop_oneof![
        6 => step().prop_map(Op::Step),
        2 => prop::collection::vec(step(), 0..6).prop_map(Op::Batch),
        1 => deps().prop_map(Op::CreateEffect),
    ];
    (
        prop::collection::vec(value(), 1..=8),
        prop::collection::vec(deps(), 0..=8),
        prop::collection::vec(deps(), 0..=8),
        prop::collection::vec(op, 0..=60),
    )
        .prop_map(|(signals, memo_deps, effect_deps, ops)| Program {
            signals,
            memo_deps,
            effect_deps,
            ops,
        })
}

/// A dependency: signal or memo index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dep {
    Sig(usize),
    Memo(usize),
}

/// Resolves random indices into distinct deps among `n_sig` signals and `n_memo` memos.
fn resolve(idx: &[Index], n_sig: usize, n_memo: usize) -> Vec<Dep> {
    let set: BTreeSet<Dep> = idx
        .iter()
        .map(|i| {
            let k = i.index(n_sig + n_memo);
            if k < n_sig {
                Dep::Sig(k)
            } else {
                Dep::Memo(k - n_sig)
            }
        })
        .collect();
    set.into_iter().collect()
}

#[derive(Clone, Copy)]
enum Handle {
    Sig(Signal<i64>),
    Memo(Memo<i64>),
}

impl Handle {
    fn get(self) -> i64 {
        match self {
            Handle::Sig(s) => s.get(),
            Handle::Memo(m) => m.get(),
        }
    }
}

struct Model {
    signals: Vec<i64>,
    memo_deps: Vec<Vec<Dep>>,
}

impl Model {
    fn value(&self, d: Dep) -> i64 {
        match d {
            Dep::Sig(i) => self.signals[i],
            Dep::Memo(m) => self.memo_deps[m].iter().map(|&d| self.value(d)).sum(),
        }
    }
}

struct TestEffect {
    id: EffectId,
    deps: Vec<Dep>,
    runs: Rc<Cell<u32>>,
    last: Rc<RefCell<Vec<i64>>>,
    alive: bool,
}

struct World {
    model: Model,
    signals: Vec<Signal<i64>>,
    memos: Vec<Memo<i64>>,
    effects: Vec<TestEffect>,
    cx: twine_reactive::Scope,
}

impl World {
    fn handle(&self, d: Dep) -> Handle {
        match d {
            Dep::Sig(i) => Handle::Sig(self.signals[i]),
            Dep::Memo(m) => Handle::Memo(self.memos[m]),
        }
    }

    fn create_effect(&mut self, deps: Vec<Dep>) {
        let handles: Vec<Handle> = deps.iter().map(|&d| self.handle(d)).collect();
        let runs = Rc::new(Cell::new(0));
        let last = Rc::new(RefCell::new(Vec::new()));
        let (r, l) = (runs.clone(), last.clone());
        let id = self.cx.effect(move || {
            let vals: Vec<i64> = handles.iter().map(|h| h.get()).collect();
            r.set(r.get() + 1);
            *l.borrow_mut() = vals;
        });
        self.effects.push(TestEffect {
            id,
            deps,
            runs,
            last,
            alive: true,
        });
    }

    fn step(&mut self, s: &Step, written: &mut BTreeSet<usize>) -> Result<(), TestCaseError> {
        match s {
            Step::Set(i, v) => {
                let i = i.index(self.signals.len());
                self.model.signals[i] = *v;
                written.insert(i);
                self.signals[i].set(*v);
            }
            Step::ReadMemo(k) => {
                if !self.memos.is_empty() {
                    let k = k.index(self.memos.len());
                    prop_assert_eq!(self.memos[k].get_untracked(), self.model.value(Dep::Memo(k)));
                }
            }
            Step::DisposeEffect(j) => {
                if !self.effects.is_empty() {
                    let j = j.index(self.effects.len());
                    self.effects[j].id.dispose();
                    self.effects[j].alive = false;
                }
            }
        }
        Ok(())
    }

    /// Checks memos and effects after one top-level operation.
    fn check(
        &self,
        runs_before: &[u32],
        last_before: &[Vec<i64>],
        written: &BTreeSet<usize>,
        memo_read_in_batch: bool,
    ) -> Result<(), TestCaseError> {
        for (m, memo) in self.memos.iter().enumerate() {
            prop_assert_eq!(memo.get_untracked(), self.model.value(Dep::Memo(m)), "memo {}", m);
        }
        for (j, e) in self.effects.iter().enumerate() {
            let before = runs_before.get(j).copied().unwrap_or(0);
            let ran = e.runs.get() - before;
            if j >= runs_before.len() {
                prop_assert_eq!(ran, 1, "new effect {} runs once on creation", j);
            } else if !e.alive {
                prop_assert!(!e.id.is_alive() && ran == 0, "disposed effect {} ran", j);
                continue;
            } else {
                let model_now: Vec<i64> = e.deps.iter().map(|&d| self.model.value(d)).collect();
                let expected = e
                    .deps
                    .iter()
                    .zip(&last_before[j])
                    .zip(&model_now)
                    .any(|((d, old), new)| match d {
                        Dep::Sig(i) => written.contains(i),
                        Dep::Memo(_) => old != new,
                    });
                prop_assert!(ran <= 1, "effect {} ran {} times in one flush", j, ran);
                if expected || !memo_read_in_batch {
                    prop_assert_eq!(ran, u32::from(expected), "effect {} run count", j);
                }
            }
            let model_now: Vec<i64> = e.deps.iter().map(|&d| self.model.value(d)).collect();
            prop_assert_eq!(&*e.last.borrow(), &model_now, "effect {} values", j);
        }
        Ok(())
    }
}

fn run(p: &Program) -> Result<(), TestCaseError> {
    twine_reactive::reset();
    let cx = create_root();
    let n_sig = p.signals.len();
    let memo_deps: Vec<Vec<Dep>> = p
        .memo_deps
        .iter()
        .enumerate()
        .map(|(m, idx)| resolve(idx, n_sig, m))
        .collect();
    let mut w = World {
        model: Model {
            signals: p.signals.clone(),
            memo_deps: memo_deps.clone(),
        },
        signals: p.signals.iter().map(|&v| cx.signal(v)).collect(),
        memos: Vec::new(),
        effects: Vec::new(),
        cx,
    };
    for deps in &memo_deps {
        let handles: Vec<Handle> = deps.iter().map(|&d| w.handle(d)).collect();
        w.memos
            .push(cx.memo(move || handles.iter().map(|h| h.get()).sum()));
    }
    let n_memo = w.memos.len();
    for idx in &p.effect_deps {
        w.create_effect(resolve(idx, n_sig, n_memo));
    }
    w.check(&[], &[], &BTreeSet::new(), false)?;
    for op in &p.ops {
        let runs_before: Vec<u32> = w.effects.iter().map(|e| e.runs.get()).collect();
        let last_before: Vec<Vec<i64>> = w.effects.iter().map(|e| e.last.borrow().clone()).collect();
        let mut written = BTreeSet::new();
        match op {
            Op::Step(s) => w.step(s, &mut written)?,
            Op::Batch(steps) => {
                let mut res = Ok(());
                let w_ref = &mut w;
                batch(|| {
                    for s in steps {
                        res = w_ref.step(s, &mut written);
                        if res.is_err() {
                            break;
                        }
                    }
                });
                res?;
            }
            Op::CreateEffect(idx) => {
                if w.effects.len() < 16 {
                    w.create_effect(resolve(idx, n_sig, n_memo));
                }
            }
        }
        let memo_read_in_batch = matches!(op, Op::Batch(steps)
            if steps.iter().any(|s| matches!(s, Step::ReadMemo(_))));
        w.check(&runs_before, &last_before, &written, memo_read_in_batch)?;
    }
    cx.dispose();
    let st = twine_reactive::debug_stats();
    prop_assert_eq!((st.nodes, st.scopes, st.pending, st.deferred), (0, 0, 0, 0));
    Ok(())
}

proptest! {
    #[test]
    fn model_random_programs_match_naive_model(p in program()) {
        run(&p)?;
    }
}
