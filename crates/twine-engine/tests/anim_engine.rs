//! Animations and timers in the engine: node properties through the setters, precise
//! invalidation, wall-clock timing, exact wake-ups and cleanup on deletion.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Mutex;

use twine_anim::{Anim, AnimProp, Easing, Repeat};
use twine_core::{Color, Duration, Opa, Rect};
use twine_engine::{Engine, NodeId, Obj, Wake, Widget, WidgetClass, WidgetCx};
use twine_style::{Length, Part, PropId, StyleProp, StyleValue};
use twine_testing::EngineHarness;

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.target() == "twine::anim" && m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            WARNINGS.lock().unwrap().push(r.args().to_string());
        }
    }
    fn flush(&self) {}
}

static LOGGER: Capture = Capture;

fn capture_warnings() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);
}

/// A 100 × 60 display with a white screen and a red 20 × 20 box at (10, 10), rendered.
fn scene() -> (EngineHarness, NodeId) {
    let mut b = None;
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        b = Some(common::boxed(e, s, Rect::from_xywh(10, 10, 20, 20), Color::RED));
    });
    h.run_until_idle();
    (h, b.unwrap())
}

fn x_of(e: &Engine, n: NodeId) -> i32 {
    e.coords(n).x0
}

#[test]
fn anim_x_moves_node_and_invalidates_old_and_new_only() {
    let (mut h, b) = scene();
    let t0 = h.now();
    h.engine_mut()
        .anim_start(b, AnimProp::X, Anim::new(10, 50).duration(Duration::ms(100)));
    // First animation step at t0: the start value equals the current position.
    h.engine_mut().run_anims(t0);
    h.engine_mut().update_layout();
    assert!(
        h.engine().invalidation_log().is_empty(),
        "{:?}",
        h.engine().invalidation_log()
    );
    h.engine_mut().run_anims(t0 + Duration::ms(50));
    h.engine_mut().update_layout();
    assert_eq!(x_of(h.engine(), b), 30);
    let old = Rect::from_xywh(10, 10, 20, 20);
    let new = Rect::from_xywh(30, 10, 20, 20);
    let log = h.engine().invalidation_log().to_vec();
    assert!(!log.is_empty());
    for (r, _) in &log {
        assert!(
            old.union(&new).contains_rect(r),
            "{r} outside the old and new areas"
        );
    }
    assert!(log.iter().any(|(r, _)| r.contains_rect(&old)));
    assert!(log.iter().any(|(r, _)| r.contains_rect(&new)));
    h.clock().set(t0 + Duration::ms(50));
    h.update();
    assert_eq!(h.pixel(45, 15), Color::RED);
    assert_eq!(h.pixel(15, 15), Color::WHITE);
}

#[test]
fn anim_wake_is_refresh_period_while_running_then_idle() {
    let (mut h, b) = scene();
    let period = h.engine().config().refr_period;
    h.engine_mut()
        .anim_start(b, AnimProp::Opa, Anim::new(255, 0).duration(Duration::ms(100)));
    let t0 = h.now();
    let mut frames = 0;
    let mut wake = h.update();
    loop {
        match wake {
            Wake::At(t) => {
                assert!(t > h.now() && t <= h.now() + period, "wake {t} at {}", h.now());
                h.clock().set(t);
                frames += 1;
                wake = h.update();
            }
            Wake::Now => wake = h.update(),
            Wake::Idle => break,
        }
        assert!(frames < 100, "never idle");
    }
    // One frame per period, no lingering wake-up after the end (P1).
    let expect = (100 / period.as_millis()) as i32;
    assert!((frames - expect).abs() <= 1, "{frames} frames");
    assert!(h.now() >= t0 + Duration::ms(100));
    assert_eq!(h.engine().anim_count(), 0);
    assert_eq!(h.engine().style_opa(b, Part::Main, PropId::Opa), Opa::TRANSP);
    h.assert_idle();
}

