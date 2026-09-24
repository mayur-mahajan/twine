//! `Arc`: LVGL defaults, value ↔ angle mapping in every mode, dragging (rate limit, no jump
//! across the gap), ring-only hit testing, keys and encoder, segment-only invalidation, and
//! the default theme's look.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::math::{cos, sin};
use twine_core::{Angle, Duration, Opa, Point};
use twine_engine::{EventCode, EventFilter, EventParam, EventResult, Key, MeasureCx, NodeId, ObjFlags};
use twine_style::{Align, Part, Selector, State, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::arc::{self, ARC_CLASS, Arc, ArcMode};

fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(160, 160, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let a = arc::create(e, screen).unwrap();
    e.align(a, Align::Center, 0, 0);
    h.run_until_idle();
    (h, a)
}

fn w(h: &EngineHarness, a: NodeId) -> &Arc {
    get::<Arc>(h, a)
}

/// The point on the middle of the ring at `deg` degrees.
fn on_ring(h: &EngineHarness, a: NodeId, deg: i32) -> Point {
    let (c, r) = w(h, a).center(&MeasureCx::new(h.engine(), a));
    let rr = i64::from(r - 6);
    Point::new(
        c.x + ((rr * i64::from(cos(Angle::deg(deg)))) >> 15) as i32,
        c.y + ((rr * i64::from(sin(Angle::deg(deg)))) >> 15) as i32,
    )
}

fn changes(h: &mut EngineHarness, a: NodeId) -> Rc<RefCell<Vec<i32>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    h.engine_mut()
        .add_event_handler(a, EventFilter::Code(EventCode::ValueChanged), move |_, ev| {
            if let EventParam::Value(v) = ev.param {
                l.borrow_mut().push(v);
            }
            EventResult::Continue
        });
    log
}

#[test]
fn arc_defaults() {
    let (mut h, a) = scene(Mode::Light);
    let n = h.engine().tree().node(a).unwrap();
    assert_eq!(n.class().name, "arc");
    assert_eq!(ARC_CLASS.parts, &[Part::Main, Part::Indicator, Part::Knob]);
    assert_eq!(ARC_CLASS.editable, twine_engine::Editable::True);
    assert!(n.flags().contains(ObjFlags::CLICKABLE));
    assert!(
        !n.flags()
            .intersects(ObjFlags::SCROLLABLE | ObjFlags::SCROLL_CHAIN)
    );
    let c = h.engine().coords(a);
    assert_eq!((c.width(), c.height()), (130, 130));
    let x = w(&h, a);
    assert_eq!(
        (x.bg_angle_start(), x.bg_angle_end()),
        (Angle::deg(135), Angle::deg(45))
    );
    assert_eq!(
        (x.angle_start(), x.angle_end()),
        (Angle::deg(135), Angle::deg(270))
    );
    assert_eq!((x.min(), x.max(), x.value()), (0, 100, 0));
    assert_eq!(
        (x.change_rate(), x.mode(), x.rotation()),
        (720, ArcMode::Normal, Angle(0))
    );
    h.assert_idle();
}

