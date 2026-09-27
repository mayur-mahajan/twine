//! Criterion benchmarks of vector drawing (320 × 240 RGB565, warm caches) and SVG parsing.
//!
//! Run with `cargo bench -p twine-vector --bench vector` or `cargo xtask bench`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[path = "common/scenes.rs"]
mod scenes;

use criterion::{Criterion, criterion_group, criterion_main};
use scenes::{Target, parse_icon, scenes};

fn draw(c: &mut Criterion) {
    let mut t = Target::new();
    for s in scenes() {
        c.bench_function(s.name, |b| b.iter(|| t.draw(&s)));
    }
}

fn parse(c: &mut Criterion) {
    c.bench_function("svg_parse_icon", |b| b.iter(parse_icon));
}

criterion_group!(benches, draw, parse);
criterion_main!(benches);
