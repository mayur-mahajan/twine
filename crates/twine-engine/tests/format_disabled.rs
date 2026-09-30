//! A display in a colour format whose renderer is not compiled in is refused (R0.S06): the
//! `add_*display` call returns `EngineError::FormatDisabled`, raises `FaultKind::FormatDisabled`
//! and adds nothing, instead of registering a display that would stay blank.
//!
//! `A8` is never a draw format, so it is disabled whatever `color-*` features the workspace
//! unifies; the sweep over every format derives its expectation from
//! `twine_render::is_format_enabled`, so it holds under any feature set.

use twine_core::ColorFormat;
use twine_core::Rect;
use twine_core::fault::FaultKind;
use twine_engine::{BufferMode, Engine, EngineConfig, EngineError};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
use twine_testing::{MemoryDisplay, MockFramebufferDisplay};

const W: u16 = 32;
const H: u16 = 16;

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len.max(4)].into_boxed_slice())
}

/// A panel of any format that accepts every flush (`MemoryDisplay` only models draw formats).
struct NullPanel(DisplayInfo, Option<DrawBufferMem>);

impl DisplayDriver for NullPanel {
    type Error = core::convert::Infallible;
    fn info(&self) -> DisplayInfo {
        self.0
    }
    fn begin_flush(&mut self, _area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        self.1 = Some(buf);
        Ok(())
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.1.take()
    }
}

fn engine() -> Engine {
    Engine::new(EngineConfig::default()).unwrap()
}

/// The error, the fault and "nothing added" for a refused `format` (drawn in `draw`).
fn assert_refused(e: &mut Engine, r: &Result<twine_engine::DisplayId, EngineError>, draw: ColorFormat) {
    assert_eq!(r, &Err(EngineError::FormatDisabled(draw)));
    assert!(e.pending_faults().contains(FaultKind::FormatDisabled));
    let rec = e.last_fault(FaultKind::FormatDisabled).copied().unwrap();
    assert_eq!(rec.display, None, "the display was never added");
    assert_eq!(
        rec.code,
        u32::from(draw as u8),
        "code = the format's LVGL discriminant"
    );
    assert_eq!(e.fault_counts().get(FaultKind::FormatDisabled), 1);
    assert_eq!(e.displays().count(), 0);
    assert_eq!(e.default_display(), None);
}

#[test]
fn partial_display_in_a_disabled_format_is_an_error() {
    let mut e = engine();
    let info = DisplayInfo::new(W, H, ColorFormat::A8);
    let r = e.add_display(
        MemoryDisplay::new(info),
        BufferMode::partial_single(leak(info.bytes_per_row() * 8)),
    );
    assert_refused(&mut e, &r, ColorFormat::A8);
}

#[test]
fn framebuffer_display_in_a_disabled_format_is_an_error() {
    let mut e = engine();
    let info = DisplayInfo::new(W, H, ColorFormat::A8);
    let r = e.add_framebuffer_display(MockFramebufferDisplay::new(info, false, 0), BufferMode::direct());
    assert_refused(&mut e, &r, ColorFormat::A8);
}

#[test]
fn chunked_display_in_a_disabled_format_is_an_error() {
    let mut e = engine();
    let info = DisplayInfo::new(W, H, ColorFormat::A8);
    let r = e.add_chunked_display(info, info.bytes_per_row() * 8);
    assert_refused(&mut e, &r, ColorFormat::A8);
}

#[test]
fn a_refused_display_leaves_the_engine_usable() {
    let mut e = engine();
    let bad = DisplayInfo::new(W, H, ColorFormat::A8);
    assert!(e.add_chunked_display(bad, bad.bytes_per_row() * 8).is_err());
    let good = DisplayInfo::new(W, H, ColorFormat::Argb8888); // always compiled
    let d = e
        .add_display(
            MemoryDisplay::new(good),
            BufferMode::partial_single(leak(good.bytes_per_row() * 8)),
        )
        .unwrap();
    assert_eq!(e.default_display(), Some(d));
    assert_eq!(e.displays().count(), 1);
    let _ = e.step(twine_core::Instant::from_millis(1));
    assert_eq!(e.last_stats(d).chunks, 2, "the valid display renders");
}

/// Every format, through every registration path: refused exactly when the format the engine
/// draws in is not compiled in (`I1` behind partial/chunked buffers is drawn in `L8`).
#[test]
fn every_format_is_accepted_iff_its_renderer_is_compiled() {
    for f in ColorFormat::ALL {
        let info = DisplayInfo::new(W, H, f).with_align(if f.bpp() < 8 { 8 } else { 1 });
        let bytes = info.bytes_per_row() * usize::from(H);
        let chunked_draw = if f == ColorFormat::I1 { ColorFormat::L8 } else { f };

        let mut e = engine();
        let r = e.add_display(NullPanel(info, None), BufferMode::partial_single(leak(bytes)));
        check(&mut e, &r, chunked_draw, "partial");

        let mut e = engine();
        let r = e.add_chunked_display(info, bytes);
        check(&mut e, &r, chunked_draw, "chunked");

        let mut e = engine();
        let fb = MockFramebufferDisplay::new(info, false, 0);
        let r = e.add_framebuffer_display(fb, BufferMode::direct());
        check(&mut e, &r, f, "framebuffer");
    }
}

fn check(e: &mut Engine, r: &Result<twine_engine::DisplayId, EngineError>, draw: ColorFormat, path: &str) {
    if twine_render::is_format_enabled(draw) {
        assert!(
            !matches!(r, Err(EngineError::FormatDisabled(_))),
            "{draw} ({path}) is compiled in: {r:?}"
        );
        assert!(!e.pending_faults().contains(FaultKind::FormatDisabled));
    } else {
        assert_refused(e, r, draw);
    }
}
