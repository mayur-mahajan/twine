//! Style transitions on state changes (LVGL `lv_obj_style.c` transition behaviour).

mod common;

use twine_core::{Color, Duration, Opa, Rect, Scale};
use twine_engine::{Easing, NodeId, State};
use twine_style::{GradDir, Part, PropId, Selector, Style, StyleProp, StyleValue, TransitionDsc};
use twine_testing::EngineHarness;

static PROPS: [PropId; 4] = [
    PropId::BgColor,
    PropId::TransformScaleX,
    PropId::TransformScaleY,
    PropId::BgGradDir,
];
static TR: TransitionDsc = TransitionDsc::new(&PROPS, Duration::ms(100), Easing::Linear);
static BASE: Style = Style::new(&[
    StyleProp::BgColor(Color::RED),
    StyleProp::BgOpa(Opa::COVER),
    StyleProp::Transition(&TR),
]);
static PRESSED: Style = Style::new(&[
    StyleProp::BgColor(Color::BLUE),
    StyleProp::TransformScaleX(Scale(320)),
    StyleProp::TransformScaleY(Scale(320)),
    StyleProp::BgGradDir(GradDir::Ver),
    StyleProp::BgGradColor(Color::BLUE),
]);
static PRESSED_SAME: Style = Style::new(&[StyleProp::BgColor(Color::RED)]);

/// A 100 × 60 white display with a 20 × 20 box at (40, 20) styled with `BASE` / `pressed`.
fn scene(pressed: &'static Style) -> (EngineHarness, NodeId) {
    let mut b = None;
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        let n = common::boxed(e, s, Rect::from_xywh(40, 20, 20, 20), Color::WHITE);
        e.remove_all_styles(n);
        e.set_pos(n, 40, 20);
        e.set_size(n, 20, 20);
        e.add_style(n, &BASE, Selector::MAIN);
        e.add_style(n, pressed, Selector::state(State::PRESSED));
        b = Some(n);
    });
    h.run_until_idle();
    (h, b.unwrap())
}

fn bg(h: &EngineHarness, b: NodeId) -> Color {
    h.engine().style_color(b, Part::Main, PropId::BgColor)
}

/// Sets the clock to `t0 + ms` and updates.
fn at(h: &mut EngineHarness, t0: twine_core::Instant, ms: u64) {
    h.clock().set(t0 + Duration::ms(ms));
    h.update();
}

/// Mix of LVGL's `trans_anim_cb` at `ms` of a 100 ms linear transition.
fn mixed(to: Color, from: Color, ms: u64) -> Color {
    let v = (255 * ((ms * 1024 / 100) as i32)) >> 10;
    Color::mix(to, from, Opa(v as u8))
}

#[test]
fn pressed_bg_color_transitions_over_duration() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    assert_eq!(bg(&h, b), Color::RED);
    assert_eq!(h.pixel(50, 30), Color::RED);
    at(&mut h, t0, 50);
    assert_eq!(bg(&h, b), mixed(Color::BLUE, Color::RED, 50));
    at(&mut h, t0, 100);
    assert_eq!(bg(&h, b), Color::BLUE);
    assert_eq!(
        h.engine().style_prop(b, Part::Main, PropId::TransformScaleX),
        StyleValue::Scale(Scale(320))
    );
    assert_eq!(h.engine().transition_count(), 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn release_mid_transition_starts_from_current_value() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    at(&mut h, t0, 50);
    let mid = bg(&h, b);
    assert_eq!(mid, mixed(Color::BLUE, Color::RED, 50));
    h.engine_mut().clear_state(b, State::PRESSED);
    // No jump: still the mixed color, now going back to red from there.
    assert_eq!(bg(&h, b), mid);
    h.update();
    assert_eq!(bg(&h, b), mid);
    assert_eq!(
        h.engine().transition_count(),
        4,
        "the pressing transitions were replaced"
    );
    at(&mut h, t0, 100);
    assert_eq!(bg(&h, b), mixed(Color::RED, mid, 50));
    at(&mut h, t0, 150);
    assert_eq!(bg(&h, b), Color::RED);
    h.run_until_idle();
    assert_eq!(h.engine().transition_count(), 0);
}

