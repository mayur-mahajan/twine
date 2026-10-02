//! `Dma2d::with_timeout` inside the engine: a hung DMA2D neither hangs rendering nor leaves
//! wrong pixels on the panel (R0.S03).

use twine_accel_stm32::Dma2d;
use twine_accel_stm32::mock::MockRegs;
use twine_core::fault::FaultKind;
use twine_core::{Color, ColorFormat, Instant, Opa};
use twine_engine::{BufferMode, Engine, EngineConfig};
use twine_hal::DisplayInfo;
use twine_style::{Selector, StyleProp};
use twine_testing::MemoryDisplay;

const W: u16 = 64;
const H: u16 = 32;
// Chunks of 16 rows (1024 px, the painter's `ACCEL_MIN_PX`): two per frame, both offered to
// the accelerator.

fn engine(regs: MockRegs) -> (Engine, twine_engine::DisplayId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_accel(Some(Box::new(Dma2d::new(regs).with_timeout(1_000))));
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let buf: &'static mut [u8] = Box::leak(vec![0u8; usize::from(W) * 2 * 16].into_boxed_slice());
    let d = e.add_display(panel, BufferMode::partial_single(buf)).unwrap();
    let s = e.active_screen(d).unwrap();
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    (e, d)
}

fn all_red(e: &Engine, d: twine_engine::DisplayId) -> bool {
    let p = e.driver::<MemoryDisplay>(d).unwrap();
    (0..u32::from(H)).all(|y| (0..u32::from(W)).all(|x| p.pixel(x, y) == Color::RED))
}

#[test]
fn working_mock_moves_no_pixels() {
    // Baseline: the mock DMA2D writes nothing, so correct pixels below can only come from
    // the software re-render.
    let (mut e, d) = engine(MockRegs::new());
    let _ = e.step(Instant::from_millis(0));
    assert!(!all_red(&e, d));
    assert_eq!(e.fault_counts().get(FaultKind::AccelTimeout), 0);
}

#[test]
fn regression_hung_dma2d_falls_back_to_software_rendering() {
    let (mut e, d) = engine(MockRegs::new().hang());
    let _ = e.step(Instant::from_millis(0));
    // The queued background fill of the first chunk hung: the wait gave up, the chunk was
    // rendered again in software, and the accelerator draws nothing from then on.
    assert!(all_red(&e, d), "every chunk on the panel is correct");
    assert_eq!(
        e.take_faults().iter().collect::<Vec<_>>(),
        [FaultKind::AccelTimeout]
    );
    let r = *e.last_fault(FaultKind::AccelTimeout).unwrap();
    assert_eq!((r.display, r.occurrences), (Some(d), 1));
}
