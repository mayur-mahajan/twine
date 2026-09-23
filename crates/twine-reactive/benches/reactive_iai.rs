//! Instruction-count benchmarks (iai-callgrind, Linux + valgrind only) for the rows of design
//! 03 §5. On other platforms this bench is an empty program.
//!
//! Run with `cargo bench -p twine-reactive --bench reactive_iai` (needs `iai-callgrind-runner`
//! of the same version on `PATH`).
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[cfg(target_os = "linux")]
mod linux {
    use std::hint::black_box;

    use iai_callgrind::{library_benchmark, library_benchmark_group};
    use twine_reactive::{Memo, create_root};

    #[library_benchmark]
    fn get_untracked() -> u32 {
        let cx = create_root();
        let s = cx.signal(1u32);
        let mut sum = 0u32;
        for _ in 0..100 {
            sum = sum.wrapping_add(black_box(s).get_untracked());
        }
        sum
    }

    #[library_benchmark]
    fn set_with_one_effect() {
        let cx = create_root();
        let s = cx.signal(0u32);
        cx.effect(move || {
            black_box(s.get());
        });
        for i in 0..100 {
            s.set(black_box(i));
        }
    }

    #[library_benchmark]
    fn memo_chain_10() {
        let cx = create_root();
        let a = cx.signal(0u64);
        let mut last: Option<Memo<u64>> = None;
        for _ in 0..10 {
            let p = last;
            last = Some(cx.memo(move || p.map_or_else(|| a.get(), |p| p.get()) + 1));
        }
        let last = last.unwrap();
        cx.effect(move || {
            black_box(last.get());
        });
        for i in 0..100 {
            a.set(black_box(i));
        }
    }

    #[library_benchmark]
    fn scope_10_signals_10_effects() {
        let root = create_root();
        for _ in 0..10 {
            let cx = root.child();
            for i in 0..10u32 {
                let s = cx.signal(i);
                cx.effect(move || {
                    black_box(s.get());
                });
            }
            cx.dispose();
        }
    }

    library_benchmark_group!(
        name = reactive;
        benchmarks = get_untracked, set_with_one_effect, memo_chain_10, scope_10_signals_10_effects
    );
}

#[cfg(target_os = "linux")]
use linux::reactive;

#[cfg(target_os = "linux")]
iai_callgrind::main!(library_benchmark_groups = reactive);

#[cfg(not(target_os = "linux"))]
fn main() {}
