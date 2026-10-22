//! Value widgets with ranges at the ends of `i32` (R5.S14): the value ↔ position and
//! value ↔ angle mappings of `Bar`, `Slider` and `Arc` compute range differences
//! (`max − min`, `v − min`, `(min + max) / 2`, `max − v + min`) in `i64`, exactly. Before,
//! they overflowed `i32` (a panic with `overflow-checks`, a wrong position without) for ranges
//! wider than `i32::MAX`. Normal ranges map exactly as before: every extreme range is
//! compared with a small range whose values sit at the same fractions.

mod common;

use common::{Mode, get, harness, with};
use twine_core::math::{cos, sin};
use twine_core::{Angle, Duration, Point, Rect};
use twine_engine::{Key, MeasureCx, NodeId};
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_widgets::arc::{self, Arc, ArcMode};
use twine_widgets::bar::{self, Bar, BarMode};
use twine_widgets::slider::{self, Slider};
use twine_widgets::spinner::Spinner;

const MIN: i32 = i32::MIN;
const MAX: i32 = i32::MAX;

/// A 200 × 20 bar in the middle of a 240 × 60 screen.
fn bar_scene() -> (EngineHarness, NodeId) {
    let mut h = harness(240, 60, Mode::Light);
    let screen = h.screen();
    let e = h.engine_mut();
    let b = bar::create(e, screen).unwrap();
    e.set_size(b, 200, 20);
    e.align(b, Align::Center, 0, 0);
    h.run_until_idle();
    (h, b)
}

/// The indicator area of a bar with `range` (as set), `mode` and `value`.
fn indicator(h: &mut EngineHarness, b: NodeId, range: (i32, i32), mode: BarMode, value: i32) -> Rect {
    with(h, b, |w: &mut Bar, cx| {
        w.set_mode(cx, mode);
        w.set_range(cx, range.0, range.1);
        w.set_value(cx, value, false);
    });
    h.run_until_idle();
    get::<Bar>(h, b).indicator_area(&MeasureCx::new(h.engine(), b))
}

#[test]
fn regression_bar_full_i32_range_maps_like_a_small_range() {
    let (mut h, b) = bar_scene();
    // Values at 0 %, 50 % and 100 % of the range: `0` is 2^31 / (2^32 − 1) of `MIN ..= MAX`,
    // which rounds down to the same pixel as 50 of `0 ..= 100` on a 200 px bar.
    for (wide, small) in [(MIN, 0), (0, 50), (MAX, 100)] {
        let expected = indicator(&mut h, b, (0, 100), BarMode::Normal, small);
        let got = indicator(&mut h, b, (MIN, MAX), BarMode::Normal, wide);
        assert_eq!(got, expected, "value {wide}");
    }
    // A reversed range draws from the other end.
    for (wide, small) in [(MIN, 0), (0, 50), (MAX, 100)] {
        let expected = indicator(&mut h, b, (100, 0), BarMode::Normal, small);
        let got = indicator(&mut h, b, (MAX, MIN), BarMode::Normal, wide);
        assert_eq!(got, expected, "reversed, value {wide}");
    }
}

#[test]
fn regression_bar_symmetrical_full_i32_range_draws_from_zero() {
    let (mut h, b) = bar_scene();
    for (wide, small) in [(MIN, -100), (0, 0), (MAX, 100)] {
        let expected = indicator(&mut h, b, (-100, 100), BarMode::Symmetrical, small);
        let got = indicator(&mut h, b, (MIN, MAX), BarMode::Symmetrical, wide);
        assert_eq!(got, expected, "value {wide}");
    }
}

#[test]
fn regression_bar_range_mode_with_extreme_values() {
    let (mut h, b) = bar_scene();
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_mode(cx, BarMode::Range);
        w.set_range(cx, MIN, MAX);
        w.set_value(cx, MAX, false);
        w.set_start_value(cx, MIN, false);
    });
    h.run_until_idle();
    let wide = get::<Bar>(&h, b).indicator_area(&MeasureCx::new(h.engine(), b));
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_range(cx, 0, 100);
        w.set_value(cx, 100, false);
        w.set_start_value(cx, 0, false);
    });
    h.run_until_idle();
    let small = get::<Bar>(&h, b).indicator_area(&MeasureCx::new(h.engine(), b));
    assert_eq!(wide, small);
}

