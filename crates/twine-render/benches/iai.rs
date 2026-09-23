//! Instruction-count benchmarks (iai-callgrind, Linux + valgrind only) of the renderer
//! scenarios in RGB565. On other platforms this bench is an empty program.
//!
//! `cargo xtask bench --iai` runs it and compares against `benches/baseline/iai.json`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[cfg(target_os = "linux")]
#[path = "common/scenarios.rs"]
mod scenarios;

#[cfg(target_os = "linux")]
mod linux {
    use std::hint::black_box;

    use iai_callgrind::{library_benchmark, library_benchmark_group};
    use twine_core::ColorFormat;

    use super::scenarios::{RotateChunk, SCENES, Target};

    fn setup(i: usize) -> (Target, usize) {
        (Target::new(ColorFormat::Rgb565, &SCENES[i]), i)
    }

    #[library_benchmark]
    #[bench::fill_fullscreen_565(setup(0))]
    #[bench::rounded_rect_r16_border_fullscreen_565(setup(1))]
    #[bench::shadow_100_w20_warm_565(setup(2))]
    #[bench::gradient_ver_fullscreen_565(setup(3))]
    #[bench::gradient_radial_200_565(setup(4))]
    #[bench::masked_fill_radius_fade_565(setup(5))]
    #[bench::layer_opa_200x200_565(setup(6))]
    #[bench::line_diag_w5_x100_565(setup(7))]
    #[bench::arc_spinner_60px_565(setup(8))]
    #[bench::polygon_star_200_565(setup(9))]
    #[bench::blit_rotate_100x100_aa_565(setup(10))]
    #[bench::layer_rotate_card_120_565(setup(11))]
    #[bench::rotate_100x100_aa_565(setup(12))]
    #[bench::blit_rgb565_same_format_320x240_565(setup(13))]
    #[bench::blit_argb8888_on_rgb565_100x100_565(setup(14))]
    #[bench::blit_rgb565a8_100x100_565(setup(15))]
    fn scene((mut t, i): (Target, usize)) {
        t.run(black_box(&SCENES[i]));
    }

    #[library_benchmark]
    #[bench::rotate_320x40_565(RotateChunk::new())]
    fn rotate(mut r: RotateChunk) {
        r.run();
        black_box(&r);
    }

    library_benchmark_group!(name = render; benchmarks = scene, rotate);
}

#[cfg(target_os = "linux")]
use linux::render;

#[cfg(target_os = "linux")]
iai_callgrind::main!(library_benchmark_groups = render);

#[cfg(not(target_os = "linux"))]
fn main() {}
