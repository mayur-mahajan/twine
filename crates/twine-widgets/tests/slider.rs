//! `Slider`: LVGL defaults, pointer dragging (threshold, tap-to-jump, RTL, range knobs), keys,
//! encoder edit mode, the extended hit area, `ValueChanged` only on change, and the default
//! theme's look.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Duration, Point};
use twine_engine::{
    EventCode, EventFilter, EventParam, EventResult, GroupDef, Key, MeasureCx, NodeId, ObjFlags,
};
use twine_style::{Align, BaseDir, Part, Selector, State, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::slider::{self, SLIDER_CLASS, Slider, SliderMode};

const W: i32 = 200;

/// A 200 × 10 slider in the middle of a 240 × 60 screen, in the default group.
fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 60, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let s = slider::create(e, screen).unwrap();
    e.set_size(s, W, 10);
    e.align(s, Align::Center, 0, 0);
    h.run_until_idle();
    (h, s)
}

/// Records the value at every `ValueChanged`.
fn changes(h: &mut EngineHarness, s: NodeId) -> Rc<RefCell<Vec<i32>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    h.engine_mut()
        .add_event_handler(s, EventFilter::Code(EventCode::ValueChanged), move |_, ev| {
            // The slider sends its new value with the event.
            let EventParam::Value(v) = ev.param else {
                panic!("no value: {:?}", ev.param);
            };
            l.borrow_mut().push(v);
            EventResult::Continue
        });
    log
}

/// The point at `pct` % of the slider's width, vertically centered.
fn at(h: &EngineHarness, s: NodeId, pct: i32) -> Point {
    let c = h.engine().coords(s);
    Point::new(c.x0 + c.width() * pct / 100, (c.y0 + c.y1) / 2)
}

fn value(h: &EngineHarness, s: NodeId) -> i32 {
    get::<Slider>(h, s).value()
}

