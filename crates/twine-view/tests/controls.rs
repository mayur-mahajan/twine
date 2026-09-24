//! The basic control views: two-way bindings round trip without loops, a signal change runs
//! one binding and redraws only its widget, and the UI is idle after every interaction.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::{Point, Rect};
use twine_hal::Key;
use twine_reactive::debug_stats;
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;
use twine_widgets::animimg::AnimImg;
use twine_widgets::arc::Arc;
use twine_widgets::bar::Bar;
use twine_widgets::image_button::ImageButton;
use twine_widgets::led::{LED_BRIGHT_MAX, LED_BRIGHT_MIN, Led};
use twine_widgets::line::Line;
use twine_widgets::slider::Slider;
use twine_widgets::spinner::Spinner;

/// Reads widget `W` of the node with test id `id`.
fn read<W: Widget, R>(t: &TestUi, id: &'static str, f: impl FnOnce(&W) -> R) -> R {
    let n = t.find(by_id(id)).id();
    let e = t.engine();
    f(e.widget::<W>(n).expect("widget type"))
}

/// The node's area including its extra draw size.
fn draw_area(t: &TestUi, id: &'static str) -> Rect {
    let n = t.find(by_id(id)).id();
    let e = t.engine();
    let node = e.tree().node(n).unwrap();
    node.coords().expand(i32::from(node.ext_draw()))
}

/// Focuses the node `id` in its (default) group.
fn focus(t: &mut TestUi, id: &'static str) {
    let n = t.find(by_id(id)).id();
    t.engine_mut().focus(n);
    t.run_until_idle();
}

/// Sets `sig` outside the `Ui` and checks that the next frame runs exactly one binding and
/// redraws only inside `id`'s area (before or after the change: content-sized widgets grow).
fn assert_one_binding_local<T: 'static>(t: &mut TestUi, sig: Signal<T>, v: T, id: &'static str) {
    let area = draw_area(t, id);
    sig.set(v); // outside the Ui: deferred to the next update
    let runs = debug_stats().effect_runs;
    let period = t.engine().config().refr_period;
    t.advance(period);
    assert_eq!(debug_stats().effect_runs - runs, 1, "one binding run");
    let area = area.union(&draw_area(t, id));
    let inv: Vec<Rect> = t.invalidations().iter().map(|(r, _)| *r).collect();
    assert!(!inv.is_empty(), "the change is drawn");
    for r in &inv {
        assert!(area.contains_rect(r), "{r:?} is outside {id}'s area {area:?}");
    }
    t.run_until_idle();
    t.assert_idle();
}

#[test]
fn bar_value_applied_after_range() {
    let mut t = TestUi::new(240, 80).mount(|_| bar(150).range(0..=200).width(200).test_id("bar"));
    t.run_until_idle();
    assert_eq!(read(&t, "bar", Bar::value), 150);
}

#[test]
fn bar_signal_runs_one_binding_and_redraws_only_the_bar() {
    let mut t = TestUi::new(240, 120).mount(|cx| {
        let level = cx.signal(20);
        cx.provide(level);
        column((bar(level).width(200).test_id("bar"), label("unrelated")))
    });
    t.run_until_idle();
    let level = t.root_scope().expect_context::<Signal<i32>>();
    assert_one_binding_local(&mut t, level, 60, "bar");
    assert_eq!(read(&t, "bar", Bar::value), 60);
}

#[test]
fn bar_animated_animates_value_changes() {
    let mut t = TestUi::new(240, 80).mount(|cx| {
        let level = cx.signal(0);
        cx.provide(level);
        bar(level).animated(Duration::ms(300)).width(200).test_id("bar")
    });
    t.run_until_idle();
    let level = t.root_scope().expect_context::<Signal<i32>>();
    level.set(80);
    t.update();
    assert!(read(&t, "bar", Bar::is_animating), "the new value animates");
    let took = t.run_until_idle();
    assert!(took >= Duration::ms(280), "{took:?}");
    assert_eq!(read(&t, "bar", Bar::value), 80);
}

