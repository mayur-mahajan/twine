//! The async runtime bounds every flush with `EngineConfig::flush_timeout` on its platform's
//! timer (R3.S01, rework F5): a flush that never completes is dropped, `FlushTimeout` is raised
//! and the display health follows exactly as on the blocking path, and the display recovers
//! with the next successful flush.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake as TaskWake, Waker};

use twine_core::{ColorFormat, Rect};
use twine_engine::EngineConfig;
use twine_hal::{AsyncDisplayDriver, DisplayInfo};
use twine_testing::MockPlatform;
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 48;
const ROWS: usize = 8;

/// The panel's memory, whether its transfers hang, and how many started.
#[derive(Clone)]
struct Panel {
    fb: Rc<RefCell<Vec<u8>>>,
    hang: Rc<Cell<bool>>,
    started: Rc<Cell<u32>>,
}

impl Panel {
    fn new() -> Self {
        Panel {
            fb: Rc::new(RefCell::new(vec![0; usize::from(W) * usize::from(H) * 2])),
            hang: Rc::new(Cell::new(false)),
            started: Rc::new(Cell::new(0)),
        }
    }
}

/// An async display whose transfer never completes while `hang` is set (a hung DMA), and
/// otherwise completes on its second poll (a real transfer takes a while).
struct HangingAsync(Panel);

impl AsyncDisplayDriver for HangingAsync {
    type Error = ();
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(W, H, ColorFormat::Rgb565)
    }
    async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), ()> {
        let p = &self.0;
        p.started.set(p.started.get() + 1);
        if p.hang.get() {
            std::future::pending::<()>().await;
        }
        let mut polled = false;
        std::future::poll_fn(|cx| {
            if polled {
                Poll::Ready(())
            } else {
                polled = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await;
        let w = area.width() as usize;
        for (row, y) in (area.y0..area.y1).enumerate() {
            let dst = (y as usize * usize::from(W) + area.x0 as usize) * 2;
            p.fb.borrow_mut()[dst..dst + w * 2].copy_from_slice(&buf[row * w * 2..(row + 1) * w * 2]);
        }
        Ok(())
    }
}

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

fn app(_cx: Scope) -> impl View {
    column((label("Twine"), button(label("Flush"))))
        .gap(4)
        .padding(4)
        .bg(Color::hex(0x1E_88_E5))
        .size(Length::Pct(100), Length::Pct(100))
}

fn ui(panel: &Panel, platform: &MockPlatform, double: bool, cfg: EngineConfig) -> AsyncUi<HangingAsync> {
    let len = usize::from(W) * 2 * ROWS;
    let bufs = if double {
        BufferMode::partial_double(leak(len), leak(len))
    } else {
        BufferMode::partial_single(leak(len))
    };
    Ui::builder_async(HangingAsync(panel.clone()))
        .runtime(Runtime::current_thread())
        .buffers(bufs)
        .config(cfg)
        .platform(platform)
        .build(app)
}

/// A task waker that counts its wake-ups.
struct Counter(AtomicUsize);

impl TaskWake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Polls `f` until it completes or stops waking itself (`None`: it waits for something).
fn poll_until_stuck<F: Future>(f: &mut Pin<&mut F>, counter: &Arc<Counter>) -> Option<F::Output> {
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);
    for _ in 0..1000 {
        let before = counter.0.load(Ordering::SeqCst);
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return Some(v);
        }
        if counter.0.load(Ordering::SeqCst) == before {
            return None;
        }
    }
    panic!("the future never settled");
}

/// Polls `f` to completion (its flushes complete on their own).
fn block_on<F: Future>(f: F) -> F::Output {
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let mut f = pin!(f);
    poll_until_stuck(&mut f, &counter).expect("the update waits for nothing")
}

/// The frame of a UI whose flushes never hang.
fn reference() -> Vec<u8> {
    let panel = Panel::new();
    let mut u = ui(&panel, &MockPlatform::new(), true, EngineConfig::default());
    let _ = block_on(u.update_async());
    panel.fb.borrow().clone()
}

