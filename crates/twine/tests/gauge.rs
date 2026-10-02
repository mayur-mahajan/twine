//! The gauge of the custom widget guide (`twine::guide::custom_widgets`, the `gauge`
//! example): defaults, idempotent setters, part styling through `.part`, keypad and encoder
//! input, the needle animation and the motion preference, design elements in every theme
//! mode, and snapshots per state and theme mode.

// The example's widget (shared by the example, these tests and the guide).
#[path = "../examples/gauge/gauge.rs"]
#[allow(dead_code)]
mod gauge;

use std::cell::Cell;
use std::rc::Rc;

use gauge::{GAUGE_CLASS, GAUGE_DEFAULT_SIZE, Gauge, INDICATOR, LABEL, NEEDLE, TRACK, gauge, style_gauge};
use twine::prelude::*;
use twine_testing::{TestUi, by_id};

/// The default theme with the gauge registered.
fn theme() -> Rc<DefaultTheme> {
    Rc::new(DefaultTheme::builder().class(&GAUGE_CLASS, style_gauge).build())
}

/// A test UI showing `app` with the gauge's theme.
fn ui<V: View>(app: impl FnOnce(Scope) -> V) -> TestUi {
    TestUi::new(200, 160).theme(theme()).mount(app)
}

/// Where the app stores a signal, for the test to drive it after mounting.
type Slot<T> = Rc<Cell<Option<Signal<T>>>>;

/// A slot (twice: one for the test, one moved into the app).
fn slot<T: 'static>() -> (Slot<T>, Slot<T>) {
    let s = Rc::new(Cell::new(None));
    (s.clone(), s)
}

/// The signal stored in `s`.
fn take<T: 'static>(s: &Slot<T>) -> Signal<T> {
    s.get().expect("the app stored its signal")
}

/// The gauge node with test id `id`.
fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

/// The gauge widget with test id `id`, read with `f`.
fn read<R>(t: &TestUi, id: &'static str, f: impl FnOnce(&Gauge) -> R) -> R {
    let n = node(t, id);
    f(t.engine().widget::<Gauge>(n).expect("a gauge"))
}

#[test]
fn defaults() {
    let g = Gauge::new();
    assert_eq!((g.value(), g.shown(), g.range()), (0, 0, 0..=100));
    let mut t = ui(|_| gauge(0).test_id("g"));
    t.run_until_idle();
    let n = node(&t, "g");
    let e = t.engine();
    let w = e.widget::<Gauge>(n).unwrap();
    assert!(w.class().is(&GAUGE_CLASS));
    assert!(GAUGE_CLASS.is_a(&twine::widgets::container::CARD_CLASS));
    assert_eq!(GAUGE_CLASS.parts, &[Part::Main, TRACK, INDICATOR, NEEDLE, LABEL]);
    assert_eq!(
        (TRACK, NEEDLE, LABEL),
        (Part::custom::<0>(), Part::custom::<1>(), Part::custom::<2>())
    );
    let c = e.coords(n);
    assert_eq!((c.width(), c.height()), (GAUGE_DEFAULT_SIZE, GAUGE_DEFAULT_SIZE));
    assert_eq!(w.text(), Some("0"));
    // `Main` is themed like the base class (a card) ...
    assert_eq!(
        e.style_color(n, Part::Main, PropId::BgColor),
        e.design_value(e.default_display().unwrap(), design::SURFACE)
            .unwrap()
    );
    // ... and the parts by the registration.
    assert_eq!(e.style_i32(n, TRACK, PropId::ArcWidth), 10);
    assert_eq!(e.style_i32(n, NEEDLE, PropId::LineWidth), 4);
    assert_eq!(e.style_i32(n, Part::Main, PropId::AnimDuration), 300);
}

#[test]
#[expect(
    clippy::reversed_empty_ranges,
    reason = "a reversed range is part of the test"
)]
fn value_is_applied_after_the_range_and_clamped() {
    let mut t = ui(|_| {
        column((
            gauge(150).range(0..=200).test_id("a"), // value written before the range
            gauge(-5).test_id("b"),                 // below the default range
            gauge(50).range(100..=0).test_id("c"),  // reversed range
        ))
    });
    t.run_until_idle();
    assert_eq!(read(&t, "a", Gauge::value), 150);
    assert_eq!(read(&t, "b", Gauge::value), 0);
    assert_eq!(read(&t, "c", |g| (g.value(), g.range())), (50, 0..=100));
    assert_eq!(read(&t, "a", |g| g.text().map(str::to_owned)), Some("150".into()));
}

