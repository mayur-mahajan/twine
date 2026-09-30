//! One waker per `Ui` (R0.S05): each `Ui`/`UiCore` is woken only by its own channels, even
//! from another thread, and mounting/disposing does not leak its waker.
//!
//! This binary installs `CountingAllocator` (per-thread counters) for the heap test.

use twine_core::ColorFormat;
use twine_hal::DisplayInfo;
use twine_testing::alloc::{CountingAllocator, current};
use twine_testing::{EngineHarness, MemoryDisplay, MockClock};
use twine_view::UiCore;
use twine_view::prelude::*;

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

fn ui_on(ch: &'static Channel<u32, 4>, seen: Signal<u32>) -> Ui {
    let display = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    Ui::builder(display).clock(MockClock::new()).build(move |cx| {
        let local = cx.signal(0u32);
        cx.on_message(ch, move |v| {
            local.set(v);
            seen.set(v);
        });
        label(text!("{}", local.get()))
    })
}

#[test]
fn two_uis_on_one_runtime_wake_only_themselves() {
    static CH1: Channel<u32, 4> = Channel::new();
    static CH2: Channel<u32, 4> = Channel::new();
    let host = twine_reactive::create_root();
    let seen1 = host.signal(0u32);
    let seen2 = host.signal(0u32);
    let mut ui1 = ui_on(&CH1, seen1);
    let mut ui2 = ui_on(&CH2, seen2);
    assert!(
        !core::ptr::eq(ui1.waker(), ui2.waker()),
        "each Ui has its own waker"
    );
    ui1.update();
    ui2.update();
    assert!(!ui1.waker().is_set() && !ui2.waker().is_set());

    CH1.try_send(1).unwrap();
    assert!(ui1.waker().is_set());
    assert!(!ui2.waker().is_set(), "ui2 is not woken by ui1's channel");
    ui1.update();
    assert_eq!(seen1.get(), 1);

    CH2.try_send(2).unwrap();
    assert!(ui2.waker().is_set());
    assert!(!ui1.waker().is_set(), "ui1 is not woken by ui2's channel");
    ui2.update();
    assert_eq!(seen2.get(), 2);

    // Mounting a second Ui no longer steals the first one's wake-ups.
    let ui3 = ui_on(&CH2, seen2);
    CH1.try_send(3).unwrap();
    assert!(ui1.waker().is_set());
    assert!(!ui3.waker().is_set());
    drop((ui1, ui2, ui3));
    host.dispose();
}

#[test]
fn send_from_another_thread_wakes_the_right_ui() {
    static CH1: Channel<u32, 4> = Channel::new();
    static CH2: Channel<u32, 4> = Channel::new();
    let host = twine_reactive::create_root();
    let seen1 = host.signal(0u32);
    let seen2 = host.signal(0u32);
    let mut ui1 = ui_on(&CH1, seen1);
    let mut ui2 = ui_on(&CH2, seen2);
    ui1.update();
    ui2.update();
    // An interrupt handler / sensor task: only the channel is used there.
    std::thread::spawn(|| CH2.try_send(42).unwrap()).join().unwrap();
    assert!(ui2.waker().is_set());
    assert!(!ui1.waker().is_set());
    ui2.update();
    assert_eq!(seen2.get(), 42, "the message is handled by the next update");
    assert_eq!(seen1.get(), 0);
    drop((ui1, ui2));
    host.dispose();
}

#[test]
fn app_provided_static_waker_is_used() {
    static WAKER: UiWaker = UiWaker::new();
    static CH: Channel<u32, 4> = Channel::new();
    let display = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    let mut ui = Ui::builder(display)
        .clock(MockClock::new())
        .waker(&WAKER)
        .build(|cx| {
            cx.on_message(&CH, |_| {});
            label("x")
        });
    assert!(core::ptr::eq(ui.waker(), &raw const WAKER));
    ui.update();
    CH.try_send(1).unwrap();
    assert!(WAKER.is_set());
    ui.update();
    assert!(!WAKER.is_set());
}

/// Before R0.S05 every `UiCore::mount` leaked a heap `UiWaker`, and `register_waker` kept only
/// the last `Ui`'s waker in every channel.
#[test]
fn regression_mount_dispose_does_not_leak_the_waker() {
    static CH: Channel<u32, 4> = Channel::new();
    let mut h = EngineHarness::new(64, 32);
    let display = h.display();
    let screen = h.screen();
    let cycle = |h: &mut EngineHarness| {
        let e = h.engine_mut();
        let core = UiCore::mount(e, display, |cx| {
            cx.on_message(&CH, |_| {});
            label("x")
        })
        .unwrap();
        let waker = core.waker();
        CH.try_send(1).unwrap();
        assert!(waker.take(), "the channel wakes the mounted UI");
        core.dispose(e);
        let _ = CH.try_recv();
        let children: Vec<_> = e.tree().children(screen).collect();
        for c in children {
            e.delete(c).unwrap();
        }
        // Disposed: the channel no longer wakes the (returned) waker.
        CH.try_send(2).unwrap();
        assert!(!waker.is_set());
        let _ = CH.try_recv();
        // Render the removal (the engine's dirty areas and debug logs are consumed by frames).
        h.run_until_idle();
    };
    // Warm-up: vectors and arenas reach their steady capacity.
    for _ in 0..4 {
        cycle(&mut h);
    }
    let before = current().live;
    for _ in 0..1000 {
        cycle(&mut h);
    }
    let growth = current().live - before;
    assert_eq!(
        growth, 0,
        "live heap grew by {growth} bytes over 1000 mount/dispose cycles"
    );
}
