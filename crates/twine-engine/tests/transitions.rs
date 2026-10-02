//! Style transitions on state changes (LVGL `lv_obj_style.c` transition behaviour).

mod common;

use twine_core::{Color, Duration, Opa, Rect, Scale};
use twine_engine::{Easing, NodeId, State};
use twine_style::{
    GradDir, Part, PropId, Props, Selector, Style, StyleProp, StyleValue, Transition, TransitionRef,
};
use twine_testing::EngineHarness;

const PROPS: Props = Props::from_ids(&[
    PropId::BgColor,
    PropId::TransformScaleX,
    PropId::TransformScaleY,
    PropId::BgGradientDir,
]);
static TR: Transition = Transition::of(PROPS, Duration::ms(100)).easing(Easing::Linear);
static BASE: Style = Style::new(&[
    StyleProp::BgColor(::twine_style::design::DesignValue::Fixed(Color::RED)),
    StyleProp::BgOpacity(::twine_style::design::DesignValue::Fixed(Opa::COVER)),
    StyleProp::Transition(TransitionRef::Static(&TR)),
]);
static PRESSED: Style = Style::new(&[
    StyleProp::BgColor(::twine_style::design::DesignValue::Fixed(Color::BLUE)),
    StyleProp::TransformScaleX(Scale::from_raw_256(320)),
    StyleProp::TransformScaleY(Scale::from_raw_256(320)),
    StyleProp::BgGradientDir(GradDir::Ver),
    StyleProp::BgGradientColor(::twine_style::design::DesignValue::Fixed(Color::BLUE)),
]);
static PRESSED_SAME: Style = Style::new(&[StyleProp::BgColor(::twine_style::design::DesignValue::Fixed(
    Color::RED,
))]);

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
    Color::mix(to, from, Opa::from_raw(v as u8))
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
        StyleValue::Scale(Scale::from_raw_256(320))
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
    let dir = |h: &EngineHarness| h.engine().style_prop(b, Part::Main, PropId::BgGradientDir);
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
        .set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::GREEN.into()));
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

#[test]
fn dsc_for_state_skips_running_transitions() {
    let (mut h, b) = scene(&PRESSED);
    let t0 = h.now();
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    at(&mut h, t0, 50);
    let e = h.engine();
    assert!(e.transition_count() > 0);
    // The current descriptor shows the transition midway...
    let now = e.rect_dsc(b, Part::Main, Opa::COVER).base.bg_color;
    assert_ne!(now, Color::BLUE);
    assert_ne!(now, Color::RED);
    // ...the state-resolved ones the end values of each state.
    let pressed = e.rect_dsc_for_state(b, Part::Main, State::PRESSED, Opa::COVER);
    assert_eq!(pressed.base.bg_color, Color::BLUE);
    let default = e.rect_dsc_for_state(b, Part::Main, State::DEFAULT, Opa::COVER);
    assert_eq!(default.base.bg_color, Color::RED);
    assert_eq!(default.base.bg_opa, Opa::COVER);
    h.run_until_idle();
    let e = h.engine();
    assert_eq!(
        e.rect_dsc_for_state(b, Part::Main, State::PRESSED, Opa::P50).base,
        e.rect_dsc(b, Part::Main, Opa::P50).base,
        "same as the current descriptor in the current state"
    );
}

#[test]
fn dsc_for_state_applies_ancestor_recolor_and_own_state_recolor() {
    let (mut h, b) = scene(&PRESSED_SAME);
    let e = h.engine_mut();
    let parent = e.tree().parent(b).unwrap();
    e.set_local_prop(parent, Selector::MAIN, StyleProp::Recolor(Color::BLACK.into()));
    e.set_local_prop(parent, Selector::MAIN, StyleProp::RecolorOpacity(Opa::P50.into()));
    let items = Selector::part(Part::Items);
    e.set_local_prop(b, items, StyleProp::TextColor(Color::WHITE.into()));
    e.set_local_prop(
        b,
        Selector::state(State::CHECKED),
        StyleProp::Recolor(Color::BLACK.into()),
    );
    e.set_local_prop(
        b,
        Selector::state(State::CHECKED),
        StyleProp::RecolorOpacity(Opa::COVER.into()),
    );
    let e = h.engine();
    let plain = e.text_dsc_for_state(b, Part::Items, State::DEFAULT, Opa::COVER);
    assert_eq!(plain.color, e.text_dsc(b, Part::Items, Opa::COVER).color);
    assert_ne!(plain.color, Color::WHITE, "the parent's recolor darkens it");
    // In `CHECKED` the node's own `Main` recolor (resolved in that state) covers it fully.
    let checked = e.text_dsc_for_state(b, Part::Items, State::CHECKED, Opa::COVER);
    assert_eq!(checked.color, Color::BLACK);
}