#[test]
fn slider_view_two_way() {
    let mut t = TestUi::new(240, 80).mount(|cx| {
        let level = cx.signal(10);
        cx.provide(level);
        slider(level)
            .range(0..=100)
            .width(200)
            .align(Align::Center)
            .test_id("s")
    });
    t.run_until_idle();
    let level = t.root_scope().expect_context::<Signal<i32>>();
    let c = t.find(by_id("s")).coords();
    // Drag the knob to three quarters.
    let from = Point::new(c.x0 + c.width() / 10, c.center().y);
    let to = Point::new(c.x0 + c.width() * 3 / 4, c.center().y);
    t.drag(from, to, Duration::ms(200));
    t.run_until_idle();
    let v = read(&t, "s", Slider::value);
    assert!((70..=80).contains(&v), "{v}");
    assert_eq!(level.get_untracked(), v, "the drag wrote the value back");
    assert_eq!(debug_stats().loop_cuts, 0);
    t.assert_idle();
    // Signal → knob, one binding, local redraw.
    assert_one_binding_local(&mut t, level, 20, "s");
    assert_eq!(read(&t, "s", Slider::value), 20);
}

#[test]
fn slider_keys_write_back() {
    let mut t = TestUi::new(240, 80).mount(|cx| {
        let level = cx.signal(50);
        cx.provide(level);
        slider(level).width(200).test_id("s")
    });
    t.run_until_idle();
    let level = t.root_scope().expect_context::<Signal<i32>>();
    focus(&mut t, "s");
    t.key(Key::Right);
    t.key(Key::Right);
    t.run_until_idle();
    assert_eq!(level.get_untracked(), 52);
    t.assert_idle();
}

#[test]
fn slider_range_mode_two_models() {
    let mut t = TestUi::new(240, 80).mount(|cx| {
        let lo = cx.signal(20);
        let hi = cx.signal(80);
        cx.provide((lo, hi));
        slider(hi)
            .mode(SliderMode::Range)
            .left_value(lo)
            .width(200)
            .align(Align::Center)
            .test_id("s")
    });
    t.run_until_idle();
    let (lo, hi) = t.root_scope().expect_context::<(Signal<i32>, Signal<i32>)>();
    assert_eq!(read(&t, "s", Slider::left_value), 20);
    let c = t.find(by_id("s")).coords();
    // Drag the left knob from 20 % to 40 %.
    let from = Point::new(c.x0 + c.width() / 5, c.center().y);
    let to = Point::new(c.x0 + c.width() * 2 / 5, c.center().y);
    t.drag(from, to, Duration::ms(200));
    t.run_until_idle();
    let l = read(&t, "s", Slider::left_value);
    assert!((35..=45).contains(&l), "{l}");
    assert_eq!(lo.get_untracked(), l);
    assert_eq!(hi.get_untracked(), 80, "the right knob did not move");
    // Signals → knobs.
    hi.set(90);
    lo.set(5);
    t.run_until_idle();
    assert_eq!(read(&t, "s", |s: &Slider| (s.left_value(), s.value())), (5, 90));
}

#[test]
fn switch_checkbox_led_share_one_signal() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let on = cx.signal(false);
        cx.provide(on);
        column((
            switch(on).test_id("sw"),
            checkbox("Enabled", on).test_id("cb"),
            led(on).test_id("led"),
        ))
    });
    t.run_until_idle();
    let on = t.root_scope().expect_context::<Signal<bool>>();
    assert_eq!(read(&t, "led", Led::brightness), LED_BRIGHT_MIN);
    t.find(by_id("sw")).click();
    t.run_until_idle();
    assert!(on.get_untracked());
    assert!(t.find(by_id("cb")).state().contains(State::CHECKED));
    assert_eq!(read(&t, "led", Led::brightness), LED_BRIGHT_MAX);
    assert_eq!(t.find(by_id("cb")).text(), "Enabled");
    t.assert_idle();
    t.find(by_id("cb")).click();
    t.run_until_idle();
    assert!(!on.get_untracked());
    assert!(!t.find(by_id("sw")).state().contains(State::CHECKED));
    assert_eq!(debug_stats().loop_cuts, 0);
    t.assert_idle();
    // Signal → switch: one binding per widget reading it.
    on.set(true);
    let runs = debug_stats().effect_runs;
    t.run_until_idle();
    assert_eq!(
        debug_stats().effect_runs - runs,
        3,
        "switch, checkbox and LED bindings"
    );
    assert!(t.find(by_id("sw")).state().contains(State::CHECKED));
}

