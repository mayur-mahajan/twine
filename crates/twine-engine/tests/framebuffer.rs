//! Framebuffer displays: `Full` (double buffer with area sync) and `Direct`.

mod common;

use common::{boxed, white_screen};
use twine_core::{Color, ColorFormat, Duration, Rect};
use twine_engine::{BufferMode, Engine, EngineConfig, EngineError, InvalidateReason, NodeId, Wake};
use twine_hal::DisplayInfo;
use twine_testing::{EngineHarness, FbMode, MockFramebufferDisplay};

const W: u16 = 120;
const H: u16 = 80;

fn scene(e: &mut Engine) -> NodeId {
    let s = white_screen(e);
    boxed(e, s, Rect::from_xywh(5, 5, 30, 20), Color::hex(0x33_66_99));
    boxed(e, s, Rect::from_xywh(0, 0, 16, 16), Color::RED)
}

/// A fresh partial-mode render of the scene with the moving box at `r`.
fn reference(r: Rect) -> Vec<u8> {
    let mut h = EngineHarness::new(W, H).no_theme().mount_engine(|e| {
        let b = scene(e);
        e.set_pos(b, r.x0, r.y0);
    });
    h.run_until_idle();
    h.panel_rgb888()
}

#[test]
fn full_mode_back_buffer_consistent_after_sync() {
    let mut b = None;
    let mut h = EngineHarness::new(W, H)
        .no_theme()
        .framebuffer(FbMode::Full, 0)
        .mount_engine(|e| b = Some(scene(e)));
    h.run_until_idle();
    let b = b.unwrap();
    for (i, pos) in [(10, 10), (40, 12), (41, 50), (90, 60), (0, 0), (104, 64)]
        .iter()
        .enumerate()
    {
        let r = Rect::from_xywh(pos.0, pos.1, 16, 16);
        h.engine_mut().place(b, r);
        h.clock().advance(Duration::ms(16));
        h.run_until_idle();
        assert_eq!(
            h.panel_rgb888(),
            reference(r),
            "frame {i}: presented buffer differs"
        );
    }
    let presents = h.framebuffer_display().unwrap().presents().to_vec();
    assert!(
        presents.windows(2).all(|w| w[0] != w[1]),
        "buffers alternate: {presents:?}"
    );
}

#[test]
fn full_mode_waits_for_present_done() {
    let mut h = EngineHarness::new(W, H)
        .no_theme()
        .framebuffer(FbMode::Full, 3)
        .mount_engine(|e| {
            white_screen(e);
        });
    assert_eq!(h.update(), Wake::Idle);
    assert_eq!(h.framebuffer_display().unwrap().presents().len(), 1);
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::from_xywh(0, 0, 10, 10), InvalidateReason::Explicit);
    h.clock().advance(Duration::ms(16));
    let now = h.now();
    // The swap has not happened yet: poll again in 1 ms, do not render.
    assert_eq!(h.update(), Wake::At(now + Duration::ms(1)));
    assert_eq!(h.framebuffer_display().unwrap().presents().len(), 1);
    h.run_until_idle();
    assert_eq!(h.framebuffer_display().unwrap().presents().len(), 2);
    assert!(!h.engine().flush_pending() || h.update() == Wake::Idle);
}

#[test]
fn area_sync_copies_only_uncovered_parts() {
    let mut h = EngineHarness::new(W, H)
        .no_theme()
        .framebuffer(FbMode::Full, 0)
        .mount_engine(|e| {
            white_screen(e);
        });
    h.run_until_idle();
    let d = h.display();
    let a = Rect::from_xywh(0, 0, 50, 50);
    h.engine_mut().invalidate_area(d, a, InvalidateReason::Explicit);
    h.clock().advance(Duration::ms(16));
    h.run_until_idle();
    // Previous frame = whole screen, current = A.
    assert_eq!(h.engine().last_sync_px(d), u64::from(W) * u64::from(H) - 2500);
    let b = Rect::from_xywh(25, 25, 50, 50);
    h.engine_mut().invalidate_area(d, b, InvalidateReason::Explicit);
    h.clock().advance(Duration::ms(16));
    h.run_until_idle();
    // prev (A) minus the overlap with B.
    assert_eq!(h.engine().last_sync_px(d), 2500 - 25 * 25);
}

#[test]
fn direct_mode_renders_in_place() {
    let mut b = None;
    let mut h = EngineHarness::new(W, H)
        .no_theme()
        .framebuffer(FbMode::Direct, 0)
        .mount_engine(|e| b = Some(scene(e)));
    h.run_until_idle();
    let r = Rect::from_xywh(60, 30, 16, 16);
    h.engine_mut().place(b.unwrap(), r);
    h.clock().advance(Duration::ms(16));
    h.run_until_idle();
    assert_eq!(h.panel_rgb888(), reference(r));
    assert!(
        h.framebuffer_display()
            .unwrap()
            .presents()
            .iter()
            .all(|p| *p == 0)
    );
    assert!(h.engine().framebuffer(h.display(), 1).is_none());
}

#[test]
fn full_mode_rejects_single_buffer() {
    let info = DisplayInfo::new(W, H, ColorFormat::Rgb565);
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(
        e.add_framebuffer_display(MockFramebufferDisplay::new(info, false, 0), BufferMode::full()),
        Err(EngineError::BufferModeMismatch)
    );
    assert!(matches!(
        e.add_framebuffer_display(
            MockFramebufferDisplay::new(info, true, 0).with_buffer_len(100),
            BufferMode::full()
        ),
        Err(EngineError::BufferTooSmall { .. })
    ));
    // Direct needs only one.
    assert!(
        e.add_framebuffer_display(MockFramebufferDisplay::new(info, false, 0), BufferMode::direct())
            .is_ok()
    );
}
