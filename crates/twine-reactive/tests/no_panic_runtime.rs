//! The `no-panic-runtime` feature (implied by `safety`): failed writes and creations, capacity
//! exhaustion and memo cycles are skipped with a fault instead of panicking; reads that have no
//! value raise the fault, then panic. Run with
//! `cargo test -p twine-reactive --features no-panic-runtime` (CI stage `test-features`).
//!
//! req: SAF-007
#![cfg(feature = "no-panic-runtime")]

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use twine_reactive::{FaultKind, Memo, Runtime};

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

/// The faults recorded since the last call (clears them).
fn take(kind: FaultKind) -> u32 {
    rt().take_faults().get(kind)
}

/// fault: `DisposedHandle`
/// req: SAF-031
#[test]
fn writes_to_disposed_handles_are_skipped_with_a_fault() {
    let _ = rt().take_faults();
    let cx = rt().create_root();
    let s = cx.signal(1u32);
    let w = s.write_only();
    let v = cx.stored_value(1u32);
    cx.dispose();
    s.set(2);
    s.set_if_changed(3);
    s.update(|n| *n += 1);
    w.set(4);
    w.update(|n| *n += 1);
    v.set(5);
    assert_eq!(take(FaultKind::DisposedHandle), 6);
}

/// fault: `ReentrantAccess`
/// req: SAF-031
#[test]
fn writes_inside_a_read_of_the_same_value_are_skipped_with_a_fault() {
    let _ = rt().take_faults();
    let cx = rt().create_root();
    let s = cx.signal(1u32);
    s.with(|_| s.set(2));
    s.with(|_| s.update(|n| *n = 3));
    assert_eq!(s.get(), 1, "the writes were skipped");
    let v = cx.stored_value(1u32);
    v.with(|_| v.set(9));
    assert_eq!(v.get(), 1);
    assert_eq!(take(FaultKind::ReentrantAccess), 3);
    cx.dispose();
}

#[test]
fn creation_in_a_disposed_scope_returns_dead_handles_with_a_fault() {
    let _ = rt().take_faults();
    let cx = rt().create_root();
    cx.dispose();
    let s = cx.signal(1u32);
    let v = cx.stored_value(1u32);
    let m = cx.memo(|| 1u32);
    let ran = Rc::new(Cell::new(false));
    let r = ran.clone();
    let e = cx.effect(move || r.set(true));
    let child = cx.child();
    let grandchild = child.child();
    assert!(!s.is_alive() && !v.is_alive() && !m.is_alive() && !e.is_alive());
    assert!(!child.is_alive() && !grandchild.is_alive());
    assert!(!ran.get(), "the effect never ran");
    assert_eq!(take(FaultKind::DisposedHandle), 6);
    // A dead handle behaves like a disposed one: writes are skipped, `try_*` reads fail.
    s.set(2);
    assert!(s.try_get().is_err());
    assert_eq!(take(FaultKind::DisposedHandle), 1);
    assert_eq!(rt().stats().nodes, 0);
}

#[test]
fn map_of_a_disposed_signal_returns_a_dead_memo() {
    let _ = rt().take_faults();
    let cx = rt().create_root();
    let s = cx.signal(2u32);
    cx.dispose();
    let m = s.map(|n| n * 2);
    assert!(!m.is_alive());
    assert_eq!(take(FaultKind::DisposedHandle), 1);
}

/// fault: `MemoCycle`
/// req: SAF-031
#[test]
fn memo_cycle_returns_the_previous_value_with_a_fault() {
    let _ = rt().take_faults();
    let cx = rt().create_root();
    let feedback = cx.signal(false);
    let slot: Rc<Cell<Option<Memo<u32>>>> = Rc::new(Cell::new(None));
    let sl = slot.clone();
    let m = cx.memo(move || {
        if feedback.get() {
            // Reads itself: the cycle yields the previous value (10).
            sl.get().map_or(0, |m| m.get()) + 1
        } else {
            10
        }
    });
    slot.set(Some(m));
    assert_eq!(m.get(), 10);
    feedback.set(true);
    assert_eq!(m.get(), 11);
    assert_eq!(take(FaultKind::MemoCycle), 1);
    cx.dispose();
}

