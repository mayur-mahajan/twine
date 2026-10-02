//! R2.S06: transitions and motion in the view layer — inline `.transition(..)` on views and in
//! state scopes, `tween` / `animation` of any `Interpolate` value with one `AnimSpec`, and the
//! global `Motion` preference (`use_motion`, tracked and settable; honoured by tweens and
//! animations unless they are essential).

use std::cell::Cell;
use std::rc::Rc;

use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;

/// A value the mounted application hands to the test.
type Slot<T> = Rc<Cell<Option<T>>>;

fn bg(t: &TestUi, id: &'static str) -> Color {
    let n = t.find(by_id(id)).id();
    t.engine().style_color(n, Part::Main, PropId::BgColor)
}

#[test]
fn inline_transition_on_a_view_animates_the_pressed_look() {
    let mut t = TestUi::new(120, 80).no_theme().mount(|_| {
        container(())
            .size(40, 40)
            .bg(Color::RED)
            .transition(Transition::all(Duration::ms(100)))
            .on_state(State::PRESSED, |s| {
                s.bg(Color::BLUE).transform_scale(Scale::pct(90))
            })
            .test_id("b")
    });
    t.run_until_idle();
    let b = t.find(by_id("b")).id();
    t.engine_mut().add_state(b, State::PRESSED);
    t.update();
    // Background and scale (x and y: the `transform_scale` shorthand), derived from the
    // pressed style.
    assert_eq!(t.engine().transition_count(), 3);
    t.advance(Duration::ms(48));
    let mid = bg(&t, "b");
    assert!(mid != Color::RED && mid != Color::BLUE, "{mid:?}");
    t.run_until_idle();
    assert_eq!(bg(&t, "b"), Color::BLUE);
}

#[test]
fn inline_transition_in_a_state_scope() {
    let mut t = TestUi::new(120, 80).no_theme().mount(|_| {
        container(())
            .size(40, 40)
            .bg(Color::RED)
            .on_state(State::CHECKED, |s| {
                s.transition(Transition::of(Props::BG, Duration::ms(100)).ease_out())
                    .bg(Color::BLUE)
                    .radius(8)
            })
            .test_id("b")
    });
    t.run_until_idle();
    let b = t.find(by_id("b")).id();
    t.engine_mut().add_state(b, State::CHECKED);
    t.update();
    // Only the background group animates; the radius (not in `Props::BG`) jumps.
    assert_eq!(t.engine().transition_count(), 1);
    t.run_until_idle();
    assert_eq!(bg(&t, "b"), Color::BLUE);
    // Leaving the state: no transition (it is the checked state's).
    t.engine_mut().clear_state(b, State::CHECKED);
    t.update();
    assert_eq!(t.engine().transition_count(), 0);
    assert_eq!(bg(&t, "b"), Color::RED);
}

#[test]
fn animation_of_colors_and_points() {
    let out: Slot<(ReadSignal<Color>, ReadSignal<Point>)> = Rc::default();
    let o2 = out.clone();
    let mut t = TestUi::new(120, 80).no_theme().mount(move |cx| {
        let spec = AnimSpec::new(Duration::ms(100));
        let (c, _) = cx.animation(Color::BLACK, Color::WHITE, spec);
        let (p, _) = cx.animation(Point::new(0, 0), Point::new(40, -20), spec);
        o2.set(Some((c, p)));
        container(()).size(10, 10).bg(c).offset(p).test_id("dot")
    });
    let (c, p) = out.get().unwrap();
    t.update();
    t.advance(Duration::ms(48));
    let (cm, pm) = (c.get_untracked(), p.get_untracked());
    assert!(cm != Color::BLACK && cm != Color::WHITE, "{cm:?}");
    assert!(pm.x > 0 && pm.x < 40 && pm.y < 0 && pm.y > -20, "{pm:?}");
    assert_eq!(bg(&t, "dot"), cm, "bound to the style");
    t.run_until_idle();
    assert_eq!(
        (c.get_untracked(), p.get_untracked()),
        (Color::WHITE, Point::new(40, -20))
    );
}

#[test]
fn tween_of_a_color_with_a_spec() {
    let mut t = TestUi::new(120, 80).no_theme().mount(|cx| {
        let on = cx.signal(false);
        cx.provide(on);
        let c = cx.tween(
            move || if on.get() { Color::BLUE } else { Color::RED },
            AnimSpec::new(Duration::ms(100)).ease_in_out(),
        );
        container(()).size(10, 10).bg(c).test_id("c")
    });
    t.run_until_idle();
    let on = t.root_scope().use_context::<Signal<bool>>().unwrap();
    on.set(true);
    t.update();
    t.advance(Duration::ms(50));
    assert!(bg(&t, "c") != Color::RED && bg(&t, "c") != Color::BLUE);
    t.run_until_idle();
    assert_eq!(bg(&t, "c"), Color::BLUE);
}

