//! Frame statistics, the performance monitor and overlay, and the refresh debug overlay.

mod common;

use std::cell::Cell;

use common::{boxed, white_screen};
use twine_core::{Color, Duration, Instant, Rect};
use twine_engine::{DrawCx, EngineConfig, InvalidateReason, NodeId, Widget, WidgetClass};
use twine_testing::EngineHarness;

thread_local! {
    static MOCK_US: Cell<u64> = const { Cell::new(0) };
}

fn mock_timer() -> Instant {
    Instant::from_micros(MOCK_US.with(Cell::get))
}

/// A widget whose drawing "takes" 4 ms of the mock high-resolution timer.
struct Slow;
static SLOW_CLASS: WidgetClass = WidgetClass::new("slow");
impl Widget for Slow {
    fn class(&self) -> &'static WidgetClass {
        &SLOW_CLASS
    }
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        MOCK_US.with(|t| t.set(t.get() + 4_000));
        cx.draw_base(twine_style::Part::Main);
    }
}

#[test]
fn stats_count_areas_px_chunks() {
    let mut h = EngineHarness::new(200, 100).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    let s = h.last_frame();
    assert_eq!(
        (s.frame, s.dirty_areas, s.dirty_px, s.chunks),
        (1, 1, 200 * 100, 3)
    );
    assert!(s.nodes_drawn >= 3);
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::from_xywh(0, 0, 10, 10), InvalidateReason::Explicit);
    h.engine_mut()
        .invalidate_area(d, Rect::from_xywh(100, 50, 20, 60), InvalidateReason::Explicit);
    h.advance(Duration::ms(16));
    let s = h.last_frame();
    assert_eq!((s.frame, s.dirty_areas, s.dirty_px), (2, 2, 100 + 20 * 50));
    // 10 rows + 50 rows (40 + 10).
    assert_eq!(s.chunks, 3);
    assert_eq!(s.render_us, 0, "no high-resolution timer configured");
}

#[test]
fn fps_and_cpu_computed_over_window() {
    MOCK_US.with(|t| t.set(0));
    let cfg = EngineConfig {
        hires_timer: Some(mock_timer),
        ..EngineConfig::default()
    };
    let mut slow: Option<NodeId> = None;
    let mut h = EngineHarness::new(64, 64)
        .no_theme()
        .config(cfg)
        .mount_engine(|e| {
            let s = white_screen(e);
            let n = e.create(s, Box::new(Slow)).unwrap();
            e.set_size(n, 10, 10);
            slow = Some(n);
        });
    let slow = slow.unwrap();
    for _ in 0..70 {
        h.engine_mut().invalidate(slow, InvalidateReason::Anim);
        h.update();
        let s = h.last_frame();
        assert!(s.render_us >= 4_000, "{s:?}");
        h.clock().advance(Duration::ms(16));
    }
    // The first window closes at the frame at 1008 ms: 64 frames of 4 ms each.
    let m = h.engine().perf_monitor();
    assert_eq!(m.fps(), 63);
    assert_eq!(m.cpu_percent(), 25);
    assert_eq!(h.last_frame().fps, 63);
    assert_eq!(h.last_frame().cpu_percent, 25);
}

#[test]
fn perf_overlay_excluded_from_dirty_px() {
    let cfg = EngineConfig {
        default_font: Some(&twine_assets::fonts::MONTSERRAT_14),
        ..EngineConfig::default()
    };
    let mut h = EngineHarness::new(200, 120)
        .no_theme()
        .config(cfg)
        .mount_engine(|e| {
            white_screen(e);
        });
    h.run_until_idle();
    let d = h.display();
    h.engine_mut().set_perf_overlay(d, true).unwrap();
    let overlay = h.engine().perf_overlay(d).unwrap();
    assert!(
        h.engine()
            .tree()
            .node(overlay)
            .unwrap()
            .widget()
            .text()
            .unwrap()
            .contains("FPS")
    );
    h.run_until_idle();
    let s = h.last_frame();
    assert_eq!(
        (s.dirty_areas, s.dirty_px),
        (0, 0),
        "overlay redraw counted: {s:?}"
    );
    assert!(!h.engine().coords(overlay).is_empty());
    // The overlay text updates once per second; its redraw stays out of the statistics.
    let before = h.last_frame().frame;
    h.advance(Duration::ms(1100));
    let s = h.last_frame();
    assert!(s.frame > before);
    assert_eq!(s.dirty_px, 0);
    h.engine_mut().set_perf_overlay(d, false).unwrap();
    assert!(h.engine().perf_overlay(d).is_none());
    assert!(!h.engine().tree().contains(overlay));
}

#[test]
fn debug_refresh_tints_only_dirty_area() {
    let mut b = None;
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        b = Some(boxed(e, s, Rect::from_xywh(10, 10, 20, 20), Color::BLUE));
    });
    h.run_until_idle();
    let clean = h.panel_rgb888();
    h.engine_mut().config_mut().debug_refresh = true;
    h.engine_mut().place(b.unwrap(), Rect::from_xywh(40, 20, 20, 20));
    h.clock().advance(Duration::ms(16));
    h.run_until_idle();
    let tinted = h.panel_rgb888();
    let dirty = [Rect::from_xywh(10, 10, 20, 20), Rect::from_xywh(40, 20, 20, 20)];
    for y in 0..60 {
        for x in 0..100 {
            let i = (y * 100 + x) * 3;
            let inside = dirty
                .iter()
                .any(|r| r.contains(twine_core::Point::new(x as i32, y as i32)));
            if !inside {
                assert_eq!(
                    tinted[i..i + 3],
                    clean[i..i + 3],
                    "pixel ({x}, {y}) changed outside the dirty areas"
                );
            }
        }
    }
    assert_ne!(h.pixel(15, 15), Color::WHITE, "old area tinted");
    h.assert_panel_snapshot("debug_refresh_tint");
}

#[test]
fn perf_overlay_uses_the_theme_font() {
    // No `default_font` configured: the display theme's font draws the overlay.
    let mut h = EngineHarness::new(200, 120);
    assert!(h.engine().config().default_font.is_none());
    h.run_until_idle();
    let d = h.display();
    h.engine_mut().set_perf_overlay(d, true).unwrap();
    h.run_until_idle();
    let overlay = h.engine().perf_overlay(d).unwrap();
    let c = h.engine().coords(overlay);
    assert!(
        c.width() > 20 && c.height() > 10,
        "overlay sized by the theme font: {c:?}"
    );
}
