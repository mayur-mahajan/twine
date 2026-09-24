//! `Bar`: LVGL defaults, value clamping, modes, reversed ranges, orientation, animated values
//! that redraw only the moving end, and the default theme's look (light and dark).

mod common;

use common::{Mode, get, harness, with};
use twine_core::{Duration, Rect};
use twine_engine::{EventCode, Key, MeasureCx, NodeId, ObjFlags};
use twine_style::{Align, Part, Selector, State, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::Orientation;
use twine_widgets::bar::{self, BAR_CLASS, Bar, BarMode};

/// A 200 × 20 bar in the middle of a 240 × 60 screen.
fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 60, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let b = bar::create(e, screen).unwrap();
    e.set_size(b, 200, 20);
    e.align(b, Align::Center, 0, 0);
    h.run_until_idle();
    (h, b)
}

fn indic(h: &EngineHarness, b: NodeId) -> Rect {
    get::<Bar>(h, b).indicator_area(&MeasureCx::new(h.engine(), b))
}

#[test]
fn bar_defaults() {
    let mut h = harness(300, 60, Mode::Light);
    let screen = h.screen();
    let b = bar::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let n = h.engine().tree().node(b).unwrap();
    assert_eq!(n.class().name, "bar");
    assert_eq!(BAR_CLASS.parts, &[Part::Main, Part::Indicator]);
    assert!(n.flags().contains(ObjFlags::CLICKABLE));
    assert!(
        !n.flags().contains(ObjFlags::SCROLLABLE),
        "LVGL: bars do not scroll"
    );
    assert!(!n.flags().contains(ObjFlags::CHECKABLE));
    // LVGL `width_def = LV_DPI_DEF * 2`, `height_def = LV_DPI_DEF / 10`.
    let c = h.engine().coords(b);
    assert_eq!((c.width(), c.height()), (260, 13));
    let w = get::<Bar>(&h, b);
    assert_eq!((w.value(), w.min(), w.max(), w.start_value()), (0, 0, 100, 0));
    assert_eq!((w.mode(), w.orientation()), (BarMode::Normal, Orientation::Auto));
    assert!(indic(&h, b).is_empty(), "value 0 draws no indicator");
    h.assert_idle();
}

#[test]
fn bar_setters_same_value_no_invalidate() {
    let (mut h, b) = scene(Mode::Light);
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_value(cx, 0, false);
        w.set_value(cx, -5, false); // clamps to the current 0
        w.set_range(cx, 0, 100);
        w.set_mode(cx, BarMode::Normal);
        w.set_orientation(cx, Orientation::Auto);
        w.set_start_value(cx, 10, false); // ignored outside range mode
    });
    assert!(h.engine().invalidation_log().is_empty());
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 40, false));
    h.run_until_idle();
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 40, true));
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn bar_value_clamped() {
    let (mut h, b) = scene(Mode::Light);
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 150, false));
    assert_eq!(get::<Bar>(&h, b).value(), 100);
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, -3, false));
    assert_eq!(get::<Bar>(&h, b).value(), 0);
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_value(cx, 90, false);
        w.set_range(cx, 0, 50);
    });
    assert_eq!(get::<Bar>(&h, b).value(), 50, "a smaller range clamps the value");
    // Range mode: the value cannot go below the start value and vice versa.
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_mode(cx, BarMode::Range);
        w.set_start_value(cx, 20, false);
        w.set_value(cx, 10, false);
    });
    assert_eq!(get::<Bar>(&h, b).value(), 20);
    with(&mut h, b, |w: &mut Bar, cx| w.set_start_value(cx, 40, false));
    assert_eq!(get::<Bar>(&h, b).start_value(), 20);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn bar_reversed_range_draws_from_the_end() {
    let (mut h, b) = scene(Mode::Light);
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_range(cx, 100, 0);
        w.set_value(cx, 25, false);
    });
    h.run_until_idle();
    let w = get::<Bar>(&h, b);
    assert_eq!((w.min(), w.max(), w.value()), (100, 0, 25));
    let c = h.engine().coords(b);
    let i = indic(&h, b);
    // LVGL: the indicator starts at the right end (the "minimum" 100 is on the left).
    assert_eq!(i.x1, c.x1);
    assert_eq!(i.width(), 50);
}

