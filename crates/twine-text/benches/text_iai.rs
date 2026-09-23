//! Instruction-count benchmarks (iai-callgrind, Linux + valgrind only) of text layout and
//! drawing. On other platforms this bench is an empty program.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[cfg(target_os = "linux")]
#[path = "common/scenarios.rs"]
mod scenarios;

#[cfg(target_os = "linux")]
mod linux {
    use std::hint::black_box;

    use iai_callgrind::{library_benchmark, library_benchmark_group};

    use super::scenarios::{Target, layout_wrap_200, measure_label_20, paragraph};

    fn warm() -> Target {
        let mut t = Target::new();
        t.glyph_cache_hit_render();
        t
    }

    #[library_benchmark]
    #[bench::draw_50_glyphs_montserrat14(Target::new())]
    fn draw_50(mut t: Target) {
        t.draw_50_glyphs_montserrat14();
        black_box(&t.glyphs);
    }

    #[library_benchmark]
    #[bench::glyph_cache_hit_render(warm())]
    fn cache_hit(mut t: Target) {
        t.glyph_cache_hit_render();
        black_box(&t.glyphs);
    }

    #[library_benchmark]
    #[bench::layout_1kb_paragraph_wrap_200px(paragraph())]
    fn layout(p: String) -> usize {
        black_box(layout_wrap_200(&p))
    }

    #[library_benchmark]
    fn measure_label_20_chars() -> i32 {
        black_box(measure_label_20())
    }

    library_benchmark_group!(name = text; benchmarks = draw_50, cache_hit, layout, measure_label_20_chars);
}

#[cfg(target_os = "linux")]
use linux::text;

#[cfg(target_os = "linux")]
iai_callgrind::main!(library_benchmark_groups = text);

#[cfg(not(target_os = "linux"))]
fn main() {}
