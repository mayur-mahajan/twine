//! The timeline and timers do not allocate per tick once warmed up.

use core::any::Any;

use twine_anim::{Anim, AnimProp, AnimTarget, Easing, Repeat, TickSink, Timeline, Timers};
use twine_core::{Duration, Instant};
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

struct Sink {
    applied: u64,
}

impl TickSink for Sink {
    fn apply(&mut self, _: AnimTarget, _: i32) {
        self.applied += 1;
    }
    fn ctx(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn tick_is_allocation_free_in_steady_state() {
    let mut tl = Timeline::new();
    let t0 = Instant::ZERO;
    for k in 0..50u32 {
        let a = Anim::new(0, 1000)
            .duration(Duration::ms(300 + u64::from(k)))
            .easing(if k % 2 == 0 {
                Easing::EaseInOut
            } else {
                Easing::Overshoot
            })
            .playback(Duration::ms(200))
            .repeat(Repeat::Infinite)
            .target(AnimTarget::Node(k, AnimProp::X));
        tl.add(a, t0);
    }
    let mut sink = Sink { applied: 0 };
    tl.tick(t0, &mut sink); // warm-up
    let ((), stats) = count_allocs(|| {
        for frame in 1..=100u64 {
            let next = tl.tick(t0 + Duration::ms(frame * 16), &mut sink);
            assert!(next.is_some());
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert!(sink.applied > 50 * 90);
}

#[test]
fn slot_reuse_is_allocation_free() {
    let mut tl = Timeline::new();
    let mut sink = Sink { applied: 0 };
    let t0 = Instant::ZERO;
    let add_round = |tl: &mut Timeline, t: Instant| {
        for k in 0..20u32 {
            tl.add(
                Anim::new(0, 10)
                    .duration(Duration::ms(10))
                    .target(AnimTarget::Node(k, AnimProp::Opa)),
                t,
            );
        }
    };
    add_round(&mut tl, t0);
    tl.tick(t0 + Duration::ms(10), &mut sink);
    let ((), stats) = count_allocs(|| {
        for round in 1..10u64 {
            let t = t0 + Duration::ms(round * 100);
            add_round(&mut tl, t);
            tl.tick(t + Duration::ms(10), &mut sink);
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert_eq!(tl.running_count(), 0);
}

#[test]
fn timers_run_is_allocation_free() {
    let mut timers = Timers::new();
    let t0 = Instant::ZERO;
    for k in 1..=10u64 {
        timers.add(Duration::ms(k), t0, |cx| {
            *cx.ctx().downcast_mut::<u64>().unwrap() += 1;
        });
    }
    let mut fired = 0u64;
    let ((), stats) = count_allocs(|| {
        for ms in 1..=100 {
            timers.run(t0 + Duration::ms(ms), &mut fired);
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert!(fired > 250);
}