#[test]
fn bar_normal_indicator_geometry() {
    let (mut h, b) = scene(Mode::Light);
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 50, false));
    h.run_until_idle();
    let c = h.engine().coords(b);
    let i = indic(&h, b);
    // No padding in the default theme: 200 px * 50 % from the left edge.
    assert_eq!(i, Rect::new(c.x0, c.y0, c.x0 + 100, c.y1));
}

#[test]
fn bar_symmetrical_from_zero() {
    let (mut h, b) = scene(Mode::Light);
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_mode(cx, BarMode::Symmetrical);
        w.set_range(cx, -50, 50);
        w.set_value(cx, -25, false);
    });
    h.run_until_idle();
    assert!(get::<Bar>(&h, b).is_symmetrical());
    let c = h.engine().coords(b);
    let zero = c.x0 + 100;
    let i = indic(&h, b);
    // LVGL's integer math makes a negative symmetrical indicator one pixel longer on each
    // side (`draw_indic`: `zero = x1 + shift`, `x2 = x1 + cur - 1`).
    assert_eq!((i.x0, i.x1), (zero - 51, zero + 1), "from -25 to zero");
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 25, false));
    h.run_until_idle();
    let i = indic(&h, b);
    assert_eq!((i.x0, i.x1), (zero, zero + 50), "from zero to 25");
    // Without a range spanning zero the mode behaves like Normal.
    with(&mut h, b, |w: &mut Bar, cx| w.set_range(cx, 0, 100));
    assert!(!get::<Bar>(&h, b).is_symmetrical());
}

#[test]
fn bar_vertical_auto_orientation() {
    let mut h = harness(100, 240, Mode::Light);
    let screen = h.screen();
    let e = h.engine_mut();
    let b = bar::create(e, screen).unwrap();
    e.set_size(b, 20, 200);
    e.align(b, Align::Center, 0, 0);
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 30, false));
    h.run_until_idle();
    assert!(!get::<Bar>(&h, b).is_horizontal(&MeasureCx::new(h.engine(), b)));
    let c = h.engine().coords(b);
    let i = indic(&h, b);
    assert_eq!(i, Rect::new(c.x0, c.y1 - 60, c.x1, c.y1), "grows from the bottom");
    // A forced horizontal orientation on the same tall bar.
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_orientation(cx, Orientation::Horizontal);
    });
    assert!(get::<Bar>(&h, b).is_horizontal(&MeasureCx::new(h.engine(), b)));
}

#[test]
fn bar_anim_reaches_value_in_duration() {
    let (mut h, b) = scene(Mode::Light);
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::AnimDuration(200));
    h.run_until_idle();
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 80, true));
    assert_eq!(
        get::<Bar>(&h, b).value(),
        80,
        "the target is reported at once (LVGL)"
    );
    assert!(get::<Bar>(&h, b).is_animating());
    h.update();
    let c = h.engine().coords(b);
    let start_w = indic(&h, b).width();
    h.advance(Duration::ms(100));
    let mid = indic(&h, b).width();
    assert!(mid > start_w && mid < 160, "halfway: {mid}");
    h.advance(Duration::ms(110));
    assert!(!get::<Bar>(&h, b).is_animating());
    assert_eq!(indic(&h, b), Rect::new(c.x0, c.y0, c.x0 + 160, c.y1));
    // Retargeting continues from the shown value.
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 20, true));
    h.update();
    h.advance(Duration::ms(50));
    let shown = indic(&h, b).width();
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 100, true));
    h.update();
    let after = indic(&h, b).width();
    assert!(
        (after - shown).abs() <= 2,
        "no jump on retarget: {shown} -> {after}"
    );
    h.run_until_idle();
    assert_eq!(indic(&h, b).width(), 200);
    h.assert_idle();
}

