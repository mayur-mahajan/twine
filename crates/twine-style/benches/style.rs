//! Criterion benchmarks of style resolution (host targets: a single resolve with ≤ 4 entries
//! under 30 ns).
//!
//! Run with `cargo bench -p twine-style --bench style`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

mod common;

use criterion::{Criterion, criterion_group, criterion_main};

fn resolve_bg_color_3_entries(c: &mut Criterion) {
    let t = common::three_entries();
    c.bench_function("resolve_bg_color_3_entries", |b| {
        b.iter(|| common::resolve_bg(&t, 0));
    });
}

fn resolve_text_color_inherited_depth_5(c: &mut Criterion) {
    let t = common::depth_5();
    c.bench_function("resolve_text_color_inherited_depth_5", |b| {
        b.iter(|| common::resolve_text(&t, 5));
    });
}

fn resolve_missing_prop_10_entries_group_skip(c: &mut Criterion) {
    let t = common::ten_entries();
    c.bench_function("resolve_missing_prop_10_entries_group_skip", |b| {
        b.iter(|| common::resolve_bg(&t, 0));
    });
}

fn stylebuf_set_20_props(c: &mut Criterion) {
    c.bench_function("stylebuf_set_20_props", |b| b.iter(common::set_20_props));
}

criterion_group!(
    benches,
    resolve_bg_color_3_entries,
    resolve_text_color_inherited_depth_5,
    resolve_missing_prop_10_entries_group_skip,
    stylebuf_set_20_props
);
criterion_main!(benches);
