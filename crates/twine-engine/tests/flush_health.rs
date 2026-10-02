//! Failed flushes and presents are retried, not dropped (R0.S02): the flush policy, the
//! display health and the driver's error code.

use twine_core::fault::{FaultKind, Faults};
use twine_core::{Color, ColorFormat, Instant, Opa, Rect};
use twine_engine::{
    BufferMode, DisplayId, DisplayState, DriverErrorCode, Engine, EngineConfig, EngineError, FlushPolicy,
    Wake,
};
use twine_hal::DisplayInfo;
use twine_style::{Selector, StyleProp};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, MemoryDisplayError, MockFramebufferDisplay, MockPresentError};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const W: u16 = 64;
const H: u16 = 32;
/// Rows per chunk: 4 chunks per full frame.
const ROWS: usize = 8;
const INJECTED: u32 = MemoryDisplayError::INJECTED_CODE;

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

fn ms(t: u64) -> Instant {
    Instant::from_millis(t)
}

fn paint(e: &mut Engine, d: DisplayId, c: Color) {
    let s = e.active_screen(d).unwrap();
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgColor(c.into()));
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
}

/// An engine with a red `MemoryDisplay` in single-buffer partial mode.
fn engine(policy: FlushPolicy, max_errors: u16) -> (Engine, DisplayId) {
    let cfg = EngineConfig {
        flush_policy: policy,
        max_consecutive_flush_errors: max_errors,
        ..EngineConfig::default()
    };
    let mut e = Engine::new(cfg).unwrap();
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let d = e
        .add_display(panel, BufferMode::partial_single(leak(usize::from(W) * 2 * ROWS)))
        .unwrap();
    paint(&mut e, d, Color::RED);
    (e, d)
}

fn panel(e: &mut Engine, d: DisplayId) -> &mut MemoryDisplay {
    e.driver_mut::<MemoryDisplay>(d).unwrap()
}

fn pixel(e: &Engine, d: DisplayId, x: u32, y: u32) -> Color {
    e.driver::<MemoryDisplay>(d).unwrap().pixel(x, y)
}

#[test]
fn regression_failed_flush_is_redrawn_on_the_next_frame() {
    let (mut e, d) = engine(FlushPolicy::Reinvalidate, 8);
    panel(&mut e, d).fail_flush_times(1);
    let wake = e.step(ms(0));
    // The first chunk (rows 0..8) failed, the other three reached the panel.
    assert_eq!(pixel(&e, d, 0, 0), Color::BLACK);
    assert_eq!(pixel(&e, d, 0, 20), Color::RED);
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushError));
    let r = *e.last_fault(FaultKind::FlushError).unwrap();
    assert_eq!((r.display, r.code), (Some(d), INJECTED));
    let h = e.display_health(d).unwrap();
    assert_eq!(h.state, DisplayState::Healthy, "later chunks succeeded");
    assert_eq!(h.last_error, Some(DriverErrorCode::new(INJECTED)));
    assert_eq!(h.last_ok, Some(ms(0)));
    // The retry is scheduled: the UI does not go to sleep with the area missing.
    assert_eq!(wake, Wake::At(ms(16)));
    let _ = panel(&mut e, d).take_flushes();

    let wake = e.step(ms(16));
    assert_eq!(pixel(&e, d, 0, 0), Color::RED);
    let flushes = panel(&mut e, d).take_flushes();
    assert_eq!(
        flushes.iter().map(|f| f.area).collect::<Vec<_>>(),
        [Rect::new(0, 0, i32::from(W), ROWS as i32)],
        "only the failed chunk is redrawn"
    );
    assert!(e.take_faults().is_empty());
    assert_eq!(wake, Wake::Idle);
}

#[test]
fn ignore_policy_drops_the_area_but_reports_it() {
    let (mut e, d) = engine(FlushPolicy::Ignore, 8);
    panel(&mut e, d).fail_flush_times(1);
    let _ = e.step(ms(0));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushError));
    assert_eq!(e.step(ms(16)), Wake::Idle);
    assert_eq!(pixel(&e, d, 0, 0), Color::BLACK, "not redrawn");
    assert_eq!(panel(&mut e, d).failed_flushes(), 1);
}

#[test]
fn health_goes_degraded_then_failed_then_recovers_on_success() {
    let (mut e, d) = engine(FlushPolicy::Reinvalidate, 6);
    let h = e.display_health(d).unwrap();
    assert_eq!(
        (h.state, h.consecutive_errors, h.last_ok),
        (DisplayState::Healthy, 0, None)
    );
    panel(&mut e, d).fail_flush_after(0);
    let _ = e.step(ms(0)); // 4 chunks fail
    let h = e.display_health(d).unwrap();
    assert_eq!((h.state, h.consecutive_errors), (DisplayState::Degraded, 4));
    let wake = e.step(ms(16)); // 4 more: 8 ≥ 6
    let h = e.display_health(d).unwrap();
    assert_eq!((h.state, h.consecutive_errors), (DisplayState::Failed, 8));
    assert_eq!(e.fault_counts().get(FaultKind::FlushError), 8);
    // `Reinvalidate` keeps retrying a failed display.
    assert_eq!(wake, Wake::At(ms(32)));
    panel(&mut e, d).stop_failing();
    let _ = e.step(ms(32));
    let h = e.display_health(d).unwrap();
    assert_eq!(
        (h.state, h.consecutive_errors, h.last_ok),
        (DisplayState::Healthy, 0, Some(ms(32)))
    );
    for (x, y) in [(0, 0), (63, 31), (10, 17)] {
        assert_eq!(pixel(&e, d, x, y), Color::RED);
    }
}

