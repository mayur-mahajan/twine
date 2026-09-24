//! Animated scrolling (LVGL `lv_obj_scroll_by` with `LV_ANIM_ON`) and scrolling children into
//! view, also through nested scrollable containers.

mod common;

use common::{EvLog, list, record, scroll_codes, white_screen};
use twine_core::{Duration, Rect};
use twine_engine::{EventCode as C, NodeId, ScrollSnap, Wake};
use twine_testing::EngineHarness;

/// A 200×200 screen with a 100×100 list at (20, 20) of 10 rows of 40 px; the list's log.
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
    (h, cont, rows, log)
}

fn scroll_count(log: &EvLog) -> usize {
    log.borrow().iter().filter(|(c, _)| *c == C::Scroll).count()
}

#[test]
fn animated_scroll_reaches_target_and_sends_begin_end() {
    let (mut h, cont, rows, log) = scene();
    h.engine_mut().scroll_to_y(cont, 100, true);
    assert_eq!(
        h.engine().scroll_offset(cont).y,
        0,
        "nothing moves before the first frame"
    );
    assert_eq!(h.engine().scroll_end(cont).y, 100);
    assert!(h.engine().is_scrolling(cont));
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 100);
    assert_eq!(h.engine().coords(rows[0]).y0, 20 - 100);
    assert!(!h.engine().is_scrolling(cont));
    assert_eq!(scroll_codes(&log), [C::ScrollBegin, C::ScrollEnd]);
    assert!(scroll_count(&log) > 3, "moves in several frames");
}

#[test]
fn animated_scroll_duration_clamped() {
    // Display height 200: speed 100 px/s, stored as 10 (LVGL `lv_anim_speed_clamped`), so the
    // time is 10 ms per pixel clamped to 200..=400 ms.
    for (dist, ms) in [(10, 200), (30, 300), (100, 400)] {
        let (mut h, cont, _, _) = scene();
        h.update();
        let start = h.now();
        h.engine_mut().scroll_to_y(cont, dist, true);
        h.update(); // the animation starts at this update
        h.clock().set(start + Duration::ms(ms - 1));
        h.update();
        assert!(
            h.engine().is_scrolling(cont),
            "{dist} px still running at {} ms",
            ms - 1
        );
        assert_ne!(h.engine().scroll_offset(cont).y, dist);
        h.clock().set(start + Duration::ms(ms));
        h.update();
        assert!(!h.engine().is_scrolling(cont), "{dist} px done at {ms} ms");
        assert_eq!(h.engine().scroll_offset(cont).y, dist);
    }
}

#[test]
fn new_scroll_anim_replaces_old() {
    let (mut h, cont, _, log) = scene();
    h.engine_mut().scroll_to_y(cont, 200, true);
    h.update();
    h.advance(Duration::ms(100));
    let mid = h.engine().scroll_offset(cont).y;
    assert!(mid > 0 && mid < 200, "{mid}");
    h.engine_mut().scroll_to_y(cont, 20, true);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 20);
    // `scroll_to_y` stops the running animation (its end), then the new one begins and ends.
    assert_eq!(
        scroll_codes(&log),
        [C::ScrollBegin, C::ScrollEnd, C::ScrollBegin, C::ScrollEnd]
    );
}

#[test]
fn instant_scroll_stops_animation() {
    let (mut h, cont, _, log) = scene();
    h.engine_mut().scroll_to_y(cont, 200, true);
    h.update();
    h.advance(Duration::ms(50));
    h.engine_mut().scroll_by(cont, 0, -10, false);
    let y = h.engine().scroll_offset(cont).y;
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, y, "the animation is gone");
    assert_eq!(
        scroll_codes(&log),
        [C::ScrollBegin, C::ScrollEnd, C::ScrollBegin, C::ScrollEnd]
    );
}

#[test]
fn scroll_to_view_minimal_distance_bottom() {
    let (mut h, cont, rows, _) = scene();
    // Row 5 spans 200..240 of the content: aligned to the bottom edge.
    h.engine_mut().scroll_to_view(rows[5], false);
    assert_eq!(h.engine().scroll_offset(cont).y, 140);
    assert_eq!(h.engine().coords(rows[5]).y1, 120);
    // Already visible: nothing moves.
    h.engine_mut().scroll_to_view(rows[4], false);
    assert_eq!(h.engine().scroll_offset(cont).y, 140);
}

#[test]
fn scroll_to_view_minimal_distance_top() {
    let (mut h, cont, rows, _) = scene();
    h.engine_mut().scroll_to_y(cont, 300, false);
    h.engine_mut().scroll_to_view(rows[1], false);
    assert_eq!(h.engine().scroll_offset(cont).y, 40);
    assert_eq!(h.engine().coords(rows[1]).y0, 20);
}

#[test]
fn scroll_to_view_animated() {
    let (mut h, cont, rows, log) = scene();
    h.engine_mut().scroll_to_view(rows[9], true);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 300);
    assert_eq!(scroll_codes(&log), [C::ScrollBegin, C::ScrollEnd]);
}

#[test]
fn scroll_to_view_with_center_snap() {
    let (mut h, cont, rows, _) = scene();
    h.engine_mut().set_scroll_snap_y(cont, ScrollSnap::Center);
    h.engine_mut().scroll_to_view(rows[5], false);
    // Row center 220 aligned with the container's center 50.
    assert_eq!(h.engine().scroll_offset(cont).y, 170);
    let c = h.engine().coords(rows[5]);
    assert_eq!(c.y0 + c.height() / 2, 70);
    // Snapping aligns even a visible child.
    h.engine_mut().scroll_to_view(rows[4], false);
    assert_eq!(h.engine().scroll_offset(cont).y, 130);
}

#[test]
fn scroll_to_view_recursive_nested() {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        // A page taller than the screen holding, far down, a list of 10 rows.
        let (page, _) = list(e, s, Rect::from_xywh(0, 0, 200, 200), 0, 0);
        let spacer = twine_testing::scenes::child_box(e, page, Rect::from_xywh(0, 0, 200, 400), &[]);
        let (inner, rows) = list(e, page, Rect::from_xywh(20, 400, 100, 100), 10, 40);
        ids = Some((page, spacer, inner, rows));
    });
    h.run_until_idle();
    let (page, _, inner, rows) = ids.unwrap();
    let target = rows[8];
    h.engine_mut().scroll_to_view_recursive(target, true);
    h.run_until_idle();
    let e = h.engine();
    let screen = Rect::new(0, 0, 200, 200);
    assert!(screen.contains_rect(&e.coords(target)), "{:?}", e.coords(target));
    assert!(e.coords(inner).contains_rect(&e.coords(target)));
    assert_eq!(e.scroll_offset(inner).y, 8 * 40 + 40 - 100);
    assert_eq!(e.scroll_offset(page).y, 300);
}

#[test]
fn idle_after_scroll_anim() {
    let (mut h, cont, _, _) = scene();
    h.engine_mut().scroll_to_y(cont, 120, true);
    assert!(matches!(h.update(), Wake::At(_)), "frames while animating");
    assert!(h.engine().is_scrolling(cont));
    h.run_until_idle();
    h.assert_idle();
}