#[test]
fn switch_owned_reports_on_change() {
    let seen: Rc<Cell<Option<bool>>> = Rc::default();
    let s = seen.clone();
    let mut t =
        TestUi::new(200, 80).mount(move |_| switch(true).on_change(move |v| s.set(Some(v))).test_id("sw"));
    t.run_until_idle();
    assert!(t.find(by_id("sw")).state().contains(State::CHECKED));
    t.find(by_id("sw")).click();
    t.run_until_idle();
    assert_eq!(seen.get(), Some(false));
}

#[test]
fn checkbox_dynamic_text() {
    let mut t = TestUi::new(200, 80).mount(|cx| {
        let n = cx.signal(1);
        cx.provide(n);
        // A fixed size: the text change redraws only the checkbox.
        checkbox(text!("Item {}", n.get()), false)
            .size(150, 30)
            .test_id("cb")
    });
    t.run_until_idle();
    let n = t.root_scope().expect_context::<Signal<i32>>();
    assert_eq!(t.find(by_id("cb")).text(), "Item 1");
    assert_one_binding_local(&mut t, n, 2, "cb");
    assert_eq!(t.find(by_id("cb")).text(), "Item 2");
}

#[test]
fn arc_view_two_way() {
    let changes: Rc<Cell<u32>> = Rc::default();
    let c = changes.clone();
    let mut t = TestUi::new(200, 200).mount(move |cx| {
        let v = cx.signal(30);
        cx.provide(v);
        arc(v)
            .range(0..=100)
            .on_change(move |_| c.set(c.get() + 1))
            .focusable(true)
            .test_id("arc")
    });
    t.run_until_idle();
    let v = t.root_scope().expect_context::<Signal<i32>>();
    focus(&mut t, "arc");
    t.key(Key::Right);
    t.run_until_idle();
    assert_eq!(v.get_untracked(), 31);
    assert_eq!(changes.get(), 1);
    t.assert_idle();
    assert_one_binding_local(&mut t, v, 60, "arc");
    assert_eq!(read(&t, "arc", Arc::value), 60);
    assert_eq!(changes.get(), 1, "programmatic changes are not user changes");
}

#[test]
fn arc_without_knob_is_display_only() {
    let mut t = TestUi::new(200, 200).mount(|_| arc(40).knob(false).test_id("arc"));
    t.run_until_idle();
    let n = t.find(by_id("arc")).id();
    assert!(!t.engine().has_flag(n, ObjFlags::CLICKABLE));
}

#[test]
fn led_brightness_while_on() {
    let mut t = TestUi::new(100, 100).mount(|cx| {
        let on = cx.signal(true);
        cx.provide(on);
        led(on).brightness(150).color(Color::RED).test_id("led")
    });
    t.run_until_idle();
    let on = t.root_scope().expect_context::<Signal<bool>>();
    assert_eq!(
        read(&t, "led", |l: &Led| (l.brightness(), l.color())),
        (150, Color::RED)
    );
    on.set(false);
    t.run_until_idle();
    assert_eq!(read(&t, "led", Led::brightness), LED_BRIGHT_MIN);
    on.set(true);
    t.run_until_idle();
    assert_eq!(read(&t, "led", Led::brightness), 150);
}

