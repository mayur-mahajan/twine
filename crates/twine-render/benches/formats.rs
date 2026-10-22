//! Criterion benchmarks of the software renderer in the draw-buffer formats the `render` bench
//! does not cover (320 × 240: RGB888, XRGB8888, ARGB8888, L8, I1), over the same scenes.
//!
//! Run with `cargo bench -p twine-render --bench formats`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[allow(dead_code)] // `RotateChunk` is used by the `render` bench only
#[path = "common/scenarios.rs"]
mod scenarios;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use scenarios::{SCENES, Target};
use twine_core::ColorFormat;

fn painter_scenes(c: &mut Criterion) {
    for (suffix, format) in [
        ("888", ColorFormat::Rgb888),
        ("x8888", ColorFormat::Xrgb8888),
        ("a8888", ColorFormat::Argb8888),
        ("l8", ColorFormat::L8),
        ("i1", ColorFormat::I1),
    ] {
        for scene in SCENES {
            let mut t = Target::new(format, scene);
            c.bench_function(&format!("{}_{suffix}", scene.name), |b| {
                b.iter(|| t.run(black_box(scene)));
            });
        }
    }
}

criterion_group!(benches, painter_scenes);
criterion_main!(benches);