#[test]
fn late_frame_jumps_to_correct_value() {
    let (mut h, b) = scene();
    h.engine_mut()
        .anim_start(b, AnimProp::X, Anim::new(0, 100).duration(Duration::ms(1000)));
    h.update();
    assert_eq!(x_of(h.engine(), b), 0);
    // A 500 ms late frame: the value of 500 ms, not the next small step.
    h.clock().advance(Duration::ms(500));
    h.update();
    assert_eq!(x_of(h.engine(), b), 50);
    assert_eq!(
        h.engine().style_prop(b, Part::Main, PropId::X),
        StyleValue::Length(Length::Px(50))
    );
}

#[test]
fn anim_started_between_updates_starts_at_next_update() {
    let (mut h, b) = scene();
    // Long idle time since the last update.
    h.clock().advance(Duration::secs(10));
    h.engine_mut()
        .anim_start(b, AnimProp::X, Anim::new(0, 100).duration(Duration::ms(100)));
    assert_eq!(h.update(), Wake::At(h.now() + h.engine().config().refr_period));
    assert_eq!(x_of(h.engine(), b), 0);
}

#[test]
fn deleting_node_removes_its_anims() {
    capture_warnings();
    let (mut h, b) = scene();
    let child = h.engine_mut().create(b, Box::new(Obj)).unwrap();
    let e = h.engine_mut();
    e.anim_start(b, AnimProp::X, Anim::new(0, 50).repeat(Repeat::Infinite));
    e.anim_start(b, AnimProp::Opa, Anim::new(0, 255).repeat(Repeat::Infinite));
    e.anim_start(child, AnimProp::Width, Anim::new(0, 10).repeat(Repeat::Infinite));
    assert_eq!(e.anims_of(b).count(), 2);
    assert_eq!(e.anim_count(), 3);
    h.advance(Duration::ms(40));
    let warnings_before = WARNINGS.lock().unwrap().len();
    h.engine_mut().delete(b).unwrap();
    assert_eq!(h.engine().anim_count(), 0);
    assert_eq!(h.engine().anims_of(b).count(), 0);
    h.run_until_idle();
    assert_eq!(WARNINGS.lock().unwrap().len(), warnings_before);
    h.assert_idle();
}

#[test]
fn replacing_anim_of_same_prop() {
    let (mut h, b) = scene();
    let e = h.engine_mut();
    let a = e.anim_start(b, AnimProp::X, Anim::new(0, 50));
    let c = e.anim_start(b, AnimProp::X, Anim::new(0, 80));
    assert!(!e.anim_exists(a));
    assert!(e.anim_exists(c));
    assert_eq!(e.anims_of(b).collect::<Vec<_>>(), [c]);
}

#[test]
fn timer_wake_deadline_exact() {
    let (mut h, _) = scene();
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    h.engine_mut()
        .timer_add(Duration::secs(1), move |_, _| r.set(r.get() + 1));
    let t0 = h.now();
    assert_eq!(h.update(), Wake::At(t0 + Duration::secs(1)));
    h.clock().set(t0 + Duration::secs(1));
    assert_eq!(h.update(), Wake::At(t0 + Duration::secs(2)));
    assert_eq!(runs.get(), 1);
}

#[test]
fn timer_callback_gets_engine_and_can_remove_itself() {
    let (mut h, b) = scene();
    let t0 = h.now();
    h.engine_mut().timer_add(Duration::ms(100), move |e, id| {
        common::style(e, b, &[StyleProp::BgColor(Color::BLUE)]);
        assert!(e.timer_remove(id));
    });
    h.update();
    h.clock().set(t0 + Duration::ms(100));
    h.update();
    assert_eq!(h.pixel(15, 15), Color::BLUE);
    h.assert_idle();
}