#[test]
fn arc_setters_same_value_no_invalidate() {
    let (mut h, a) = scene(Mode::Light);
    with(&mut h, a, |x: &mut Arc, cx| x.set_value(cx, 30));
    h.run_until_idle();
    with(&mut h, a, |x: &mut Arc, cx| {
        x.set_value(cx, 30);
        x.set_range(cx, 0, 100);
        x.set_bg_angles(cx, Angle::deg(135), Angle::deg(45 + 360));
        x.set_rotation(cx, Angle::deg(360));
        x.set_mode(cx, ArcMode::Normal);
        x.set_change_rate(cx, 720);
        x.set_knob_offset(cx, Angle(0));
        let (s, e) = (x.angle_start(), x.angle_end());
        x.set_angles(cx, s, e);
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn arc_value_to_angle_mapping() {
    let (mut h, a) = scene(Mode::Light);
    // (mode, value, start, end) in tenths of a degree, as LVGL `value_update` computes them
    // (background 135° … 45°).
    let table = [
        (ArcMode::Normal, 0, 1350, 1350),
        (ArcMode::Normal, 50, 1350, 2700),
        (ArcMode::Normal, 100, 1350, 450),
        (ArcMode::Normal, 25, 1350, 2025),
        (ArcMode::Reverse, 25, 3375, 450),
        (ArcMode::Reverse, 100, 1350, 450),
        (ArcMode::Symmetrical, 50, 2700, 2700),
        (ArcMode::Symmetrical, 75, 2700, 3375),
        (ArcMode::Symmetrical, 20, 1890, 2700),
    ];
    for (mode, v, s, e) in table {
        with(&mut h, a, |x: &mut Arc, cx| {
            x.set_mode(cx, mode);
            x.set_value(cx, v);
        });
        let x = w(&h, a);
        assert_eq!((x.angle_start().0, x.angle_end().0), (s, e), "{mode:?} {v}");
    }
    // A range change re-maps the value.
    with(&mut h, a, |x: &mut Arc, cx| {
        x.set_mode(cx, ArcMode::Normal);
        x.set_value(cx, 50);
        x.set_range(cx, 0, 200);
    });
    assert_eq!(w(&h, a).angle_end().0, 2025);
}

#[test]
fn arc_reverse_mode() {
    let (mut h, a) = scene(Mode::Light);
    with(&mut h, a, |x: &mut Arc, cx| {
        x.set_mode(cx, ArcMode::Reverse);
        x.set_value(cx, 10);
    });
    let x = w(&h, a);
    assert_eq!(x.angle_end(), Angle::deg(45), "anchored at the background's end");
    // 45° + 360° - 27° (10 % of 270°), counter-clockwise from the end.
    assert_eq!(x.angle_start(), Angle::deg(18), "grows counter-clockwise");
    // The knob sits at the start angle.
    let k = x.knob_area(&MeasureCx::new(h.engine(), a));
    let p = on_ring(&h, a, 18);
    assert!(k.expand(3).contains(p), "{k} {p}");
}

#[test]
fn arc_drag_follows_pointer() {
    let (mut h, a) = scene(Mode::Light);
    with(&mut h, a, |x: &mut Arc, cx| x.set_value(cx, 0));
    let log = changes(&mut h, a);
    // Press on the ring at the start (135°) and move clockwise to 270° slowly.
    h.press(on_ring(&h, a, 136));
    let mut last = 0;
    for deg in (136..=270).step_by(3) {
        h.clock().advance(Duration::ms(20));
        h.move_to(on_ring(&h, a, deg));
        let v = w(&h, a).value();
        assert!(v >= last, "{v} < {last} at {deg}°");
        last = v;
    }
    h.release();
    let v = w(&h, a).value();
    assert!((48..=52).contains(&v), "value {v} at 270°");
    assert!(!log.borrow().is_empty());
    assert!(log.borrow().windows(2).all(|p| p[0] != p[1]));
    assert!(!w(&h, a).is_dragging());
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn arc_change_rate_limits_speed() {
    let (mut h, a) = scene(Mode::Light);
    with(&mut h, a, |x: &mut Arc, cx| {
        x.set_value(cx, 0);
        x.set_change_rate(cx, 90);
    });
    h.press(on_ring(&h, a, 136));
    h.clock().advance(Duration::ms(20));
    h.move_to(on_ring(&h, a, 140));
    // Jump the pointer to 270° (135° away) and hold it there for 500 ms: at 90°/s the arc
    // follows by about 45° (≈ 17 of 100 over 270°), not all the way.
    for _ in 0..5 {
        h.clock().advance(Duration::ms(100));
        h.move_to(on_ring(&h, a, 270));
    }
    let v = w(&h, a).value();
    assert!((10..=25).contains(&v), "rate limited value {v}");
    for _ in 0..30 {
        h.clock().advance(Duration::ms(100));
        h.move_to(on_ring(&h, a, 270));
    }
    let v = w(&h, a).value();
    assert!((48..=52).contains(&v), "catches up: {v}");
    h.release();
}

#[test]
fn arc_no_wraparound_jump() {
    let (mut h, a) = scene(Mode::Light);
    with(&mut h, a, |x: &mut Arc, cx| x.set_value(cx, 95));
    h.run_until_idle();
    // Near the maximum end (45°), then in one quick step across the gap at the bottom to the
    // minimum end (135°): the value does not jump to the minimum, it moves by at most one
    // change-rate step (720°/s).
    h.press(on_ring(&h, a, 40));
    h.clock().advance(Duration::ms(16));
    h.move_to(on_ring(&h, a, 44));
    let before = w(&h, a).value();
    assert!(before >= 95, "{before}");
    h.clock().advance(Duration::ms(16));
    h.move_to(on_ring(&h, a, 136));
    let after = w(&h, a).value();
    assert!(before - after <= 6, "jumped from {before} to {after}");
    // In the gap itself nothing happens.
    h.clock().advance(Duration::ms(16));
    h.move_to(on_ring(&h, a, 90));
    assert_eq!(w(&h, a).value(), after);
    h.release();
}

#[test]
fn arc_hit_test_ring_only() {
    let (mut h, a) = scene(Mode::Light);
    let d = h.display();
    let c = h.engine().coords(a);
    // LVGL: without advanced hit testing the whole box (+ LV_DPI_DEF / 10) is clickable.
    assert_eq!(h.engine_mut().hit_test(d, c.center()), Some(a));
    h.engine_mut().set_flag(a, ObjFlags::ADV_HITTEST, true);
    assert_ne!(
        h.engine_mut().hit_test(d, c.center()),
        Some(a),
        "the middle misses"
    );
    let ring = on_ring(&h, a, 200);
    assert_eq!(h.engine_mut().hit_test(d, ring), Some(a), "the ring hits");
    let gap = on_ring(&h, a, 90);
    assert_ne!(
        h.engine_mut().hit_test(d, gap),
        Some(a),
        "the gap between the ends misses"
    );
    let near_end = on_ring(&h, a, 48);
    assert_eq!(
        h.engine_mut().hit_test(d, near_end),
        Some(a),
        "within the end tolerance"
    );
}

#[test]
fn arc_keys_and_encoder() {
    let mut h = harness(160, 160, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let a = arc::create(h.engine_mut(), screen).unwrap();
    let b = arc::create(h.engine_mut(), screen).unwrap();
    assert_eq!(
        h.engine().group_of(a),
        None,
        "LVGL: arcs are not in the default group"
    );
    h.engine_mut().group_add(g, a);
    h.engine_mut().group_add(g, b);
    let log = changes(&mut h, a);
    let _ = h.keypad_input();
    h.engine_mut().focus(a);
    h.run_until_idle();
    h.key(Key::Right);
    h.key(Key::Up);
    h.key(Key::Left);
    assert_eq!(w(&h, a).value(), 1);
    let _ = h.encoder_input();
    h.encoder_click(); // edit mode
    assert!(h.engine().group_editing(g));
    h.encoder(5);
    assert_eq!(w(&h, a).value(), 6);
    h.encoder_click(); // leaves edit mode
    assert!(!h.engine().group_editing(g));
    assert_eq!(*log.borrow(), vec![1, 2, 1, 2, 3, 4, 5, 6]);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn arc_invalidation_is_segment_box() {
    let (mut h, a) = scene(Mode::Light);
    with(&mut h, a, |x: &mut Arc, cx| x.set_value(cx, 50));
    h.run_until_idle();
    let n = h.engine().tree().node(a).unwrap();
    let full = n.coords().expand(i32::from(n.ext_draw())).area();
    // 4 steps of 2.7° = 10.8°.
    with(&mut h, a, |x: &mut Arc, cx| x.set_value(cx, 54));
    h.advance(Duration::ms(40));
    let dirty = u64::from(h.last_frame().dirty_px);
    assert!(dirty > 0 && dirty * 2 < full, "dirty {dirty} of {full}");
    h.assert_idle();
}

#[test]
fn arc_idle_after_interaction() {
    let (mut h, a) = scene(Mode::Dark);
    let from = on_ring(&h, a, 150);
    let to = on_ring(&h, a, 200);
    h.drag(from, to, Duration::ms(300));
    h.run_until_idle();
    h.assert_idle();
}

fn snap(name: &str, f: impl Fn(&mut EngineHarness, NodeId)) {
    for m in Mode::ALL {
        let (mut h, a) = scene(m);
        with(&mut h, a, |x: &mut Arc, cx| x.set_value(cx, 60));
        f(&mut h, a);
        h.run_until_idle();
        h.assert_snapshot(&format!("arc_{name}_{}", m.suffix()));
    }
}

#[test]
fn snapshot_arc() {
    let states: [(&str, State); 4] = [
        ("default", State::DEFAULT),
        ("pressed", State::PRESSED),
        ("disabled", State::DISABLED),
        ("focused", State::FOCUSED.union(State::FOCUS_KEY)),
    ];
    for (name, st) in states {
        snap(name, |h, a| h.engine_mut().add_state(a, st));
    }
    snap("symmetrical", |h, a| {
        with(h, a, |x: &mut Arc, cx| {
            x.set_mode(cx, ArcMode::Symmetrical);
            x.set_value(cx, 30);
        });
    });
    snap("reverse", |h, a| {
        with(h, a, |x: &mut Arc, cx| x.set_mode(cx, ArcMode::Reverse));
    });
    snap("rotated", |h, a| {
        with(h, a, |x: &mut Arc, cx| {
            x.set_bg_angles(cx, Angle::deg(0), Angle::deg(180));
            x.set_rotation(cx, Angle::deg(180));
        });
    });
    snap("no_knob", |h, a| {
        let e = h.engine_mut();
        e.set_local_prop(a, Selector::part(Part::Knob), StyleProp::BgOpa(Opa::TRANSP));
        e.set_flag(a, ObjFlags::CLICKABLE, false);
    });
}
