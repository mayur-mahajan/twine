//! Partitioned runtimes (R5.S07, ADR-001): one independent reactive runtime per partition.
//!
//! Built with `--features partitions` (`cargo xtask ci` stage `test-features`); without the
//! feature there is one partition and this file is empty.
//!
//! req: SAF-034
#![cfg(feature = "partitions")]

use std::cell::Cell;
use std::rc::Rc;

use twine_core::alloc_guard::{self, AllocSite};
use twine_core::fault::CapacityFault;
use twine_core::{Limits, Partition};
use twine_reactive::{Channel, FaultKind, HandleKind, Latest, ReactiveError, Runtime};

/// Two distinct partitions.
fn ab() -> (Runtime, Runtime) {
    let [a, b, ..] = Partition::ALL;
    (
        Runtime::current_thread_partition(a),
        Runtime::current_thread_partition(b),
    )
}

#[test]
fn each_partition_is_taken_once_and_out_of_range_is_none() {
    assert_eq!(Partition::COUNT, 4);
    for p in Partition::ALL {
        let rt = Runtime::take_partition(p).expect("first take of the partition");
        assert_eq!(rt.partition(), p);
        assert!(Runtime::take_partition(p).is_none(), "second take of {p}");
    }
    assert!(Runtime::take().is_none(), "`take` is the main partition's claim");
    assert_eq!(Partition::new(4), None);
    assert_eq!(Partition::new(u8::MAX), None);
}

#[test]
fn claims_are_per_thread() {
    let last = Partition::ALL[Partition::COUNT - 1];
    assert!(Runtime::take_partition(last).is_some());
    let other = std::thread::spawn(move || Runtime::take_partition(last).is_some())
        .join()
        .unwrap();
    assert!(other, "another thread has its own runtimes");
}

#[test]
fn handles_report_their_partition_and_equal_keys_stay_distinct() {
    let (a, b) = ab();
    let (ca, cb) = (a.create_root(), b.create_root());
    // Fresh runtimes: the first scope and signal get the same arena key in both.
    let (sa, sb) = (ca.signal(1u32), cb.signal(2u32));
    assert_eq!(ca.partition(), a.partition());
    assert_eq!(cb.partition(), b.partition());
    assert_eq!(sa.runtime(), a);
    assert_eq!(sb.runtime(), b);
    assert_eq!(cb.runtime(), b);
    assert_ne!(ca, cb, "scopes of two partitions are distinct");
    assert_ne!(sa.read_only(), sb.read_only());
    assert_eq!((sa.get(), sb.get()), (1, 2), "each handle reads its own runtime");
    sb.set(20);
    assert_eq!((sa.get(), sb.get()), (1, 20));
    ca.dispose();
    assert!(!sa.is_alive());
    assert!(sb.is_alive(), "disposing in A never touches B");
    cb.dispose();
}

#[test]
fn arenas_and_limits_are_per_partition() {
    let (a, b) = ab();
    let small = Limits::DEFAULT.max_reactive_nodes(2).preallocated();
    b.set_limits(&small).unwrap();
    let (ca, cb) = (a.create_root(), b.create_root());
    let _b1 = cb.signal(0u8);
    let _b2 = cb.signal(0u8);
    assert_eq!(
        cb.try_signal(0u8).err(),
        Some(ReactiveError::Capacity(HandleKind::Signal)),
        "B is full"
    );
    // A keeps creating far beyond B's bound.
    let many: Vec<_> = (0..16).map(|i| ca.signal(i)).collect();
    assert_eq!(many[15].get(), 15);
    assert_eq!(a.stats().nodes, 16);
    assert_eq!(b.stats().nodes, 2);
    assert!(a.take_capacity_faults().iter().all(|&(_, n)| n == 0));
    ca.dispose();
    cb.dispose();
}