fn hung_flush_times_out_and_recovers(double: bool) {
    let panel = Panel::new();
    let platform = MockPlatform::new();
    let mut u = ui(&panel, &platform, double, EngineConfig::default());
    panel.hang.set(true);
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let wake = {
        let mut update = pin!(u.update_async());
        assert!(
            poll_until_stuck(&mut update, &counter).is_none(),
            "the update waits for the hung flush"
        );
        platform.advance(Duration::ms(499));
        assert!(
            poll_until_stuck(&mut update, &counter).is_none(),
            "still within the timeout"
        );
        let before = counter.0.load(Ordering::SeqCst);
        platform.advance(Duration::ms(1));
        assert!(
            counter.0.load(Ordering::SeqCst) > before,
            "the timer wakes the task at the deadline"
        );
        poll_until_stuck(&mut update, &counter).expect("the update returns after the timeout")
    };
    assert_eq!(
        panel.started.get(),
        1,
        "the frame is abandoned after the hung flush"
    );
    assert_eq!(u.take_faults(), Faults::from(FaultKind::FlushTimeout));
    let r = *u.last_fault(FaultKind::FlushTimeout).unwrap();
    assert_eq!(r.code, 500, "ms waited");
    assert_eq!(r.display, Some(u.engine().default_display().unwrap()));
    let h = u.display_health().unwrap();
    assert_eq!(h.state, DisplayState::Failed);
    assert!(h.consecutive_errors >= 8);
    // The next frame comes `refr_period` after the timeout, as on the blocking path.
    let due = Instant::from_millis(516);
    assert_eq!(wake, Wake::At(due));
    // The driver works again (e.g. the application reset the bus): the redrawn frame is
    // complete and the display healthy.
    panel.hang.set(false);
    platform.set(due);
    let _ = block_on(u.update_async());
    assert_eq!(u.display_health().unwrap().state, DisplayState::Healthy);
    assert_eq!(*panel.fb.borrow(), reference(), "every abandoned row was redrawn");
    assert!(u.take_faults().is_empty());
}

#[test]
fn regression_hung_async_flush_times_out_single_buffer() {
    hung_flush_times_out_and_recovers(false);
}

#[test]
fn regression_hung_async_flush_times_out_double_buffer() {
    hung_flush_times_out_and_recovers(true);
}

#[test]
fn hung_async_flush_halts_the_display_under_halt_policy() {
    let panel = Panel::new();
    let platform = MockPlatform::new();
    let cfg = EngineConfig {
        flush_policy: FlushPolicy::Halt,
        ..EngineConfig::default()
    };
    let mut u = ui(&panel, &platform, true, cfg);
    panel.hang.set(true);
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let wake = {
        let mut update = pin!(u.update_async());
        assert!(poll_until_stuck(&mut update, &counter).is_none());
        platform.advance(Duration::ms(500));
        poll_until_stuck(&mut update, &counter).unwrap()
    };
    assert_eq!(u.display_health().unwrap().state, DisplayState::Halted);
    assert_eq!(wake, Wake::Idle, "a halted display does not keep the UI awake");
    // Recovery is the application's decision.
    panel.hang.set(false);
    u.recover_display();
    platform.advance(Duration::ms(100));
    let _ = block_on(u.update_async());
    assert_eq!(u.display_health().unwrap().state, DisplayState::Healthy);
    assert_eq!(*panel.fb.borrow(), reference());
}

#[test]
fn without_flush_timeout_a_hung_async_flush_is_awaited() {
    let panel = Panel::new();
    let platform = MockPlatform::new();
    let cfg = EngineConfig {
        flush_timeout: None,
        ..EngineConfig::default()
    };
    let mut u = ui(&panel, &platform, false, cfg);
    panel.hang.set(true);
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let mut update = pin!(u.update_async());
    assert!(poll_until_stuck(&mut update, &counter).is_none());
    platform.advance(Duration::secs(3600));
    assert!(poll_until_stuck(&mut update, &counter).is_none(), "no deadline");
    assert_eq!(counter.0.load(Ordering::SeqCst), 0, "no timer was armed");
}
