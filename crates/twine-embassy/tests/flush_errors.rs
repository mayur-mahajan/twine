//! The async runtime reports every flush to the engine (R0.S02): a failed flush raises
//! `FlushError` with the driver's error code, its area is redrawn by the next frame, the run
//! loop wakes up for that frame, and the display health follows.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use twine_core::{ColorFormat, Rect};
use twine_engine::EngineConfig;
use twine_hal::{AsyncDisplayDriver, DisplayInfo};
use twine_testing::MockPlatform;
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 48;
const ROWS: usize = 8;
const CODE: u32 = 0x5A;

type Fb = Rc<RefCell<Vec<u8>>>;

/// Shared test state: the panel's memory and the failures to inject.
#[derive(Clone, Default)]
struct Panel {
    fb: Fb,
    /// Flushes still to fail (`u32::MAX`: fail forever).
    fail: Rc<Cell<u32>>,
    flushes: Rc<Cell<u32>>,
}

impl Panel {
    fn new() -> Self {
        Panel {
            fb: Rc::new(RefCell::new(vec![0; usize::from(W) * usize::from(H) * 2])),
            ..Panel::default()
        }
    }
}

#[derive(Debug)]
struct BusError(u32);

/// An async display that completes each flush on its first poll, failing on request.
struct FlakyAsync(Panel);

impl AsyncDisplayDriver for FlakyAsync {
    type Error = BusError;
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(W, H, ColorFormat::Rgb565)
    }
    async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), BusError> {
        std::future::ready(()).await; // a transfer that completes at once
        let p = &self.0;
        p.flushes.set(p.flushes.get() + 1);
        match p.fail.get() {
            0 => {}
            u32::MAX => return Err(BusError(CODE)),
            n => {
                p.fail.set(n - 1);
                return Err(BusError(CODE));
            }
        }
        let w = area.width() as usize;
        for (row, y) in (area.y0..area.y1).enumerate() {
            let dst = (y as usize * usize::from(W) + area.x0 as usize) * 2;
            p.fb.borrow_mut()[dst..dst + w * 2].copy_from_slice(&buf[row * w * 2..(row + 1) * w * 2]);
        }
        Ok(())
    }
    fn error_code(&self, error: &BusError) -> u32 {
        error.0
    }
}

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

fn app(_cx: Scope) -> impl View {
    column((label("Twine"), button(label("Retry"))))
        .gap(4)
        .padding(4)
        .bg(Color::hex(0x1E_88_E5))
        .size(Length::Pct(100), Length::Pct(100))
}

fn ui(panel: &Panel, platform: &MockPlatform, double: bool, cfg: EngineConfig) -> AsyncUi<FlakyAsync> {
    let len = usize::from(W) * 2 * ROWS;
    let bufs = if double {
        BufferMode::partial_double(leak(len), leak(len))
    } else {
        BufferMode::partial_single(leak(len))
    };
    Ui::builder_async(FlakyAsync(panel.clone()))
        .runtime(Runtime::current_thread())
        .buffers(bufs)
        .config(cfg)
        .platform(platform)
        .build(app)
}

/// Polls a future to completion (the mock never returns `Pending`).
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// The frame of a UI whose flushes never fail.
fn reference() -> Vec<u8> {
    let panel = Panel::new();
    let mut u = ui(&panel, &MockPlatform::new(), true, EngineConfig::default());
    let _ = block_on(u.update_async());
    panel.fb.borrow().clone()
}

fn failed_flush_is_retried(double: bool) {
    let panel = Panel::new();
    let clock = MockPlatform::new();
    let mut u = ui(&panel, &clock, double, EngineConfig::default());
    panel.fail.set(1);
    let wake = block_on(u.update_async());
    assert_eq!(u.take_faults(), Faults::from(FaultKind::FlushError));
    let r = *u.last_fault(FaultKind::FlushError).unwrap();
    assert_eq!(r.code, CODE);
    let h = u.display_health().unwrap();
    assert_eq!(h.last_error.map(twine_engine::DriverErrorCode::get), Some(CODE));
    assert_eq!(h.state, DisplayState::Healthy, "the later chunks succeeded");
    let row = usize::from(W) * 2;
    assert!(
        panel.fb.borrow()[..row * ROWS].iter().all(|b| *b == 0),
        "the failed chunk did not reach the panel"
    );
    // The run loop is told to come back for the retry.
    let due = Instant::from_millis(16);
    assert!(matches!(wake, Wake::At(t) if t <= due), "{wake:?}");
    clock.set(due);
    let flushes = panel.flushes.get();
    let _ = block_on(u.update_async());
    assert_eq!(
        panel.flushes.get() - flushes,
        1,
        "only the failed chunk is redrawn"
    );
    assert_eq!(*panel.fb.borrow(), reference(), "the retried frame is complete");
    assert!(u.take_faults().is_empty());
}

#[test]
fn regression_failed_async_flush_is_redrawn_single_buffer() {
    failed_flush_is_retried(false);
}

#[test]
fn regression_failed_async_flush_is_redrawn_double_buffer() {
    failed_flush_is_retried(true);
}

#[test]
fn async_display_halts_and_recovers() {
    let panel = Panel::new();
    let clock = MockPlatform::new();
    let cfg = EngineConfig {
        flush_policy: FlushPolicy::Halt,
        max_consecutive_flush_errors: 2,
        ..EngineConfig::default()
    };
    let mut u = ui(&panel, &clock, false, cfg);
    panel.fail.set(u32::MAX);
    let _ = block_on(u.update_async());
    assert_eq!(
        panel.flushes.get(),
        2,
        "no chunk is rendered once the display halted"
    );
    let h = u.display_health().unwrap();
    assert_eq!((h.state, h.consecutive_errors), (DisplayState::Halted, 2));
    clock.set(Instant::from_millis(100));
    let _ = block_on(u.update_async());
    assert_eq!(panel.flushes.get(), 2, "a halted display is not refreshed");
    // The application resets the panel, then recovers the display.
    panel.fail.set(0);
    u.recover_display();
    clock.set(Instant::from_millis(200));
    let _ = block_on(u.update_async());
    assert_eq!(u.display_health().unwrap().state, DisplayState::Healthy);
    assert_eq!(*panel.fb.borrow(), reference());
}