/// G1: an effect loop in B is cut by B's limit and drops only B's effects.
#[test]
fn loop_cut_in_one_partition_keeps_the_others_effects() {
    let (a, b) = ab();
    b.set_flush_iterations_limit(3);
    let (ca, cb) = (a.create_root(), b.create_root());
    let seen = Rc::new(Cell::new(0u32));
    let src = ca.signal(0u32);
    let s = seen.clone();
    ca.effect(move || s.set(src.get()));
    let spin = cb.signal(0u32);
    cb.effect(move || spin.set(spin.get() + 1)); // B's loop
    assert_eq!(b.take_faults().get(FaultKind::EffectLoopCut), 1);
    assert!(a.take_faults().kinds().is_empty(), "the cut is B's fault only");
    src.set(7);
    assert_eq!(seen.get(), 7, "A's effect still runs");
    // A's loop limit is its own (the default).
    let chain = ca.signal(0u32);
    ca.effect(move || {
        if chain.get() < 50 {
            chain.set(chain.get() + 1);
        }
    });
    assert_eq!(chain.get(), 50, "A's flush limit was not lowered by B");
    assert!(a.take_faults().kinds().is_empty());
    ca.dispose();
    cb.dispose();
}

thread_local! {
    static HOOK_A: Cell<u32> = const { Cell::new(0) };
    static HOOK_B: Cell<u32> = const { Cell::new(0) };
}

fn hook_a(_: FaultKind, n: u32) {
    HOOK_A.with(|c| c.set(c.get() + n));
}

fn hook_b(_: FaultKind, n: u32) {
    HOOK_B.with(|c| c.set(c.get() + n));
}

/// G4/G9: faults, fault counts and fault hooks are per partition.
#[test]
fn faults_and_hooks_are_per_partition() {
    let (a, b) = ab();
    a.set_fault_hook(Some(hook_a));
    b.set_fault_hook(Some(hook_b));
    b.set_flush_iterations_limit(2);
    let cb = b.create_root();
    let s = cb.signal(0u32);
    cb.effect(move || s.set(s.get() + 1));
    assert_eq!(HOOK_B.with(Cell::get), 1);
    assert_eq!(HOOK_A.with(Cell::get), 0, "A's hook never sees B's fault");
    assert_eq!(b.stats().faults.get(FaultKind::EffectLoopCut), 1);
    assert_eq!(a.stats().faults.get(FaultKind::EffectLoopCut), 0);
    a.set_fault_hook(None);
    b.set_fault_hook(None);
    cb.dispose();
}

/// G5: seal domains are per partition at the runtime's growth points.
#[test]
fn growth_after_seal_is_noted_in_the_owning_partition() {
    let (a, b) = ab();
    let (pa, pb) = (a.partition(), b.partition());
    alloc_guard::seal(pa);
    let cb = b.create_root();
    let _x = cb.signal(1u32); // B grows: not A's evidence
    assert_eq!(alloc_guard::take(pa), None);
    assert_eq!(alloc_guard::total(pa), 0);
    let ca = a.create_root();
    let _y = ca.signal(2u32);
    let noted = alloc_guard::take(pa).expect("A's growth is A's evidence");
    assert!(noted.count >= 1);
    assert!(matches!(
        noted.last,
        AllocSite::ReactiveNode | AllocSite::ReactiveScope | AllocSite::ArenaSlots
    ));
    assert_eq!(alloc_guard::take(pb), None, "B was never sealed");
    alloc_guard::unseal(pa);
    ca.dispose();
    cb.dispose();
}

static OWNED_CH: Channel<u8, 4> = Channel::new();
static OWNED_LATEST: Latest<u8> = Latest::new(0);

/// § 5.5: a `Channel` / `Latest` has one consuming partition.
///
/// fault: `PartitionViolation`
#[test]
fn a_second_partition_is_refused_as_consumer() {
    let (a, b) = ab();
    let (ca, cb) = (a.create_root(), b.create_root());
    let got = ca.signal(0u8);
    ca.on_message(&OWNED_CH, move |v| got.set(v));
    let shown = ca.watch(&OWNED_LATEST);
    // B tries to consume the same sources.
    let stolen = cb.signal(0u8);
    cb.on_message(&OWNED_CH, move |v| stolen.set(v));
    let b_view = cb.watch(&OWNED_LATEST);
    cb.on_latest(&OWNED_LATEST, |_| {});
    assert_eq!(b.take_faults().get(FaultKind::PartitionViolation), 3);
    assert!(a.take_faults().kinds().is_empty());
    assert_eq!(b.stats().channels, 0, "no registration in B");
    OWNED_CH.try_send(5).unwrap();
    OWNED_LATEST.set(9);
    assert_eq!(b.drain_channels(8), 0);
    assert_eq!(a.drain_channels(8), 2);
    assert_eq!((got.get(), shown.get()), (5, 9));
    assert_eq!((stolen.get(), b_view.get()), (0, 0), "B never consumes");
    // The owner may register again (another handler of the same partition).
    ca.on_message(&OWNED_CH, |_| {});
    assert!(a.take_faults().kinds().is_empty());
    ca.dispose();
    cb.dispose();
}

