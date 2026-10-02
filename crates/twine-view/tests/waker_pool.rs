//! An exhausted waker pool (R3.S02, rework F10a): the mount still succeeds, with the shared
//! fallback waker, and raises `FaultKind::Capacity` with code `CapacityFault::WakerPool`
//! instead of allocating without bound.
//!
//! A test binary of its own: the pool and its heap limit are process-wide.

use twine_reactive::WakerLease;
use twine_testing::EngineHarness;
use twine_view::prelude::*;
use twine_view::{CapacityFault, UiCore};

#[test]
fn exhausted_waker_pool_raises_a_fault_and_shares_a_waker() {
    WakerLease::set_heap_limit(0); // static slots only: the bound a safety profile would set
    let mut h = EngineHarness::new(64, 32);
    let d = h.display();
    let rt = Runtime::current_thread();
    let mut cores = Vec::new();
    for _ in 0..WakerLease::STATIC_SLOTS {
        cores.push(UiCore::mount(rt, h.engine_mut(), d, |_| label("x")).unwrap());
        assert!(
            !h.engine_mut().take_faults().contains(FaultKind::Capacity),
            "the static slots serve the first UIs"
        );
    }
    let fifth = UiCore::mount(rt, h.engine_mut(), d, |_| label("x")).unwrap();
    let faults = fifth.take_faults(h.engine_mut());
    assert!(faults.contains(FaultKind::Capacity));
    let r = h.engine().last_fault(FaultKind::Capacity).copied().unwrap();
    assert_eq!(CapacityFault::from_code(r.code), Some(CapacityFault::WakerPool));
    assert_eq!(r.display, Some(d));
    assert!(
        cores.iter().all(|c| !core::ptr::eq(c.waker(), fifth.waker())),
        "the fallback is not one of the leased wakers"
    );
    assert_eq!(WakerLease::heap_allocated(), 0, "nothing was allocated");

    // A sixth UI shares the same fallback; returning a slot serves the next mount again.
    let sixth = UiCore::mount(rt, h.engine_mut(), d, |_| label("x")).unwrap();
    assert!(core::ptr::eq(sixth.waker(), fifth.waker()));
    let _ = h.engine_mut().take_faults();
    cores.pop().unwrap().dispose(h.engine_mut());
    let again = UiCore::mount(rt, h.engine_mut(), d, |_| label("x")).unwrap();
    assert!(!core::ptr::eq(again.waker(), fifth.waker()));
    assert!(!h.engine_mut().take_faults().contains(FaultKind::Capacity));
    for c in cores.into_iter().chain([fifth, sixth, again]) {
        c.dispose(h.engine_mut());
    }
    WakerLease::set_heap_limit(WakerLease::DEFAULT_HEAP_LIMIT);
}
