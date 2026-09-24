//! Criterion benchmarks of [`twine_anim::Timeline::tick`]: 1000 running animations (every
//! value changes each tick), and 1000 animations of which only 10 are running (the others are
//! paused).
//!
//! Run with `cargo bench -p twine-anim --bench timeline`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use core::any::Any;

use criterion::{Criterion, criterion_group, criterion_main};
use twine_anim::{Anim, AnimProp, AnimTarget, Easing, Repeat, TickSink, Timeline};
use twine_core::{Duration, Instant};

/// Sums the applied values (so the work is not optimized away).
struct Sum(i64);

impl TickSink for Sum {
    fn apply(&mut self, _target: AnimTarget, value: i32) {
        self.0 += i64::from(value);
    }
    fn ctx(&mut self) -> &mut dyn Any {
        self
    }
}

fn timeline(n: u32, running: u32) -> Timeline {
    let mut tl = Timeline::new();
    for i in 0..n {
        let id = tl.add(
            Anim::new(0, 10_000)
                .duration(Duration::ms(1000 + u64::from(i)))
                .easing(if i % 2 == 0 {
                    Easing::EaseInOut
                } else {
                    Easing::Linear
                })
                .playback(Duration::ms(1000))
                .repeat(Repeat::Infinite)
                .target(AnimTarget::Node(i, AnimProp::X)),
            Instant::ZERO,
        );
        if i >= running {
            tl.pause(id, Instant::ZERO);
        }
    }
    tl
}

fn tick(c: &mut Criterion, name: &str, mut tl: Timeline) {
    let mut sink = Sum(0);
    let mut t = Instant::ZERO;
    c.bench_function(name, |b| {
        b.iter(|| {
            t += Duration::ms(16);
            tl.tick(t, &mut sink)
        });
    });
    assert!(sink.0 > 0);
}

fn tick_1000_anims(c: &mut Criterion) {
    tick(c, "timeline_tick_1000_anims", timeline(1000, 1000));
}

fn tick_1000_anims_10_running(c: &mut Criterion) {
    tick(c, "timeline_tick_1000_anims_10_running", timeline(1000, 10));
}

criterion_group!(benches, tick_1000_anims, tick_1000_anims_10_running);
criterion_main!(benches);
