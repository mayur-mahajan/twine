//! `twine_embassy::run_with` (R3.S04): runs both runtimes (`AsyncUi` and the blocking `Ui`),
//! reports `BeforeUpdate` / `Idle(deadline)` / `Woken` in order with access to the UI, sleeps
//! until the UI's deadline on the embassy timer, and wakes on channel messages.
//!
//! One test: the embassy mock time driver is process-wide, so the scenarios run in sequence.

use std::cell::RefCell;
use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Wake as TaskWake, Waker};

use embassy_time::{Duration as EmbDuration, MockDriver};
use twine_core::{ColorFormat, Rect};
use twine_embassy::{LoopEvent, RunUi, UiBuilderExt};
use twine_hal::{AsyncDisplayDriver, DisplayInfo};
use twine_testing::MemoryDisplay;
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 32;

/// An async display whose flushes complete at once.
struct Instant0;

impl AsyncDisplayDriver for Instant0 {
    type Error = ();
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(W, H, ColorFormat::Rgb565)
    }
    fn flush(&mut self, _area: Rect, _buf: &[u8]) -> impl Future<Output = Result<(), ()>> {
        std::future::ready(Ok(()))
    }
}

/// A waker that counts its wake-ups.
struct Counter(AtomicUsize);

impl TaskWake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Polls the run loop until it stops waking itself (asleep).
fn run_until_asleep<F: Future>(fut: &mut Pin<&mut F>, counter: &Arc<Counter>) {
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);
    for _ in 0..1000 {
        let before = counter.0.load(Ordering::SeqCst);
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        if counter.0.load(Ordering::SeqCst) == before {
            return;
        }
    }
    panic!("the run loop never went to sleep");
}

type Events = Rc<RefCell<Vec<LoopEvent>>>;

fn take(events: &Events) -> Vec<LoopEvent> {
    std::mem::take(&mut *events.borrow_mut())
}

static MESSAGES: Channel<u32, 4> = Channel::new();

fn channel_app(cx: Scope) -> impl View {
    let n = cx.signal(0u32);
    cx.on_message(&MESSAGES, move |v| n.set(v));
    label(text!("{}", n.get()))
}

fn ticking_app(cx: Scope) -> impl View {
    let n = cx.signal(0u32);
    cx.interval(Duration::ms(100), move || n.update(|v| *v += 1));
    label(text!("{}", n.get()))
}

/// Records every event, and checks that the UI is reachable from the callback.
fn recorder<U: RunUi, C: Fn(&mut U) + 'static>(
    events: &Events,
    check: C,
) -> impl FnMut(&mut U, LoopEvent) + use<U, C> {
    let events = events.clone();
    move |ui, e| {
        check(ui);
        events.borrow_mut().push(e);
    }
}

#[test]
fn run_with_reports_events_for_async_ui_and_ui() {
    let counter = Arc::new(Counter(AtomicUsize::new(0)));

    // --- AsyncUi: idle until a channel message.
    let events: Events = Rc::default();
    let ui = Ui::builder_async(Instant0)
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .with_embassy_platform()
        .build(channel_app);
    let on_event = recorder(&events, |ui: &mut AsyncUi<Instant0>| {
        assert!(ui.engine().default_display().is_some());
    });
    {
        let mut run = pin!(twine_embassy::run_with(ui, on_event));
        run_until_asleep(&mut run, &counter);
        assert_eq!(take(&events), [LoopEvent::BeforeUpdate, LoopEvent::Idle(None)]);
        MESSAGES.try_send(7).unwrap();
        run_until_asleep(&mut run, &counter);
        assert_eq!(
            take(&events),
            [LoopEvent::Woken, LoopEvent::BeforeUpdate, LoopEvent::Idle(None)]
        );
    }

    // --- The blocking Ui: sleeps until its deadline on the embassy timer.
    let start = embassy_time::Instant::now();
    let events: Events = Rc::default();
    let ui = Ui::builder(MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .with_embassy_platform()
        .build(ticking_app);
    let on_event = recorder(&events, |ui: &mut Ui| {
        // The display stays reachable (e.g. for its backlight).
        let d = ui.display();
        assert!(ui.engine_mut().driver_mut::<MemoryDisplay>(d).is_some());
    });
    let mut run = pin!(twine_embassy::run_with(ui, on_event));
    run_until_asleep(&mut run, &counter);
    let first = Instant::from_micros(start.as_micros()) + Duration::ms(100);
    assert_eq!(
        take(&events),
        [LoopEvent::BeforeUpdate, LoopEvent::Idle(Some(first))]
    );
    // Not before the deadline...
    MockDriver::get().advance(EmbDuration::from_millis(50));
    run_until_asleep(&mut run, &counter);
    assert_eq!(take(&events), []);
    // ...but at it.
    MockDriver::get().advance(EmbDuration::from_millis(50));
    run_until_asleep(&mut run, &counter);
    assert_eq!(
        take(&events),
        [
            LoopEvent::Woken,
            LoopEvent::BeforeUpdate,
            LoopEvent::Idle(Some(first + Duration::ms(100)))
        ]
    );
}
