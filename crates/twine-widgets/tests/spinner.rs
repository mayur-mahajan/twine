//! `Spinner`: LVGL defaults and animation parameters, an endless and continuous animation,
//! not clickable, small per-frame invalidation, and snapshots at t = 0, 250 and 500 ms.

mod common;

use common::{Mode, get, harness, with};
use twine_core::{Angle, Duration};
use twine_engine::{NodeId, ObjFlags};
use twine_hal::BufferSpec;
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_widgets::spinner::{self, SPINNER_DEFAULT_PERIOD, SPINNER_DEFAULT_SWEEP, Spinner};

fn scene(mode: Mode, w: u16, h: u16, size: i32) -> (EngineHarness, NodeId) {
    let mut hh = harness(w, h, mode);
    let screen = hh.screen();
    let e = hh.engine_mut();
    let s = spinner::create(e, screen).unwrap();
    e.set_size(s, size, size);
    e.align(s, Align::Center, 0, 0);
    hh.update();
    (hh, s)
}

#[test]
fn spinner_defaults() {
    let (mut h, s) = scene(Mode::Light, 160, 160, 130);
    h.engine_mut()
        .set_size(s, twine_style::Length::Content, twine_style::Length::Content);
    let mut h2 = harness(160, 160, Mode::Light);
    let screen = h2.screen();
    let d = spinner::create(h2.engine_mut(), screen).unwrap();
    h2.update();
    assert_eq!(h2.engine().coords(d).size(), twine_core::Size::new(130, 130));
    let n = h.engine().tree().node(s).unwrap();
    assert_eq!(n.class().name, "spinner");
    let w = get::<Spinner>(&h, s);
    assert_eq!(
        (w.period(), w.sweep()),
        (SPINNER_DEFAULT_PERIOD, SPINNER_DEFAULT_SWEEP)
    );
    let arc = w.arc();
    assert_eq!(
        (arc.bg_angle_start(), arc.bg_angle_end()),
        (Angle::deg(0), Angle::deg(360))
    );
    assert_eq!(arc.rotation(), Angle::deg(270));
    assert!(h.engine().anim_count() >= 1);
}

#[test]
fn spinner_anim_params_same_value_no_restart() {
    let (mut h, s) = scene(Mode::Light, 160, 160, 60);
    h.advance(Duration::ms(100));
    let before = get::<Spinner>(&h, s).arc().angle_end();
    // The running animation's own invalidations of the last frame are still in the log.
    let logged = h.engine().invalidation_log().len();
    with(&mut h, s, |w: &mut Spinner, cx| {
        w.set_anim_params(cx, SPINNER_DEFAULT_PERIOD, SPINNER_DEFAULT_SWEEP);
        w.set_period(cx, SPINNER_DEFAULT_PERIOD);
        w.set_arc_sweep(cx, SPINNER_DEFAULT_SWEEP);
    });
    assert_eq!(h.engine().invalidation_log().len(), logged);
    assert_eq!(get::<Spinner>(&h, s).arc().angle_end(), before, "not restarted");
    with(&mut h, s, |w: &mut Spinner, cx| {
        w.set_period(cx, Duration::ms(2000));
    });
    assert_eq!(get::<Spinner>(&h, s).period(), Duration::ms(2000));
}

#[test]
fn spinner_angles_follow_lvgl() {
    let w = Spinner::new();
    // v = 0: start 0°, end = sweep.
    assert_eq!(w.angles_at(0), (Angle(0), Angle::deg(200)));
    // v = 1024: a full turn for both.
    assert_eq!(w.angles_at(1024), (Angle::deg(360), Angle::deg(560)));
    // The end is linear, the start follows LVGL's bezier; the arc never vanishes.
    let (_, e) = w.angles_at(256);
    assert_eq!(e, Angle::deg(290));
    for v in 0..=1024 {
        let (s, e) = w.angles_at(v);
        assert!(e.0 - s.0 > 0 && e.0 - s.0 <= 2000, "v {v}: {s} .. {e}");
    }
}

#[test]
fn spinner_anim_is_infinite_and_continuous() {
    let (mut h, s) = scene(Mode::Light, 160, 160, 60);
    let mut prev = get::<Spinner>(&h, s).arc().angle_end().0;
    let mut moved = 0;
    for _ in 0..(5000 / 16) {
        h.advance(Duration::ms(16));
        let end = get::<Spinner>(&h, s).arc().angle_end().0;
        let step = (end - prev).rem_euclid(3600);
        // Linear end: 360° per second = 5.76° per 16 ms frame.
        assert!(step <= 70, "jump of {step} (0.1°)");
        if step > 0 {
            moved += 1;
        }
        prev = end;
    }
    assert!(moved > 290, "{moved} frames moved");
    assert!(h.engine().anim_count() >= 1, "still running after 5 s");
}

#[test]
fn spinner_not_clickable() {
    let (mut h, s) = scene(Mode::Light, 160, 160, 60);
    assert!(!h.engine().has_flag(s, ObjFlags::CLICKABLE));
    let c = h.engine().coords(s);
    let d = h.display();
    assert_ne!(h.engine_mut().hit_test(d, c.center()), Some(s));
}

#[test]
fn spinner_frame_invalidation_small() {
    let mut h = harness(320, 240, Mode::Light).buffers(BufferSpec::PartialDouble { rows: 40 });
    let screen = h.screen();
    let e = h.engine_mut();
    let s = spinner::create(e, screen).unwrap();
    e.set_size(s, 60, 60);
    e.align(s, Align::Center, 0, 0);
    h.update();
    h.advance(Duration::ms(100));
    let mut max = 0;
    let mut total = 0u64;
    let frames = 60;
    let t0 = std::time::Instant::now();
    for _ in 0..frames {
        h.advance(Duration::ms(16));
        let px = h.last_frame().dirty_px;
        max = max.max(px);
        total += u64::from(px);
    }
    assert!(max < 60 * 60, "a frame redrew {max} px");
    eprintln!(
        "spinner 60x60: max {max} px/frame, mean {} px/frame, {} us/frame (wall, incl. harness)",
        total / frames,
        t0.elapsed().as_micros() / u128::from(frames)
    );
}

#[test]
fn snapshot_spinner() {
    for m in Mode::ALL {
        for t in [0u64, 250, 500] {
            let (mut h, _s) = scene(m, 100, 100, 60);
            if t > 0 {
                h.advance(Duration::ms(t));
            }
            h.assert_panel_snapshot(&format!("spinner_t{t}_{}", m.suffix()));
        }
    }
}
