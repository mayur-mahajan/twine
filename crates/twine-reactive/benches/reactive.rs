//! Criterion benchmarks for every row of design 03 §5 (host). Results go to `docs/perf.md`.
//!
//! Run with `cargo bench -p twine-reactive --bench reactive`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use twine_reactive::{Memo, create_root};

/// `signal.get()` untracked (target < 20 ns).
fn get_untracked(c: &mut Criterion) {
    let cx = create_root();
    let s = cx.signal(1u32);
    c.bench_function("get_untracked", |b| b.iter(|| black_box(s).get_untracked()));
    cx.dispose();
}

/// `set` with one subscribed effect, including the flush (target < 150 ns).
fn set_with_one_effect(c: &mut Criterion) {
    let cx = create_root();
    let s = cx.signal(0u32);
    cx.effect(move || {
        black_box(s.get());
    });
    let mut i = 0u32;
    c.bench_function("set_with_one_effect", |b| {
        b.iter(|| {
            i = i.wrapping_add(1);
            s.set(black_box(i));
        });
    });
    cx.dispose();
}

/// A chain of 10 memos observed by an effect; one source change (target < 1 µs).
fn memo_chain_10(c: &mut Criterion) {
    let cx = create_root();
    let a = cx.signal(0u64);
    let mut last: Option<Memo<u64>> = None;
    for _ in 0..10 {
        let p = last;
        last = Some(cx.memo(move || p.map_or_else(|| a.get(), |p| p.get()) + 1));
    }
    let last = last.unwrap();
    cx.effect(move || {
        black_box(last.get());
    });
    let mut i = 0u64;
    c.bench_function("memo_chain_10", |b| {
        b.iter(|| {
            i += 1;
            a.set(black_box(i));
        });
    });
    cx.dispose();
}

/// Create and dispose a scope with 10 signals and 10 effects (target < 5 µs).
fn scope_10_signals_10_effects(c: &mut Criterion) {
    let root = create_root();
    c.bench_function("scope_10_signals_10_effects", |b| {
        b.iter(|| {
            let cx = root.child();
            for i in 0..10u32 {
                let s = cx.signal(i);
                cx.effect(move || {
                    black_box(s.get());
                });
            }
            cx.dispose();
        });
    });
    root.dispose();
}

criterion_group!(
    benches,
    get_untracked,
    set_with_one_effect,
    memo_chain_10,
    scope_10_signals_10_effects
);
criterion_main!(benches);
