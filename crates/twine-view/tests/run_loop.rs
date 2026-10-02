//! `twine_view::run::blocking_with` (R3.S04): the blocking run loop waits with the deadlines
//! the `Ui` returns (none when idle), steps again at once on `Wake::Now` (yielding, never
//! waiting), installs the platform's notify on the waker so a wake-up from another thread ends
//! a wait, reports its events in order, and allocates nothing per iteration.
//!
//! The loop never returns: each test leaves it by unwinding with a private payload from the
//! callback once it has seen enough.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::thread;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use twine_core::ColorFormat;
use twine_hal::{DisplayInfo, Platform, StdPlatform};
use twine_testing::alloc::{CountingAllocator, current};
use twine_testing::{MemoryDisplay, MockPlatform};
use twine_view::prelude::*;
use twine_view::run::{self, LoopEvent};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// The payload that ends a loop.
struct Stop;

/// Runs `ui` on `platform` until `on_event` returns `true`.
fn run_until<P: Platform>(ui: Ui, platform: &mut P, mut on_event: impl FnMut(&mut Ui, LoopEvent) -> bool) {
    let result = catch_unwind(AssertUnwindSafe(|| -> () {
        run::blocking_with(ui, platform, |ui, event| {
            if on_event(ui, event) {
                resume_unwind(Box::new(Stop));
            }
        })
    }));
    match result {
        Err(payload) if payload.is::<Stop>() => {}
        Err(payload) => resume_unwind(payload),
        Ok(()) => unreachable!("the run loop returned"),
    }
}

fn panel() -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565))
}

fn ui_on(platform: &MockPlatform, app: impl FnOnce(Scope) -> AnyView) -> Ui {
    Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .platform(platform)
        .build(app)
}

#[test]
fn idle_ui_waits_without_deadline() {
    let mut platform = MockPlatform::new();
    let ui = ui_on(&platform, |_| label("static").into_any());
    let mut events = Vec::new();
    run_until(ui, &mut platform, |_, e| {
        events.push(e);
        events.len() == 7
    });
    assert_eq!(
        events,
        [
            LoopEvent::BeforeUpdate, // first frame, then nothing to do
            LoopEvent::Idle(None),
            LoopEvent::Woken,
            LoopEvent::BeforeUpdate,
            LoopEvent::Idle(None),
            LoopEvent::Woken,
            LoopEvent::BeforeUpdate,
        ]
    );
    assert_eq!(
        platform.deadlines(),
        [None, None],
        "no deadline: no spinning, no timer"
    );
    assert_eq!(platform.yields(), 0);
}

#[test]
fn deadlines_of_the_ui_are_the_waits_of_the_loop() {
    let mut platform = MockPlatform::new();
    let ui = ui_on(&platform, |cx| {
        let n = cx.signal(0u32);
        cx.interval(Duration::ms(100), move || n.update(|v| *v += 1));
        label(text!("{}", n.get())).into_any()
    });
    let mut idles = Vec::new();
    let clock = platform.clock().clone();
    run_until(ui, &mut platform, |ui, e| {
        if let LoopEvent::Idle(deadline) = e {
            // Reported before the wait, on the clock the `Ui` and the platform share.
            assert_eq!(ui.now(), twine_hal::Clock::now(&clock));
            if ui.now() >= Instant::from_millis(300) {
                return true;
            }
            idles.push(deadline);
        }
        false
    });
    // The MockPlatform sleeps until each deadline: the waits are exactly the `Wake::At`s.
    assert_eq!(platform.deadlines(), idles);
    assert!(
        idles.iter().all(Option::is_some),
        "the interval keeps a deadline: {idles:?}"
    );
    // Non-decreasing, not increasing: `MockPlatform` notifications are process-wide, so a test
    // running in parallel may end a wait early (a spurious return the `Platform` contract
    // allows), and the next update returns the same deadline again.
    assert!(idles.windows(2).all(|w| w[0] <= w[1]), "{idles:?}");
    assert_eq!(idles.last(), Some(&Some(Instant::from_millis(300))));
}

static BURST: Channel<u32, 64> = Channel::new();

