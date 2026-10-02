//! Bounded waits (R0.S03): a display driver that never hands a buffer back (hung DMA) cannot
//! hang the step. `EngineConfig::flush_timeout` raises `FlushTimeout`, marks the display
//! failed and the step returns; a draw accelerator's timeouts become `AccelTimeout` faults.

use std::cell::Cell;

use twine_core::fault::{FaultKind, Faults};
use twine_core::{Color, ColorFormat, Duration, Instant, Opa, Rect};
use twine_engine::{BufferMode, DisplayId, DisplayState, Engine, EngineConfig, FlushPolicy, Wake};
use twine_hal::DisplayInfo;
use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};
use twine_style::{Selector, StyleProp};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, MockFramebufferDisplay};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const W: u16 = 64;
const H: u16 = 32;
/// Rows per chunk: 4 chunks per full frame.
const ROWS: usize = 8;
const TIMEOUT: Duration = Duration::ms(50);

thread_local! {
    /// The mock high-resolution clock (µs); every read advances it by 1 ms.
    static CLOCK: Cell<u64> = const { Cell::new(0) };
}

/// A high-resolution timer that advances 1 ms per read (per test thread).
fn ticking_timer() -> Instant {
    CLOCK.with(|c| {
        let t = c.get();
        c.set(t + 1000);
        Instant::from_micros(t)
    })
}

fn clock_us() -> u64 {
    CLOCK.with(Cell::get)
}

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

/// An engine with a red, hung `MemoryDisplay` (single partial buffer, 4 chunks per frame).
fn engine(cfg: EngineConfig) -> (Engine, DisplayId) {
    let mut e = Engine::new(cfg).unwrap();
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).never_complete();
    let d = e
        .add_display(panel, BufferMode::partial_single(leak(usize::from(W) * 2 * ROWS)))
        .unwrap();
    paint(&mut e, d, Color::RED);
    (e, d)
}

fn hires(policy: FlushPolicy) -> EngineConfig {
    EngineConfig {
        flush_timeout: Some(TIMEOUT),
        hires_timer: Some(ticking_timer),
        flush_policy: policy,
        ..EngineConfig::default()
    }
}

fn panel(e: &mut Engine, d: DisplayId) -> &mut MemoryDisplay {
    e.driver_mut::<MemoryDisplay>(d).unwrap()
}

fn pixel(e: &Engine, d: DisplayId, x: u32, y: u32) -> Color {
    e.driver::<MemoryDisplay>(d).unwrap().pixel(x, y)
}

#[test]
fn regression_hung_flush_does_not_hang_the_step() {
    let (mut e, d) = engine(hires(FlushPolicy::Reinvalidate));
    let t0 = clock_us();
    let wake = e.step(ms(0));
    // The step spun (reading the timer) for about the timeout, not forever.
    let waited = clock_us() - t0;
    assert!(
        (TIMEOUT.as_micros()..TIMEOUT.as_micros() + 20_000).contains(&waited),
        "waited {waited} µs"
    );
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushTimeout));
    let r = *e.last_fault(FaultKind::FlushTimeout).unwrap();
    assert_eq!(r.display, Some(d));
    assert!(
        u64::from(r.code) >= TIMEOUT.as_millis(),
        "code = ms waited: {}",
        r.code
    );
    let h = e.display_health(d).unwrap();
    assert_eq!(h.state, DisplayState::Failed);
    assert_eq!(h.consecutive_errors, 8, "raised to max_consecutive_flush_errors");
    // Only the first chunk reached the panel; the step asks to retry after a period.
    assert_eq!(panel(&mut e, d).flushes().len(), 1);
    assert_eq!(wake, Wake::At(ms(16)));
    // While the driver keeps the buffer: no frame, no second fault, no busy loop.
    let t1 = clock_us();
    assert_eq!(e.step(ms(16)), Wake::At(ms(32)));
    assert!(
        clock_us() - t1 < TIMEOUT.as_micros(),
        "a known hung driver is not waited for"
    );
    assert!(e.take_faults().is_empty());
    assert_eq!(e.fault_counts().get(FaultKind::FlushTimeout), 1);
    assert_eq!(panel(&mut e, d).flushes().len(), 1);
}

