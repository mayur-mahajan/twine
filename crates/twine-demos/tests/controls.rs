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
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(app_static);
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
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(app_static);
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
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(app_static);
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
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.advance(Duration::ms(500));
    assert!(t.engine().anim_count() > 0, "spinner and animation run");
    let mut s = TestUi::new(240, 320)
        .app_config(twine_demos::config())
        .mount(app_static);
    s.run_until_idle();
    s.assert_idle();
}

#[test]
fn snapshots_initial() {
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(app_static);
    t.run_until_idle();
    t.assert_snapshot("controls_initial_light");
    let mut t = TestUi::new(320, 240)
        .theme(DefaultTheme::dark())
        .mount(app_static);
    t.run_until_idle();
    t.assert_snapshot("controls_initial_dark");
}

/// R2.S03: the controls (slider, bar, arc, switch, checkbox, buttons, labels) in every mode of
/// the default theme, switched at run time; the keypad focus ring is shown on the slider.
#[test]
fn snapshots_theme_modes() {
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(app_static);
    t.run_until_idle();
    let slider = id(&t, "slider");
    t.engine_mut().focus(slider);
    t.engine_mut().add_state(slider, State::FOCUS_KEY);
    t.run_until_idle();
    let theme = use_theme(t.root_scope());
    assert_eq!(theme.modes(), &ThemeMode::ALL);
    for (mode, name) in [
        (ThemeMode::Light, "light"),
        (ThemeMode::Dark, "dark"),
        (ThemeMode::Night, "night"),
        (ThemeMode::HighContrast, "high_contrast"),
    ] {
        theme.set_mode(mode);
        t.run_until_idle();
        assert_eq!(theme.mode(), mode);
        t.assert_snapshot(&format!("controls_mode_{name}"));
    }
}

/// R2.S02: switching the default theme from light to dark at run time (a design element
/// table swap, no re-applied styles, no rebuilt views) draws exactly what installing the dark
/// theme draws.
#[test]
fn mode_switch_draws_like_the_dark_theme() {
    let mut switched = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(app_static);
    switched.run_until_idle();
    use_theme(switched.root_scope()).set_mode(ThemeMode::Dark);
    switched.run_until_idle();
    switched.harness_mut().render_full();
    let mut dark = TestUi::new(320, 240)
        .theme(DefaultTheme::dark())
        .mount(app_static);
    dark.run_until_idle();
    dark.harness_mut().render_full();
    assert!(
        switched.harness_mut().panel_rgb888() == dark.harness_mut().panel_rgb888(),
        "light → dark switch differs from the dark theme"
    );
}
