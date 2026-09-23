//! Criterion benchmarks of the software renderer (320 × 240, RGB565 and RGB565-swapped).
//!
//! Run with `cargo bench -p twine-render --bench render` or `cargo xtask bench`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[path = "common/scenarios.rs"]
mod scenarios;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use scenarios::{RotateChunk, SCENES, Target};
use twine_core::ColorFormat;

fn painter_scenes(c: &mut Criterion) {
    for scene in SCENES {
        for (suffix, format) in [("565", ColorFormat::Rgb565), ("565s", ColorFormat::Rgb565Swapped)] {
            let mut t = Target::new(format, scene);
            c.bench_function(&format!("{}_{suffix}", scene.name), |b| {
                b.iter(|| t.run(black_box(scene)));
            });
        }
    }
}

fn rotate(c: &mut Criterion) {
    let mut r = RotateChunk::new();
    c.bench_function("rotate_320x40_565", |b| b.iter(|| r.run()));
}

criterion_group!(benches, painter_scenes, rotate);
criterion_main!(benches);