#[test]
fn regression_bar_animation_across_the_full_range() {
    let (mut h, b) = bar_scene();
    h.engine_mut().set_local_prop(
        b,
        twine_style::Selector::MAIN,
        twine_style::StyleProp::AnimDuration(Duration::ms(200).into()),
    );
    with(&mut h, b, |w: &mut Bar, cx| {
        w.set_range(cx, MIN, MAX);
        w.set_value(cx, MIN, false);
        w.set_value(cx, MAX, true);
    });
    // Every animation step maps between the ends without overflow.
    for _ in 0..30 {
        h.advance(Duration::ms(10));
        let a = get::<Bar>(&h, b).indicator_area(&MeasureCx::new(h.engine(), b));
        assert!(a.width() <= 200, "{a}");
    }
    h.run_until_idle();
    assert!(!get::<Bar>(&h, b).is_animating());
}

/// A 200 × 10 slider in the middle of a 240 × 60 screen, with `range`.
fn slider_scene(range: (i32, i32)) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 60, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let s = slider::create(e, screen).unwrap();
    e.set_size(s, 200, 10);
    e.align(s, Align::Center, 0, 0);
    with(&mut h, s, |w: &mut Slider, cx| w.set_range(cx, range.0, range.1));
    h.run_until_idle();
    (h, s)
}

/// Drags the slider's knob from its left end to `x` (absolute) and returns the value.
fn drag_to(h: &mut EngineHarness, s: NodeId, x: i32) -> i32 {
    let c = h.engine().coords(s);
    let y = (c.y0 + c.y1) / 2;
    h.press(Point::new(c.x0 + 2, y));
    for step in [c.x0 + 20, x] {
        h.clock().advance(Duration::ms(10));
        h.move_to(Point::new(step, y));
    }
    let v = get::<Slider>(h, s).value();
    h.release();
    h.run_until_idle();
    v
}

/// req: SAF-020
#[test]
fn regression_slider_drag_over_full_i32_range() {
    let (mut h, s) = slider_scene((MIN, MAX));
    let c = h.engine().coords(s);
    // Past both ends: exactly the ends.
    assert_eq!(drag_to(&mut h, s, c.x1 + 40), MAX);
    assert_eq!(drag_to(&mut h, s, c.x0 - 40), MIN);
    // In between: the same fraction of the range as on a small range (within one pixel's
    // share, 2^32 / 200).
    let (mut hs, ss) = slider_scene((-100, 100));
    for x in [c.x0 + 50, (c.x0 + c.x1) / 2, c.x1 - 30] {
        let wide = drag_to(&mut h, s, x);
        let small = drag_to(&mut hs, ss, x);
        let expected = i64::from(small) * (i64::from(MAX) - i64::from(MIN)) / 200;
        let pixel = (i64::from(MAX) - i64::from(MIN)) / 200 + 1;
        assert!(
            (i64::from(wide) - expected).abs() <= pixel,
            "x {x}: {wide} vs {expected} (small {small})"
        );
    }
}

/// req: SAF-020
#[test]
fn regression_slider_keys_at_the_ends_of_i32() {
    let (mut h, s) = slider_scene((MIN, MAX));
    let _ = h.keypad_input();
    h.engine_mut().focus(s);
    with(&mut h, s, |w: &mut Slider, cx| w.set_value(cx, MAX, false));
    h.run_until_idle();
    h.key(Key::Right); // `MAX + 1` overflowed
    h.run_until_idle();
    assert_eq!(get::<Slider>(&h, s).value(), MAX);
    with(&mut h, s, |w: &mut Slider, cx| w.set_value(cx, MIN, false));
    h.run_until_idle();
    h.key(Key::Left); // `MIN − 1` overflowed
    h.run_until_idle();
    assert_eq!(get::<Slider>(&h, s).value(), MIN);
}

