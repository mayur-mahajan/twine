//! The `Lottie` widget: frame-change-only redraw, playback timing, looping, idle while paused,
//! a placeholder for invalid data, no allocation per frame.

use twine_core::{Color, Duration};
use twine_engine::{Engine, NodeId, WidgetCx};
use twine_lottie::widget::{self, Lottie};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{EngineHarness, capture_logs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

static LOADER: &[u8] = include_bytes!("data/loader.json");

fn with<R>(h: &mut EngineHarness, id: NodeId, f: impl FnOnce(&mut Lottie, &mut WidgetCx<'_>) -> R) -> R {
    h.engine_mut().with_widget_mut(id, f).expect("a lottie widget")
}

fn get(e: &Engine, id: NodeId) -> &Lottie {
    e.widget::<Lottie>(id).expect("a lottie widget")
}

/// A 100 × 100 loader (30 fps, 60 frames) at the top left of a 120 × 120 screen.
fn scene() -> (EngineHarness, NodeId) {
    let mut h = EngineHarness::new(120, 120);
    let screen = h.screen();
    let l = widget::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().set_size(l, 100, 100);
    with(&mut h, l, |w, cx| w.set_src(cx, LOADER));
    h.run_until_idle();
    (h, l)
}

#[test]
fn same_frame_no_rerender_no_invalidate() {
    let (mut h, l) = scene();
    with(&mut h, l, |w, cx| w.set_frame(cx, 5));
    h.run_until_idle();
    assert!(!h.flushes().is_empty(), "a new frame is drawn");
    with(&mut h, l, |w, cx| w.set_frame(cx, 5));
    h.run_until_idle();
    assert!(h.flushes().is_empty(), "the same frame changes nothing");
    assert_eq!(get(h.engine(), l).current_frame(), 5);
    with(&mut h, l, |w, cx| w.set_frame(cx, 1000));
    assert_eq!(
        get(h.engine(), l).current_frame(),
        59,
        "clamped to the last frame"
    );
}

#[test]
fn playback_advances_frames_at_fps() {
    let (mut h, l) = scene();
    with(&mut h, l, |w, cx| {
        w.set_loop(cx, false);
        w.play(cx);
    });
    h.advance(Duration::ms(1000));
    let f = get(h.engine(), l).current_frame();
    assert!((28..=31).contains(&f), "frame {f} after 1 s at 30 fps");
    with(&mut h, l, |w, cx| w.set_speed(cx, twine_core::Scale(512)));
    h.advance(Duration::ms(250));
    let g = get(h.engine(), l).current_frame();
    assert!((f + 13..=f + 17).contains(&g), "2x speed: {f} -> {g} in 250 ms");
}

#[test]
fn loop_wraps() {
    let (mut h, l) = scene();
    with(&mut h, l, twine_lottie::widget::Lottie::play);
    h.advance(Duration::ms(2500)); // 75 frames: wrapped once
    let f = get(h.engine(), l).current_frame();
    assert!((13..=17).contains(&f), "frame {f} after 2.5 s of a 2 s loop");
    assert!(get(h.engine(), l).is_playing());
}

#[test]
fn non_looping_finishes_and_goes_idle() {
    let (mut h, l) = scene();
    with(&mut h, l, |w, cx| {
        w.set_loop(cx, false);
        w.play(cx);
    });
    h.advance(Duration::ms(2100));
    assert_eq!(get(h.engine(), l).current_frame(), 59);
    assert!(!get(h.engine(), l).is_playing());
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn pause_stops_wakeups() {
    let (mut h, l) = scene();
    with(&mut h, l, twine_lottie::widget::Lottie::play);
    h.advance(Duration::ms(300));
    with(&mut h, l, twine_lottie::widget::Lottie::pause);
    let f = get(h.engine(), l).current_frame();
    h.run_until_idle();
    h.assert_idle();
    assert_eq!(get(h.engine(), l).current_frame(), f);
    // Resumes where it stopped.
    with(&mut h, l, twine_lottie::widget::Lottie::play);
    h.advance(Duration::ms(100));
    let g = get(h.engine(), l).current_frame();
    assert!((f + 2..=f + 4).contains(&g), "{f} -> {g}");
}

#[test]
fn seek_while_paused_resumes_from_the_new_frame() {
    let (mut h, l) = scene();
    with(&mut h, l, twine_lottie::widget::Lottie::play);
    h.advance(Duration::ms(300));
    with(&mut h, l, |w, cx| {
        w.pause(cx);
        w.set_frame(cx, 40);
        w.play(cx);
    });
    h.advance(Duration::ms(100));
    let f = get(h.engine(), l).current_frame();
    assert!((42..=44).contains(&f), "{f}");
}

#[test]
fn invalid_data_placeholder() {
    let (mut h, l) = scene();
    let ((), logs) = capture_logs(|| {
        with(&mut h, l, |w, cx| w.set_src(cx, b"{ not lottie"));
        h.run_until_idle();
    });
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.target == "twine::lottie"),
        "{logs:?}"
    );
    assert_eq!(get(h.engine(), l).total_frames(), 0);
    let c = h.pixel(50, 50);
    let grey = Color::hex(0x00D0_D0D0);
    let near = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= 8;
    assert!(
        near(c.r, grey.r) && near(c.g, grey.g) && near(c.b, grey.b),
        "{c:?}: grey placeholder"
    );
    with(&mut h, l, twine_lottie::widget::Lottie::play);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn no_alloc_per_frame() {
    let (mut h, l) = scene();
    with(&mut h, l, twine_lottie::widget::Lottie::play);
    h.advance(Duration::ms(2100)); // warm-up: every frame once, buffers at their largest
    let ((), stats) = count_allocs(|| h.advance(Duration::ms(1000)));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn frame_snapshots() {
    let (mut h, l) = scene();
    for f in [0, 20, 40] {
        with(&mut h, l, |w, cx| w.set_frame(cx, f));
        h.run_until_idle();
        h.assert_snapshot(&format!("lottie_widget_loader_{f}"));
    }
}