// ---- R2.S06: property sets, derived lists, inline transitions ---------------------------------

/// A drawn red 20 × 20 box; `setup` adds its transitions and pressed look.
fn inline_scene(setup: impl FnOnce(&mut twine_engine::Engine, NodeId)) -> (EngineHarness, NodeId) {
    let mut b = None;
    let mut setup = Some(setup);
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        let n = common::boxed(e, s, Rect::from_xywh(40, 20, 20, 20), Color::RED);
        (setup.take().unwrap())(e, n);
        b = Some(n);
    });
    h.run_until_idle();
    (h, b.unwrap())
}

fn scale_x(h: &EngineHarness, b: NodeId) -> StyleValue {
    h.engine().style_prop(b, Part::Main, PropId::TransformScaleX)
}

const PRESSED_SEL: Selector = Selector::state(State::PRESSED);

/// The pressed look of the R2.S06 tests: background, scale (interpolable) and a gradient
/// direction (discrete).
fn pressed_look(e: &mut twine_engine::Engine, n: NodeId) {
    e.set_local_prop(n, PRESSED_SEL, StyleProp::BgColor(Color::BLUE.into()));
    e.set_local_prop(
        n,
        PRESSED_SEL,
        StyleProp::TransformScaleX(Scale::from_raw_256(320)),
    );
    e.set_local_prop(n, PRESSED_SEL, StyleProp::BgGradientDir(GradDir::Ver));
}

#[test]
fn inline_transition_animates_only_what_the_state_changes() {
    let (mut h, b) = inline_scene(|e, n| {
        // Set on the default state, built at run time: no `'static`, no property list.
        e.set_local_transition(n, Selector::MAIN, Transition::all(Duration::ms(100)));
        pressed_look(e, n);
    });
    let t0 = h.now();
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    // Background and scale animate; the discrete gradient direction switches at once; the
    // radius, width, … are not touched.
    assert_eq!(h.engine().transition_count(), 2);
    assert_eq!(
        h.engine().style_prop(b, Part::Main, PropId::BgGradientDir),
        StyleValue::from(GradDir::Ver)
    );
    at(&mut h, t0, 50);
    assert_eq!(bg(&h, b), mixed(Color::BLUE, Color::RED, 50));
    assert_ne!(scale_x(&h, b), StyleValue::Scale(Scale::from_raw_256(320)));
    at(&mut h, t0, 100);
    assert_eq!(bg(&h, b), Color::BLUE);
    assert_eq!(scale_x(&h, b), StyleValue::Scale(Scale::from_raw_256(320)));
    assert_eq!(h.engine().transition_count(), 0);
    // Releasing animates back the same way.
    h.engine_mut().clear_state(b, State::PRESSED);
    h.update();
    assert_eq!(h.engine().transition_count(), 2);
    h.run_until_idle();
    assert_eq!(bg(&h, b), Color::RED);
}

#[test]
fn group_transition_animates_its_groups_only() {
    let (mut h, b) = inline_scene(|e, n| {
        e.set_local_transition(n, Selector::MAIN, Transition::of(Props::BG, Duration::ms(100)));
        pressed_look(e, n);
    });
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    // `BgColor` (and the listed, discrete `BgGradientDir`, which switches at the end, as in
    // LVGL); the scale is not in `Props::BG` and jumps.
    assert_eq!(h.engine().transition_count(), 2);
    assert_eq!(scale_x(&h, b), StyleValue::Scale(Scale::from_raw_256(320)));
    assert_eq!(bg(&h, b), Color::RED);
    h.run_until_idle();
    assert_eq!(bg(&h, b), Color::BLUE);
}

#[test]
fn the_higher_state_weight_wins_per_property() {
    let (mut h, b) = inline_scene(|e, n| {
        e.set_local_transition(n, Selector::MAIN, Transition::all(Duration::ms(300)));
        // Entering the pressed state: the background with the pressed transition (100 ms),
        // everything else with the default one (300 ms).
        e.set_local_transition(n, PRESSED_SEL, Transition::of(Props::BG, Duration::ms(100)));
        pressed_look(e, n);
    });
    let t0 = h.now();
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    at(&mut h, t0, 100);
    assert_eq!(bg(&h, b), Color::BLUE, "the pressed transition's 100 ms");
    assert!(
        scale_x(&h, b) != StyleValue::Scale(Scale::from_raw_256(320)),
        "300 ms"
    );
    at(&mut h, t0, 300);
    assert_eq!(scale_x(&h, b), StyleValue::Scale(Scale::from_raw_256(320)));
    // Leaving the pressed state, only the default transition applies (300 ms for both).
    let t1 = h.now();
    h.engine_mut().clear_state(b, State::PRESSED);
    h.update();
    at(&mut h, t1, 100);
    assert_ne!(bg(&h, b), Color::RED);
    at(&mut h, t1, 300);
    assert_eq!(bg(&h, b), Color::RED);
}

