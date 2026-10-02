//! Cross-context data flow through a whole `Ui` (R3.S05): `Channel` (both overflow policies),
//! `Latest` (`watch`) and `Outbox` allocate nothing when sending, setting, draining and
//! receiving in steady state; `messages_per_channel` spreads a burst over updates.

use twine_core::ColorFormat;
use twine_hal::DisplayInfo;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, MockClock};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// The cross-context objects of the test application (the ports pattern).
#[derive(Clone, Copy)]
struct Ports {
    events: &'static Channel<u32, 8>,
    telemetry: &'static Channel<u32, 4>,
    level: &'static Latest<u32>,
    out: &'static Outbox<u32, 8>,
}

fn app(cx: Scope, p: Ports) -> impl View {
    let events = cx.signal(0u32);
    let telemetry = cx.signal(0u32);
    let level = cx.watch(p.level);
    cx.on_message(p.events, move |v| {
        events.set(v);
        // Commands out of a handler: the outbox is filled and drained in the same cycle.
        let _ = p.out.try_send(v);
    });
    cx.on_message(p.telemetry, move |v| telemetry.set(v));
    column((
        label(text!("{}", events.get())),
        label(text!("{}", telemetry.get())),
        label(text!("{}", level.get())),
    ))
}

fn ui(p: Ports, messages_per_channel: usize) -> Ui {
    Ui::builder(MemoryDisplay::new(DisplayInfo::new(96, 48, ColorFormat::Rgb565)))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .clock(MockClock::new())
        .messages_per_channel(messages_per_channel)
        .build(move |cx| app(cx, p))
}

fn update_until_idle(ui: &mut Ui) -> u32 {
    let mut updates = 1;
    while ui.update() == Wake::Now {
        updates += 1;
        assert!(updates < 100, "the UI does not settle");
    }
    updates
}

fn leak<T>(v: T) -> &'static T {
    Box::leak(Box::new(v))
}

fn ports() -> Ports {
    Ports {
        events: leak(Channel::new()),
        telemetry: leak(Channel::new().on_full(Overflow::DropOldest)),
        level: leak(Latest::new(0)),
        out: leak(Outbox::new()),
    }
}

#[test]
fn no_allocation_on_send_set_drain_and_receive() {
    let p = ports();
    let mut ui = ui(p, 16);
    // Warm-up: the first rounds size the label texts' buffers.
    for i in 0..3 {
        p.events.try_send(100_000 + i).unwrap();
        p.telemetry.try_send(100_000 + i).unwrap();
        p.level.set(100_000 + i);
        update_until_idle(&mut ui);
        while p.out.try_recv().is_some() {}
    }
    let ((), stats) = count_allocs(|| {
        for round in 0..20u32 {
            for i in 0..4 {
                p.events.try_send(200_000 + round * 4 + i).unwrap();
            }
            for i in 0..10 {
                // Overflows (DropOldest): evicts, counts, reports a fault at the drain.
                p.telemetry.try_send(300_000 + round * 10 + i).unwrap();
            }
            p.level.set(400_000 + round);
            update_until_idle(&mut ui);
            let mut received = 0;
            while p.out.try_recv().is_some() {
                received += 1;
            }
            assert_eq!(received, 4);
        }
    });
    assert_eq!(stats.allocs, 0, "allocations in 20 send/drain rounds: {stats:?}");
    // Every round was drained.
    assert_eq!(p.telemetry.len(), 0);
    // 6 evictions per round, reported as the unchanged `ChannelOverflow` fault.
    assert!(ui.take_faults().contains(FaultKind::ChannelOverflow));
    assert_eq!(ui.fault_counts().get(FaultKind::ChannelOverflow), 20 * 6);
}

#[test]
fn messages_per_channel_spreads_a_burst_over_updates() {
    let p = ports();
    let mut ui = ui(p, 2);
    assert_eq!(ui.messages_per_channel(), 2);
    update_until_idle(&mut ui);
    for i in 0..8 {
        p.events.try_send(i).unwrap();
    }
    assert_eq!(ui.update(), Wake::Now, "6 messages left");
    assert_eq!(p.events.len(), 6);
    assert_eq!(update_until_idle(&mut ui), 3, "2 per update");
    assert_eq!(p.out.len(), 8, "every message handled, in order");
    assert_eq!(p.out.try_recv(), Some(0));

    ui.set_messages_per_channel(0); // counts as 1: every update makes progress
    assert_eq!(ui.messages_per_channel(), 1);
    for i in 0..3 {
        p.events.try_send(i).unwrap();
    }
    assert_eq!(update_until_idle(&mut ui), 3);
    assert!(p.events.is_empty());
}

#[test]
fn default_messages_per_channel() {
    let p = ports();
    let ui = Ui::builder(MemoryDisplay::new(DisplayInfo::new(96, 48, ColorFormat::Rgb565)))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
        .clock(MockClock::new())
        .build(move |cx| app(cx, p));
    assert_eq!(
        ui.messages_per_channel(),
        twine_view::DEFAULT_MESSAGES_PER_CHANNEL
    );
    assert_eq!(twine_view::DEFAULT_MESSAGES_PER_CHANNEL, 16);
}

#[test]
fn latest_set_from_another_thread_wakes_and_updates_the_ui() {
    let p = ports();
    let mut ui = ui(p, 16);
    update_until_idle(&mut ui);
    assert_eq!(ui.update(), Wake::Idle);
    let level = p.level;
    std::thread::spawn(move || level.set(42)).join().unwrap();
    assert!(ui.waker().is_set(), "the set woke the UI");
    update_until_idle(&mut ui);
    assert_eq!(ui.update(), Wake::Idle);
}
