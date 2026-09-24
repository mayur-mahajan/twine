//! Pointer processing: exact LVGL event sequences for taps, multi-clicks, long presses,
//! press lost / press lock, hit testing, checkable nodes and hover.

mod common;

use common::{EvLog, clickable, input_codes, record, white_screen};
use twine_core::{Color, Duration, Opa, Point, Rect};
use twine_engine::{EventCode as C, EventFilter, EventResult, NodeId, ObjFlags, State};
use twine_style::{Selector, StyleProp};
use twine_testing::EngineHarness;

/// A 100×100 harness with one clickable box at (10, 10, 40, 40); its event log.
fn setup() -> (EngineHarness, NodeId, EvLog) {
    let mut b = None;
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        b = Some(clickable(e, s, Rect::from_xywh(10, 10, 40, 40)));
    });
    let b = b.unwrap();
    let log = EvLog::default();
    record(h.engine_mut(), b, &log);
    h.run_until_idle();
    (h, b, log)
}

fn state(h: &EngineHarness, n: NodeId) -> State {
    h.engine().tree().node(n).unwrap().state()
}

#[test]
fn tap_sends_pressed_released_short_single_clicked_in_order() {
    let (mut h, b, log) = setup();
    h.press(Point::new(20, 20));
    assert!(state(&h, b).contains(State::PRESSED));
    h.release();
    assert!(!state(&h, b).contains(State::PRESSED));
    assert_eq!(
        input_codes(&log),
        [
            C::Pressed,
            C::Focused,
            C::Pressing,
            C::Released,
            C::ShortClicked,
            C::SingleClicked,
            C::Clicked
        ]
    );
}

#[test]
fn double_tap_within_time_sends_double_clicked() {
    let (mut h, _, log) = setup();
    h.tap(Point::new(20, 20));
    h.clock().advance(Duration::ms(200));
    h.tap(Point::new(22, 21));
    let codes = input_codes(&log);
    assert_eq!(codes.iter().filter(|c| **c == C::SingleClicked).count(), 1);
    assert_eq!(codes.iter().filter(|c| **c == C::DoubleClicked).count(), 1);
}

#[test]
fn triple_tap_sends_triple_clicked() {
    let (mut h, _, log) = setup();
    for _ in 0..4 {
        h.tap(Point::new(20, 20));
        h.clock().advance(Duration::ms(100));
    }
    let multi: Vec<C> = input_codes(&log)
        .into_iter()
        .filter(|c| matches!(c, C::SingleClicked | C::DoubleClicked | C::TripleClicked))
        .collect();
    assert_eq!(
        multi,
        [
            C::SingleClicked,
            C::DoubleClicked,
            C::TripleClicked,
            C::SingleClicked
        ]
    );
}

#[test]
fn slow_second_tap_is_single() {
    let (mut h, _, log) = setup();
    h.tap(Point::new(20, 20));
    h.clock().advance(Duration::ms(301));
    h.tap(Point::new(20, 20));
    let codes = input_codes(&log);
    assert_eq!(codes.iter().filter(|c| **c == C::SingleClicked).count(), 2);
    assert!(!codes.contains(&C::DoubleClicked));
}

#[test]
fn far_second_tap_is_single() {
    let (mut h, _, log) = setup();
    h.tap(Point::new(20, 20));
    h.clock().advance(Duration::ms(50));
    h.tap(Point::new(30, 30));
    let codes = input_codes(&log);
    assert_eq!(codes.iter().filter(|c| **c == C::SingleClicked).count(), 2);
}

#[test]
fn long_press_and_repeat_timing() {
    let (mut h, _, log) = setup();
    let t0 = h.now();
    h.press(Point::new(20, 20));
    let count = |c: C| input_codes(&log).iter().filter(|x| **x == c).count();
    h.clock().set(t0 + Duration::ms(399));
    h.update();
    assert_eq!(count(C::LongPressed), 0);
    h.clock().set(t0 + Duration::ms(400));
    h.update();
    assert_eq!(count(C::LongPressed), 1);
    h.clock().set(t0 + Duration::ms(499));
    h.update();
    assert_eq!(count(C::LongPressedRepeat), 0);
    h.clock().set(t0 + Duration::ms(500));
    h.update();
    assert_eq!(count(C::LongPressedRepeat), 1);
    h.clock().set(t0 + Duration::ms(600));
    h.update();
    assert_eq!(count(C::LongPressedRepeat), 2);
    h.release();
    let codes = input_codes(&log);
    // No short click after a long press, but `Clicked` (LVGL).
    assert!(!codes.contains(&C::ShortClicked));
    assert_eq!(codes.last(), Some(&C::Clicked));
}

