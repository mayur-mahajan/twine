//! The `controls` demo: shared signals keep the controls in sync, the keypad reaches every
//! control, and the static variant is idle between inputs.

use twine::core::{Duration, Point};
use twine::engine::NodeId;
use twine::hal::Key;
use twine::prelude::*;
use twine::widgets::arc::Arc;
use twine::widgets::bar::Bar;
use twine::widgets::slider::Slider;
use twine_demos::controls::{app, app_static};
use twine_testing::{TestUi, by_id};

fn id(t: &TestUi, s: &'static str) -> NodeId {
    t.find(by_id(s)).id()
}

fn value<W: Widget>(t: &TestUi, s: &'static str, f: impl FnOnce(&W) -> i32) -> i32 {
    let n = id(t, s);
    f(t.engine().widget::<W>(n).unwrap())
}

fn checked(t: &TestUi, s: &'static str) -> bool {
    t.find(by_id(s)).state().contains(State::CHECKED)
}

#[test]
fn slider_drives_bar_arc_and_label() {
    let mut t = TestUi::new(320, 240).mount(app_static);
    t.run_until_idle();
    let c = t.find(by_id("slider")).coords();
    let knob = Point::new(c.x0 + c.width() * 40 / 100, c.center().y);
    t.drag(
        knob,
        Point::new(c.x0 + c.width() * 3 / 4, c.center().y),
        Duration::ms(200),
    );
    t.run_until_idle();
    let v = value(&t, "slider", Slider::value);
    assert!((70..=80).contains(&v), "{v}");
    assert_eq!(value(&t, "bar", Bar::value), v);
    assert_eq!(value(&t, "arc", Arc::value), v);
    assert_eq!(t.find(by_id("level")).text(), format!("{v}%"));
    t.assert_idle();
}

#[test]
fn switch_checkbox_and_power_button_stay_in_sync() {
    let mut t = TestUi::new(320, 240).mount(app_static);
    t.run_until_idle();
    t.find(by_id("switch")).click();
    t.run_until_idle();
    assert!(checked(&t, "switch") && checked(&t, "check") && checked(&t, "power"));
    t.assert_idle();
    t.find(by_id("power")).tap();
    t.run_until_idle();
    assert!(!checked(&t, "switch") && !checked(&t, "check") && !checked(&t, "power"));
    t.assert_idle();
}

#[test]
fn keypad_reaches_every_control() {
    let mut t = TestUi::new(320, 240).mount(app_static);
    t.run_until_idle();
    let g = t.engine().default_group().unwrap();
    let mut seen = Vec::new();
    for _ in 0..8 {
        let f = t.engine().focused(g).unwrap();
        seen.push(
            t.engine()
                .tree()
                .node(f)
                .and_then(twine::engine::Node::test_id)
                .unwrap_or("?"),
        );
        t.key(Key::Next);
        t.run_until_idle();
    }
    for c in ["slider", "arc", "switch", "power", "check"] {
        assert!(seen.contains(&c), "{c} not reached: {seen:?}");
    }
    // Enter on the focused switch toggles the shared state.
    let sw = id(&t, "switch");
    t.engine_mut().focus(sw);
    t.key(Key::Enter);
    t.run_until_idle();
    assert!(checked(&t, "check"));
    t.assert_idle();
}

#[test]
fn animated_variant_runs_and_idle_variant_is_idle() {
    let mut t = TestUi::new(320, 240).mount(app);
    t.advance(Duration::ms(500));
    assert!(t.engine().anim_count() > 0, "spinner and animation run");
    let mut s = TestUi::new(240, 320).mount(app_static);
    s.run_until_idle();
    s.assert_idle();
}

#[test]
fn snapshots_initial() {
    use std::rc::Rc;
    let mut t = TestUi::new(320, 240).mount(app_static);
    t.run_until_idle();
    t.assert_snapshot("controls_initial_light");
    let mut t = TestUi::new(320, 240)
        .theme(Rc::new(DefaultTheme::dark()))
        .mount(app_static);
    t.run_until_idle();
    t.assert_snapshot("controls_initial_dark");
}
