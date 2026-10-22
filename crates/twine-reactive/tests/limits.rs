//! R5.S03: the runtime's share of `Limits` — every bound refuses (an error or a `Capacity`
//! fault with its precise code) instead of growing, preallocation reserves the bounded
//! storage, and creations after a seal are noted by the allocation guard.
//!
//! req: SAF-008

use std::cell::Cell;
use std::rc::Rc;

use twine_core::alloc_guard::{self, AllocSite};
use twine_core::fault::{CapacityFault, FaultKind};
use twine_core::{Error, Limits, Partition};
use twine_reactive::{HandleKind, ReactiveError, Runtime};

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

#[test]
fn invalid_limits_are_refused_and_nothing_is_applied() {
    let rt = rt();
    assert_eq!(
        rt.set_limits(&Limits::DEFAULT.max_scopes(0)),
        Err(Error::InvalidArgument("Limits::max_scopes must be >= 1"))
    );
    let cx = rt.create_root();
    assert!(cx.try_child().is_ok(), "the default bound is still in force");
    cx.dispose();
}

#[test]
fn node_limit_refuses_creation_without_growth() {
    let rt = rt();
    rt.set_limits(&Limits::DEFAULT.max_reactive_nodes(4).preallocated())
        .unwrap();
    let cx = rt.create_root();
    let arenas = rt.memory().arenas;
    let kept: Vec<_> = (0..4).map(|i| cx.signal(i)).collect();
    assert_eq!(
        cx.try_signal(9).err(),
        Some(ReactiveError::Capacity(HandleKind::Signal))
    );
    assert_eq!(
        cx.try_memo(|| 1).err(),
        Some(ReactiveError::Capacity(HandleKind::Memo))
    );
    assert_eq!(rt.memory().arenas, arenas, "the preallocated arenas did not grow");
    assert_eq!(rt.stats().nodes, 4);
    // A disposed node makes room again.
    let child = cx.child();
    drop(kept);
    cx.dispose();
    assert!(!child.is_alive());
    let cx = rt.create_root();
    assert!(cx.try_signal(1).is_ok());
    cx.dispose();
}

#[test]
fn scope_limit_refuses_children() {
    let rt = rt();
    // Preallocated: the safety profile accepts nothing else.
    let limits = Limits::DEFAULT
        .max_scopes(3)
        .max_reactive_nodes(16)
        .max_pending_effects(16)
        .preallocated();
    rt.set_limits(&limits).unwrap();
    let root = rt.create_root();
    let _a = root.child();
    let _b = root.child();
    assert_eq!(root.try_child(), Err(ReactiveError::Capacity(HandleKind::Scope)));
    assert_eq!(
        rt.try_create_root().err(),
        Some(ReactiveError::Capacity(HandleKind::Scope))
    );
    // `try_*` reports no fault.
    assert!(rt.take_capacity_faults().iter().all(|&(_, n)| n == 0));
    root.dispose();
    assert!(rt.try_create_root().is_ok_and(|s| {
        s.dispose();
        true
    }));
}

#[test]
fn pending_effect_limit_raises_a_capacity_fault_with_its_code() {
    let rt = rt();
    rt.set_limits(&Limits::DEFAULT.max_pending_effects(2).preallocated())
        .unwrap();
    let cx = rt.create_root();
    let s = cx.signal(0u32);
    let runs = Rc::new(Cell::new(0u32));
    for _ in 0..3 {
        let r = runs.clone();
        cx.effect(move || {
            s.get();
            r.set(r.get() + 1);
        });
    }
    assert_eq!(runs.get(), 3, "each ran once when created");
    let _ = rt.take_faults();
    let queues = rt.memory().queues;
    // One write queues three effects: the third does not fit.
    s.set(1);
    assert_eq!(runs.get(), 5, "the two queued effects ran");
    let faults = rt.take_faults();
    assert_eq!(faults.get(FaultKind::Capacity), 1);
    let by_bound = rt.take_capacity_faults();
    assert_eq!(by_bound[2], (CapacityFault::PendingEffects, 1));
    assert_eq!(by_bound[0].1 + by_bound[1].1, 0);
    assert_eq!(rt.memory().queues, queues, "the effect queue did not grow");
    cx.dispose();
}

#[test]
fn preallocated_runtime_creates_within_limits_without_growing_its_arenas() {
    let rt = rt();
    rt.set_limits(
        &Limits::DEFAULT
            .max_reactive_nodes(64)
            .max_scopes(16)
            .max_pending_effects(16)
            .preallocated(),
    )
    .unwrap();
    let before = rt.memory();
    let cx = rt.create_root();
    for _ in 0..8 {
        let page = cx.child();
        let a = page.signal(1);
        let b = page.memo(move || a.get() * 2);
        page.effect(move || {
            b.get();
        });
        a.set(2);
        page.dispose();
    }
    assert_eq!(rt.memory().arenas, before.arenas);
    cx.dispose();
}

#[test]
fn creations_after_a_seal_are_noted() {
    let rt = rt();
    let cx = rt.create_root();
    let s = cx.signal(0);
    alloc_guard::seal(Partition::MAIN);
    s.set(1); // a write allocates nothing
    assert_eq!(alloc_guard::total(Partition::MAIN), 0);
    let _t = cx.signal(1);
    assert_eq!(
        alloc_guard::take(Partition::MAIN).map(|r| r.last),
        Some(AllocSite::ReactiveNode)
    );
    cx.provide(5u8);
    assert_eq!(
        alloc_guard::take(Partition::MAIN).map(|r| r.last),
        Some(AllocSite::ReactiveContext)
    );
    let captured = 7u32;
    cx.on_cleanup(move || {
        let _ = captured;
    });
    assert_eq!(
        alloc_guard::take(Partition::MAIN).map(|r| r.last),
        Some(AllocSite::ReactiveCleanup)
    );
    alloc_guard::unseal(Partition::MAIN);
    cx.dispose();
}