/// Two threads, each running two partitions: every thread's runtimes are its own.
#[test]
fn two_threads_with_two_partitions_each() {
    let run = || {
        let (a, b) = ab();
        let (ca, cb) = (a.create_root(), b.create_root());
        let x = ca.signal(0u32);
        let y = cb.signal(0u32);
        let n = if cfg!(miri) { 20 } else { 2_000 };
        for i in 0..n {
            x.set(i);
            y.set(i * 2);
        }
        let r = (x.get(), y.get(), a.stats().scopes, b.stats().scopes);
        ca.dispose();
        cb.dispose();
        r
    };
    let t1 = std::thread::spawn(run);
    let t2 = std::thread::spawn(run);
    let n = if cfg!(miri) { 20 } else { 2_000 };
    for r in [t1.join().unwrap(), t2.join().unwrap()] {
        assert_eq!(r, (n - 1, (n - 1) * 2, 1, 1));
    }
}

/// N1 (documented, not a target claim): with unwinding on a hosted OS, a panic in partition
/// B's thread leaves partition A's thread running.
#[test]
#[cfg_attr(miri, ignore = "unwinding across threads is slow under Miri")]
fn hosted_panic_in_one_partition_thread_is_contained() {
    let b = std::thread::spawn(|| {
        let rt = Runtime::current_thread_partition(Partition::ALL[1]);
        let cx = rt.create_root();
        let s = cx.signal(0u32);
        cx.effect(move || assert!(s.get() != 1, "partition B fails"));
        s.set(1);
    });
    assert!(b.join().is_err());
    let a = Runtime::current_thread_partition(Partition::MAIN);
    let cx = a.create_root();
    let s = cx.signal(1u32);
    s.set(2);
    assert_eq!(s.get(), 2);
    cx.dispose();
}

/// Capacity faults keep their bound per partition.
#[test]
fn capacity_codes_are_per_partition() {
    let (a, b) = ab();
    b.set_limits(&Limits::DEFAULT.max_scopes(1).preallocated())
        .unwrap();
    let cb = b.create_root();
    assert!(cb.try_child().is_err());
    let counts = b.take_capacity_faults();
    assert_eq!(counts[1].0, CapacityFault::ReactiveScopes);
    let ca = a.create_root();
    let _ = ca.child();
    assert!(a.take_capacity_faults().iter().all(|&(_, n)| n == 0));
    ca.dispose();
    cb.dispose();
}

/// § 5.4: with `debug-checks` (host), a handle of one partition used while another partition's
/// batch runs panics.
#[cfg(feature = "debug-checks")]
#[test]
#[should_panic(expected = "a handle of one partition was used")]
fn debug_checks_catch_a_handle_used_across_partitions() {
    let (a, b) = ab();
    let ca = a.create_root();
    let sa = ca.signal(0u32);
    b.batch(|| sa.set(1));
}

#[cfg(feature = "debug-checks")]
#[test]
fn debug_checks_allow_each_partition_outside_the_others_update() {
    let (a, b) = ab();
    let (ca, cb) = (a.create_root(), b.create_root());
    let (sa, sb) = (ca.signal(0u32), cb.signal(0u32));
    a.batch(|| sa.set(1));
    b.batch(|| sb.set(2));
    sa.set(3); // outside every update: allowed
    assert_eq!((sa.get(), sb.get()), (3, 2));
    ca.dispose();
    cb.dispose();
}
