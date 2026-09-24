//! Instruction-count benchmarks of style resolution (iai-callgrind, Linux + valgrind only). On
//! other platforms this bench is an empty program.
//!
//! Run with `cargo bench -p twine-style --bench style_iai` (needs `iai-callgrind-runner` of the
//! same version on `PATH`).
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
mod linux {
    use iai_callgrind::{library_benchmark, library_benchmark_group};

    use super::common;

    #[library_benchmark]
    fn resolve_bg_color_3_entries() -> u32 {
        let t = common::three_entries();
        let mut n = 0;
        for _ in 0..100 {
            n += u32::from(common::resolve_bg(&t, 0).as_color().is_some());
        }
        n
    }

    #[library_benchmark]
    fn resolve_text_color_inherited_depth_5() -> u32 {
        let t = common::depth_5();
        let mut n = 0;
        for _ in 0..100 {
            n += u32::from(common::resolve_text(&t, 5).as_color().is_some());
        }
        n
    }

    #[library_benchmark]
    fn resolve_missing_prop_10_entries_group_skip() -> u32 {
        let t = common::ten_entries();
        let mut n = 0;
        for _ in 0..100 {
            n += u32::from(common::resolve_bg(&t, 0).as_color().is_some());
        }
        n
    }

    #[library_benchmark]
    fn stylebuf_set_20_props() -> usize {
        common::set_20_props().len()
    }

    library_benchmark_group!(
        name = style;
        benchmarks = resolve_bg_color_3_entries, resolve_text_color_inherited_depth_5,
            resolve_missing_prop_10_entries_group_skip, stylebuf_set_20_props
    );
}

#[cfg(target_os = "linux")]
use linux::style;

#[cfg(target_os = "linux")]
iai_callgrind::main!(library_benchmark_groups = style);

#[cfg(not(target_os = "linux"))]
fn main() {}
