//! `AnimImg`: LVGL defaults, frame stepping over the period, repeat counts ending idle,
//! frame changes redrawing only the image, GIF frames with their own delays, snapshots.

mod common;

use std::sync::OnceLock;

use common::{Mode, get, harness, solid_image, with};
use twine_core::{Color, Duration, Size};
use twine_engine::{MeasureCx, NodeId, Repeat, Wake};
use twine_image::ImageSource;
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_widgets::animimg::{self, ANIMIMG_DEFAULT_PERIOD, AnimImg};

fn frames() -> &'static [ImageSource] {
    static F: OnceLock<Vec<ImageSource>> = OnceLock::new();
    F.get_or_init(|| {
        [
            Color::new(0xF4, 0x43, 0x36),
            Color::new(0x4C, 0xAF, 0x50),
            Color::new(0x21, 0x96, 0xF3),
            Color::new(0xFF, 0xC1, 0x07),
        ]
        .into_iter()
        .map(|c| solid_image(24, 24, c))
        .collect()
    })
}

fn scene(mode: Mode, period_ms: u64, repeat: Repeat) -> (EngineHarness, NodeId) {
    let mut h = harness(80, 60, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let a = animimg::create(e, screen).unwrap();
    e.align(a, Align::Center, 0, 0);
    with(&mut h, a, |w: &mut AnimImg, cx| {
        w.set_frames(cx, frames());
        w.set_period(cx, Duration::ms(period_ms));
        w.set_repeat(cx, repeat);
    });
    h.run_until_idle();
    (h, a)
}

fn idx(h: &EngineHarness, a: NodeId) -> usize {
    get::<AnimImg>(h, a).frame_index()
}

fn playing(h: &EngineHarness, a: NodeId) -> bool {
    get::<AnimImg>(h, a).is_playing(&MeasureCx::new(h.engine(), a))
}

#[test]
fn animimg_defaults() {
    let mut h = harness(80, 60, Mode::Light);
    let screen = h.screen();
    let a = animimg::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    assert_eq!(h.engine().tree().node(a).unwrap().class().name, "animimg");
    let w = get::<AnimImg>(&h, a);
    assert_eq!(
        (w.period(), w.repeat()),
        (ANIMIMG_DEFAULT_PERIOD, Repeat::Infinite)
    );
    assert!(w.frames().is_empty());
    assert!(!playing(&h, a), "LVGL: nothing plays before start");
    h.assert_idle();
}

#[test]
fn animimg_setters_same_value_no_invalidate() {
    let (mut h, a) = scene(Mode::Light, 400, Repeat::Infinite);
    with(&mut h, a, |w: &mut AnimImg, cx| {
        w.set_frames(cx, frames());
        w.set_period(cx, Duration::ms(400));
        w.set_repeat(cx, Repeat::Infinite);
    });
    assert!(h.engine().invalidation_log().is_empty());
    assert_eq!(
        h.engine().coords(a).size(),
        Size::new(24, 24),
        "the first frame is shown"
    );
    h.assert_idle();
}

#[test]
fn animimg_advances_frames() {
    let (mut h, a) = scene(Mode::Light, 400, Repeat::Infinite);
    with(&mut h, a, |w: &mut AnimImg, cx| w.start(cx));
    h.update();
    assert_eq!(idx(&h, a), 0);
    h.advance(Duration::ms(110));
    assert_eq!(idx(&h, a), 1);
    h.advance(Duration::ms(100));
    assert_eq!(idx(&h, a), 2);
    h.advance(Duration::ms(100));
    assert_eq!(idx(&h, a), 3);
    h.advance(Duration::ms(100));
    assert_eq!(idx(&h, a), 0, "repeats");
    // A frame change redraws only the image.
    let n = h.engine().tree().node(a).unwrap();
    let own = n.coords().expand(i32::from(n.ext_draw()));
    for _ in 0..10 {
        h.advance(Duration::ms(40));
        for f in h.flushes() {
            assert!(own.contains_rect(&f.area), "{} outside {own}", f.area);
        }
    }
    with(&mut h, a, |w: &mut AnimImg, cx| w.stop(cx));
    assert!(!playing(&h, a));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn animimg_stops_after_repeat_count_and_idles() {
    let (mut h, a) = scene(Mode::Light, 200, Repeat::Count(1));
    with(&mut h, a, |w: &mut AnimImg, cx| w.start(cx));
    h.update();
    assert!(playing(&h, a));
    let t = h.run_until_idle();
    // `Count(1)`: once and one more time (twine's `Repeat`; LVGL's repeat count 2).
    assert!(
        t >= Duration::ms(390) && t <= Duration::ms(460),
        "played for {t:?}"
    );
    assert_eq!(idx(&h, a), 3, "the last frame stays");
    assert!(!playing(&h, a));
    assert_eq!(h.update(), Wake::Idle);
    h.assert_idle();
}

/// A 1 × 1 GIF with two frames: red for 200 ms, then blue for 50 ms, played once.
static GIF: &[u8] = &[
    b'G', b'I', b'F', b'8', b'9', b'a', 1, 0, 1, 0, 0x80, 0, 0, // screen, 2-color table
    0xFF, 0, 0, 0, 0, 0xFF, // palette: red, blue
    0x21, 0xF9, 4, 0, 20, 0, 0, 0, // 200 ms
    0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0, // index 0
    0x21, 0xF9, 4, 0, 5, 0, 0, 0, // 50 ms
    0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4C, 0x01, 0, // index 1
    0x3B,
];

#[test]
fn animimg_gif_uses_frame_delays() {
    let mut h = harness(40, 40, Mode::Light);
    let screen = h.screen();
    let a = h
        .engine_mut()
        .create(screen, Box::new(AnimImg::from_gif(GIF).unwrap()))
        .unwrap();
    with(&mut h, a, |w: &mut AnimImg, cx| w.start(cx));
    h.update();
    assert_eq!(h.engine().coords(a).size(), Size::new(1, 1));
    assert_eq!(idx(&h, a), 0);
    let red = h.pixel(0, 0);
    assert!(red.r > 200 && red.b < 40, "{red:?}");
    h.advance(Duration::ms(150));
    assert_eq!(idx(&h, a), 0, "the first frame lasts 200 ms");
    h.advance(Duration::ms(70));
    assert_eq!(idx(&h, a), 1);
    let blue = h.pixel(0, 0);
    assert!(blue.b > 200 && blue.r < 40, "{blue:?}");
    assert!(playing(&h, a));
    let t = h.run_until_idle();
    assert!(
        t <= Duration::ms(60),
        "the second frame lasts 50 ms, then the GIF ends ({t:?})"
    );
    assert!(!playing(&h, a));
    h.assert_idle();
}

#[test]
fn snapshot_animimg_frames() {
    for (name, ms) in [("0", 0u64), ("3", 330)] {
        let (mut h, a) = scene(Mode::Light, 400, Repeat::Infinite);
        with(&mut h, a, |w: &mut AnimImg, cx| w.start(cx));
        h.update();
        if ms > 0 {
            h.advance(Duration::ms(ms));
        }
        assert_eq!(idx(&h, a).to_string(), name);
        h.assert_panel_snapshot(&format!("animimg_frame_{name}"));
    }
}