#[test]
fn memo_cycle_on_the_first_computation_raises_the_fault_then_panics() {
    let _ = rt().take_faults();
    let cx = rt().create_root();
    let slot: Rc<Cell<Option<Memo<u32>>>> = Rc::new(Cell::new(None));
    let sl = slot.clone();
    let m = cx.memo(move || sl.get().map_or(0, |m| m.get()) + 1);
    slot.set(Some(m));
    let r = catch_unwind(AssertUnwindSafe(|| m.get()));
    assert!(r.is_err(), "no previous value to return");
    assert_eq!(take(FaultKind::MemoCycle), 1);
    cx.dispose();
}

#[test]
fn read_of_a_disposed_handle_calls_the_fault_hook_before_panicking() {
    thread_local! {
        static HOOKED: Cell<u32> = const { Cell::new(0) };
    }
    fn hook(kind: FaultKind, n: u32) {
        if kind == FaultKind::DisposedHandle {
            HOOKED.with(|h| h.set(h.get() + n));
        }
    }
    let _ = rt().take_faults();
    rt().set_fault_hook(Some(hook));
    let cx = rt().create_root();
    let s = cx.signal(1u32);
    cx.dispose();
    let r = catch_unwind(AssertUnwindSafe(|| s.get()));
    rt().set_fault_hook(None);
    let msg = r
        .unwrap_err()
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(msg.contains("signal used after its scope was disposed"), "{msg}");
    assert_eq!(HOOKED.with(Cell::get), 1, "the hook ran before the panic");
    assert_eq!(take(FaultKind::DisposedHandle), 1);
}

/// 65 535 live nodes fill the arena: further creations are skipped with a `Capacity` fault.
/// Not under Miri (65 535 allocations).
#[cfg(not(miri))]
#[test]
fn full_node_arena_returns_dead_handles_with_a_capacity_fault() {
    struct Flag(Rc<Cell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let _ = rt().take_faults();
    let cx = rt().create_root();
    let live = rt().stats().nodes;
    let room = 65_535 - live;
    for _ in 0..room {
        let _ = cx.stored_value(());
    }
    let dropped = Rc::new(Cell::new(false));
    let s = cx.signal(Flag(dropped.clone()));
    assert!(!s.is_alive());
    assert!(dropped.get(), "the refused value was dropped");
    assert_eq!(take(FaultKind::Capacity), 1);
    assert!(cx.try_signal(0).is_err());
    cx.dispose();
    let again = rt().create_root();
    assert!(again.signal(1).is_alive(), "room again after the dispose");
    again.dispose();
}

/// R5.S03: a small `Limits` bound behaves like the full arena, with the bound's own code
/// (nodes and scopes counted apart). Runs under Miri (a handful of nodes).
#[test]
fn limit_bound_creations_return_dead_handles_with_their_capacity_code() {
    use twine_core::Limits;
    use twine_core::fault::CapacityFault;
    let _ = rt().take_faults();
    let _ = rt().take_capacity_faults();
    // Preallocated: the safety profile (which implies `no-panic-runtime`) accepts nothing else.
    let limits = Limits::DEFAULT
        .max_reactive_nodes(2)
        .max_scopes(2)
        .max_pending_effects(4)
        .preallocated();
    rt().set_limits(&limits).unwrap();
    let cx = rt().create_root();
    let _a = cx.signal(1);
    let _b = cx.signal(2);
    assert!(!cx.signal(3).is_alive());
    assert!(!cx.memo(|| 4).is_alive());
    let _child = cx.child();
    assert!(!cx.child().is_alive());
    assert_eq!(take(FaultKind::Capacity), 3);
    let by_bound = rt().take_capacity_faults();
    assert_eq!(by_bound[0], (CapacityFault::ReactiveNodes, 2));
    assert_eq!(by_bound[1], (CapacityFault::ReactiveScopes, 1));
    cx.dispose();
    // The runtime is per test thread; restore the default where the profile accepts it.
    if !twine_core::SAFETY_PROFILE {
        rt().set_limits(&Limits::DEFAULT).unwrap();
    }
}
