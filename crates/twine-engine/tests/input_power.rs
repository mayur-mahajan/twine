//! Phase acceptance for input: an idle touch UI never wakes the CPU, a press wakes it only
//! while held, and the input path allocates nothing.

mod common;

use common::{clickable, white_screen};
use twine_core::{Duration, Point, Rect};
use twine_engine::{EventCode, EventFilter, EventResult, Wake};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn harness() -> EngineHarness {
    let mut h = EngineHarness::new(120, 120).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        for i in 0..4 {
            clickable(
                e,
                s,
                Rect::from_xywh(10 + (i % 2) * 55, 10 + (i / 2) * 55, 45, 45),
            );
        }
    });
    h.pointer_input();
    h.run_until_idle();
    h
}

#[test]
fn idle_touch_ui_never_wakes() {
    let mut h = harness();
    let (_, p) = h.pointer_input();
    let frames = h.last_frame().frame;
    // 10 s of simulated time, polled every 10 ms by a (hypothetical) scheduler.
    for _ in 0..1000 {
        h.clock().advance(Duration::ms(10));
        assert_eq!(h.update(), Wake::Idle);
        assert!(h.flushes().is_empty());
    }
    assert_eq!(p.reads(), 0);
    assert_eq!(h.last_frame().frame, frames, "no frames");
}

#[test]
fn press_wakes_only_during_press() {
    let mut h = harness();
    let rp = h.engine().config().read_period;
    h.press(Point::new(20, 20));
    // Follow the engine's wake requests for 1 s while held: never idle, never later than a
    // read period.
    let t0 = h.now();
    while h.now() < t0 + Duration::secs(1) {
        match h.update() {
            Wake::At(t) => {
                assert!(t <= h.now() + rp);
                h.clock().set(t.max(h.now()));
            }
            Wake::Now => {}
            Wake::Idle => panic!("idle while pressed"),
        }
    }
    h.release();
    let released = h.now();
    let mut last_wake = released;
    for _ in 0..100 {
        match h.update() {
            Wake::Idle => break,
            Wake::At(t) => {
                last_wake = t;
                h.clock().set(t.max(h.now()));
            }
            Wake::Now => {}
        }
    }
    assert!(
        last_wake.saturating_duration_since(released) <= rp,
        "wakes stop within a read period"
    );
    assert_eq!(h.update(), Wake::Idle);
}

#[test]
fn input_path_allocates_nothing() {
    let mut h = harness();
    let s = h.screen();
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let b = h.engine().tree().children(s).next().unwrap();
    let c = hits.clone();
    h.engine_mut()
        .add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |_, _| {
            c.set(c.get() + 1);
            EventResult::Continue
        });
    // Warm-up: every buffer reaches its steady size.
    for _ in 0..3 {
        h.tap(Point::new(20, 20));
        h.clock().advance(Duration::ms(500));
        h.run_until_idle();
    }
    let ((), stats) = count_allocs(|| {
        h.tap(Point::new(20, 20));
        h.clock().advance(Duration::ms(500));
        h.run_until_idle();
    });
    assert_eq!(hits.get(), 4);
    assert_eq!((stats.allocs, stats.reallocs), (0, 0), "{stats:?}");
}