#[test]
fn timer_removed_by_earlier_timer_does_not_run() {
    let (mut h, _) = scene();
    let log = Rc::new(RefCell::new(Vec::new()));
    let victim = Rc::new(Cell::new(None));
    let (l1, v1) = (log.clone(), victim.clone());
    h.engine_mut().timer_add(Duration::ms(10), move |e, _| {
        l1.borrow_mut().push(1);
        if let Some(v) = v1.get() {
            e.timer_remove(v);
        }
    });
    let l2 = log.clone();
    let v = h
        .engine_mut()
        .timer_add(Duration::ms(10), move |_, _| l2.borrow_mut().push(2));
    victim.set(Some(v));
    h.update();
    h.clock().advance(Duration::ms(10));
    h.update();
    assert_eq!(*log.borrow(), [1]);
    assert!(!h.engine().timer_exists(v));
}

#[test]
fn anim_fn_exec_receives_engine() {
    let (mut h, b) = scene();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    h.engine_mut().anim_start_fn(
        Anim::new(0, 255).duration(Duration::ms(32)),
        move |e: &mut Engine, v| {
            s.borrow_mut().push(v);
            common::style(e, b, &[StyleProp::BgColor(Color::new(0, 0, v as u8))]);
        },
    );
    h.run_until_idle();
    assert_eq!(seen.borrow().first(), Some(&0));
    assert_eq!(seen.borrow().last(), Some(&255));
    assert_eq!(h.pixel(15, 15), Color::new(0, 0, 255));
}

#[test]
fn callbacks_can_defer_engine_work() {
    let (mut h, b) = scene();
    let a = Anim::new(0, 1).duration(Duration::ms(20)).on_complete(move |cx| {
        twine_engine::defer(cx, move |e| e.delete(b).unwrap());
    });
    h.engine_mut().anim_start(b, AnimProp::Opa, a);
    h.run_until_idle();
    assert!(!h.engine().tree().contains(b));
}

#[test]
fn anim_scroll_prop_scrolls() {
    capture_warnings();
    let warnings_before = WARNINGS.lock().unwrap().len();
    let (mut h, b) = scene();
    let e = h.engine_mut();
    let child = e.create(b, Box::new(Obj)).unwrap();
    e.set_size(child, 20, 100);
    h.update();
    h.engine_mut()
        .anim_start(b, AnimProp::ScrollY, Anim::new(0, 40).duration(Duration::ms(50)));
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(b).y, 40);
    assert_eq!(
        h.engine().coords(child).y0,
        10 - 40,
        "children move with the offset"
    );
    assert_eq!(
        h.engine().coords(b),
        Rect::from_xywh(10, 10, 20, 20),
        "the node itself stays"
    );
    assert_eq!(WARNINGS.lock().unwrap().len(), warnings_before);
}

struct Gauge {
    value: Rc<Cell<i32>>,
    custom: Rc<Cell<(u16, i32)>>,
}
static GAUGE: WidgetClass = WidgetClass::new("gauge");
impl Widget for Gauge {
    fn class(&self) -> &'static WidgetClass {
        &GAUGE
    }
    fn anim_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        self.value.set(v);
        cx.invalidate();
    }
    fn anim_custom(&mut self, _cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        self.custom.set((id, v));
    }
}

#[test]
fn value_and_custom_props_reach_widget_hooks() {
    let (mut h, _) = scene();
    let value = Rc::new(Cell::new(-1));
    let custom = Rc::new(Cell::new((0, -1)));
    let s = h.screen();
    let g = h
        .engine_mut()
        .create(
            s,
            Box::new(Gauge {
                value: value.clone(),
                custom: custom.clone(),
            }),
        )
        .unwrap();
    let e = h.engine_mut();
    e.anim_start(g, AnimProp::Value, Anim::new(0, 70).duration(Duration::ms(20)));
    e.anim_start(g, AnimProp::Custom(3), Anim::new(5, 9).duration(Duration::ms(20)));
    h.run_until_idle();
    assert_eq!(value.get(), 70);
    assert_eq!(custom.get(), (3, 9));
}