/// A default arc (130 × 130) in the middle of a 160 × 160 screen.
fn arc_scene() -> (EngineHarness, NodeId) {
    let mut h = harness(160, 160, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let a = arc::create(e, screen).unwrap();
    e.align(a, Align::Center, 0, 0);
    h.run_until_idle();
    (h, a)
}

/// The indicator angles of the arc after setting `mode`, `range` and `value`.
fn arc_angles(
    h: &mut EngineHarness,
    a: NodeId,
    mode: ArcMode,
    range: (i32, i32),
    value: i32,
) -> (Angle, Angle) {
    with(h, a, |w: &mut Arc, cx| {
        w.set_mode(cx, mode);
        w.set_range(cx, range.0, range.1);
        w.set_value(cx, value);
    });
    let w = get::<Arc>(h, a);
    (w.angle_start(), w.angle_end())
}

#[test]
fn regression_arc_full_i32_range_maps_like_a_small_range() {
    let (mut h, a) = arc_scene();
    for mode in [ArcMode::Normal, ArcMode::Reverse, ArcMode::Symmetrical] {
        for (wide, small) in [(MIN, -100), (0, 0), (MAX, 100)] {
            let expected = arc_angles(&mut h, a, mode, (-100, 100), small);
            let got = arc_angles(&mut h, a, mode, (MIN, MAX), wide);
            assert_eq!(got, expected, "{mode:?}, value {wide}");
        }
    }
    // `(min + max) / 2` overflowed for two large bounds.
    let (s, e) = arc_angles(&mut h, a, ArcMode::Symmetrical, (MAX - 100, MAX), MAX);
    assert_eq!((s, e), arc_angles(&mut h, a, ArcMode::Symmetrical, (0, 100), 100));
}

/// The point on the middle of the arc's ring at `deg` degrees.
fn on_ring(h: &EngineHarness, a: NodeId, deg: i32) -> Point {
    let (c, r) = get::<Arc>(h, a).center(&MeasureCx::new(h.engine(), a));
    let rr = i64::from(r - 6);
    Point::new(
        c.x + ((rr * i64::from(cos(Angle::deg(deg)))) >> 15) as i32,
        c.y + ((rr * i64::from(sin(Angle::deg(deg)))) >> 15) as i32,
    )
}

/// req: SAF-020
#[test]
fn regression_arc_drag_over_full_i32_range() {
    for mode in [ArcMode::Normal, ArcMode::Reverse, ArcMode::Symmetrical] {
        let (mut h, a) = arc_scene();
        with(&mut h, a, |w: &mut Arc, cx| {
            w.set_mode(cx, mode);
            w.set_range(cx, MIN, MAX);
            w.set_value(cx, 0);
        });
        h.run_until_idle();
        // `max − min` (the step count) and the reverse mode's `max − v + min` overflowed.
        h.press(on_ring(&h, a, 270));
        for deg in (270..=400).step_by(10).chain((140..=270).rev().step_by(10)) {
            h.clock().advance(Duration::ms(50));
            h.move_to(on_ring(&h, a, deg));
        }
        h.release();
        h.run_until_idle();
        let v = get::<Arc>(&h, a).value();
        assert!((MIN..=MAX).contains(&v), "{mode:?}: {v}");
    }
}

/// req: SAF-020
#[test]
fn regression_arc_keys_at_the_ends_of_i32() {
    let (mut h, a) = arc_scene();
    let _ = h.keypad_input();
    h.engine_mut().focus(a);
    with(&mut h, a, |w: &mut Arc, cx| {
        w.set_range(cx, MIN, MAX);
        w.set_value(cx, MAX);
    });
    h.run_until_idle();
    h.key(Key::Right); // `MAX + 1` overflowed
    assert_eq!(get::<Arc>(&h, a).value(), MAX);
    with(&mut h, a, |w: &mut Arc, cx| w.set_value(cx, MIN));
    h.key(Key::Left);
    assert_eq!(get::<Arc>(&h, a).value(), MIN);
}

#[test]
fn regression_arc_extreme_knob_offset_and_angles() {
    let (mut h, a) = arc_scene();
    // The knob offset was added to the knob angle unreduced (overflow); it is taken modulo a
    // turn, which is where the knob was drawn for every offset that did not overflow.
    with(&mut h, a, |w: &mut Arc, cx| {
        w.set_knob_offset(cx, Angle::deci_deg(3600 * 3 + 100));
    });
    let small = get::<Arc>(&h, a).knob_area(&MeasureCx::new(h.engine(), a));
    with(&mut h, a, |w: &mut Arc, cx| {
        w.set_knob_offset(cx, Angle::deci_deg(100));
    });
    assert_eq!(get::<Arc>(&h, a).knob_area(&MeasureCx::new(h.engine(), a)), small);
    with(&mut h, a, |w: &mut Arc, cx| {
        w.set_knob_offset(cx, Angle::deci_deg(MAX));
    });
    let _ = get::<Arc>(&h, a).knob_area(&MeasureCx::new(h.engine(), a));
    // Angles far outside a turn are normalized in constant time (the old loops stepped by
    // 360° up to 600 000 times) to the same angle as their small equivalent.
    with(&mut h, a, |w: &mut Arc, cx| {
        w.set_bg_angles(cx, Angle::deci_deg(MIN), Angle::deci_deg(MAX));
        w.set_angles(cx, Angle::deci_deg(MAX), Angle::deci_deg(MIN));
    });
    let w = get::<Arc>(&h, a);
    assert_eq!(w.bg_angle_start(), Angle::deci_deg(MIN.rem_euclid(3600)));
    assert_eq!(w.bg_angle_end(), Angle::deci_deg((MAX - 1) % 3600 + 1));
    h.run_until_idle();
}

#[test]
fn regression_spinner_angles_at_any_value() {
    let s = Spinner::default();
    // `v · 3600` overflowed for values beyond the documented `0 ..= 1024`.
    for v in [MIN, -1, 0, 512, 1024, MAX] {
        let _ = s.angles_at(v);
    }
    assert_eq!(s.angles_at(1024), Spinner::default().angles_at(1024));
}