#[test]
fn slider_defaults() {
    let mut h = harness(300, 60, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let s = slider::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let n = h.engine().tree().node(s).unwrap();
    assert_eq!(n.class().name, "slider");
    assert_eq!(SLIDER_CLASS.parts, &[Part::Main, Part::Indicator, Part::Knob]);
    assert_eq!(SLIDER_CLASS.group_def, GroupDef::True);
    assert_eq!(h.engine().group_of(s), Some(g), "sliders join the default group");
    assert!(!n.flags().contains(ObjFlags::SCROLLABLE));
    assert!(
        !n.flags().contains(ObjFlags::SCROLL_CHAIN_HOR),
        "horizontal: no horizontal chain"
    );
    assert!(n.flags().contains(ObjFlags::SCROLL_CHAIN_VER));
    let c = h.engine().coords(s);
    assert_eq!((c.width(), c.height()), (260, 13));
    let w = get::<Slider>(&h, s);
    assert_eq!((w.value(), w.left_value(), w.min(), w.max()), (0, 0, 0, 100));
    assert_eq!(w.mode(), SliderMode::Normal);
    assert!(!w.is_dragged());
    // The knob: centered on the indicator end, the slider's height plus dpx(6) padding.
    let (knob, left) = w.knob_areas(&MeasureCx::new(h.engine(), s));
    assert!(left.is_none());
    let pad = 5; // dpx(6) at 130 dpi
    assert_eq!(knob.height(), 13 + 2 * pad);
    assert_eq!(knob.width(), 13 + 2 * pad);
    assert!(knob.contains(Point::new(c.x0, c.y0 + 6)));
    // The knob is inside the extra draw area.
    let ext = i32::from(n.ext_draw());
    assert!(c.expand(ext).contains_rect(&knob));
    h.assert_idle();
}

#[test]
fn slider_setters_same_value_no_invalidate() {
    let (mut h, s) = scene(Mode::Light);
    with(&mut h, s, |w: &mut Slider, cx| {
        w.set_value(cx, 0, false);
        w.set_left_value(cx, 0, false);
        w.set_range(cx, 0, 100);
        w.set_mode(cx, SliderMode::Normal);
        w.set_orientation(cx, twine_widgets::Orientation::Auto);
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn slider_press_jumps_to_point() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    let p = at(&h, s, 25);
    h.tap(p);
    assert_eq!(value(&h, s), 25, "a tap sets the value on release (LVGL)");
    assert_eq!(*log.borrow(), vec![25]);
    // A press alone does not move the knob (the drag threshold).
    h.press(at(&h, s, 75));
    assert_eq!(value(&h, s), 25);
    h.release();
    assert_eq!(value(&h, s), 75);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn slider_drag_updates_value_monotonic() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    let c = h.engine().coords(s);
    let y = (c.y0 + c.y1) / 2;
    h.press(Point::new(c.x0 + 2, y));
    let mut last = value(&h, s);
    for x in (c.x0 + 2..c.x1 + 20).step_by(3) {
        h.clock().advance(Duration::ms(10));
        h.move_to(Point::new(x, y));
        let v = value(&h, s);
        assert!(v >= last, "{v} < {last} at x {x}");
        last = v;
        assert!(get::<Slider>(&h, s).is_dragged() || x - c.x0 < 12);
    }
    h.release();
    assert_eq!(value(&h, s), 100, "clamped at the end");
    assert!(!get::<Slider>(&h, s).is_dragged());
    // Done-when: at most (max - min) events and none without a change.
    let log = log.borrow();
    assert!(log.len() <= 100, "{} events", log.len());
    assert!(
        log.windows(2).all(|w| w[0] != w[1]),
        "redundant ValueChanged: {log:?}"
    );
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn slider_value_changed_only_on_change() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    let c = h.engine().coords(s);
    let y = (c.y0 + c.y1) / 2;
    // A slow drag (sub-value moves) across the whole slider and back.
    h.press(Point::new(c.x0, y));
    for x in (c.x0..c.x1).chain((c.x0..c.x1).rev()) {
        h.clock().advance(Duration::ms(5));
        h.move_to(Point::new(x, y));
    }
    h.release();
    let log = log.borrow();
    assert!(log.windows(2).all(|w| w[0] != w[1]));
    assert!(log.len() <= 2 * 100, "{} events", log.len());
    assert_eq!(value(&h, s), 0);
}

#[test]
fn slider_rtl_reverses() {
    let (mut h, s) = scene(Mode::Light);
    h.engine_mut()
        .set_local_prop(s, Selector::MAIN, StyleProp::BaseDir(BaseDir::Rtl));
    h.run_until_idle();
    let p = at(&h, s, 25);
    h.tap(p);
    assert_eq!(value(&h, s), 75, "RTL: the maximum is on the left");
    let (knob, _) = get::<Slider>(&h, s).knob_areas(&MeasureCx::new(h.engine(), s));
    assert!(knob.contains(p), "the knob follows the tap");
}

#[test]
fn slider_range_picks_nearest_knob() {
    let (mut h, s) = scene(Mode::Light);
    with(&mut h, s, |w: &mut Slider, cx| {
        w.set_mode(cx, SliderMode::Range);
        w.set_value(cx, 60, false);
        w.set_left_value(cx, 40, false);
    });
    h.run_until_idle();
    // Nearer to the left knob (40): the left value moves.
    h.tap(at(&h, s, 45));
    let w = get::<Slider>(&h, s);
    assert_eq!((w.left_value(), w.value()), (45, 60));
    assert!(w.left_knob_focused());
    // Beyond the right knob: the value moves.
    h.tap(at(&h, s, 90));
    let w = get::<Slider>(&h, s);
    assert_eq!((w.left_value(), w.value()), (45, 90));
    // Left of the left knob.
    h.tap(at(&h, s, 10));
    let w = get::<Slider>(&h, s);
    assert_eq!((w.left_value(), w.value()), (10, 90));
    // The left knob cannot pass the right one.
    let (r, l) = get::<Slider>(&h, s).knob_areas(&MeasureCx::new(h.engine(), s));
    assert!(l.unwrap().x0 < r.x0);
}

#[test]
fn slider_key_right_increments() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    let _ = h.keypad_input();
    h.engine_mut().focus(s);
    h.run_until_idle();
    h.key(Key::Right);
    h.key(Key::Up);
    assert_eq!(value(&h, s), 2);
    h.key(Key::Left);
    assert_eq!(value(&h, s), 1);
    h.key(Key::Down);
    h.key(Key::Down); // at the minimum: no change, no event
    assert_eq!(value(&h, s), 0);
    assert_eq!(*log.borrow(), vec![1, 2, 1, 0]);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn slider_keypad_navigation_focuses() {
    let (mut h, s) = scene(Mode::Light);
    let screen = h.screen();
    let other = slider::create(h.engine_mut(), screen).unwrap();
    let _ = h.keypad_input();
    h.run_until_idle();
    h.key(Key::Next);
    let focused = [s, other].into_iter().find(|n| {
        h.engine()
            .tree()
            .node(*n)
            .unwrap()
            .state()
            .contains(State::FOCUS_KEY)
    });
    assert!(focused.is_some(), "Tab focuses a slider");
}

#[test]
fn slider_encoder_edit_mode_changes_value() {
    let (mut h, s) = scene(Mode::Light);
    let screen = h.screen();
    let _second = slider::create(h.engine_mut(), screen).unwrap();
    let g = h.engine().group_of(s).unwrap();
    let log = changes(&mut h, s);
    let _ = h.encoder_input();
    h.engine_mut().focus(s);
    h.run_until_idle();
    // Navigate mode: turning moves the focus, a click enters edit mode.
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    assert!(h.engine().tree().node(s).unwrap().state().contains(State::EDITED));
    h.encoder(3);
    assert_eq!(
        value(&h, s),
        3,
        "one step per detent (keys, the Rotary event is not doubled)"
    );
    h.encoder(-1);
    assert_eq!(value(&h, s), 2);
    assert_eq!(log.borrow().len(), 4);
    // A click in edit mode leaves it (LVGL: release leaves edit mode).
    h.encoder_click();
    assert!(!h.engine().group_editing(g));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn slider_encoder_range_switches_knob() {
    let (mut h, s) = scene(Mode::Light);
    let screen = h.screen();
    let _second = slider::create(h.engine_mut(), screen).unwrap();
    with(&mut h, s, |w: &mut Slider, cx| {
        w.set_mode(cx, SliderMode::Range);
        w.set_value(cx, 50, false);
        w.set_left_value(cx, 20, false);
    });
    let g = h.engine().group_of(s).unwrap();
    let _ = h.encoder_input();
    h.engine_mut().focus(s);
    h.run_until_idle();
    h.encoder_click();
    h.encoder(1);
    assert_eq!(value(&h, s), 51, "the right knob first");
    h.encoder_click();
    assert!(
        h.engine().group_editing(g),
        "range mode: the first click switches knobs"
    );
    h.encoder(-2);
    assert_eq!(get::<Slider>(&h, s).left_value(), 18);
    h.encoder_click();
    assert!(!h.engine().group_editing(g));
}

#[test]
fn slider_hit_area_includes_knob() {
    let (mut h, s) = scene(Mode::Light);
    let c = h.engine().coords(s);
    let d = h.display();
    // dpx(8) = 7 px above the slider still hits it (LVGL ext_click_area).
    let p = Point::new(c.x0 + 50, c.y0 - 6);
    assert_eq!(h.engine_mut().hit_test(d, p), Some(s));
    let far = Point::new(c.x0 + 50, c.y0 - 10);
    assert_ne!(h.engine_mut().hit_test(d, far), Some(s));
    // With ADV_HITTEST only the knob reacts.
    h.engine_mut().set_flag(s, ObjFlags::ADV_HITTEST, true);
    assert_ne!(h.engine_mut().hit_test(d, p), Some(s));
    let knob = Point::new(c.x0, c.y0 - 3);
    assert_eq!(h.engine_mut().hit_test(d, knob), Some(s));
}

#[test]
fn slider_drag_redraws_only_around_knob() {
    let (mut h, s) = scene(Mode::Light);
    with(&mut h, s, |w: &mut Slider, cx| w.set_value(cx, 50, false));
    h.run_until_idle();
    let c = h.engine().coords(s);
    let y = (c.y0 + c.y1) / 2;
    h.press(Point::new(c.x0 + 100, y));
    h.move_to(Point::new(c.x0 + 115, y));
    // Let the knob's press transition (grow) finish.
    h.advance(Duration::ms(300));
    h.move_to(Point::new(c.x0 + 120, y));
    let dirty = h.last_frame().dirty_px;
    let full = u32::try_from(c.expand(20).area()).unwrap();
    assert!(dirty > 0 && dirty < full / 3, "dirty {dirty} of {full}");
    h.release();
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn slider_idle_after_interaction() {
    let (mut h, s) = scene(Mode::Dark);
    let c = h.engine().coords(s);
    h.drag(
        Point::new(c.x0 + 10, c.y0 + 5),
        Point::new(c.x0 + 150, c.y0 + 5),
        Duration::ms(200),
    );
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn snapshot_slider_states() {
    let states: [(&str, State); 5] = [
        ("default", State::DEFAULT),
        ("pressed", State::PRESSED),
        ("disabled", State::DISABLED),
        ("focused", State::FOCUSED.union(State::FOCUS_KEY)),
        (
            "edited",
            State::FOCUSED.union(State::FOCUS_KEY).union(State::EDITED),
        ),
    ];
    for m in Mode::ALL {
        for (name, st) in states {
            let (mut h, s) = scene(m);
            with(&mut h, s, |w: &mut Slider, cx| w.set_value(cx, 40, false));
            h.engine_mut().add_state(s, st);
            h.run_until_idle();
            h.assert_snapshot(&format!("slider_{name}_{}", m.suffix()));
        }
        let (mut h, s) = scene(m);
        with(&mut h, s, |w: &mut Slider, cx| {
            w.set_mode(cx, SliderMode::Range);
            w.set_value(cx, 80, false);
            w.set_left_value(cx, 30, false);
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("slider_range_{}", m.suffix()));
    }
}

#[test]
fn slider_rotary_event_steps_value_once() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    // `Rotary` comes from the application (e.g. a mouse wheel), whatever device is active.
    let _ = h.encoder_input();
    h.engine_mut()
        .send_event(s, EventCode::Rotary, EventParam::Rotary(5));
    assert_eq!(value(&h, s), 5);
    h.engine_mut()
        .send_event(s, EventCode::Rotary, EventParam::Rotary(-2));
    assert_eq!(value(&h, s), 3);
    assert_eq!(*log.borrow(), [5, 3]);
}

#[test]
fn slider_value_changed_handler_reads_the_slider() {
    let (mut h, s) = scene(Mode::Light);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sn = seen.clone();
    h.engine_mut()
        .add_event_handler(s, EventFilter::Code(EventCode::ValueChanged), move |cx, ev| {
            let w = cx.engine().widget::<Slider>(ev.target).map(Slider::value);
            sn.borrow_mut().push((ev.value(), w));
            EventResult::Continue
        });
    let c = h.engine().coords(s);
    h.tap(Point::new(c.x0 + W * 3 / 4, (c.y0 + c.y1) / 2));
    h.key(Key::Right);
    let seen = seen.borrow();
    assert!(!seen.is_empty());
    for &(param, widget) in seen.iter() {
        assert_eq!(widget, param, "the handler sees the slider with the new value");
    }
}
