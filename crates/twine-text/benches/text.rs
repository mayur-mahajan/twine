//! Criterion benchmarks of text layout and drawing (RGB565, 320 × 240).
//!
//! Run with `cargo bench -p twine-text --bench text`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[path = "common/scenarios.rs"]
mod scenarios;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use scenarios::{Target, layout_wrap_200, measure_label_20, paragraph};

fn text(c: &mut Criterion) {
    let mut t = Target::new();
    c.bench_function("draw_50_glyphs_montserrat14", |b| {
        b.iter(|| t.draw_50_glyphs_montserrat14())
    });
    let para = paragraph();
    c.bench_function("layout_1kb_paragraph_wrap_200px", |b| {
        b.iter(|| layout_wrap_200(black_box(&para)))
    });
    c.bench_function("measure_label_20_chars", |b| b.iter(measure_label_20));
    let mut t = Target::new();
    t.glyph_cache_hit_render();
    c.bench_function("glyph_cache_hit_render", |b| {
        b.iter(|| t.glyph_cache_hit_render())
    });
}

criterion_group!(benches, text);
criterion_main!(benches);