#[test]
fn line_points_binding() {
    static ZIGZAG: [Point; 3] = [Point::new(0, 0), Point::new(10, 10), Point::new(20, 0)];
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let pts = cx.signal(vec![Point::new(0, 0), Point::new(40, 20)]);
        cx.provide(pts);
        column((
            line(pts).width(3).rounded(true).size(50, 30).test_id("l"),
            line_static(&ZIGZAG).dash(2, 2).test_id("z"),
        ))
    });
    t.run_until_idle();
    let pts = t.root_scope().expect_context::<Signal<Vec<Point>>>();
    assert_eq!(read(&t, "z", |l: &Line| l.points().len()), 3);
    assert_one_binding_local(&mut t, pts, vec![Point::new(0, 0), Point::new(30, 20)], "l");
    assert_eq!(
        read(&t, "l", |l: &Line| l.points().to_vec()),
        [Point::new(0, 0), Point::new(30, 20)]
    );
}

#[test]
fn spinner_params_and_not_clickable() {
    let mut t = TestUi::new(100, 100).mount(|_| {
        spinner()
            .period(Duration::ms(1500))
            .arc_angle(Angle::deg(90))
            .size(60, 60)
            .test_id("sp")
    });
    t.advance(Duration::ms(100));
    assert_eq!(
        read(&t, "sp", |s: &Spinner| (s.period(), s.sweep())),
        (Duration::ms(1500), Angle::deg(90))
    );
    let n = t.find(by_id("sp")).id();
    assert!(!t.engine().has_flag(n, ObjFlags::CLICKABLE));
}

/// A 16×16 A8 square (`value` everywhere).
macro_rules! square {
    ($name:ident, $value:expr) => {
        static $name: twine_image::Image = twine_image::Image::new_static(
            twine_image::ImageHeader {
                format: twine_image::ColorFormat::A8,
                w: 16,
                h: 16,
                stride: 16,
                flags: twine_image::ImageFlags::empty(),
            },
            &[$value; 256],
        );
    };
}

square!(OFF, 0x80);
square!(ON, 0xFF);

#[test]
fn image_button_checked_two_way() {
    let mut t = TestUi::new(120, 80).mount(|cx| {
        let on = cx.signal(false);
        cx.provide(on);
        image_button(ImageSource::from(&OFF), ImageSource::from(&OFF))
            .checked_images(ImageSource::from(&ON), ImageSource::from(&ON))
            .checkable(true)
            .checked(on)
            .test_id("ib")
    });
    t.run_until_idle();
    let on = t.root_scope().expect_context::<Signal<bool>>();
    t.find(by_id("ib")).click();
    t.run_until_idle();
    assert!(on.get_untracked());
    assert!(read(&t, "ib", |b: &ImageButton| b
        .src(ImageButtonState::CheckedReleased)[1]
        .is_some()));
    t.assert_idle();
    assert_one_binding_local(&mut t, on, false, "ib");
    assert!(!t.find(by_id("ib")).state().contains(State::CHECKED));
}

#[test]
fn animimg_playing_binding() {
    static FRAMES: [ImageSource; 3] = [
        ImageSource::Symbol(symbols::PLAY),
        ImageSource::Symbol(symbols::PAUSE),
        ImageSource::Symbol(symbols::STOP),
    ];
    let mut t = TestUi::new(100, 100).mount(|cx| {
        let run = cx.signal(false);
        cx.provide(run);
        animimg(&FRAMES, Duration::ms(300))
            .repeat(Repeat::Count(0))
            .playing(run)
            .test_id("a")
    });
    t.run_until_idle();
    t.assert_idle();
    let run = t.root_scope().expect_context::<Signal<bool>>();
    run.set(true);
    t.advance(Duration::ms(150));
    let n = t.find(by_id("a")).id();
    assert!(t.engine().widget::<AnimImg>(n).unwrap().frame_index() >= 1);
    t.run_until_idle();
    t.assert_idle();
}

#[test]
fn animimg_plays_by_default() {
    static FRAMES: [ImageSource; 2] = [
        ImageSource::Symbol(symbols::PLAY),
        ImageSource::Symbol(symbols::PAUSE),
    ];
    let mut t = TestUi::new(100, 100).mount(|_| animimg(&FRAMES, Duration::ms(200)).test_id("a"));
    t.advance(Duration::ms(150));
    let n = t.find(by_id("a")).id();
    assert_eq!(t.engine().widget::<AnimImg>(n).unwrap().frame_index(), 1);
}
