//! `Switch`: LVGL defaults, toggling by click / keys / encoder, the knob animation (duration
//! from the style, only the switch redrawn), programmatic changes, orientation, and the
//! default theme's look.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Duration, Point};
use twine_engine::{EventCode, EventFilter, EventResult, GroupDef, Key, MeasureCx, NodeId, ObjFlags};
use twine_style::{Align, Part, Selector, State, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::Orientation;
use twine_widgets::switch::{self, SWITCH_CLASS, Switch};

fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(100, 60, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let s = switch::create(e, screen).unwrap();
    e.align(s, Align::Center, 0, 0);
    h.run_until_idle();
    (h, s)
}

fn checked(h: &EngineHarness, s: NodeId) -> bool {
    h.engine()
        .tree()
        .node(s)
        .unwrap()
        .state()
        .contains(State::CHECKED)
}

fn center(h: &EngineHarness, s: NodeId) -> Point {
    h.engine().coords(s).center()
}

fn count(h: &mut EngineHarness, s: NodeId) -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0));
    let c = n.clone();
    h.engine_mut()
        .add_event_handler(s, EventFilter::Code(EventCode::ValueChanged), move |_, _| {
            c.set(c.get() + 1);
            EventResult::Continue
        });
    n
}

fn knob(h: &EngineHarness, s: NodeId) -> twine_core::Rect {
    get::<Switch>(h, s).knob_area(&MeasureCx::new(h.engine(), s))
}

#[test]
fn switch_defaults() {
    let (mut h, s) = scene(Mode::Light);
    let n = h.engine().tree().node(s).unwrap();
    assert_eq!(n.class().name, "switch");
    assert_eq!(SWITCH_CLASS.parts, &[Part::Main, Part::Indicator, Part::Knob]);
    assert_eq!(SWITCH_CLASS.group_def, GroupDef::True);
    assert!(h.engine().group_of(s).is_some());
    assert!(
        n.flags()
            .contains(ObjFlags::CHECKABLE | ObjFlags::CLICKABLE | ObjFlags::SCROLL_ON_FOCUS)
    );
    assert!(!n.flags().contains(ObjFlags::SCROLLABLE));
    // LVGL `width_def = 4 * LV_DPI_DEF / 10`, `height_def = 4 * LV_DPI_DEF / 17`.
    let c = h.engine().coords(s);
    assert_eq!((c.width(), c.height()), (52, 30));
    assert!(!checked(&h, s));
    let w = get::<Switch>(&h, s);
    assert!(!w.is_animating());
    assert_eq!(w.orientation(), Orientation::Auto);
    // Off: the knob is at the left, shrunk by the theme's -dpx(4) padding.
    let k = knob(&h, s);
    assert_eq!(k, c.inset(twine_core::Insets::new(3, 3, 3 + 22, 3)));
    // The theme: 120 ms (LVGL `anim_fast`).
    assert_eq!(
        h.engine()
            .style_i32(s, Part::Main, twine_style::PropId::AnimDuration),
        120
    );
    h.assert_idle();
}