#[test]
fn drag_without_scroll_still_long_presses() {
    // LVGL: only an actual scroll suppresses long presses; nothing here can scroll.
    let (mut h, _, log) = setup();
    h.drag(Point::new(15, 20), Point::new(45, 20), Duration::ms(600));
    let codes = input_codes(&log);
    assert!(codes.contains(&C::LongPressed), "{codes:?}");
    assert!(!codes.contains(&C::ShortClicked), "{codes:?}");
    assert!(codes.contains(&C::Clicked));
}

#[test]
fn press_lost_when_leaving_without_press_lock() {
    let (mut h, b, log) = setup();
    h.engine_mut().set_flag(b, ObjFlags::PRESS_LOCK, false);
    h.press(Point::new(20, 20));
    h.move_to(Point::new(80, 80)); // onto the screen
    assert!(!state(&h, b).contains(State::PRESSED));
    h.release();
    let codes = input_codes(&log);
    assert!(codes.contains(&C::PressLost));
    assert!(
        !codes.contains(&C::Released) && !codes.contains(&C::Clicked),
        "{codes:?}"
    );
}

#[test]
fn press_lock_keeps_pressing_outside() {
    let (mut h, b, log) = setup();
    h.press(Point::new(20, 20));
    h.move_to(Point::new(80, 80));
    h.move_to(Point::new(90, 90));
    assert!(state(&h, b).contains(State::PRESSED));
    h.release();
    let codes = input_codes(&log);
    assert!(!codes.contains(&C::PressLost));
    assert_eq!(codes.iter().filter(|c| **c == C::Pressing).count(), 3);
    assert!(codes.contains(&C::Clicked));
}

#[test]
fn non_clickable_child_passes_press_to_parent() {
    let (mut h, b, log) = setup();
    let child = clickable(h.engine_mut(), b, Rect::from_xywh(15, 15, 10, 10));
    h.engine_mut().set_flag(child, ObjFlags::CLICKABLE, false);
    let d = h.display();
    assert_eq!(h.engine_mut().hit_test(d, Point::new(18, 18)), Some(b));
    h.tap(Point::new(18, 18));
    assert!(input_codes(&log).contains(&C::Clicked));
}

#[test]
fn hidden_child_not_hit() {
    let (mut h, b, _) = setup();
    let child = clickable(h.engine_mut(), b, Rect::from_xywh(15, 15, 10, 10));
    let d = h.display();
    assert_eq!(h.engine_mut().hit_test(d, Point::new(18, 18)), Some(child));
    h.engine_mut().set_flag(child, ObjFlags::HIDDEN, true);
    assert_eq!(h.engine_mut().hit_test(d, Point::new(18, 18)), Some(b));
    // Outside the parent a child is not hit (clipped by the parent).
    let out = clickable(h.engine_mut(), b, Rect::from_xywh(45, 45, 20, 20));
    assert_eq!(h.engine_mut().hit_test(d, Point::new(60, 60)), Some(h.screen()));
    h.engine_mut().set_flag(b, ObjFlags::OVERFLOW_VISIBLE, true);
    // With OVERFLOW_VISIBLE only the extra draw area extends the search (none here).
    assert_eq!(h.engine_mut().hit_test(d, Point::new(60, 60)), Some(h.screen()));
    assert_eq!(h.engine_mut().hit_test(d, Point::new(47, 47)), Some(out));
}

#[test]
fn disabled_blocks_input() {
    let (mut h, b, log) = setup();
    let child = clickable(h.engine_mut(), b, Rect::from_xywh(15, 15, 10, 10));
    let clog = EvLog::default();
    record(h.engine_mut(), child, &clog);
    h.engine_mut().add_state(child, State::DISABLED);
    let d = h.display();
    assert_eq!(h.engine_mut().hit_test(d, Point::new(18, 18)), None);
    log.borrow_mut().clear();
    h.tap(Point::new(18, 18));
    assert!(input_codes(&clog).is_empty());
    assert!(input_codes(&log).is_empty(), "the parent gets nothing either");
}

