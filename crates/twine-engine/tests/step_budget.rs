//! `Engine::step_budgeted` (R3.S04) on a full double framebuffer: the budget counts the
//! frame's dirty areas, the frame continues over the following steps (`Wake::Now`), and the
//! buffers swap once, after the last area.

use twine_core::{ColorFormat, Instant, Rect};
use twine_engine::{BufferMode, Engine, EngineConfig, InvalidateReason, StepBudget, Wake};
use twine_hal::DisplayInfo;
use twine_testing::MockFramebufferDisplay;

#[test]
fn framebuffer_frame_spread_over_steps_presents_once() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let info = DisplayInfo::new(64, 48, ColorFormat::Rgb565);
    let d = engine
        .add_framebuffer_display(MockFramebufferDisplay::new(info, true, 0), BufferMode::full())
        .unwrap();
    let presents = |e: &Engine| e.driver::<MockFramebufferDisplay>(d).unwrap().presents().len();
    let mut now = Instant::from_millis(0);
    // The first frame: one area (the whole screen), within a budget of one.
    assert_ne!(engine.step_budgeted(now, StepBudget::chunks(1)), Wake::Now);
    assert_eq!(presents(&engine), 1);

    // Three separate areas: three steps, one swap at the end.
    now += engine.config().refr_period;
    for y in [0, 20, 40] {
        engine.invalidate_area(d, Rect::new(0, y, 8, y + 4), InvalidateReason::Explicit);
    }
    assert_eq!(engine.step_budgeted(now, StepBudget::chunks(1)), Wake::Now);
    assert_eq!(engine.step_budgeted(now, StepBudget::chunks(1)), Wake::Now);
    assert_eq!(presents(&engine), 1, "no swap before the frame is complete");
    assert_ne!(engine.step_budgeted(now, StepBudget::chunks(1)), Wake::Now);
    assert_eq!(presents(&engine), 2, "one swap for the whole frame");
    assert_eq!(engine.last_stats(d).dirty_areas, 3);
}