#[test]
fn setters_are_idempotent() {
    let (vs, vs_app) = slot::<i32>();
    let (rs, rs_app) = slot::<core::ops::RangeInclusive<i32>>();
    let mut t = ui(move |cx| {
        let v = cx.signal(40);
        let r = cx.signal(0..=100);
        vs_app.set(Some(v));
        rs_app.set(Some(r));
        gauge(v).range(r).test_id("g")
    });
    t.run_until_idle();
    let (v, r) = (take(&vs), take(&rs));
    // Unchanged values: the bindings re-run, the setters do nothing.
    v.set(40);
    r.set(0..=100);
    t.assert_idle();
    // A change redraws; the needle moves to it.
    v.set(60);
    t.run_until_idle();
    assert_eq!(read(&t, "g", |g| (g.value(), g.shown())), (60, 60));
    t.assert_idle();
    // A value outside the range is kept: widening the range shows it.
    v.set(150);
    t.run_until_idle();
    assert_eq!(read(&t, "g", Gauge::value), 100);
    r.set(0..=200);
    t.run_until_idle();
    assert_eq!(read(&t, "g", Gauge::value), 150);
}

#[test]
fn parts_are_styled_with_part_scopes() {
    let (alarm_slot, app_slot) = slot::<bool>();
    let mut t = ui(move |cx| {
        let alarm = cx.signal(false);
        app_slot.set(Some(alarm));
        gauge(30)
            .part(NEEDLE, |s| s.line_color(Color::RED).line_width(2))
            .part(INDICATOR, |s| {
                s.arc_color(move || {
                    if alarm.get() {
                        design::DANGER.into()
                    } else {
                        design::ColorValue::from(Color::BLUE)
                    }
                })
            })
            .part(LABEL, |s| s.text_color(design::PRIMARY))
            .test_id("g")
    });
    t.run_until_idle();
    let n = node(&t, "g");
    let display = t.engine().default_display().unwrap();
    {
        let e = t.engine();
        assert_eq!(e.style_color(n, NEEDLE, PropId::LineColor), Color::RED);
        assert_eq!(e.style_i32(n, NEEDLE, PropId::LineWidth), 2);
        assert_eq!(e.style_color(n, INDICATOR, PropId::ArcColor), Color::BLUE);
        assert_eq!(
            e.style_color(n, LABEL, PropId::TextColor),
            e.design_value(display, design::PRIMARY).unwrap()
        );
        // Untouched parts keep the theme's look.
        assert_eq!(e.style_i32(n, TRACK, PropId::ArcWidth), 10);
    }
    take(&alarm_slot).set(true);
    t.run_until_idle();
    let e = t.engine();
    assert_eq!(
        e.style_color(n, INDICATOR, PropId::ArcColor),
        e.design_value(display, design::DANGER).unwrap()
    );
}

#[test]
fn keypad_and_encoder_change_the_value() {
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let (vs, app_slot) = slot::<i32>();
    let mut t = ui(move |cx| {
        let v = cx.signal(50);
        app_slot.set(Some(v));
        gauge(v)
            .on_change(move |x| {
                v.set(x);
                seen.set(seen.get() + 1);
            })
            .test_id("g")
    });
    t.run_until_idle();
    let v = take(&vs);
    // The gauge is in the default group: the keypad steps it.
    t.key(Key::Right);
    t.key(Key::Up);
    t.key(Key::Left);
    t.run_until_idle();
    assert_eq!(read(&t, "g", Gauge::value), 51);
    assert_eq!((v.get(), changes.get()), (51, 3));
    assert!(t.find(by_id("g")).state().contains(State::FOCUSED));
    // At the end of the range a key changes nothing and reports nothing.
    v.set(100);
    t.run_until_idle();
    t.key(Key::Right);
    t.run_until_idle();
    assert_eq!((v.get(), changes.get()), (100, 3));
    // The encoder: a click enters edit mode, turning steps the value.
    t.encoder_click();
    t.encoder(-4);
    t.run_until_idle();
    assert_eq!(read(&t, "g", Gauge::value), 96);
    assert_eq!(v.get(), 96);
}

#[test]
fn the_needle_moves_and_follows_the_motion_preference() {
    let (vs, app_slot) = slot::<i32>();
    let mut t = ui(move |cx| {
        let v = cx.signal(0);
        app_slot.set(Some(v));
        gauge(v).test_id("g")
    });
    t.run_until_idle();
    let v = take(&vs);
    v.set(100);
    t.advance(Duration::ms(100));
    let mid = read(&t, "g", Gauge::shown);
    assert!(0 < mid && mid < 100, "moving: {mid}");
    assert_eq!(read(&t, "g", Gauge::value), 100);
    t.run_until_idle();
    assert_eq!(read(&t, "g", Gauge::shown), 100);
    // No motion: the needle jumps at the next update.
    t.engine_mut().set_motion(Motion::None);
    v.set(20);
    t.update();
    t.update();
    assert_eq!(read(&t, "g", Gauge::shown), 20);
    // A local `anim_duration` of zero jumps too.
    t.engine_mut().set_motion(Motion::Full);
    let n = node(&t, "g");
    t.engine_mut()
        .set_local_prop(n, Selector::MAIN, StyleProp::AnimDuration(Duration::ZERO.into()));
    v.set(70);
    t.update();
    assert_eq!(read(&t, "g", Gauge::shown), 70);
}