#[test]
fn bar_anim_invalidates_indicator_only() {
    let (mut h, b) = scene(Mode::Light);
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::AnimDuration(400));
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 10, false));
    h.run_until_idle();
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 90, true));
    h.update();
    let c = h.engine().coords(b);
    let radius = c.height() / 2;
    let mut prev = indic(&h, b);
    let mut frames = 0;
    for _ in 0..30 {
        h.advance(Duration::ms(16));
        let now = indic(&h, b);
        if now == prev {
            continue;
        }
        frames += 1;
        let delta = (now.x1 - prev.x1).abs();
        // Done-when: the redrawn columns are at most the indicator delta plus 2 × radius
        // (+ one pixel of rounding on each side).
        let dirty_px = h.last_frame().dirty_px;
        let limit = u32::try_from((delta + 2 * radius + 2) * c.height()).unwrap();
        assert!(
            dirty_px <= limit,
            "frame redrew {dirty_px} px > {limit} (delta {delta})"
        );
        for f in h.flushes() {
            assert!(
                f.area.x0 >= prev.x1.min(now.x1) - radius - 1,
                "{} left of the moving end",
                f.area
            );
        }
        prev = now;
    }
    assert!(frames >= 10, "{frames} animated frames");
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn bar_keypad_focus_shows_outline() {
    let mut h = harness(240, 60, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let first = bar::create(h.engine_mut(), screen).unwrap();
    let b = bar::create(h.engine_mut(), screen).unwrap();
    assert_eq!(
        h.engine().group_of(b),
        None,
        "LVGL: bars are not added to the default group"
    );
    h.engine_mut().group_add(g, first);
    h.engine_mut().group_add(g, b);
    let _ = h.keypad_input();
    h.run_until_idle();
    h.key(Key::Next);
    let st = h.engine().tree().node(b).unwrap().state();
    assert!(st.contains(State::FOCUSED | State::FOCUS_KEY));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn bar_idle_after_interaction() {
    let (mut h, b) = scene(Mode::Light);
    let c = h.engine().coords(b);
    h.tap(c.center());
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::AnimDuration(200));
    with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 60, true));
    h.run_until_idle();
    h.assert_idle();
    let _ = EventCode::ValueChanged;
}

fn snap(name: &str, f: impl Fn(&mut EngineHarness, NodeId)) {
    for m in Mode::ALL {
        let (mut h, b) = scene(m);
        with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 60, false));
        f(&mut h, b);
        h.run_until_idle();
        h.assert_snapshot(&format!("bar_{name}_{}", m.suffix()));
    }
}

#[test]
fn snapshot_bar_states() {
    let states: [(&str, State); 4] = [
        ("default", State::DEFAULT),
        ("pressed", State::PRESSED),
        ("disabled", State::DISABLED),
        ("focused", State::FOCUSED.union(State::FOCUS_KEY)),
    ];
    for (name, st) in states {
        snap(name, |h, b| h.engine_mut().add_state(b, st));
    }
}

#[test]
fn snapshot_bar_modes() {
    snap("symmetrical", |h, b| {
        with(h, b, |w: &mut Bar, cx| {
            w.set_mode(cx, BarMode::Symmetrical);
            w.set_range(cx, -100, 100);
            w.set_value(cx, -40, false);
        });
    });
    snap("range", |h, b| {
        with(h, b, |w: &mut Bar, cx| {
            w.set_mode(cx, BarMode::Range);
            w.set_value(cx, 80, false);
            w.set_start_value(cx, 30, false);
        });
    });
    // A small value: the indicator is narrower than its radius and is clipped by the
    // rounded background (LVGL's mask branch).
    snap("small", |h, b| {
        with(h, b, |w: &mut Bar, cx| w.set_value(cx, 3, false));
    });
    for m in Mode::ALL {
        let mut h = harness(60, 240, m);
        let screen = h.screen();
        let e = h.engine_mut();
        let b = bar::create(e, screen).unwrap();
        e.set_size(b, 20, 200);
        e.align(b, Align::Center, 0, 0);
        with(&mut h, b, |w: &mut Bar, cx| w.set_value(cx, 70, false));
        h.run_until_idle();
        h.assert_snapshot(&format!("bar_vertical_{}", m.suffix()));
    }
}
