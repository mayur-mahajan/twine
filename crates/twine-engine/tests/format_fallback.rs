//! The render-time fallback (R0.S06): drawing into a format whose renderer is not compiled in
//! (possible only outside the engine's displays, which are checked when added) is recorded
//! once by the renderer and raised once as `FaultKind::FormatDisabled` by the next
//! `take_faults`. A separate test binary: the record is process-wide.

use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::ColorFormat;
use twine_core::fault::FaultKind;
use twine_engine::{Engine, EngineConfig, FaultRecord};

static HOOK_CALLS: AtomicU32 = AtomicU32::new(0);

fn on_fault(r: &FaultRecord) {
    if r.kind == FaultKind::FormatDisabled {
        HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn render_fallback_raises_the_fault_once() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_fault_hook(Some(on_fault));
    assert!(e.take_faults().is_empty());
    // What the software renderer does when asked to draw into a disabled format.
    for _ in 0..3 {
        twine_render::report_format_disabled(ColorFormat::A4);
    }
    assert!(e.take_faults().contains(FaultKind::FormatDisabled));
    let r = e.last_fault(FaultKind::FormatDisabled).copied().unwrap();
    assert_eq!((r.display, r.code), (None, u32::from(ColorFormat::A4 as u8)));
    assert_eq!(HOOK_CALLS.load(Ordering::Relaxed), 1);
    // Reported once per process: more hits raise nothing.
    twine_render::report_format_disabled(ColorFormat::A4);
    assert!(e.take_faults().is_empty());
    assert_eq!(e.fault_counts().get(FaultKind::FormatDisabled), 1);
}
