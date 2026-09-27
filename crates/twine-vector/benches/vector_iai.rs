//! Instruction-count benchmarks (iai-callgrind, Linux + valgrind only) of the vector scenes.
//! On other platforms this bench is an empty program.
//!
//! `cargo xtask bench --iai` runs it and compares against `benches/baseline/iai.json`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[cfg(target_os = "linux")]
#[path = "common/scenes.rs"]
mod scenes;

#[cfg(target_os = "linux")]
mod linux {
    use std::hint::black_box;

    use iai_callgrind::{library_benchmark, library_benchmark_group};

    use super::scenes::{Target, parse_icon, scenes};

    fn setup(i: usize) -> (Target, usize) {
        (Target::new(), i)
    }

    #[library_benchmark]
    #[bench::fill_circle_100(setup(0))]
    #[bench::stroke_polyline_200_round(setup(1))]
    #[bench::radial_gradient_200(setup(2))]
    fn draw((mut t, i): (Target, usize)) {
        t.draw(black_box(&scenes()[i]));
    }

    #[library_benchmark]
    fn svg_parse_icon() -> usize {
        black_box(parse_icon())
    }

    library_benchmark_group!(name = vector; benchmarks = draw, svg_parse_icon);
}

#[cfg(target_os = "linux")]
use linux::vector;

#[cfg(target_os = "linux")]
iai_callgrind::main!(library_benchmark_groups = vector);

#[cfg(not(target_os = "linux"))]
fn main() {}