#[test]
fn now_steps_again_without_waiting_and_yields() {
    let mut platform = MockPlatform::new();
    let received = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let r = received.clone();
    let ui = ui_on(&platform, move |cx| {
        cx.on_message(&BURST, move |_| r.set(r.get() + 1));
        label("burst").into_any()
    });
    // 40 messages: delivered 16 per update, so the first two updates return `Wake::Now`.
    for i in 0..40 {
        BURST.try_send(i).unwrap();
    }
    let mut events = Vec::new();
    let mut yields_at_idle = None;
    let probe = platform.clone(); // clones share the records
    run_until(ui, &mut platform, |_, e| {
        events.push(e);
        if matches!(e, LoopEvent::Idle(_)) {
            yields_at_idle = Some(probe.yields());
            return true;
        }
        false
    });
    assert_eq!(
        events,
        [
            LoopEvent::BeforeUpdate,
            LoopEvent::BeforeUpdate,
            LoopEvent::BeforeUpdate,
            LoopEvent::Idle(None),
        ],
        "`Now` steps again with no wait in between"
    );
    assert_eq!(yields_at_idle, Some(2), "one yield per `Wake::Now`");
    assert_eq!(platform.waits(), 0);
    assert_eq!(
        received.get(),
        40,
        "every message delivered before the loop slept"
    );
}

static FROM_THREAD: Channel<u32, 4> = Channel::new();

#[test]
fn wake_from_another_thread_ends_the_wait_without_builder_platform() {
    // The `Ui` only has the platform's clock: the run loop itself installs `notify` on the
    // waker. `StdPlatform::wait(None)` really blocks; only the notify can end it.
    let mut platform = StdPlatform::new();
    let got = std::rc::Rc::new(std::cell::Cell::new(None));
    let g = got.clone();
    let ui = Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .clock(platform.clone())
        .build(move |cx| {
            cx.on_message(&FROM_THREAD, move |v| g.set(Some(v)));
            label("waiting")
        });
    let start = StdInstant::now();
    let mut sender = None;
    let mut events = Vec::new();
    run_until(ui, &mut platform, |_, e| {
        events.push(e);
        match e {
            LoopEvent::Idle(None) if sender.is_none() => {
                sender = Some(thread::spawn(|| {
                    thread::sleep(StdDuration::from_millis(20));
                    FROM_THREAD.try_send(7).unwrap();
                    // Safety net: a lost wake-up fails the test below instead of hanging it.
                    thread::sleep(StdDuration::from_secs(5));
                    StdPlatform::notify();
                }));
                false
            }
            // The second update delivered the message.
            LoopEvent::Idle(None) => got.get().is_some(),
            _ => false,
        }
    });
    assert!(
        start.elapsed() < StdDuration::from_secs(2),
        "the channel send did not end the wait ({:?})",
        start.elapsed()
    );
    assert_eq!(got.get(), Some(7));
    assert_eq!(
        events,
        [
            LoopEvent::BeforeUpdate,
            LoopEvent::Idle(None),
            LoopEvent::Woken,
            LoopEvent::BeforeUpdate,
            LoopEvent::Idle(None),
        ]
    );
    drop(sender);
}

#[test]
fn callback_keeps_access_to_the_ui() {
    let mut platform = MockPlatform::new();
    let ui = ui_on(&platform, |_| label("hi").into_any());
    let mut seen_driver = false;
    run_until(ui, &mut platform, |ui, e| {
        if e == LoopEvent::Idle(None) {
            // The display (e.g. its backlight) and the engine stay reachable.
            let d = ui.display();
            seen_driver = ui.engine_mut().driver_mut::<MemoryDisplay>(d).is_some();
            ui.set_motion(Motion::Reduced);
            return true;
        }
        false
    });
    assert!(seen_driver);
}

#[test]
fn no_allocation_per_iteration() {
    let mut platform = MockPlatform::new();
    // A spinner animates forever: every iteration updates, renders and waits.
    let ui = ui_on(&platform, |_| spinner().into_any());
    let mut iterations = 0u32;
    let mut before = None;
    let mut after = None;
    run_until(ui, &mut platform, |_, e| {
        if e == LoopEvent::BeforeUpdate {
            iterations += 1;
            match iterations {
                20 => before = Some(current()),
                220 => {
                    after = Some(current());
                    return true;
                }
                _ => {}
            }
        }
        false
    });
    let (before, after) = (before.unwrap(), after.unwrap());
    assert_eq!(
        after.allocs - before.allocs,
        0,
        "allocations in 200 loop iterations"
    );
    assert!(
        platform.waits() >= 200,
        "every iteration waited for the next frame"
    );
}
