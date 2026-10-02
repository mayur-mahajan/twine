//! `UiBuilder::platform` (R3.S01): one platform is the `Ui`'s clock, the waker's notify
//! function and the runtime's interrupt probe, so a run loop can sleep in `Platform::wait`
//! and every wake-up ends the sleep.

use std::thread;
use std::time::Duration as StdDuration;

use twine_core::ColorFormat;
use twine_hal::{Clock, DisplayInfo, Platform, StdPlatform};
use twine_testing::{MemoryDisplay, MockPlatform};
use twine_view::prelude::*;

fn panel() -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565))
}

fn app(cx: Scope) -> impl View {
    let n = cx.signal(0u32);
    cx.interval(Duration::ms(100), move || n.update(|v| *v += 1));
    label(text!("{}", n.get()))
}

#[test]
fn platform_is_the_clock_the_notify_and_the_interrupt_probe() {
    let mut platform = MockPlatform::new();
    let mut ui = Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .platform(&platform)
        .build(app);
    // The clock: the `Ui` reads the platform's time.
    platform.advance(Duration::ms(7));
    assert_eq!(ui.now(), Instant::from_millis(7));
    // The interrupt probe: registered on the runtime.
    assert!(!twine_reactive::in_interrupt());
    assert!(MockPlatform::interrupt(twine_reactive::in_interrupt));
    // A run loop: step, then wait as the `Ui` says; the deadlines are on the same clock.
    for _ in 0..10 {
        match ui.update() {
            Wake::Now => {}
            Wake::At(t) => platform.wait(Some(t)),
            Wake::Idle | Wake::IdleFor(_) => platform.wait(None),
        }
    }
    let deadlines = platform.deadlines();
    assert!(!deadlines.is_empty(), "the loop slept");
    assert!(
        deadlines.iter().all(Option::is_some),
        "the interval keeps a deadline"
    );
    assert!(
        ui.now() >= Instant::from_millis(100),
        "time moved through the waits"
    );
}

#[test]
fn notify_input_from_another_thread_notifies_the_platform() {
    let mut platform = MockPlatform::new();
    let mut ui = Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .platform(&platform)
        .build(|_| label("idle"));
    while ui.update() == Wake::Now {}
    let before = MockPlatform::notifications();
    let waker = ui.waker();
    // e.g. a touch interrupt, or another task, calling `notify_input`.
    thread::spawn(move || waker.wake()).join().unwrap();
    assert!(
        MockPlatform::notifications() > before,
        "the waker called Platform::notify"
    );
    let t = platform.now();
    platform.wait(Some(t + Duration::secs(10)));
    assert_eq!(platform.now(), t, "the wait returned at once instead of sleeping");
    assert!(ui.waker().is_set(), "the next update sees the wake-up");
}

#[test]
fn std_platform_wait_wakes_on_ui_waker_wake() {
    let mut platform = StdPlatform::new();
    let mut ui = Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .platform(&platform)
        .build(|_| label("idle"));
    while ui.update() == Wake::Now {}
    let waker = ui.waker();
    let sender = thread::spawn(move || {
        thread::sleep(StdDuration::from_millis(50));
        waker.wake(); // e.g. a channel send from a sensor thread
    });
    let t0 = platform.now();
    // A generous deadline: returning well before it proves the wake-up ended the wait.
    platform.wait(Some(t0 + Duration::secs(20)));
    let slept = platform.now().saturating_duration_since(t0);
    sender.join().unwrap();
    assert!(
        slept < Duration::secs(10),
        "woken by the UiWaker, not the deadline ({slept})"
    );
    assert!(ui.waker().is_set());
    // The deadline alone also ends a wait.
    let t1 = platform.now();
    platform.wait(Some(t1 + Duration::ms(20)));
    assert!(platform.now() >= t1 + Duration::ms(20));
}

/// The interrupt probe `UiBuilder::platform` registers is asserted on every runtime access
/// (`twine-reactive`'s `debug-checks`, enabled for these tests): a reactive API reached from a
/// (simulated) interrupt handler panics before it touches the runtime, while the cross-context
/// types keep working there and the UI is intact afterwards.
#[test]
fn reactive_api_in_an_interrupt_panics_with_debug_checks() {
    static CH: Channel<u32, 4> = Channel::new();
    let platform = MockPlatform::new();
    let mut seen = None;
    let mut ui = Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .platform(&platform)
        .build(|cx| {
            let n = cx.signal(0u32);
            seen = Some(n);
            cx.on_message(&CH, move |v| n.set(v));
            label(text!("{}", n.get()))
        });
    let n = seen.unwrap();
    while ui.update() == Wake::Now {}

    let read = std::panic::catch_unwind(|| MockPlatform::interrupt(|| n.get()));
    assert!(read.is_err(), "a signal read from an interrupt handler panics");
    let write = std::panic::catch_unwind(|| MockPlatform::interrupt(|| n.set(5)));
    assert!(write.is_err(), "and so does a write");
    // What an interrupt handler may do: send a message and wake the UI.
    MockPlatform::interrupt(|| {
        CH.try_send(7).unwrap();
        ui.waker().wake();
    });

    while ui.update() == Wake::Now {}
    assert_eq!(n.get(), 7, "the message arrived; the runtime is intact");
}
