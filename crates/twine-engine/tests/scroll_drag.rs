//! Scrolling by dragging (LVGL `lv_indev_scroll.c`): scroll target search with chaining,
//! direction lock, press handling, elastic edges, precise invalidation and no allocation.

mod common;

use common::{EvLog, codes_of, list, record, scroll_codes, white_screen};
use twine_core::{Duration, Point, Rect};
use twine_engine::{Dir, EventCode as C, NodeId, ObjFlags, SCROLL_ELASTIC_FACTOR, State};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A 200×200 screen with a 100×100 list at (20, 20) of 10 rows of 40 px. The log records the
/// list and every row.
fn scene() -> (EngineHarness, NodeId, Vec<NodeId>, EvLog) {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        ids = Some(list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40));
    });
    h.run_until_idle();
    let (cont, rows) = ids.unwrap();
    let log = EvLog::default();
    record(h.engine_mut(), cont, &log);
    for &r in &rows {
        record(h.engine_mut(), r, &log);
    }
    (h, cont, rows, log)
}

/// Moves the pressed pointer to `p` one read period later.
fn step_to(h: &mut EngineHarness, p: Point) {
    h.clock().advance(h.engine().config().read_period);
    h.move_to(p);
}

fn state(h: &EngineHarness, n: NodeId) -> State {
    h.engine().tree().node(n).unwrap().state()
}

#[test]
fn vertical_drag_scrolls_list() {
    let (mut h, cont, rows, log) = scene();
    h.press(Point::new(70, 80));
    step_to(&mut h, Point::new(70, 75));
    assert_eq!(h.engine().scroll_offset(cont).y, 0, "below the scroll limit");
    // The movement since the press reaches 10 px: scrolling starts with this read's movement.
    step_to(&mut h, Point::new(70, 70));
    assert_eq!(h.engine().scroll_offset(cont).y, 5);
    step_to(&mut h, Point::new(70, 40));
    assert_eq!(h.engine().scroll_offset(cont).y, 35);
    assert_eq!(h.engine().coords(rows[0]).y0, 20 - 35);
    assert!(h.engine().is_scrolling(cont));
    assert_eq!(scroll_codes(&log), [C::ScrollBegin]);
}

#[test]
fn click_suppressed_after_scroll() {
    let (mut h, _, rows, log) = scene();
    h.drag(Point::new(70, 80), Point::new(70, 20), Duration::ms(120));
    let row = codes_of(&log, rows[1]);
    assert!(row.contains(&C::Pressed) && row.contains(&C::Released), "{row:?}");
    assert!(!row.contains(&C::ShortClicked), "{row:?}");
    assert!(!row.contains(&C::Clicked), "{row:?}");
}

#[test]
fn short_drag_below_limit_clicks() {
    let (mut h, cont, rows, log) = scene();
    h.drag(Point::new(70, 80), Point::new(70, 75), Duration::ms(60));
    assert_eq!(h.engine().scroll_offset(cont).y, 0);
    let row = codes_of(&log, rows[1]);
    assert!(
        row.contains(&C::ShortClicked) && row.contains(&C::Clicked),
        "{row:?}"
    );
    assert!(scroll_codes(&log).is_empty());
}

#[test]
fn pressed_child_loses_pressed_state_when_scroll_starts() {
    let (mut h, _, rows, log) = scene();
    h.press(Point::new(70, 80));
    assert!(state(&h, rows[1]).contains(State::PRESSED));
    step_to(&mut h, Point::new(70, 68));
    assert!(!state(&h, rows[1]).contains(State::PRESSED));
    // LVGL removes the state without `PressLost`; the row keeps getting `Pressing`.
    let row = codes_of(&log, rows[1]);
    assert!(!row.contains(&C::PressLost), "{row:?}");
    step_to(&mut h, Point::new(70, 50));
    assert!(!state(&h, rows[1]).contains(State::PRESSED));
    assert_eq!(codes_of(&log, rows[1]).last(), Some(&C::Pressing));
}

#[test]
fn scrolled_state_set_during_drag() {
    let (mut h, cont, _, log) = scene();
    h.press(Point::new(70, 80));
    step_to(&mut h, Point::new(70, 60));
    assert!(state(&h, cont).contains(State::SCROLLED));
    step_to(&mut h, Point::new(70, 50));
    h.release();
    assert!(state(&h, cont).contains(State::SCROLLED), "the throw goes on");
    h.run_until_idle();
    assert!(!state(&h, cont).contains(State::SCROLLED));
    assert!(!h.engine().is_scrolling(cont));
    assert_eq!(
        scroll_codes(&log),
        [C::ScrollBegin, C::ScrollThrowBegin, C::ScrollEnd]
    );
}