#[test]
fn style_prop_and_transform_targets() {
    let (mut h, b) = scene();
    let e = h.engine_mut();
    e.anim_start(
        b,
        AnimProp::StyleProp(PropId::Radius as u8),
        Anim::new(0, 6).duration(Duration::ms(20)),
    );
    e.anim_start(
        b,
        AnimProp::ScaleX,
        Anim::new(256, 512).duration(Duration::ms(20)),
    );
    e.anim_start(
        b,
        AnimProp::Rotation,
        Anim::new(0, 900).duration(Duration::ms(20)),
    );
    e.anim_start(
        b,
        AnimProp::TranslateY,
        Anim::new(0, 5).duration(Duration::ms(20)),
    );
    h.run_until_idle();
    let e = h.engine();
    assert_eq!(e.style_i32(b, Part::Main, PropId::Radius), 6);
    assert_eq!(
        e.style_prop(b, Part::Main, PropId::TransformScaleX),
        StyleValue::Scale(twine_core::Scale(512))
    );
    assert_eq!(
        e.style_prop(b, Part::Main, PropId::TransformRotation),
        StyleValue::Angle(twine_core::Angle(900))
    );
    assert_eq!(e.coords(b).y0, 15);
}

#[test]
fn pause_resume_between_updates() {
    let (mut h, b) = scene();
    // 1 px per ms, exact at multiples of 32 ms.
    let id = h
        .engine_mut()
        .anim_start(b, AnimProp::X, Anim::new(0, 128).duration(Duration::ms(128)));
    h.update();
    h.clock().advance(Duration::ms(32));
    h.update();
    assert_eq!(x_of(h.engine(), b), 32);
    assert!(h.engine_mut().anim_pause(id));
    // Paused: nothing to wake up for.
    h.run_until_idle();
    h.clock().advance(Duration::secs(5));
    h.update();
    assert_eq!(x_of(h.engine(), b), 32);
    assert!(h.engine_mut().anim_resume(id));
    h.update();
    h.clock().advance(Duration::ms(32));
    h.update();
    assert_eq!(x_of(h.engine(), b), 64);
}

/// The `anim_gallery` screen 1 scene: easing rows, a pulsing box, all infinite.
#[test]
fn anim_gallery_scene_idle_when_paused() {
    let (mut h, _) = scene();
    let s = h.screen();
    let easings = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::Overshoot,
        Easing::Bounce,
        Easing::Step,
        Easing::CubicBezier(200, 0, 300, 1024),
    ];
    let mut ids = Vec::new();
    for (i, easing) in easings.into_iter().enumerate() {
        let b = common::boxed(
            h.engine_mut(),
            s,
            Rect::from_xywh(0, i as i32 * 6, 5, 5),
            Color::BLUE,
        );
        ids.push(
            h.engine_mut().anim_start(
                b,
                AnimProp::X,
                Anim::new(0, 80)
                    .duration(Duration::ms(1500))
                    .easing(easing)
                    .playback(Duration::ms(1500))
                    .repeat(Repeat::Infinite),
            ),
        );
    }
    let pulse = common::boxed(h.engine_mut(), s, Rect::from_xywh(50, 50, 8, 8), Color::GREEN);
    ids.push(
        h.engine_mut().anim_start(
            pulse,
            AnimProp::Opa,
            Anim::new(255, 60)
                .duration(Duration::ms(800))
                .playback(Duration::ms(800))
                .repeat(Repeat::Infinite),
        ),
    );
    h.advance(Duration::ms(300));
    assert!(matches!(h.update(), Wake::At(_)));
    for &id in &ids {
        assert!(h.engine_mut().anim_pause(id));
    }
    // Frames already due are rendered, then nothing wakes the CPU any more (P1).
    let t = h.now();
    h.run_until_idle();
    assert!(h.now() <= t + h.engine().config().refr_period);
    h.assert_idle();
    for &id in &ids {
        h.engine_mut().anim_resume(id);
    }
    assert!(!h.update().is_idle());
}