#[test]
fn hung_display_recovers_when_the_driver_completes_again() {
    let (mut e, d) = engine(hires(FlushPolicy::Reinvalidate));
    let _ = e.step(ms(0));
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Failed);
    panel(&mut e, d).complete_again();
    let wake = e.step(ms(16));
    // The abandoned chunks and the one in flight are redrawn; a successful flush heals.
    for (x, y) in [(0, 0), (0, 8), (63, 31)] {
        assert_eq!(pixel(&e, d, x, y), Color::RED);
    }
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    assert_eq!(wake, Wake::Idle);
    assert_eq!(e.fault_counts().get(FaultKind::FlushTimeout), 1);
}

#[test]
fn ignore_policy_does_not_redraw_the_chunk_in_flight() {
    let (mut e, d) = engine(hires(FlushPolicy::Ignore));
    let _ = e.step(ms(0));
    panel(&mut e, d).complete_again();
    let _ = panel(&mut e, d).take_flushes();
    let _ = e.step(ms(16));
    let areas: Vec<Rect> = panel(&mut e, d).take_flushes().iter().map(|f| f.area).collect();
    // The unrendered rest of the frame is drawn; the chunk in flight is not redrawn.
    assert_eq!(areas.first().map(|a| a.y0), Some(ROWS as i32), "{areas:?}");
}

#[test]
fn halt_policy_halts_and_recover_display_restarts_the_timeout() {
    let (mut e, d) = engine(hires(FlushPolicy::Halt));
    assert_eq!(
        e.step(ms(0)),
        Wake::Idle,
        "a halted display does not keep the UI awake"
    );
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Halted);
    let _ = e.take_faults();
    // Recovering while the driver is still hung: the timeout is measured again.
    e.recover_display(d).unwrap();
    let _ = e.step(ms(100));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushTimeout));
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Halted);
    // Recovering once the driver works again redraws everything.
    panel(&mut e, d).complete_again();
    e.recover_display(d).unwrap();
    let _ = e.step(ms(200));
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    assert_eq!(pixel(&e, d, 63, 31), Color::RED);
}

#[test]
fn without_hires_timer_the_wait_yields_and_is_measured_across_steps() {
    let cfg = EngineConfig {
        flush_timeout: Some(TIMEOUT),
        ..EngineConfig::default()
    };
    let (mut e, d) = engine(cfg);
    // No time source inside a step: the step does not spin, it asks to be called again.
    assert_eq!(e.step(ms(0)), Wake::Now);
    assert_eq!(e.step(ms(10)), Wake::Now);
    assert_eq!(e.step(ms(49)), Wake::Now);
    assert!(e.take_faults().is_empty());
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    // 50 ms after the flush began (at 0 ms): timed out.
    let wake = e.step(ms(50));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushTimeout));
    assert_eq!(e.last_fault(FaultKind::FlushTimeout).unwrap().code, 50);
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Failed);
    assert_eq!(wake, Wake::At(ms(66)), "retry one period later");
    assert_eq!(e.step(ms(60)), Wake::At(ms(76)));
}

#[test]
fn cooperative_flush_yields_and_stops_asking_once_timed_out() {
    let cfg = EngineConfig {
        cooperative_flush: true,
        ..hires(FlushPolicy::Reinvalidate)
    };
    let (mut e, d) = engine(cfg);
    let mut steps = 0;
    let mut t = 0;
    let wake = loop {
        let w = e.step(ms(t));
        steps += 1;
        if w != Wake::Now {
            break w;
        }
        t += 1;
        assert!(steps < 1000, "cooperative waiting never timed out");
    };
    assert!(steps > 1, "the cooperative step yielded");
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushTimeout));
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Failed);
    assert!(matches!(wake, Wake::At(_)), "{wake:?}");
}

