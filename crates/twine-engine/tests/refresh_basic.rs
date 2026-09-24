//! First end-to-end frames: area rounding, chunking, refresh period, idle, buffer validation.

mod common;

use common::{boxed, white_screen};
use twine_core::{Color, ColorFormat, Duration, Rect};
use twine_engine::{BufferMode, Engine, EngineConfig, EngineError, InvalidateReason, Wake};
use twine_hal::{BufferSpec, DisplayInfo, DrawBufferMem};
use twine_testing::{EngineHarness, MemoryDisplay, leak_buffer};

/// A harness with a rendered white screen and nothing pending.
fn idle_harness(w: u16, h: u16) -> EngineHarness {
    let mut h = EngineHarness::new(w, h).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    h
}

#[test]
fn single_invalidation_produces_single_flush_of_that_area() {
    let mut h = idle_harness(320, 240);
    let area = Rect::from_xywh(13, 17, 29, 11);
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, area, InvalidateReason::Explicit);
    h.advance(Duration::ms(16));
    assert_eq!(h.flushes().len(), 1, "{:?}", h.flushes());
    assert_eq!(h.flushes()[0].area, area);
    assert_eq!(h.last_frame().dirty_px, 29 * 11);
}

#[test]
fn align_8_rounds_areas_and_chunks() {
    let mut h = EngineHarness::new(128, 64).no_theme().align(8).mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::new(3, 5, 21, 30), InvalidateReason::Explicit);
    h.advance(Duration::ms(16));
    assert!(!h.flushes().is_empty());
    for f in h.flushes() {
        let a = f.area;
        assert!(
            a.x0 % 8 == 0 && a.y0 % 8 == 0 && a.x1 % 8 == 0 && a.y1 % 8 == 0,
            "{a}"
        );
    }
    assert_eq!(h.flushes()[0].area, Rect::new(0, 0, 24, 32));
}

#[test]
fn chunks_respect_buffer_rows() {
    let mut h = idle_harness(320, 240);
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::new(0, 50, 320, 150), InvalidateReason::Explicit);
    h.advance(Duration::ms(16));
    let rows: Vec<i32> = h.flushes().iter().map(|f| f.area.height()).collect();
    assert_eq!(rows, [40, 40, 20]);
    assert_eq!(h.last_frame().chunks, 3);
}

#[test]
fn refresh_respects_refr_period() {
    let mut h = idle_harness(64, 64);
    let d = h.display();
    // The last frame was just rendered: a new invalidation waits for the period.
    h.engine_mut()
        .invalidate_area(d, Rect::new(0, 0, 8, 8), InvalidateReason::Explicit);
    let w = h.update();
    assert!(matches!(w, Wake::At(_)), "{w:?}");
    assert!(h.flushes().is_empty());
    h.clock().advance(Duration::ms(5));
    h.engine_mut()
        .invalidate_area(d, Rect::new(30, 30, 38, 38), InvalidateReason::Explicit);
    assert!(matches!(h.update(), Wake::At(_)));
    assert!(h.flushes().is_empty());
    h.clock().advance(Duration::ms(11));
    assert_eq!(h.update(), Wake::Idle);
    let frames: Vec<u32> = h.flushes().iter().map(|f| f.frame).collect();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0], frames[1], "both areas in one frame");
}

#[test]
fn idle_when_nothing_dirty() {
    let mut h = idle_harness(64, 64);
    for _ in 0..10 {
        h.clock().advance(Duration::ms(100));
        assert_eq!(h.update(), Wake::Idle);
        assert!(h.flushes().is_empty());
    }
}

fn engine() -> Engine {
    Engine::new(EngineConfig::default()).unwrap()
}

#[test]
fn buffer_too_small_is_rejected() {
    let info = DisplayInfo::new(100, 10, ColorFormat::Rgb565);
    let r = engine().add_display(
        MemoryDisplay::new(info),
        BufferMode::Partial {
            a: leak_buffer(199),
            b: None,
        },
    );
    assert_eq!(
        r,
        Err(EngineError::BufferTooSmall {
            needed: 200,
            got: 199
        })
    );
    // With alignment 8 at least 8 rows are needed.
    let r = engine().add_display(
        MemoryDisplay::new(info.with_align(8)),
        BufferMode::Partial {
            a: leak_buffer(200 * 4),
            b: None,
        },
    );
    assert!(matches!(r, Err(EngineError::BufferTooSmall { .. })));
}

#[test]
fn misaligned_buffer_is_rejected() {
    let info = DisplayInfo::new(10, 10, ColorFormat::Rgb565);
    let mem: &'static mut [u8] = Box::leak(vec![0u8; 20 * 10 + 8].into_boxed_slice());
    let off = mem.as_ptr().align_offset(4) + 1;
    let r = engine().add_display(
        MemoryDisplay::new(info),
        BufferMode::Partial {
            a: DrawBufferMem::new(&mut mem[off..off + 200]),
            b: None,
        },
    );
    assert_eq!(r, Err(EngineError::BufferMisaligned));
}

#[test]
fn refresh_basic_boxes() {
    let mut h = EngineHarness::new(96, 64)
        .no_theme()
        .buffers(BufferSpec::PartialSingle { rows: 10 })
        .mount_engine(|e| {
            let s = white_screen(e);
            boxed(e, s, Rect::from_xywh(8, 8, 24, 24), Color::RED);
            boxed(e, s, Rect::from_xywh(36, 16, 24, 24), Color::new(0, 160, 0));
            boxed(e, s, Rect::from_xywh(64, 24, 24, 24), Color::BLUE);
        });
    h.run_until_idle();
    assert_eq!(h.pixel(10, 10), Color::RED);
    assert_eq!(h.pixel(90, 60), Color::WHITE);
    h.assert_snapshot("refresh_basic_boxes");
}