#[test]
fn halt_policy_stops_refreshing_until_recover_display() {
    let (mut e, d) = engine(FlushPolicy::Halt, 2);
    panel(&mut e, d).fail_flush_after(1);
    let wake = e.step(ms(0));
    // Chunk 1 ok, chunks 2 and 3 fail → halted; chunk 4 is never attempted.
    assert_eq!(panel(&mut e, d).failed_flushes(), 2);
    assert_eq!(panel(&mut e, d).take_flushes().len(), 1);
    let h = e.display_health(d).unwrap();
    assert_eq!((h.state, h.consecutive_errors), (DisplayState::Halted, 2));
    assert_eq!(wake, Wake::Idle, "a halted display does not keep the UI awake");
    // Changes accumulate but nothing is rendered or flushed.
    paint(&mut e, d, Color::GREEN);
    let _ = e.step(ms(100));
    let _ = e.step(ms(200));
    assert_eq!(panel(&mut e, d).failed_flushes(), 2);
    assert!(panel(&mut e, d).take_flushes().is_empty());
    // The application resets the panel and recovers the display: everything is redrawn.
    panel(&mut e, d).stop_failing();
    e.recover_display(d).unwrap();
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    let _ = e.step(ms(300));
    assert_eq!(pixel(&e, d, 0, 0), Color::GREEN);
    assert_eq!(pixel(&e, d, 63, 31), Color::GREEN);
    assert!(matches!(
        e.recover_display(bogus()),
        Err(EngineError::DisplayNotFound(_))
    ));
}

/// A display id that does not exist (the engine hands out ids in order).
fn bogus() -> DisplayId {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let mut last = None;
    for _ in 0..2 {
        let p = MemoryDisplay::new(DisplayInfo::new(8, 8, ColorFormat::L8));
        last = Some(e.add_display(p, BufferMode::partial_single(leak(64))).unwrap());
    }
    last.unwrap()
}

#[test]
fn failed_present_is_not_swapped_and_is_redrawn() {
    let info = DisplayInfo::new(W, H, ColorFormat::Rgb565);
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e
        .add_framebuffer_display(MockFramebufferDisplay::new(info, true, 0), BufferMode::full())
        .unwrap();
    paint(&mut e, d, Color::BLUE);
    e.driver_mut::<MockFramebufferDisplay>(d)
        .unwrap()
        .fail_present_times(1);
    let wake = e.step(ms(0));
    let fb = e.driver::<MockFramebufferDisplay>(d).unwrap();
    assert!(fb.presents().is_empty());
    let r = *e.last_fault(FaultKind::FlushError).unwrap();
    assert_eq!((r.display, r.code), (Some(d), MockPresentError::CODE));
    let h = e.display_health(d).unwrap();
    assert_eq!((h.state, h.consecutive_errors), (DisplayState::Degraded, 1));
    assert_eq!(wake, Wake::At(ms(16)));
    // The retry presents the same back buffer (no swap happened), fully drawn.
    let _ = e.step(ms(16));
    assert_eq!(e.driver::<MockFramebufferDisplay>(d).unwrap().presents(), [1]);
    let blue = Color::BLUE.to_rgb565().to_le_bytes();
    let back = e.framebuffer(d, 1).unwrap();
    assert!(
        back.chunks(2).all(|p| p == blue),
        "presented buffer holds the whole frame"
    );
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
}

#[test]
fn failed_direct_present_is_redrawn() {
    let info = DisplayInfo::new(W, H, ColorFormat::Rgb565);
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e
        .add_framebuffer_display(MockFramebufferDisplay::new(info, false, 0), BufferMode::direct())
        .unwrap();
    paint(&mut e, d, Color::BLUE);
    e.driver_mut::<MockFramebufferDisplay>(d)
        .unwrap()
        .fail_present_times(1);
    assert_eq!(e.step(ms(0)), Wake::At(ms(16)));
    assert_eq!(e.last_stats(d).dirty_areas, 1);
    let _ = e.step(ms(16));
    assert_eq!(e.driver::<MockFramebufferDisplay>(d).unwrap().presents(), [0]);
    assert_eq!(e.last_stats(d).dirty_px, u32::from(W) * u32::from(H));
    assert_eq!(e.display_health(d).unwrap().consecutive_errors, 0);
}

#[test]
fn driver_error_code_is_kept_in_engine_errors() {
    let err = EngineError::driver(DriverErrorCode::new(7), &MemoryDisplayError::Busy);
    assert_eq!(err.driver_code(), Some(DriverErrorCode::new(7)));
    assert_eq!(EngineError::TooManyDisplays.driver_code(), None);
    assert_eq!(err.to_string(), "driver error 0x00000007: Busy");
}

#[test]
fn failing_flushes_do_not_allocate() {
    let (mut e, d) = engine(FlushPolicy::Reinvalidate, 4);
    panel(&mut e, d).fail_flush_after(0);
    // Warm up: first frames grow the logs and queues once.
    let mut t = 0;
    for _ in 0..4 {
        let _ = e.step(ms(t));
        t += 16;
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..4 {
            let _ = e.step(ms(t));
            t += 16;
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Failed);
}