#[test]
fn non_interpolable_prop_switches_at_end() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    let dir = |h: &EngineHarness| h.engine().style_prop(b, Part::Main, PropId::BgGradDir);
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    assert_eq!(dir(&h), StyleValue::from(GradDir::None));
    at(&mut h, t0, 99);
    assert_eq!(dir(&h), StyleValue::from(GradDir::None));
    at(&mut h, t0, 100);
    assert_eq!(dir(&h), StyleValue::from(GradDir::Ver));
}

#[test]
fn transition_entry_removed_after_completion() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    let len = |h: &EngineHarness| h.engine().tree().node(b).unwrap().styles().len();
    let before = len(&h);
    h.engine_mut().add_state(b, State::PRESSED);
    assert_eq!(len(&h), before + 1, "one transition entry for the part");
    h.update();
    at(&mut h, t0, 60);
    assert_eq!(len(&h), before + 1);
    at(&mut h, t0, 100);
    assert_eq!(len(&h), before);
    assert_eq!(bg(&h, b), Color::BLUE);
}

#[test]
fn no_transition_when_value_unchanged() {
    let (mut h, b) = scene(&PRESSED_SAME);
    h.engine_mut().add_state(b, State::PRESSED);
    assert_eq!(h.engine().transition_count(), 0);
    assert_eq!(h.engine().anim_count(), 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn transition_invalidates_only_node() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    // A frame clears the invalidation log.
    at(&mut h, t0, 16);
    h.clock().set(t0 + Duration::ms(50));
    h.engine_mut().run_anims(t0 + Duration::ms(50));
    h.engine_mut().update_layout();
    let area = h
        .engine()
        .coords(b)
        .expand(i32::from(h.engine().tree().node(b).unwrap().ext_draw()));
    let log = h.engine().invalidation_log();
    assert!(!log.is_empty());
    for (r, _) in log {
        assert!(area.contains_rect(r), "{r} outside {area}");
    }
}

#[test]
fn never_drawn_node_changes_instantly() {
    let mut h = EngineHarness::new(40, 40).no_theme();
    let s = h.screen();
    let e = h.engine_mut();
    let n = e.create(s, Box::new(twine_engine::Obj)).unwrap();
    e.add_style(n, &BASE, Selector::MAIN);
    e.add_style(n, &PRESSED, Selector::state(State::PRESSED));
    e.add_state(n, State::PRESSED);
    assert_eq!(e.transition_count(), 0);
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::BLUE);
}

#[test]
fn setting_local_prop_stops_its_transition() {
    let (mut h, b) = scene(&PRESSED);
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::GREEN));
    assert_eq!(h.engine().transition_count(), 3);
    // The transition value is gone: the pressed style (higher state weight than the local
    // default-state property) applies directly.
    assert_eq!(bg(&h, b), Color::BLUE);
}

#[test]
fn deleting_node_removes_transitions() {
    let (mut h, b) = scene(&PRESSED);
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    assert_eq!(h.engine().transition_count(), 4);
    h.engine_mut().delete(b).unwrap();
    assert_eq!(h.engine().transition_count(), 0);
    assert_eq!(h.engine().anim_count(), 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn transition_press_snapshots() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    let d = h.display();
    h.engine_mut().add_state(b, State::PRESSED);
    for (ms, name) in [
        (0, "transition_press_0"),
        (50, "transition_press_50"),
        (100, "transition_press_100"),
    ] {
        h.clock().set(t0 + Duration::ms(ms) + Duration::ms(16));
        h.engine_mut().run_anims(t0 + Duration::ms(ms));
        let area = Rect::from_xywh(0, 0, 100, 60);
        h.engine_mut()
            .invalidate_area(d, area, twine_engine::InvalidateReason::Explicit);
        // Render at the sampled time (the refresh period has passed since the last frame).
        h.engine_mut().update_layout();
        let now = h.now();
        h.engine_mut().refresh(now);
        h.assert_panel_snapshot(name);
    }
}