#[test]
fn adv_hittest_event_can_veto() {
    let (mut h, b, log) = setup();
    h.engine_mut().set_flag(b, ObjFlags::ADV_HITTEST, true);
    h.engine_mut()
        .add_event_handler(b, EventFilter::Code(C::HitTest), |_, ev| match ev.param {
            twine_engine::EventParam::Point(p) if p.x < 30 => EventResult::Stop,
            _ => EventResult::Continue,
        });
    let d = h.display();
    assert_eq!(h.engine_mut().hit_test(d, Point::new(20, 20)), Some(h.screen()));
    assert_eq!(h.engine_mut().hit_test(d, Point::new(35, 20)), Some(b));
    log.borrow_mut().clear();
    h.tap(Point::new(20, 20));
    assert!(!input_codes(&log).contains(&C::Clicked));
}

#[test]
fn checkable_toggles_checked_and_sends_value_changed() {
    let (mut h, b, log) = setup();
    h.engine_mut().set_flag(b, ObjFlags::CHECKABLE, true);
    h.tap(Point::new(20, 20));
    assert!(state(&h, b).contains(State::CHECKED));
    let codes = input_codes(&log);
    // LVGL toggles in the object's own `Released` handling: before user handlers see
    // `Released`, and before the clicks.
    let rel = codes.iter().position(|c| *c == C::Released).unwrap();
    assert_eq!(codes[rel - 1], C::ValueChanged);
    h.clock().advance(Duration::ms(500));
    h.tap(Point::new(20, 20));
    assert!(!state(&h, b).contains(State::CHECKED));
    assert_eq!(
        input_codes(&log)
            .iter()
            .filter(|c| **c == C::ValueChanged)
            .count(),
        2
    );
}

#[test]
fn hover_over_and_leave_for_mouse() {
    let (mut h, b, log) = setup();
    h.move_to(Point::new(20, 20));
    assert!(state(&h, b).contains(State::HOVERED));
    h.move_to(Point::new(25, 20));
    h.move_to(Point::new(80, 80));
    assert!(!state(&h, b).contains(State::HOVERED));
    assert_eq!(input_codes(&log), [C::HoverOver, C::HoverLeave]);
}

#[test]
fn deleting_pressed_node_in_handler_is_safe() {
    let (mut h, b, _) = setup();
    h.engine_mut()
        .add_event_handler(b, EventFilter::Code(C::Pressing), |cx, _| {
            let n = cx.node();
            cx.engine_mut().delete(n).unwrap();
            EventResult::Continue
        });
    let slog = EvLog::default();
    let s = h.screen();
    record(h.engine_mut(), s, &slog);
    h.press(Point::new(20, 20));
    assert!(!h.engine().tree().contains(b));
    h.move_to(Point::new(22, 22));
    h.release();
    // The press was lost with the node; the screen below gets nothing from it.
    assert!(
        !input_codes(&slog).contains(&C::Clicked),
        "{:?}",
        input_codes(&slog)
    );
    h.tap(Point::new(20, 20));
    assert!(input_codes(&slog).contains(&C::Clicked));
    h.engine().tree().check_invariants().unwrap();
}

#[test]
fn pressed_state_redraws_only_node() {
    let (mut h, b, _) = setup();
    h.engine_mut().set_local_prop(
        b,
        Selector::MAIN.with_state(State::PRESSED),
        StyleProp::BgColor(Color::RED),
    );
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
    // Focus changes from the tap do not change the look (no FOCUSED style).
    h.run_until_idle();
    h.clock().advance(Duration::ms(20));
    h.press(Point::new(20, 20));
    // Read, state change, invalidation and frame happen in the same update.
    let areas: Vec<Rect> = h.flushes().iter().map(|f| f.area).collect();
    assert_eq!(areas, [Rect::from_xywh(10, 10, 40, 40)]);
    assert_eq!(h.last_frame().dirty_px, 40 * 40);
    assert_eq!(h.pixel(20, 20), Color::RED);
}
