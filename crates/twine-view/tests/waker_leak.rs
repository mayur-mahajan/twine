//! Mounting and disposing a `UiCore` does not leak its waker (R0.S05).
//!
//! A test binary of its own (Cargo runs each `tests/*.rs` as a separate process): the test returns
//! wakers to the process-wide waker pool and then checks that the returned waker is no longer
//! woken, which a test running in parallel in the same process could disturb by leasing that
//! waker. Installs `CountingAllocator` (per-thread counters) for the heap check.

use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, current};
use twine_view::UiCore;
use twine_view::prelude::*;

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

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
        let core = UiCore::mount(Runtime::current_thread(), e, display, |cx| {
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
