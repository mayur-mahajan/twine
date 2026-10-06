//! Criterion benchmarks of the keyed collections of `twine-view` (`for_each`, `virtual_list`):
//! the cost of one change of the items — the reconcile, the bindings it runs and the `Ui`
//! update around it — for an unchanged list, a few changed values, a structural change, and a
//! virtual list scrolled by one row. Each benchmark first prints the heap allocations of one
//! such change (counted by the global allocator) and checks the allocation-free cases.
//!
//! Run with `cargo bench -p twine-bench --bench collections`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use criterion::{Criterion, criterion_group, criterion_main};
use twine_reactive::{Channel, Signal};
use twine_testing::TestUi;
use twine_testing::alloc::{AllocStats, CountingAllocator, count_allocs};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// Rows of the `for_each` benchmarks.
const ROWS: u32 = 200;

/// What a change does to the items (sent through a channel, so it is applied inside the update
/// like a sensor task's message).
#[derive(Clone, Copy)]
enum Change {
    /// Notify without changing anything.
    Touch,
    /// Set the value of rows 50, 120 and 190.
    Values(i32),
    /// Move the first row to the end.
    Rotate,
}

/// A 320 × 240 UI with a column of `ROWS` labels, one per `(id, value)` item of a signal,
/// changed through the returned channel. Most rows are below the screen.
fn table() -> (TestUi, &'static Channel<Change, 4>) {
    let changes: &'static Channel<Change, 4> = TestUi::channel();
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        let items: Signal<Vec<(u32, i32)>> = cx.signal((0..ROWS).map(|i| (i, 0i32)).collect());
        cx.on_message(changes, move |c| match c {
            Change::Touch => items.update(|_| {}),
            Change::Values(k) => items.update(|v| {
                v[50].1 = k;
                v[120].1 = k;
                v[190].1 = k;
            }),
            Change::Rotate => items.update(|v| v.rotate_left(1)),
        });
        column(for_each(
            items,
            |(id, _)| *id,
            |_cx, item| label(text!("{}: {}", item.with(|i| i.0), item.with(|i| i.1))),
        ))
        .gap(0)
    });
    t.run_until_idle();
    (t, changes)
}

/// Sends `c` and runs the update that applies it.
fn apply(t: &mut TestUi, changes: &Channel<Change, 4>, c: Change) {
    let _ = changes.try_send(c);
    t.update();
}

/// Prints the allocations of one change.
fn report(name: &str, s: &AllocStats) {
    eprintln!("{name}: {} allocations, {} bytes per change", s.allocs, s.bytes);
}

fn for_each_unchanged_200(c: &mut Criterion) {
    let (mut t, changes) = table();
    // Warm up (the effect queues).
    for _ in 0..3 {
        apply(&mut t, changes, Change::Touch);
    }
    let ((), s) = count_allocs(|| apply(&mut t, changes, Change::Touch));
    report("for_each_unchanged_200", &s);
    assert_eq!(s.allocs, 0, "an unchanged list allocates nothing");
    c.bench_function("for_each_unchanged_200", |b| {
        b.iter(|| apply(&mut t, changes, Change::Touch));
    });
}

fn for_each_3_of_200_values(c: &mut Criterion) {
    let (mut t, changes) = table();
    let mut k = 0;
    let mut change = |t: &mut TestUi| {
        k = (k + 1) % 1000;
        apply(t, changes, Change::Values(k));
    };
    // Warm up the label buffers.
    for _ in 0..3 {
        change(&mut t);
    }
    let ((), s) = count_allocs(|| change(&mut t));
    report("for_each_3_of_200_values", &s);
    assert_eq!(s.allocs, 0, "changed values allocate nothing");
    c.bench_function("for_each_3_of_200_values", |b| b.iter(|| change(&mut t)));
}

fn for_each_rotate_200(c: &mut Criterion) {
    let (mut t, changes) = table();
    let ((), s) = count_allocs(|| apply(&mut t, changes, Change::Rotate));
    report("for_each_rotate_200 (one move)", &s);
    c.bench_function("for_each_rotate_200", |b| {
        b.iter(|| apply(&mut t, changes, Change::Rotate));
    });
}

fn virtual_list_scroll_row_10k(c: &mut Criterion) {
    let mut t = TestUi::new(320, 240).mount(|_cx| {
        let rows: Vec<u32> = (0..10_000).collect();
        virtual_list(rows, |i| *i, 30, |_cx, i| label(text!("Row {}", i.get())))
            .size(320, 240)
            .test_id("vl")
    });
    t.run_until_idle();
    let vl = t.find(twine_testing::by_id("vl")).id();
    let mut y = 0;
    let mut scroll = |t: &mut TestUi| {
        y = (y + 30) % (30 * 9_000);
        t.engine_mut().scroll_to_y(vl, y, false);
        t.update();
    };
    for _ in 0..10 {
        scroll(&mut t);
    }
    let ((), s) = count_allocs(|| scroll(&mut t));
    report("virtual_list_scroll_row_10k (one row in, one out)", &s);
    c.bench_function("virtual_list_scroll_row_10k", |b| b.iter(|| scroll(&mut t)));
}

criterion_group!(
    benches,
    for_each_unchanged_200,
    for_each_3_of_200_values,
    for_each_rotate_200,
    virtual_list_scroll_row_10k
);
criterion_main!(benches);
