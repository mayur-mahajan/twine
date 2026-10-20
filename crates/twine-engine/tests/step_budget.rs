//! `Engine::step_budgeted` (R3.S04) on a full double framebuffer: the budget counts the
//! frame's dirty areas, the frame continues over the following steps (`Wake::Now`), and the
//! buffers swap once, after the last area.
//!
//! req: SAF-012

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

// ---- R5.S05: time budget, `FrameOverrun`, high-water marks, `on_chunk` ----

use std::cell::Cell;
use twine_core::Duration;
use twine_core::fault::FaultKind;
use twine_engine::{BufferSpec, TimingReport};
use twine_testing::MemoryDisplay;

thread_local! {
    /// The fake high-resolution clock (µs) and how far each reading advances it.
    static CLOCK: Cell<u64> = const { Cell::new(0) };
    static TICK: Cell<u64> = const { Cell::new(100) };
    static CHUNKS: Cell<u32> = const { Cell::new(0) };
}

/// A timer that advances `TICK` µs per reading (a step's time grows with its work).
fn hires() -> Instant {
    CLOCK.with(|c| {
        c.set(c.get() + TICK.with(Cell::get));
        Instant::from_micros(c.get())
    })
}

fn count_chunk() {
    CHUNKS.with(|c| c.set(c.get() + 1));
}

/// A 64 × 32 display with 8-row buffers: a full frame is 4 chunks.
fn engine(config: EngineConfig) -> (Engine, twine_engine::DisplayId) {
    let mut e = Engine::new(config).unwrap();
    let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    let d = e
        .add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .unwrap();
    (e, d)
}

fn timed() -> EngineConfig {
    EngineConfig {
        hires_timer: Some(hires),
        ..EngineConfig::default()
    }
}

/// fault: `FrameOverrun`
/// req: SAF-031
#[test]
fn time_budget_defers_the_rest_of_the_frame_and_raises_frame_overrun() {
    // ~10 readings per chunk at 100 µs each: a 1 µs budget is spent after the first chunk.
    let (mut e, d) = engine(EngineConfig {
        step_budget: StepBudget::time(Duration::us(1)),
        ..timed()
    });
    let now = Instant::from_millis(0);
    let mut steps = 1;
    while e.step(now) == Wake::Now {
        steps += 1;
        assert!(steps < 20, "the frame must finish");
    }
    assert_eq!(steps, 4, "one chunk per step: deferred, never dropped");
    let r = e.last_fault(FaultKind::FrameOverrun).expect("overrun recorded");
    assert_eq!(r.display, Some(d), "the display whose frame continued");
    assert!(r.code > 0, "the step's time in µs");
    // The last step deferred nothing but still ended past its time: an overrun too.
    assert_eq!(e.fault_counts().get(FaultKind::FrameOverrun), 4);
    // The frame is complete: the same pixels as an unbudgeted frame.
    let (mut reference, _) = engine(timed());
    let _ = reference.step(now);
    let px = |e: &Engine| e.driver::<MemoryDisplay>(d).unwrap().framebuffer().to_vec();
    assert_eq!(px(&e), px(&reference));
}

#[test]
fn generous_time_budget_raises_nothing() {
    let (mut e, _) = engine(EngineConfig {
        step_budget: StepBudget::time(Duration::secs(10)),
        ..timed()
    });
    assert_ne!(e.step(Instant::from_millis(0)), Wake::Now);
    assert!(e.last_fault(FaultKind::FrameOverrun).is_none());
}

#[test]
fn per_call_budget_overrides_the_configured_one() {
    let (mut e, _) = engine(EngineConfig {
        step_budget: StepBudget::chunks(1),
        ..EngineConfig::default()
    });
    assert_ne!(
        e.step_budgeted(Instant::from_millis(0), StepBudget::UNLIMITED),
        Wake::Now,
        "an unlimited call renders the whole frame"
    );
    let (mut e, _) = engine(EngineConfig {
        step_budget: StepBudget::chunks(1),
        ..EngineConfig::default()
    });
    assert_eq!(
        e.step(Instant::from_millis(0)),
        Wake::Now,
        "the configured budget"
    );
}

#[test]
fn high_water_marks_track_the_worst_update_render_and_flush() {
    let (mut e, d) = engine(timed());
    assert_eq!(e.timing(), TimingReport::default());
    let _ = e.step(Instant::from_millis(0));
    let t = e.timing();
    assert!(t.worst_render_us > 0 && t.worst_flush_us > 0);
    assert!(
        t.worst_update_us > t.worst_render_us,
        "the update contains the renders"
    );
    // A slower clock makes the next frame's chunks slower: the marks rise.
    TICK.with(|k| k.set(1000));
    e.invalidate_area(d, Rect::new(0, 0, 64, 32), InvalidateReason::Explicit);
    let _ = e.step(Instant::from_millis(100));
    let t2 = e.timing();
    assert!(t2.worst_render_us > t.worst_render_us && t2.worst_update_us > t.worst_update_us);
    // A faster frame never lowers them.
    TICK.with(|k| k.set(1));
    e.invalidate_area(d, Rect::new(0, 0, 8, 8), InvalidateReason::Explicit);
    let _ = e.step(Instant::from_millis(200));
    assert_eq!(e.timing(), t2);
    e.reset_worst();
    assert_eq!(e.timing(), TimingReport::default());
    TICK.with(|k| k.set(100));
}

#[test]
fn without_a_timer_nothing_is_measured() {
    let (mut e, _) = engine(EngineConfig::default());
    let _ = e.step(Instant::from_millis(0));
    assert_eq!(e.timing(), TimingReport::default());
    // A per-call time limit cannot be measured: ignored, the frame renders in one step.
    assert_ne!(
        engine(EngineConfig::default())
            .0
            .step_budgeted(Instant::from_millis(0), StepBudget::time(Duration::us(1))),
        Wake::Now
    );
}

#[test]
fn on_chunk_runs_once_per_chunk() {
    CHUNKS.with(|c| c.set(0));
    let (mut e, _) = engine(EngineConfig {
        on_chunk: Some(count_chunk),
        ..EngineConfig::default()
    });
    let _ = e.step(Instant::from_millis(0));
    assert_eq!(CHUNKS.with(Cell::get), 4, "a 4-chunk frame");
    // Spread over steps: still once per chunk.
    CHUNKS.with(|c| c.set(0));
    let (mut e, _) = engine(EngineConfig {
        on_chunk: Some(count_chunk),
        step_budget: StepBudget::chunks(1),
        ..EngineConfig::default()
    });
    while e.step(Instant::from_millis(0)) == Wake::Now {}
    assert_eq!(CHUNKS.with(Cell::get), 4);
}

#[test]
fn on_chunk_runs_for_the_chunk_level_api() {
    CHUNKS.with(|c| c.set(0));
    let mut e = Engine::new(EngineConfig {
        on_chunk: Some(count_chunk),
        ..EngineConfig::default()
    })
    .unwrap();
    let info = DisplayInfo::new(64, 32, ColorFormat::Rgb565);
    let chunk = 64 * 2 * 8;
    e.add_chunked_display(info, chunk).unwrap();
    let _ = e.step(Instant::from_millis(0));
    assert!(e.refresh_begin(Instant::from_millis(0)).is_some());
    let mut buf = vec![0u8; chunk];
    let mut n = 0;
    while e.render_chunk(&mut buf).is_some() {
        n += 1;
    }
    e.refresh_end();
    assert_eq!(CHUNKS.with(Cell::get), n);
    assert_eq!(n, 4);
}