#[test]
fn switch_setters_same_value_no_invalidate() {
    let (mut h, s) = scene(Mode::Light);
    with(&mut h, s, |w: &mut Switch, cx| {
        w.set_checked(cx, false, true);
        w.set_orientation(cx, Orientation::Auto);
    });
    assert!(h.engine().invalidation_log().is_empty());
    with(&mut h, s, |w: &mut Switch, cx| w.set_checked(cx, true, false));
    h.run_until_idle();
    with(&mut h, s, |w: &mut Switch, cx| w.set_checked(cx, true, true));
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn switch_click_toggles_and_animates() {
    let (mut h, s) = scene(Mode::Light);
    let changed = count(&mut h, s);
    let off = knob(&h, s);
    h.tap(center(&h, s));
    assert!(checked(&h, s));
    assert_eq!(changed.get(), 1);
    assert!(get::<Switch>(&h, s).is_animating());
    h.advance(Duration::ms(60));
    let mid = knob(&h, s);
    assert!(mid.x0 > off.x0, "the knob moves right");
    h.advance(Duration::ms(80));
    assert!(!get::<Switch>(&h, s).is_animating());
    let c = h.engine().coords(s);
    assert_eq!(knob(&h, s).x1, c.x1 - 3, "at the right end");
    // Toggling back mid-way continues from where the knob is.
    h.tap(center(&h, s));
    h.advance(Duration::ms(30));
    let back = knob(&h, s);
    h.tap(center(&h, s));
    h.update();
    let again = knob(&h, s);
    assert!((again.x0 - back.x0).abs() <= 2, "{back} -> {again}");
    h.run_until_idle();
    assert!(checked(&h, s));
    assert_eq!(changed.get(), 3);
    h.assert_idle();
}

#[test]
fn switch_anim_only_redraws_switch() {
    let (mut h, s) = scene(Mode::Light);
    h.tap(center(&h, s));
    let n = h.engine().tree().node(s).unwrap();
    let own = n.coords().expand(i32::from(n.ext_draw()));
    let mut frames = 0;
    for _ in 0..10 {
        h.advance(Duration::ms(16));
        for f in h.flushes() {
            frames += 1;
            assert!(own.contains_rect(&f.area), "{} outside {own}", f.area);
        }
    }
    assert!(frames >= 5);
}

#[test]
fn switch_set_checked_no_anim() {
    let (mut h, s) = scene(Mode::Light);
    let changed = count(&mut h, s);
    with(&mut h, s, |w: &mut Switch, cx| w.set_checked(cx, true, false));
    assert!(checked(&h, s));
    assert!(!get::<Switch>(&h, s).is_animating());
    assert_eq!(changed.get(), 0, "no ValueChanged from code");
    let c = h.engine().coords(s);
    assert_eq!(knob(&h, s).x1, c.x1 - 3, "jumps to the end");
    with(&mut h, s, |w: &mut Switch, cx| w.set_checked(cx, false, true));
    assert!(get::<Switch>(&h, s).is_animating(), "asked to animate");
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn switch_anim_duration_from_style() {
    let (mut h, s) = scene(Mode::Light);
    h.engine_mut()
        .set_local_prop(s, Selector::MAIN, StyleProp::AnimDuration(400));
    h.run_until_idle();
    h.tap(center(&h, s));
    h.advance(Duration::ms(300));
    assert!(get::<Switch>(&h, s).is_animating(), "still running at 300 ms");
    h.advance(Duration::ms(120));
    assert!(!get::<Switch>(&h, s).is_animating());
    // No duration: no animation.
    h.engine_mut()
        .set_local_prop(s, Selector::MAIN, StyleProp::AnimDuration(0));
    h.tap(center(&h, s));
    assert!(!get::<Switch>(&h, s).is_animating());
}

#[test]
fn switch_vertical_auto() {
    let (mut h, s) = scene(Mode::Light);
    h.engine_mut().set_size(s, 30, 52);
    h.run_until_idle();
    let c = h.engine().coords(s);
    assert_eq!(knob(&h, s).y1, c.y1 - 3, "off: at the bottom");
    with(&mut h, s, |w: &mut Switch, cx| w.set_checked(cx, true, false));
    assert_eq!(knob(&h, s).y0, c.y0 + 3, "on: at the top");
    with(&mut h, s, |w: &mut Switch, cx| {
        w.set_orientation(cx, Orientation::Horizontal);
    });
    assert_eq!(knob(&h, s).height(), c.height() - 6);
}

#[test]
fn switch_key_enter_toggles() {
    let (mut h, s) = scene(Mode::Light);
    let changed = count(&mut h, s);
    let _ = h.keypad_input();
    h.engine_mut().focus(s);
    h.run_until_idle();
    h.key(Key::Enter);
    assert!(checked(&h, s));
    assert!(get::<Switch>(&h, s).is_animating());
    h.key(Key::Enter);
    assert!(!checked(&h, s));
    h.key(Key::Right);
    assert!(checked(&h, s), "Right checks (LVGL)");
    h.key(Key::Right);
    assert_eq!(changed.get(), 3);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn switch_encoder_click_toggles() {
    let (mut h, s) = scene(Mode::Light);
    let _ = h.encoder_input();
    h.engine_mut().focus(s);
    h.run_until_idle();
    h.encoder_click();
    assert!(checked(&h, s));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn snapshot_switch_states() {
    let states: [(&str, State); 5] = [
        ("default", State::DEFAULT),
        ("pressed", State::PRESSED),
        ("checked", State::CHECKED),
        ("disabled", State::DISABLED),
        ("focused", State::FOCUSED.union(State::FOCUS_KEY)),
    ];
    for m in Mode::ALL {
        for (name, st) in states {
            let (mut h, s) = scene(m);
            h.engine_mut().add_state(s, st);
            h.run_until_idle();
            h.assert_snapshot(&format!("switch_{name}_{}", m.suffix()));
        }
    }
    // Halfway through the knob animation.
    let (mut h, s) = scene(Mode::Light);
    h.engine_mut()
        .set_local_prop(s, Selector::MAIN, StyleProp::AnimDuration(200));
    h.run_until_idle();
    h.tap(center(&h, s));
    h.advance(Duration::ms(100));
    let st = get::<Switch>(&h, s).anim_state().unwrap();
    assert!((100..=156).contains(&st), "{st}");
    h.assert_panel_snapshot("switch_anim_half_light");
}
