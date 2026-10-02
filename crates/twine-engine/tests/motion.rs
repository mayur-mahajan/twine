//! The motion preference (R2.S06): what `Motion::Reduced` and `Motion::None` do to every
//! animation kind of the engine — style transitions, node and closure animations, screen loads,
//! scroll animations — that essential animations are untouched, and what changing the
//! preference does to running animations.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{boxed, list, white_screen};
use twine_anim::{Anim, AnimSpec, Motion, Repeat};
use twine_core::{Color, Duration, Rect};
use twine_engine::{AnimProp, NodeId, ScreenAnim, State};
use twine_style::{Part, PropId, Selector, StyleProp, Transition};
use twine_testing::EngineHarness;

/// A drawn 20 × 20 red box whose pressed state is blue, animated by `t` (on the default state:
/// every state change).
fn scene(motion: Motion, t: Transition) -> (EngineHarness, NodeId) {
    let mut b = None;
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let n = boxed(e, s, Rect::from_xywh(40, 20, 20, 20), Color::RED);
        e.set_local_prop(
            n,
            Selector::state(State::PRESSED),
            StyleProp::BgColor(Color::BLUE.into()),
        );
        e.set_local_transition(n, Selector::MAIN, t);
        b = Some(n);
    });
    h.run_until_idle();
    h.engine_mut().set_motion(motion);
    (h, b.unwrap())
}

fn bg(h: &EngineHarness, b: NodeId) -> Color {
    h.engine().style_color(b, Part::Main, PropId::BgColor)
}

fn press(h: &mut EngineHarness, b: NodeId) {
    h.engine_mut().add_state(b, State::PRESSED);
    h.update();
}

#[test]
fn transitions_play_in_full() {
    let (mut h, b) = scene(Motion::Full, Transition::all(Duration::ms(300)));
    press(&mut h, b);
    assert_eq!(h.engine().transition_count(), 1);
    h.advance(Duration::ms(150));
    assert!(
        bg(&h, b) != Color::RED && bg(&h, b) != Color::BLUE,
        "mixed half way"
    );
    h.advance(Duration::ms(160));
    assert_eq!(bg(&h, b), Color::BLUE);
}

#[test]
fn reduced_motion_caps_transitions() {
    let (mut h, b) = scene(Motion::Reduced, Transition::all(Duration::ms(300)));
    press(&mut h, b);
    assert_eq!(h.engine().transition_count(), 1, "still a (short) fade");
    h.advance(Duration::ms(50));
    assert!(bg(&h, b) != Color::RED && bg(&h, b) != Color::BLUE);
    h.advance(Duration::ms(60));
    assert_eq!(bg(&h, b), Color::BLUE, "done after the 100 ms cap, not 300 ms");
    assert_eq!(h.engine().transition_count(), 0);
}

#[test]
fn no_motion_creates_no_transition() {
    let (mut h, b) = scene(
        Motion::None,
        Transition::all(Duration::ms(300)).delay(Duration::ms(50)),
    );
    press(&mut h, b);
    assert_eq!(h.engine().transition_count(), 0);
    assert_eq!(h.engine().anim_count(), 0);
    assert_eq!(bg(&h, b), Color::BLUE, "the new value at once");
}

#[test]
fn essential_transitions_ignore_the_preference() {
    let (mut h, b) = scene(Motion::None, Transition::all(Duration::ms(300)).essential());
    press(&mut h, b);
    assert_eq!(h.engine().transition_count(), 1);
    h.advance(Duration::ms(150));
    assert_ne!(bg(&h, b), Color::BLUE, "still playing");
    h.run_until_idle();
    assert_eq!(bg(&h, b), Color::BLUE);
}

/// An engine with a drawn box and `motion`.
fn engine_scene(motion: Motion) -> (EngineHarness, NodeId) {
    let (h, b) = scene(motion, Transition::all(Duration::ZERO));
    (h, b)
}

#[test]
fn reduced_motion_shortens_node_animations_and_stops_loops() {
    let (mut h, b) = engine_scene(Motion::Reduced);
    let e = h.engine_mut();
    e.anim_start(b, AnimProp::X, Anim::new(0, 50).duration(Duration::secs(1)));
    e.anim_start(
        b,
        AnimProp::Y,
        Anim::new(0, 50)
            .duration(Duration::secs(1))
            .repeat(Repeat::Forever),
    );
    h.advance(Duration::ms(120));
    assert_eq!(
        h.engine().anim_count(),
        0,
        "both capped to 100 ms, the loop played once"
    );
    assert_eq!(
        h.engine().style_prop(b, Part::Main, PropId::X),
        StyleProp::X(twine_style::Length::Px(50).into()).value()
    );
}

