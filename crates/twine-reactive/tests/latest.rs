//! `Latest`, `Scope::watch` and `Scope::on_latest` (R3.S05): overwrite semantics, untorn
//! reads while another thread (standing in for an interrupt) writes, wake-up routing,
//! coalescing and disposal.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use twine_reactive::{Latest, Runtime, UiWaker};

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

#[test]
fn latest_overwrites_and_counts_versions() {
    let l = Latest::new(1u8);
    assert_eq!((l.get(), l.version()), (1, 0));
    l.set(2);
    l.set(3);
    assert_eq!((l.get(), l.version()), (3, 2));
    assert_eq!(l.replace(4), 3);
    assert_eq!(l.version(), 3);
    assert_eq!(Latest::<u8>::default().get(), 0);
}

#[test]
fn latest_holds_non_copy_values() {
    let l = Latest::new(String::from("idle"));
    l.set(String::from("heating"));
    assert_eq!(l.get(), "heating");
    assert_eq!(l.replace(String::from("off")), "heating");
    // The replaced value is dropped by `set`, exactly once.
    let token = Rc::new(());
    let l = Latest::new(token.clone());
    assert_eq!(Rc::strong_count(&token), 2);
    l.set(Rc::new(()));
    assert_eq!(Rc::strong_count(&token), 1);
}

#[test]
fn watch_starts_with_the_current_value_and_follows_sets() {
    static L: Latest<i16> = Latest::new(200);
    let cx = rt().create_root();
    L.set(205); // before the watch: its value is the starting one
    let temp = cx.watch(&L);
    assert_eq!(temp.get(), 205);
    assert_eq!(rt().drain_channels(16), 0, "nothing new since the watch");
    L.set(210);
    assert_eq!(rt().drain_channels(16), 1);
    assert_eq!(temp.get(), 210);
    cx.dispose();
}

#[test]
fn watch_coalesces_sets_into_one_update() {
    static L: Latest<u32> = Latest::new(0);
    let cx = rt().create_root();
    let v = cx.watch(&L);
    let runs = Rc::new(RefCell::new(Vec::new()));
    let r = runs.clone();
    cx.effect(move || r.borrow_mut().push(v.get()));
    for i in 1..=100 {
        L.set(i);
    }
    assert_eq!(rt().drain_channels(16), 1, "one delivery, not 100");
    assert_eq!(
        *runs.borrow(),
        [0, 100],
        "the effect ran once, with the newest value"
    );
    cx.dispose();
}

#[test]
fn on_latest_delivers_only_later_values() {
    static L: Latest<u8> = Latest::new(0);
    L.set(1);
    let cx = rt().create_root();
    let got = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    cx.on_latest(&L, move |v| g.borrow_mut().push(v));
    rt().drain_channels(16);
    assert!(
        got.borrow().is_empty(),
        "the value set before the registration is not delivered"
    );
    L.set(2);
    rt().drain_channels(16);
    L.set(3);
    rt().drain_channels(16);
    assert_eq!(*got.borrow(), [2, 3]);
    cx.dispose();
}

#[test]
fn regression_every_watcher_of_one_latest_sees_each_new_value() {
    // Each registration remembers the version it saw: one watcher draining the value must not
    // hide it from the others.
    static L: Latest<u8> = Latest::new(0);
    let cx = rt().create_root();
    let a = cx.watch(&L);
    let b = cx.child().watch(&L);
    L.set(7);
    assert_eq!(rt().drain_channels(16), 2);
    assert_eq!((a.get(), b.get()), (7, 7));
    cx.dispose();
}

#[test]
fn latest_set_wakes_the_ui_of_the_watching_root() {
    static L: Latest<u8> = Latest::new(0);
    static W: UiWaker = UiWaker::new();
    let root = rt().create_root();
    let _v = root.child().watch(&L);
    root.set_ui_waker(&W);
    W.take();
    L.set(1); // e.g. from an interrupt
    assert!(W.take(), "the set woke the UI");
    // Disposing the watcher's scope undoes the routing.
    root.dispose();
    L.set(2);
    assert!(!W.is_set(), "no wake-up after the watcher is gone");
    assert_eq!(rt().drain_channels(16), 0, "the registration is gone");
}

#[test]
fn on_latest_on_a_dead_scope_is_ignored() {
    static L: Latest<u8> = Latest::new(0);
    let cx = rt().create_root();
    cx.dispose();
    let before = rt().stats().channels;
    cx.on_latest(&L, |_| {});
    assert_eq!(rt().stats().channels, before, "a warning and no registration");
}

/// Iterations of the threaded tests (Miri is ~1000x slower).
const SETS: u32 = if cfg!(miri) { 50 } else { 20_000 };

#[test]
fn latest_isr_simulation_never_tears_and_the_latest_wins() {
    // A writer thread (standing in for an interrupt handler) sets 8-word values whose words
    // are all equal; the UI side reads through `watch` while it writes. Every value the UI
    // sees must be one complete value (no mix of two writes), in non-decreasing order, and
    // after the writer stops the UI holds the last one.
    static L: Latest<[u32; 8]> = Latest::new([0; 8]);
    static W: UiWaker = UiWaker::new();
    let root = rt().create_root();
    let v = root.watch(&L);
    root.set_ui_waker(&W);
    W.take();
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let writer = std::thread::spawn(move || {
        for i in 1..=SETS {
            L.set([i; 8]);
        }
        d.store(true, Ordering::Release);
    });
    let mut last = 0;
    let mut deliveries = 0u32;
    loop {
        let finished = done.load(Ordering::Acquire);
        if W.take() {
            deliveries += rt().drain_channels(16) as u32;
            let seen = v.get();
            assert!(seen.iter().all(|&w| w == seen[0]), "torn read: {seen:?}");
            assert!(seen[0] >= last, "went back from {last} to {}", seen[0]);
            last = seen[0];
        } else {
            std::thread::yield_now();
        }
        if finished && !W.is_set() {
            break;
        }
    }
    writer.join().unwrap();
    rt().drain_channels(16);
    assert_eq!(v.get(), [SETS; 8], "the latest value wins");
    assert!((1..=SETS).contains(&deliveries));
    root.dispose();
}