#[test]
fn a_state_transition_runs_when_entering_that_state_only() {
    let (mut h, b) = inline_scene(|e, n| {
        e.set_local_transition(n, PRESSED_SEL, Transition::all(Duration::ms(100)).ease_out());
        pressed_look(e, n);
    });
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
    assert_eq!(h.engine().transition_count(), 2);
    h.run_until_idle();
    h.engine_mut().clear_state(b, State::PRESSED);
    h.update();
    assert_eq!(
        h.engine().transition_count(),
        0,
        "no transition into the default state"
    );
    assert_eq!(bg(&h, b), Color::RED);
}

#[test]
fn inline_transitions_are_idempotent_and_owned_by_the_style() {
    let (mut h, b) = inline_scene(|e, n| {
        e.set_local_transition(n, Selector::MAIN, Transition::all(Duration::ms(100)));
        pressed_look(e, n);
    });
    h.engine_mut()
        .set_local_transition(b, Selector::MAIN, Transition::all(Duration::ms(100)));
    assert_eq!(h.update(), twine_engine::Wake::Idle);
    assert!(h.flushes().is_empty(), "an equal transition changes nothing");
    let shared = std::rc::Rc::new(Transition::all(Duration::ms(50)));
    h.engine_mut()
        .set_local_transition(b, Selector::MAIN, shared.clone());
    assert_eq!(
        std::rc::Rc::strong_count(&shared),
        2,
        "held by the node's local style"
    );
    h.engine_mut().delete(b).unwrap();
    assert_eq!(std::rc::Rc::strong_count(&shared), 1, "released with the node");
}

// ---- R2.S07: state precedence and application states ----------------------------------------

#[test]
fn custom_state_transitions_derive_props_and_outrank_disabled() {
    const ALARM: State = State::custom::<0>();
    let alarm = Selector::state(ALARM);
    let disabled = Selector::state(State::DISABLED);
    let (mut h, b) = inline_scene(|e, n| {
        e.set_local_transition(n, Selector::MAIN, Transition::all(Duration::ms(300)));
        e.set_local_prop(n, disabled, StyleProp::BgColor(Color::GREEN.into()));
        e.set_local_transition(n, disabled, Transition::of(Props::BG, Duration::ms(200)));
        e.set_local_prop(n, alarm, StyleProp::BgColor(Color::BLUE.into()));
        e.set_local_prop(n, alarm, StyleProp::TransformScaleX(Scale::from_raw_256(320)));
        e.set_local_transition(n, alarm, Transition::of(Props::BG, Duration::ms(100)));
    });
    let t0 = h.now();
    h.engine_mut().add_state(b, State::DISABLED | ALARM);
    h.update();
    // Derived from the entries that start applying: background and scale (both interpolable).
    assert_eq!(h.engine().transition_count(), 2);
    at(&mut h, t0, 50);
    assert_eq!(
        bg(&h, b),
        mixed(Color::BLUE, Color::RED, 50),
        "towards ALARM's blue, not DISABLED's green"
    );
    at(&mut h, t0, 100);
    assert_eq!(
        bg(&h, b),
        Color::BLUE,
        "ALARM's own 100 ms transition (it outranks DISABLED's)"
    );
    assert_ne!(
        scale_x(&h, b),
        StyleValue::Scale(Scale::from_raw_256(320)),
        "default 300 ms"
    );
    h.run_until_idle();
    assert_eq!(scale_x(&h, b), StyleValue::Scale(Scale::from_raw_256(320)));
    // Leaving ALARM: DISABLED's look applies, through the default transition.
    h.engine_mut().clear_state(b, ALARM);
    h.update();
    h.run_until_idle();
    assert_eq!(bg(&h, b), Color::GREEN);
}

/// An `ANY`-state style applied in resolution but was skipped when transitions started (its
/// weight was read as the raw bits there); now every part of the engine uses
/// `Selector::state_matches` / `Selector::weight`.
#[test]
fn regression_any_state_transition_starts_like_a_default_one() {
    for sel in [Selector::MAIN, Selector::state(State::ANY)] {
        let (mut h, b) = inline_scene(|e, n| {
            e.set_local_transition(n, sel, Transition::all(Duration::ms(100)));
            pressed_look(e, n);
        });
        let t0 = h.now();
        h.engine_mut().add_state(b, State::PRESSED);
        h.update();
        assert_eq!(h.engine().transition_count(), 2, "{sel:?}");
        at(&mut h, t0, 50);
        assert_eq!(bg(&h, b), mixed(Color::BLUE, Color::RED, 50), "{sel:?}");
        h.run_until_idle();
        assert_eq!(bg(&h, b), Color::BLUE);
    }
}
