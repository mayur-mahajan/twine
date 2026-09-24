//! Swipe gestures with the thresholds of `EngineConfig`.

mod common;

use common::{EvLog, clickable, record, white_screen};
use twine_core::{Duration, Point, Rect};
use twine_engine::{Dir, EventCode, EventParam, NodeId, ObjFlags};
use twine_testing::EngineHarness;

/// A 200×200 harness with a box at (20, 20, 160, 160) (with `GESTURE_BUBBLE`, the `Obj`
/// default). Returns the harness, the box, and the gesture logs of box and screen.
fn setup() -> (EngineHarness, NodeId, EvLog, EvLog) {
    let mut b = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        b = Some(clickable(e, s, Rect::from_xywh(20, 20, 160, 160)));
    });
    let b = b.unwrap();
    let (blog, slog) = (EvLog::default(), EvLog::default());
    let s = h.screen();
    record(h.engine_mut(), b, &blog);
    record(h.engine_mut(), s, &slog);
    h.run_until_idle();
    (h, b, blog, slog)
}

fn gestures(log: &EvLog) -> usize {
    log.borrow()
        .iter()
        .filter(|(c, _)| *c == EventCode::Gesture)
        .count()
}

/// The directions of the gesture events recorded by a param-aware handler.
fn gesture_dirs(h: &mut EngineHarness, n: NodeId) -> std::rc::Rc<std::cell::RefCell<Vec<Dir>>> {
    let dirs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let d = dirs.clone();
    h.engine_mut().add_event_handler(
        n,
        twine_engine::EventFilter::Code(EventCode::Gesture),
        move |_, ev| {
            if let EventParam::Dir(x) = ev.param {
                d.borrow_mut().push(x);
            }
            twine_engine::EventResult::Continue
        },
    );
    dirs
}

#[test]
fn swipe_left_sends_gesture_left() {
    let (mut h, _, _, _) = setup();
    let s = h.screen();
    let dirs = gesture_dirs(&mut h, s);
    h.drag(Point::new(150, 100), Point::new(50, 100), Duration::ms(150));
    assert_eq!(*dirs.borrow(), [Dir::LEFT]);
    let (id, _) = h.pointer_input();
    assert_eq!(h.engine().gesture_dir(id), Some(Dir::LEFT));
}

#[test]
fn slow_drag_sends_no_gesture() {
    let (mut h, _, blog, slog) = setup();
    // 2 px per read period: below `gesture_min_velocity` (3), the sum restarts each read.
    h.drag(Point::new(150, 100), Point::new(90, 100), Duration::ms(900));
    assert_eq!(gestures(&blog) + gestures(&slog), 0);
}

#[test]
fn gesture_bubbles_to_parent_with_flag() {
    let (mut h, b, blog, slog) = setup();
    h.drag(Point::new(150, 100), Point::new(50, 100), Duration::ms(150));
    assert_eq!(gestures(&blog), 0);
    assert_eq!(gestures(&slog), 1);
    // Without GESTURE_BUBBLE the box itself gets it.
    h.engine_mut().set_flag(b, ObjFlags::GESTURE_BUBBLE, false);
    h.clock().advance(Duration::ms(500));
    h.drag(Point::new(150, 100), Point::new(50, 100), Duration::ms(150));
    assert_eq!(gestures(&blog), 1);
    assert_eq!(gestures(&slog), 1);
}

#[test]
fn only_one_gesture_per_press() {
    let (mut h, _, _, slog) = setup();
    h.drag(Point::new(170, 100), Point::new(30, 100), Duration::ms(300));
    assert_eq!(gestures(&slog), 1);
}

#[test]
fn vertical_dominant_swipe_is_top_or_bottom() {
    let (mut h, _, _, _) = setup();
    let s = h.screen();
    let dirs = gesture_dirs(&mut h, s);
    h.drag(Point::new(100, 40), Point::new(120, 160), Duration::ms(150));
    h.clock().advance(Duration::ms(500));
    h.drag(Point::new(100, 160), Point::new(90, 40), Duration::ms(150));
    assert_eq!(*dirs.borrow(), [Dir::BOTTOM, Dir::TOP]);
}

#[test]
fn gesture_limits_can_be_tuned() {
    let (mut h, _, _, slog) = setup();
    h.engine_mut().set_gesture_limits(150, 3);
    h.drag(Point::new(150, 100), Point::new(50, 100), Duration::ms(150));
    assert_eq!(gestures(&slog), 0);
}