#[test]
fn horizontal_drag_in_vertical_only_container_chains_to_parent() {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        // A horizontally scrollable pane (content 400 px wide) holding a vertical list.
        let (pane, _) = list(e, s, Rect::from_xywh(0, 0, 200, 200), 0, 0);
        let wide = twine_testing::scenes::child_box(e, pane, Rect::from_xywh(0, 0, 400, 10), &[]);
        let (inner, _) = list(e, pane, Rect::from_xywh(20, 20, 100, 100), 10, 40);
        e.set_scroll_dir(inner, Dir::VER);
        ids = Some((pane, wide, inner));
    });
    h.run_until_idle();
    let (pane, _, inner) = ids.unwrap();
    h.press(Point::new(100, 60));
    step_to(&mut h, Point::new(85, 60));
    step_to(&mut h, Point::new(55, 60));
    assert_eq!(h.engine().scroll_offset(inner), Point::ZERO);
    assert_eq!(h.engine().scroll_offset(pane), Point::new(45, 0));
    // Without the chain flag the search stops at the list.
    h.release();
    h.run_until_idle();
    h.engine_mut().scroll_to_x(pane, 0, false);
    h.engine_mut().set_flag(inner, ObjFlags::SCROLL_CHAIN_HOR, false);
    let before = h.engine().scroll_offset(pane);
    h.press(Point::new(100, 60));
    step_to(&mut h, Point::new(85, 60));
    step_to(&mut h, Point::new(55, 60));
    assert_eq!(h.engine().scroll_offset(pane), before);
}

#[test]
fn direction_locked_after_start() {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let (panel, _) = list(e, s, Rect::from_xywh(20, 20, 100, 100), 0, 0);
        let content = twine_testing::scenes::child_box(e, panel, Rect::from_xywh(0, 0, 400, 400), &[]);
        ids = Some((panel, content));
    });
    h.run_until_idle();
    let (panel, _) = ids.unwrap();
    h.press(Point::new(100, 100));
    step_to(&mut h, Point::new(100, 88)); // vertical start
    step_to(&mut h, Point::new(40, 70)); // mostly horizontal afterwards
    assert_eq!(h.engine().scroll_offset(panel), Point::new(0, 30));
}

#[test]
fn elastic_overscroll_slowed_by_factor() {
    let (mut h, cont, _, _) = scene();
    h.press(Point::new(70, 30));
    // At the top: the first 10 px still move freely (the content is not out yet) ...
    step_to(&mut h, Point::new(70, 40));
    assert_eq!(h.engine().scroll_offset(cont).y, -10);
    // ... then beyond the edge the movement is divided by the elastic factor (rounded).
    step_to(&mut h, Point::new(70, 70));
    assert_eq!(SCROLL_ELASTIC_FACTOR, 4);
    assert_eq!(h.engine().scroll_offset(cont).y, -10 - (30 + 2) / 4);
    h.release();
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 0, "springs back");
}

#[test]
fn no_elastic_clamps_at_edge() {
    let (mut h, cont, _, _) = scene();
    h.engine_mut().set_flag(cont, ObjFlags::SCROLL_ELASTIC, false);
    h.press(Point::new(70, 30));
    step_to(&mut h, Point::new(70, 40));
    step_to(&mut h, Point::new(70, 90));
    assert_eq!(h.engine().scroll_offset(cont).y, 0);
    // At the bottom edge as well.
    h.release();
    h.run_until_idle();
    h.engine_mut().scroll_to_y(cont, 290, false);
    h.press(Point::new(70, 70));
    step_to(&mut h, Point::new(70, 60));
    step_to(&mut h, Point::new(70, 20));
    assert_eq!(h.engine().scroll_offset(cont).y, 300);
}

#[test]
fn scroll_drag_invalidates_container_only() {
    let (mut h, cont, _, _) = scene();
    h.press(Point::new(70, 80));
    step_to(&mut h, Point::new(70, 68));
    h.advance(Duration::ms(100));
    let list_area = h.engine().coords(cont);
    for k in 1..=5 {
        h.clock().advance(h.engine().config().read_period);
        h.move_to(Point::new(70, 68 - k * 7));
        for (a, _) in h.engine().invalidation_log() {
            assert!(list_area.contains_rect(a), "{a:?} outside {list_area:?}");
        }
        for f in h.flushes() {
            assert!(list_area.contains_rect(&f.area), "{:?}", f.area);
        }
    }
    assert!(!h.engine().layout_pending());
}

#[test]
fn scroll_drag_allocates_nothing() {
    let (mut h, cont, _, log) = scene();
    log.borrow_mut().reserve(1000); // the test's own event log must not count
    h.press(Point::new(70, 90));
    step_to(&mut h, Point::new(70, 80));
    // Warm-up: the scroll started, frames rendered, buffers grown.
    for k in 0..10 {
        step_to(&mut h, Point::new(70, 70 - k * 3));
    }
    let ((), stats) = count_allocs(|| {
        for k in 0..20 {
            h.clock().advance(h.engine().config().read_period);
            h.move_to(Point::new(70, 40 - k * 3));
        }
    });
    assert!(h.engine().scroll_offset(cont).y > 80);
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn long_press_suppressed_only_by_actual_scroll() {
    let (mut h, _, rows, log) = scene();
    h.press(Point::new(70, 80));
    step_to(&mut h, Point::new(70, 60));
    h.advance(Duration::ms(600));
    assert!(!codes_of(&log, rows[1]).contains(&C::LongPressed));
}

#[test]
fn deleting_scrolled_node_during_drag_is_safe() {
    let (mut h, cont, _, _) = scene();
    h.press(Point::new(70, 80));
    step_to(&mut h, Point::new(70, 60));
    assert!(h.engine().is_scrolling(cont));
    h.engine_mut().delete(cont).unwrap();
    step_to(&mut h, Point::new(70, 40));
    h.release();
    h.run_until_idle();
    assert_eq!(h.engine().input_deadline(), None);
    h.assert_idle();
}