#[test]
fn no_motion_makes_tweens_and_animations_jump_unless_essential() {
    let sigs: Slot<(ReadSignal<i32>, ReadSignal<i32>, ReadSignal<i32>)> = Rc::default();
    let s2 = sigs.clone();
    let mut t = TestUi::new(120, 80)
        .no_theme()
        .mount_engine(|e| e.set_motion(Motion::None))
        .mount(move |cx| {
            let on = cx.signal(false);
            cx.provide(on);
            let tw = cx.tween(move || if on.get() { 100 } else { 0 }, Duration::secs(1));
            let (deco, _) = cx.animation(0, 50, AnimSpec::new(Duration::secs(1)).forever());
            let (busy, _) = cx.animation(0, 360, AnimSpec::new(Duration::secs(1)).forever().essential());
            s2.set(Some((tw, deco, busy)));
            container(()).width(tw)
        });
    let (tw, deco, busy) = sigs.get().unwrap();
    t.update();
    t.advance(Duration::ms(16));
    assert_eq!(deco.get_untracked(), 50, "a decorative loop jumps to its end");
    let on = t.root_scope().use_context::<Signal<bool>>().unwrap();
    on.set(true);
    t.update();
    t.advance(Duration::ms(16));
    assert_eq!(tw.get_untracked(), 100, "the tween jumps");
    t.advance(Duration::ms(200));
    let b = busy.get_untracked();
    assert!(b > 0 && b < 360, "the essential animation keeps moving: {b}");
    assert_eq!(t.engine().anim_count(), 1, "only the essential one runs");
}

#[test]
fn reduced_motion_caps_tweens() {
    let mut t = TestUi::new(120, 80)
        .no_theme()
        .mount_engine(|e| e.set_motion(Motion::Reduced))
        .mount(|cx| {
            let on = cx.signal(false);
            cx.provide(on);
            let w = cx.tween(move || if on.get() { 100 } else { 0 }, Duration::secs(1));
            cx.provide(w);
            container(())
        });
    t.run_until_idle();
    let on = t.root_scope().use_context::<Signal<bool>>().unwrap();
    let w = t.root_scope().use_context::<ReadSignal<i32>>().unwrap();
    on.set(true);
    t.update();
    t.advance(Duration::ms(50));
    assert!(w.get_untracked() > 0 && w.get_untracked() < 100);
    t.advance(Duration::ms(80)); // the tween started one frame after the change
    assert_eq!(w.get_untracked(), 100, "capped at 100 ms instead of 1 s");
}

#[test]
fn use_motion_is_tracked_and_settable() {
    let reads = Rc::new(Cell::new(0));
    let r2 = reads.clone();
    let mut t = TestUi::new(120, 80).no_theme().mount(move |cx| {
        let motion = use_motion(cx);
        cx.provide(motion);
        cx.effect(move || {
            let _ = motion.get();
            r2.set(r2.get() + 1);
        });
        label(move || format!("{:?}", motion.get())).test_id("m")
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("m")).text(), "Full");
    let motion = t.root_scope().use_context::<MotionHandle>().unwrap();
    let before = reads.get();
    // Between updates: queued for the engine, visible to the reactive layer at once.
    motion.set(Motion::Reduced);
    assert_eq!(t.engine().motion(), Motion::Full, "applied at the next update");
    t.run_until_idle();
    assert_eq!(t.engine().motion(), Motion::Reduced);
    assert_eq!(t.find(by_id("m")).text(), "Reduced");
    assert_eq!(reads.get(), before + 1, "the effect re-ran once");
    // Set on the engine directly: seen at the next update.
    t.engine_mut().set_motion(Motion::None);
    t.run_until_idle();
    assert_eq!(t.find(by_id("m")).text(), "None");
    // Idempotent: an unchanged preference re-runs nothing.
    let before = reads.get();
    motion.set(Motion::None);
    t.run_until_idle();
    assert_eq!(reads.get(), before);
}

#[test]
fn a_restart_after_the_preference_changed_plays_the_full_spec() {
    let ctl: Slot<(ReadSignal<i32>, AnimController)> = Rc::default();
    let c2 = ctl.clone();
    let mut t = TestUi::new(120, 80)
        .no_theme()
        .mount_engine(|e| e.set_motion(Motion::None))
        .mount(move |cx| {
            c2.set(Some(cx.animation(0, 100, Duration::secs(1))));
            container(())
        });
    let (v, ctl) = ctl.get().unwrap();
    t.run_until_idle();
    assert_eq!(v.get_untracked(), 100);
    t.engine_mut().set_motion(Motion::Full);
    t.update();
    ctl.restart();
    t.update();
    t.advance(Duration::ms(500));
    let mid = v.get_untracked();
    assert!(mid > 20 && mid < 80, "plays its full second again: {mid}");
}