#[test]
fn no_motion_jumps_and_still_completes() {
    let (mut h, b) = engine_scene(Motion::None);
    let done = Rc::new(Cell::new(false));
    let d2 = done.clone();
    let seen = Rc::new(Cell::new(0));
    let s2 = seen.clone();
    let e = h.engine_mut();
    e.anim_start(
        b,
        AnimProp::Width,
        Anim::new(10, 70)
            .duration(Duration::secs(2))
            .delay(Duration::secs(1))
            .on_complete(move |_| d2.set(true)),
    );
    // A closure animation (what the view layer's tweens use) of a non-integer value.
    e.anim_start_fn(
        Anim::with_spec(0, 1024, AnimSpec::new(Duration::secs(3)).ease_in_out()),
        move |_, v| s2.set(v),
    );
    h.update();
    assert!(done.get(), "on_complete ran");
    assert_eq!(seen.get(), 1024, "end value at once");
    assert_eq!(h.engine().coords(b).width(), 70);
    assert_eq!(h.engine().anim_count(), 0);
}

#[test]
fn essential_animations_are_untouched() {
    for motion in [Motion::Reduced, Motion::None] {
        let (mut h, b) = engine_scene(motion);
        let id = h.engine_mut().anim_start(
            b,
            AnimProp::Custom(1),
            Anim::new(0, 100)
                .duration(Duration::ms(500))
                .repeat(Repeat::Forever)
                .essential(),
        );
        h.advance(Duration::secs(2));
        assert!(
            h.engine().anim_exists(id),
            "{motion:?}: an essential loop keeps running"
        );
        assert_eq!(h.engine().anim_count(), 1);
    }
}

#[test]
fn a_stricter_preference_ends_running_animations() {
    let (mut h, b) = engine_scene(Motion::Full);
    let e = h.engine_mut();
    let lp = e.anim_start(
        b,
        AnimProp::X,
        Anim::new(0, 50)
            .duration(Duration::ms(200))
            .repeat(Repeat::Forever),
    );
    let fin = e.anim_start(b, AnimProp::Y, Anim::new(0, 50).duration(Duration::secs(5)));
    let ess = e.anim_start(
        b,
        AnimProp::Custom(2),
        Anim::new(0, 9)
            .duration(Duration::ms(100))
            .repeat(Repeat::Forever)
            .essential(),
    );
    h.advance(Duration::ms(50));
    // Reduced: the endless decorative loop ends, the finite one finishes as started.
    h.engine_mut().set_motion(Motion::Reduced);
    h.update();
    assert!(!h.engine().anim_exists(lp));
    assert_eq!(
        h.engine().style_prop(b, Part::Main, PropId::X),
        StyleProp::X(twine_style::Length::Px(50).into()).value()
    );
    assert!(h.engine().anim_exists(fin) && h.engine().anim_exists(ess));
    // None: every non-essential animation jumps to its end.
    h.engine_mut().set_motion(Motion::None);
    h.update();
    assert!(!h.engine().anim_exists(fin));
    assert_eq!(
        h.engine().coords(b).y0 - h.engine().coords(h.engine().tree().parent(b).unwrap()).y0,
        50
    );
    assert!(h.engine().anim_exists(ess), "essential: untouched");
    // Idempotent.
    h.engine_mut().set_motion(Motion::None);
    assert_eq!(h.engine().motion(), Motion::None);
}

#[test]
fn no_motion_loads_screens_at_once() {
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    let d = h.display();
    h.engine_mut().set_motion(Motion::None);
    let second = h.engine_mut().create_screen(d).unwrap();
    h.engine_mut().load_screen_anim(
        second,
        ScreenAnim::MoveLeft(Duration::ms(500)).delay(Duration::ms(100)),
    );
    h.update();
    assert!(!h.engine().screen_anim_running(d));
    assert_eq!(h.engine().active_screen(d), Some(second));
}

#[test]
fn reduced_motion_shortens_screen_loads() {
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    let d = h.display();
    h.engine_mut().set_motion(Motion::Reduced);
    let second = h.engine_mut().create_screen(d).unwrap();
    h.engine_mut()
        .load_screen_anim(second, ScreenAnim::FadeIn(Duration::ms(500)));
    h.update();
    assert!(h.engine().screen_anim_running(d));
    h.advance(Duration::ms(120));
    assert!(!h.engine().screen_anim_running(d), "capped at 100 ms");
}

#[test]
fn no_motion_scrolls_without_animation() {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        ids = Some(list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40));
    });
    h.run_until_idle();
    let (cont, _) = ids.unwrap();
    h.engine_mut().set_motion(Motion::None);
    h.engine_mut().scroll_to_y(cont, 100, true);
    h.update();
    assert_eq!(h.engine().scroll_offset(cont).y, 100);
    assert!(!h.engine().is_scrolling(cont));
}