#[test]
fn colors_follow_every_theme_mode() {
    let mut t = ui(|_| gauge(40).test_id("g"));
    t.run_until_idle();
    let n = node(&t, "g");
    let display = t.engine().default_display().unwrap();
    for mode in ThemeMode::ALL {
        t.engine_mut().set_theme_mode(display, mode);
        t.run_until_idle();
        let e = t.engine();
        assert_eq!(e.theme_mode(display), mode);
        for (part, prop, element) in [
            (TRACK, PropId::ArcColor, design::SURFACE_VARIANT),
            (INDICATOR, PropId::ArcColor, design::PRIMARY),
            (NEEDLE, PropId::LineColor, design::ON_SURFACE),
            (LABEL, PropId::TextColor, design::ON_SURFACE),
        ] {
            assert_eq!(
                e.style_color(n, part, prop),
                e.design_value(display, element).unwrap(),
                "{mode:?} {part:?}"
            );
        }
    }
}

#[test]
fn snapshots_per_state_and_theme_mode() {
    let (ds, app_slot) = slot::<bool>();
    let mut t = TestUi::new(272, 140).theme(theme()).mount(move |cx| {
        let disabled = cx.signal(false);
        app_slot.set(Some(disabled));
        row((gauge(65).test_id("g"), gauge(30).disabled(disabled).test_id("d")))
            .gap(8)
            .padding(8)
    });
    t.run_until_idle();
    let display = t.engine().default_display().unwrap();
    for (mode, name) in [
        (ThemeMode::Light, "gauge_light"),
        (ThemeMode::Dark, "gauge_dark"),
        (ThemeMode::Night, "gauge_night"),
        (ThemeMode::HighContrast, "gauge_high_contrast"),
    ] {
        t.engine_mut().set_theme_mode(display, mode);
        t.run_until_idle();
        t.assert_snapshot(name);
    }
    t.engine_mut().set_theme_mode(display, ThemeMode::Light);
    take(&ds).set(true);
    // Keypad focus: `Next` moves it to the second gauge and back to the first.
    t.key(Key::Next);
    t.key(Key::Next);
    t.run_until_idle();
    assert!(t.find(by_id("g")).state().contains(State::FOCUS_KEY));
    assert!(t.find(by_id("d")).state().contains(State::DISABLED));
    t.assert_snapshot("gauge_focused_and_disabled");
}

/// The guide's excerpts (code blocks starting with `// examples/…`) are still the example's
/// code: their visible lines appear in that file, in order (lines may be left out).
#[test]
fn guide_excerpts_match_the_example() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let guide = std::fs::read_to_string(dir.join("src/guide/custom_widgets.md")).expect("the guide");
    let mut excerpts = 0;
    let mut lines = guide.lines();
    while let Some(line) = lines.next() {
        if !line.starts_with("```rust") {
            continue;
        }
        // rustdoc hides `# …` and `#` lines; the rest is what the reader sees.
        let visible: Vec<&str> = lines
            .by_ref()
            .take_while(|l| !l.starts_with("```"))
            .filter(|l| *l != "#" && !l.starts_with("# "))
            .collect();
        let Some(file) = visible
            .first()
            .and_then(|l| l.strip_prefix("// "))
            .filter(|f| f.starts_with("examples/"))
        else {
            continue;
        };
        let source = std::fs::read_to_string(dir.join(file)).expect("the excerpted file");
        let mut rest = source.lines();
        for want in visible[1..].iter().filter(|l| !l.trim().is_empty()) {
            assert!(
                rest.any(|have| have.trim_end() == want.trim_end()),
                "the guide's excerpt of {file} has `{want}`, which is not (or not in this order) in the file"
            );
        }
        excerpts += 1;
    }
    assert_eq!(excerpts, 5, "every step of the guide shows the example's code");
}

/// A view written outside twine-view applies its value after its range with
/// `bind_after_children`, like the built-in `bar`; with a plain `bind` in chain order the
/// value meets the default range first and is clamped (the bar does not remember it).
#[test]
fn external_view_binds_its_value_after_the_range() {
    use twine::widgets::bar::Bar;
    fn bar_view(v: i32, late: bool) -> WidgetView<Bar> {
        let set = |b: &mut Bar, cx: &mut WidgetCx<'_>, v: i32| b.set_value(cx, v, false);
        let w = widget_view(Bar::new);
        let w = if late {
            w.bind_after_children(v, set)
        } else {
            w.bind(v, set)
        };
        w.bind(0..=200, |b: &mut Bar, cx, r: core::ops::RangeInclusive<i32>| {
            b.set_range(cx, *r.start(), *r.end());
        })
    }
    let mut t = ui(|_| {
        column((
            bar_view(150, true).test_id("late"),
            bar_view(150, false).test_id("early"),
        ))
    });
    t.run_until_idle();
    let value = |t: &TestUi, id| t.engine().widget::<Bar>(node(t, id)).unwrap().value();
    assert_eq!(value(&t, "late"), 150);
    assert_eq!(value(&t, "early"), 100);
}
