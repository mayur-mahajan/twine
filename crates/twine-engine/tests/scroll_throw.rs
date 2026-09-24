//! Momentum after a drag (LVGL `lv_indev_scroll_throw_handler`): decay, edges, elastic
//! return, `ScrollEnd`, idle afterwards and no allocation per throw step.

mod common;

use common::{EvLog, list, record, scroll_codes, white_screen};
use twine_core::{Duration, Point, Rect};
use twine_engine::{EventCode as C, NodeId, ObjFlags, Wake};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A 200×200 screen with a 100×100 list at (20, 20) of 10 rows of 40 px (300 px to scroll).
fn scene() -> (EngineHarness, NodeId, EvLog) {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        ids = Some(list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40).0);
    });
    h.run_until_idle();
    let cont = ids.unwrap();
    let log = EvLog::default();
    record(h.engine_mut(), cont, &log);
    (h, cont, log)
}

/// Presses at (70, `y`) and moves by `dy` per read `n` times (no release).
fn drag(h: &mut EngineHarness, y: i32, dy: i32, n: i32) {
    h.press(Point::new(70, y));
    for k in 1..=n {
        h.clock().advance(h.engine().config().read_period);
        h.move_to(Point::new(70, y + k * dy));
    }
}

/// Runs one read period at a time until the pointer no longer scrolls; returns the offsets
/// after each read.
fn throw_steps(h: &mut EngineHarness, cont: NodeId) -> Vec<i32> {
    let mut v = Vec::new();
    for _ in 0..200 {
        h.advance(h.engine().config().read_period);
        v.push(h.engine().scroll_offset(cont).y);
        if !h.engine().is_scrolling(cont) {
            break;
        }
    }
    v
}

#[test]
fn throw_continues_after_release_and_decays() {
    let (mut h, cont, log) = scene();
    drag(&mut h, 90, -10, 4);
    assert_eq!(h.engine().scroll_offset(cont).y, 40);
    h.release();
    assert_eq!(scroll_codes(&log), [C::ScrollBegin, C::ScrollThrowBegin]);
    let steps = throw_steps(&mut h, cont);
    let deltas: Vec<i32> = steps
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d != 0)
        .collect();
    assert!(deltas.len() > 5, "{deltas:?}");
    assert!(deltas.windows(2).all(|w| w[1] <= w[0]), "decaying: {deltas:?}");
    assert!(steps.last().copied().unwrap() > 40);
}

#[test]
fn throw_distance_is_deterministic() {
    // Released after four reads of -10 px each (30 ms apart): the throw vector is the sum of
    // the movements decayed by age (LVGL): -10 + -6 (30 ms) + -3 (60 ms) + 0 (90 ms) = -19.
    // Each read keeps 90 % (`scroll_throw` = 10): 17, 15, 13, 11, 9, 8, 7, 6, 5, 4, 3, 2, 1 →
    // 101 px after the release.
    let (mut h, cont, _) = scene();
    drag(&mut h, 90, -10, 4);
    h.release();
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 40 + 101);
}

#[test]
fn throw_stops_at_edge_without_elastic() {
    let (mut h, cont, _) = scene();
    h.engine_mut().set_flag(cont, ObjFlags::SCROLL_ELASTIC, false);
    h.engine_mut().scroll_to_y(cont, 200, false);
    drag(&mut h, 90, -20, 3);
    h.release();
    let steps = throw_steps(&mut h, cont);
    assert!(steps.iter().all(|y| *y <= 300), "{steps:?}");
    assert_eq!(h.engine().scroll_offset(cont).y, 300);
}

#[test]
fn elastic_overscroll_springs_back() {
    let (mut h, cont, log) = scene();
    h.engine_mut().scroll_to_y(cont, 250, false);
    log.borrow_mut().clear();
    drag(&mut h, 70, -20, 3);
    h.release();
    let steps = throw_steps(&mut h, cont);
    let max = steps.iter().copied().max().unwrap();
    assert!(max > 300, "overscrolled: {steps:?}");
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 300);
    // The drag's scroll, its throw end, and the animation back to the edge.
    assert_eq!(
        scroll_codes(&log),
        [
            C::ScrollBegin,
            C::ScrollThrowBegin,
            C::ScrollBegin,
            C::ScrollEnd,
            C::ScrollEnd
        ]
    );
}

#[test]
fn no_momentum_flag_stops_immediately() {
    let (mut h, cont, log) = scene();
    h.engine_mut().set_flag(cont, ObjFlags::SCROLL_MOMENTUM, false);
    drag(&mut h, 90, -10, 4);
    h.release();
    let y = h.engine().scroll_offset(cont).y;
    h.advance(h.engine().config().read_period);
    assert!(!h.engine().is_scrolling(cont));
    assert_eq!(h.engine().scroll_offset(cont).y, y);
    assert_eq!(
        scroll_codes(&log),
        [C::ScrollBegin, C::ScrollThrowBegin, C::ScrollEnd]
    );
}

#[test]
fn scroll_end_sent_once_after_throw() {
    let (mut h, cont, log) = scene();
    drag(&mut h, 90, -10, 4);
    h.release();
    h.run_until_idle();
    assert_eq!(
        scroll_codes(&log),
        [C::ScrollBegin, C::ScrollThrowBegin, C::ScrollEnd]
    );
    assert!(!h.engine().is_scrolling(cont));
}

#[test]
fn press_during_throw_stops_it() {
    let (mut h, cont, log) = scene();
    drag(&mut h, 90, -15, 4);
    h.release();
    h.advance(Duration::ms(60));
    assert!(h.engine().is_scrolling(cont));
    let y = h.engine().scroll_offset(cont).y;
    h.press(Point::new(70, 60));
    assert!(!h.engine().is_scrolling(cont));
    assert_eq!(h.engine().scroll_offset(cont).y, y);
    assert_eq!(
        scroll_codes(&log),
        [C::ScrollBegin, C::ScrollThrowBegin, C::ScrollEnd]
    );
    h.release();
}

#[test]
fn idle_after_throw() {
    let (mut h, cont, _) = scene();
    drag(&mut h, 90, -10, 4);
    assert!(matches!(h.release(), Wake::At(_)), "throw reads follow");
    h.run_until_idle();
    assert!(!h.engine().is_scrolling(cont));
    assert_eq!(h.engine().input_deadline(), None);
    h.assert_idle();
}

#[test]
fn throw_allocates_nothing() {
    let (mut h, cont, log) = scene();
    log.borrow_mut().reserve(1000);
    // Warm-up with a first throw.
    drag(&mut h, 90, -10, 3);
    h.release();
    h.run_until_idle();
    h.engine_mut().scroll_to_y(cont, 0, false);
    h.run_until_idle();
    drag(&mut h, 90, -12, 4);
    h.release();
    let ((), stats) = count_allocs(|| {
        for _ in 0..8 {
            h.advance(h.engine().config().read_period);
        }
    });
    assert!(h.engine().is_scrolling(cont), "still throwing");
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}
