//! No process-wide "format disabled" record (R3.S02, rework F6): a format whose renderer is
//! not compiled in is rejected where a draw target is created — `DrawBuf::new` for canvases
//! and custom buffers, `add_*display` for the engine's displays — and the fault is raised on
//! the engine that refused it, never on another one.

use twine_core::fault::FaultKind;
use twine_core::{ColorFormat, Rect};
use twine_engine::{BufferMode, Engine, EngineConfig, EngineError};
use twine_hal::DisplayInfo;
use twine_render::{DrawBuf, RenderError, is_draw_format, is_format_enabled};
use twine_testing::MemoryDisplay;

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len.max(4)].into_boxed_slice())
}

/// Two engines: the one adding a display in a disabled format gets the fault (with no display:
/// it was not added); the other one, rendering normally, never sees it.
#[test]
fn format_disabled_is_credited_to_the_engine_that_refused_the_display() {
    let mut a = Engine::new(EngineConfig::default()).unwrap();
    let mut b = Engine::new(EngineConfig::default()).unwrap();
    let good = DisplayInfo::new(32, 16, ColorFormat::Argb8888); // always compiled
    let d = b
        .add_display(
            MemoryDisplay::new(good),
            BufferMode::partial_single(leak(good.bytes_per_row() * 8)),
        )
        .unwrap();

    let bad = DisplayInfo::new(32, 16, ColorFormat::A8); // never a draw format
    let r = a.add_chunked_display(bad, bad.bytes_per_row() * 8);
    assert_eq!(r, Err(EngineError::FormatDisabled(ColorFormat::A8)));

    let _ = b.step(twine_core::Instant::from_millis(1));
    assert!(b.last_stats(d).chunks > 0, "b renders");
    assert!(
        !b.take_faults().contains(FaultKind::FormatDisabled),
        "not b's fault"
    );
    assert_eq!(b.fault_counts().get(FaultKind::FormatDisabled), 0);

    assert!(a.take_faults().contains(FaultKind::FormatDisabled));
    let rec = a.last_fault(FaultKind::FormatDisabled).copied().unwrap();
    assert_eq!((rec.display, rec.code), (None, u32::from(ColorFormat::A8 as u8)));
    assert!(a.take_faults().is_empty(), "raised once, when refused");
}

/// A draw target (a canvas, a custom renderer's buffer) cannot be created in a disabled draw
/// format: `RenderError::FormatDisabled`, distinct from formats that are never drawn into.
#[test]
fn draw_target_in_a_disabled_format_cannot_be_created() {
    let area = Rect::from_xywh(0, 0, 8, 2);
    for f in ColorFormat::ALL {
        let mut data = vec![0u8; 8 * 2 * 4];
        let r = DrawBuf::new_packed(&mut data, f, area).map(|b| b.format());
        if is_format_enabled(f) {
            assert_eq!(r, Ok(f), "{f} is compiled in");
        } else if is_draw_format(f) {
            assert_eq!(
                r,
                Err(RenderError::FormatDisabled(f)),
                "{f} is a disabled draw format"
            );
        } else {
            assert_eq!(
                r,
                Err(RenderError::UnsupportedFormat(f)),
                "{f} is never drawn into"
            );
        }
    }
}
