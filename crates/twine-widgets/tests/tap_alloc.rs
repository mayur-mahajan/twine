//! Allocation audit of input with the default theme: once warmed up, tapping a themed button
//! (press and release style transitions, events, redraws) and pressing a button of a button
//! matrix allocate nothing (P4).

mod common;

use common::{Mode, harness};
use twine_core::{Duration, Point};
use twine_engine::NodeId;
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_testing::alloc::{AllocStats, CountingAllocator, count_allocs};
use twine_widgets::{button, buttonmatrix, label};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn center(h: &EngineHarness, n: NodeId) -> Point {
    let c = h.engine().coords(n);
    Point::new((c.x0 + c.x1) / 2, (c.y0 + c.y1) / 2)
}

/// Presses at `p`, lets the press transition run, releases and lets the release transition
/// run until the engine is idle.
fn tap_and_settle(h: &mut EngineHarness, p: Point) {
    h.press(p);
    h.advance(Duration::ms(300));
    h.release();
    h.advance(Duration::ms(300));
    h.run_until_idle();
}

fn assert_no_allocs(stats: AllocStats, what: &str) {
    assert_eq!(
        stats.allocs + stats.reallocs,
        0,
        "{what} allocated after warm-up: {stats:?}"
    );
}

#[test]
fn themed_button_tap_allocates_nothing_after_warm_up() {
    for mode in Mode::ALL {
        let mut h = harness(160, 80, mode);
        let screen = h.screen();
        let e = h.engine_mut();
        let b = button::create(e, screen).unwrap();
        e.align(b, Align::Center, 0, 0);
        let l = label::create_with(e, b, "Button").unwrap();
        e.align(l, Align::Center, 0, 0);
        h.run_until_idle();
        let p = center(&h, b);
        // Warm-up: every buffer (transition styles, animation slots, queues) reaches its size.
        for _ in 0..2 {
            tap_and_settle(&mut h, p);
        }
        let ((), stats) = count_allocs(|| tap_and_settle(&mut h, p));
        assert_eq!(h.engine().transition_count(), 0);
        assert_no_allocs(stats, "a themed button tap");
    }
}

#[test]
fn buttonmatrix_press_allocates_nothing_after_warm_up() {
    let mut h = harness(260, 140, Mode::Light);
    let screen = h.screen();
    let m = buttonmatrix::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let c = h.engine().coords(m);
    let p = Point::new(c.x0 + 30, c.y0 + 30);
    for _ in 0..2 {
        tap_and_settle(&mut h, p);
    }
    let ((), stats) = count_allocs(|| tap_and_settle(&mut h, p));
    assert_no_allocs(stats, "a button matrix press");
}