#[test]
fn cooperative_pending_flush_after_the_frame_times_out_too() {
    // The last chunk's buffer never comes back after a complete frame: the cooperative
    // `Wake::Now` for the pending flush stops once the flush timed out.
    let cfg = EngineConfig {
        cooperative_flush: true,
        flush_timeout: Some(TIMEOUT),
        ..EngineConfig::default()
    };
    let mut e = Engine::new(cfg).unwrap();
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).never_complete();
    // One chunk per frame.
    let d = e
        .add_display(
            panel,
            BufferMode::partial_single(leak(usize::from(W) * 2 * usize::from(H))),
        )
        .unwrap();
    paint(&mut e, d, Color::RED);
    assert_eq!(e.step(ms(0)), Wake::Now, "waiting for the flush");
    assert_eq!(e.step(ms(20)), Wake::Now);
    // Timed out: no more `Wake::Now`; the chunk in flight is redrawn one period later.
    assert_eq!(e.step(ms(60)), Wake::At(ms(76)));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushTimeout));
    assert!(e.flush_pending(), "the driver still holds the buffer");
}

#[test]
fn hung_framebuffer_swap_times_out() {
    let info = DisplayInfo::new(W, H, ColorFormat::Rgb565);
    let cfg = EngineConfig {
        flush_timeout: Some(TIMEOUT),
        ..EngineConfig::default()
    };
    let mut e = Engine::new(cfg).unwrap();
    let d = e
        .add_framebuffer_display(
            MockFramebufferDisplay::new(info, true, u32::MAX),
            BufferMode::full(),
        )
        .unwrap();
    paint(&mut e, d, Color::BLUE);
    let _ = e.step(ms(0));
    paint(&mut e, d, Color::GREEN);
    assert_eq!(e.step(ms(20)), Wake::At(ms(21)), "waiting for the swap");
    assert!(e.take_faults().is_empty());
    let wake = e.step(ms(60));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushTimeout));
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Failed);
    assert_eq!(wake, Wake::At(ms(76)));
    assert_eq!(e.driver::<MockFramebufferDisplay>(d).unwrap().presents(), [1]);
}

#[test]
fn zero_flush_timeout_is_rejected() {
    let cfg = EngineConfig {
        flush_timeout: Some(Duration::ms(0)),
        ..EngineConfig::default()
    };
    assert!(Engine::new(cfg).is_err());
}

#[test]
fn hung_display_steps_do_not_allocate() {
    let (mut e, _d) = engine(hires(FlushPolicy::Reinvalidate));
    let mut t = 0;
    for _ in 0..3 {
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
}

/// An accelerator that declines everything and reports one timeout per chunk.
struct TimingOut;

impl DrawAccel for TimingOut {
    fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult {
        AccelResult::Unsupported
    }
    fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult {
        AccelResult::Unsupported
    }
    fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult {
        AccelResult::Unsupported
    }
    fn wait(&mut self) {}
    fn take_timeouts(&mut self) -> u32 {
        1
    }
}

#[test]
fn accelerator_timeouts_are_raised_and_drawn_in_software() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_accel(TimingOut);
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let d = e
        .add_display(panel, BufferMode::partial_single(leak(usize::from(W) * 2 * ROWS)))
        .unwrap();
    paint(&mut e, d, Color::RED);
    let _ = e.step(ms(0));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::AccelTimeout));
    let r = *e.last_fault(FaultKind::AccelTimeout).unwrap();
    assert_eq!((r.display, r.occurrences), (Some(d), 1));
    assert_eq!(e.fault_counts().get(FaultKind::AccelTimeout), 4, "one per chunk");
    assert_eq!(pixel(&e, d, 63, 31), Color::RED);
}

fn host_timer() -> Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    Instant::from_micros(start.elapsed().as_micros() as u64)
}

#[test]
fn step_returns_within_the_timeout_in_wall_time() {
    let cfg = EngineConfig {
        flush_timeout: Some(Duration::ms(20)),
        hires_timer: Some(host_timer),
        ..EngineConfig::default()
    };
    let (mut e, d) = engine(cfg);
    let t0 = std::time::Instant::now();
    let _ = e.step(ms(0));
    let took = t0.elapsed();
    assert!(took >= std::time::Duration::from_millis(20), "{took:?}");
    assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    assert_eq!(e.display_health(d).unwrap().state, DisplayState::Failed);
}
