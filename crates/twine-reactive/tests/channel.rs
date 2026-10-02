//! `Channel`, `UiWaker`, `Scope::on_message`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};

use twine_reactive::{Channel, FaultKind, Overflow, Runtime, UiWaker, WakerLease};

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

struct CountWaker(AtomicUsize);

impl Wake for CountWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn count_waker() -> (Arc<CountWaker>, Waker) {
    let c = Arc::new(CountWaker(AtomicUsize::new(0)));
    (c.clone(), Waker::from(c))
}

/// A fresh `'static` channel (each expansion is its own `static`; tests run in parallel).
macro_rules! static_channel {
    ($t:ty, $n:expr) => {{
        static CH: Channel<$t, $n> = Channel::new();
        &CH
    }};
}

#[test]
fn channel_send_recv_fifo() {
    let ch: Channel<u32, 4> = Channel::new();
    for i in 0..4 {
        ch.try_send(i).unwrap();
    }
    assert_eq!(ch.len(), 4);
    for i in 0..4 {
        assert_eq!(ch.try_recv(), Some(i));
    }
    assert_eq!(ch.try_recv(), None);
    assert!(ch.is_empty());
}

#[test]
fn channel_full_channel_returns_err_and_counts_dropped() {
    let ch: Channel<u8, 2> = Channel::new();
    ch.try_send(1).unwrap();
    ch.try_send(2).unwrap();
    assert_eq!(ch.try_send(3), Err(3));
    assert_eq!(ch.try_send(4), Err(4));
    assert_eq!(ch.take_dropped(), 2);
    assert_eq!(ch.take_dropped(), 0);
    assert_eq!(ch.try_recv(), Some(1));
}

#[test]
fn channel_drop_newest_keeps_the_queued_values() {
    let ch: Channel<u8, 2> = Channel::new().on_full(Overflow::DropNewest);
    assert_eq!(ch.overflow(), Overflow::DropNewest);
    assert_eq!(
        Channel::<u8, 2>::new().overflow(),
        Overflow::DropNewest,
        "the default"
    );
    ch.waker().take();
    ch.try_send(1).unwrap();
    ch.try_send(2).unwrap();
    assert!(ch.waker().take());
    assert_eq!(ch.try_send(3), Err(3));
    assert!(!ch.waker().is_set(), "a refused value does not wake the UI");
    assert_eq!(ch.take_dropped(), 1);
    assert_eq!(
        (ch.try_recv(), ch.try_recv(), ch.try_recv()),
        (Some(1), Some(2), None)
    );
}

#[test]
fn channel_drop_oldest_keeps_the_newest_values() {
    static CH: Channel<u8, 3> = Channel::new().on_full(Overflow::DropOldest);
    assert_eq!(CH.overflow(), Overflow::DropOldest);
    for i in 0..10 {
        assert_eq!(CH.try_send(i), Ok(()), "a DropOldest channel always accepts");
    }
    assert!(CH.waker().take(), "every accepted value wakes the UI");
    assert_eq!(CH.take_dropped(), 7);
    assert_eq!(CH.len(), 3);
    assert_eq!(
        (CH.try_recv(), CH.try_recv(), CH.try_recv()),
        (Some(7), Some(8), Some(9))
    );
}

#[test]
fn channel_drop_oldest_drops_the_evicted_value_outside_the_queue() {
    // The evicted value is dropped by `try_send` (in the sender's context), exactly once.
    let token = Rc::new(());
    let ch: Channel<Rc<()>, 1> = Channel::new().on_full(Overflow::DropOldest);
    ch.try_send(token.clone()).unwrap();
    assert_eq!(Rc::strong_count(&token), 2);
    ch.try_send(Rc::new(())).unwrap();
    assert_eq!(Rc::strong_count(&token), 1, "the oldest value was dropped");
}

#[test]
fn channel_overflow_faults_count_lost_values_for_both_policies() {
    let _ = rt().take_faults();
    let newest = static_channel!(u8, 2);
    let oldest: &'static Channel<u8, 2> = {
        static CH: Channel<u8, 2> = Channel::new().on_full(Overflow::DropOldest);
        &CH
    };
    let cx = rt().create_root();
    let got = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    cx.on_message(newest, move |v| g.borrow_mut().push(v));
    let g = got.clone();
    cx.on_message(oldest, move |v| g.borrow_mut().push(v + 100));
    for i in 0..5 {
        let _ = newest.try_send(i);
        let _ = oldest.try_send(i);
    }
    rt().drain_channels(16);
    assert_eq!(*got.borrow(), [0, 1, 103, 104], "first two kept vs last two kept");
    // 3 refused + 3 evicted, reported as the unchanged `ChannelOverflow` fault.
    assert_eq!(rt().take_faults().get(FaultKind::ChannelOverflow), 6);
    cx.dispose();
}

#[test]
fn channel_send_wakes_registered_waker() {
    let ch: Channel<u8, 4> = Channel::new();
    let (count, waker) = count_waker();
    assert!(!ch.waker().is_set());
    ch.waker().register(&waker);
    ch.waker().register(&waker); // same waker: kept
    ch.try_send(1).unwrap();
    ch.try_send(2).unwrap();
    assert_eq!(count.0.load(Ordering::SeqCst), 2);
    assert!(ch.waker().take());
    assert!(!ch.waker().take());
    // A failed send does not wake.
    ch.try_send(3).unwrap();
    ch.try_send(4).unwrap();
    assert!(ch.try_send(5).is_err());
    assert_eq!(count.0.load(Ordering::SeqCst), 4);
    // Replacing the waker.
    let (count2, waker2) = count_waker();
    ch.waker().register(&waker2);
    ch.waker().wake();
    assert_eq!(count2.0.load(Ordering::SeqCst), 1);
    assert_eq!(count.0.load(Ordering::SeqCst), 4);
}

#[test]
fn channel_on_message_drains_into_signal_and_triggers_effect() {
    let ch = static_channel!(i32, 8);
    let cx = rt().create_root();
    let temp = cx.signal(0);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.effect(move || s.borrow_mut().push(temp.get()));
    cx.on_message(ch, move |v| temp.set(v));
    ch.try_send(20).unwrap();
    ch.try_send(21).unwrap();
    assert!(rt().any_channel_pending());
    assert_eq!(rt().drain_channels(16), 2);
    assert!(!rt().any_channel_pending());
    // Both messages were handled inside one batch: the effect ran once, with the last value.
    assert_eq!(*seen.borrow(), [0, 21]);
    assert_eq!(rt().drain_channels(16), 0);
}

#[test]
fn channel_drain_respects_max_per_channel() {
    let ch1 = static_channel!(u8, 8);
    let ch2 = static_channel!(u8, 8);
    let cx = rt().create_root();
    let got = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    cx.on_message(ch1, move |v| g.borrow_mut().push(v));
    let g = got.clone();
    cx.on_message(ch2, move |v| g.borrow_mut().push(v + 100));
    for i in 0..5 {
        ch1.try_send(i).unwrap();
        ch2.try_send(i).unwrap();
    }
    assert_eq!(rt().drain_channels(2), 4);
    assert_eq!(*got.borrow(), [0, 1, 100, 101]);
    assert_eq!(rt().drain_channels(10), 6);
    assert_eq!(got.borrow().len(), 10);
}

#[test]
fn channel_dispose_scope_unregisters_channel() {
    let ch = static_channel!(u8, 4);
    let root = rt().create_root();
    let child = root.child();
    let n = Rc::new(Cell::new(0));
    let c = n.clone();
    child.on_message(ch, move |_| c.set(c.get() + 1));
    assert_eq!(rt().stats().channels, 1);
    child.dispose();
    assert_eq!(rt().stats().channels, 0);
    ch.try_send(1).unwrap();
    assert_eq!(rt().drain_channels(8), 0);
    assert_eq!(n.get(), 0);
    assert!(!rt().any_channel_pending());
    // Registering on a dead scope is ignored.
    child.on_message(ch, |_| panic!("never"));
    assert_eq!(rt().stats().channels, 0);
}

#[test]
fn channel_drain_reentrancy_registering_inside_handler() {
    let ch = static_channel!(u8, 4);
    let ch2 = static_channel!(u8, 4);
    let root = rt().create_root();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let registered = Rc::new(Cell::new(false));
    let child = root.child();
    child.on_message(ch, move |v| {
        l.borrow_mut().push(format!("ch {v}"));
        if !registered.get() {
            registered.set(true);
            // Nested drain is safe (the running handler is skipped).
            assert_eq!(rt().drain_channels(8), 0);
            let l2 = l.clone();
            // R1: registering a new handler from inside a handler.
            root.on_message(ch2, move |v| l2.borrow_mut().push(format!("ch2 {v}")));
        }
        if v == 9 {
            child.dispose(); // unregisters this very handler while it runs
        }
    });
    ch.try_send(1).unwrap();
    ch2.try_send(7).unwrap();
    assert_eq!(
        rt().drain_channels(8),
        1,
        "the new registration is served next call"
    );
    assert_eq!(rt().drain_channels(8), 1);
    assert_eq!(*log.borrow(), ["ch 1", "ch2 7"]);
    ch.try_send(9).unwrap();
    ch.try_send(10).unwrap();
    assert_eq!(rt().drain_channels(1), 1);
    assert!(!child.is_alive());
    assert_eq!(rt().stats().channels, 1);
    assert_eq!(rt().drain_channels(8), 0, "ch is no longer registered");
}

#[test]
fn channel_root_waker_reaches_every_registered_channel() {
    static W: UiWaker = UiWaker::new();
    let ch1 = static_channel!(u8, 2);
    let ch2 = static_channel!(u8, 2);
    let cx = rt().create_root();
    cx.on_message(ch1, |_| {});
    cx.child().on_message(ch2, |_| {});
    let (count, waker) = count_waker();
    W.register(&waker);
    cx.set_ui_waker(&W);
    ch1.try_send(1).unwrap();
    ch2.try_send(1).unwrap();
    assert_eq!(count.0.load(Ordering::SeqCst), 2);
    assert!(W.take());
    cx.dispose();
}

#[test]
fn channel_multi_thread_producers() {
    static CH: Channel<u32, 64> = Channel::new();
    const PRODUCERS: u32 = 4;
    // Fewer messages under Miri (it interprets every instruction and thread switch).
    const PER: u32 = if cfg!(miri) { 50 } else { 1000 };
    let producers: Vec<_> = (0..PRODUCERS)
        .map(|p| {
            std::thread::spawn(move || {
                for i in 0..PER {
                    let mut v = (p << 16) | i;
                    while let Err(back) = CH.try_send(v) {
                        v = back;
                        std::thread::yield_now();
                    }
                }
            })
        })
        .collect();
    let cx = rt().create_root();
    let last = Rc::new(RefCell::new([None::<u32>; PRODUCERS as usize]));
    let received = cx.signal(0u32);
    let l = last.clone();
    cx.on_message(&CH, move |v| {
        let (p, i) = ((v >> 16) as usize, v & 0xffff);
        let prev = l.borrow()[p];
        assert_eq!(prev.map_or(0, |x| x + 1), i, "per-producer order preserved");
        l.borrow_mut()[p] = Some(i);
        received.update(|n| *n += 1);
    });
    while received.get_untracked() < PRODUCERS * PER {
        if rt().drain_channels(16) == 0 {
            std::thread::yield_now();
        }
    }
    for p in producers {
        p.join().unwrap();
    }
    assert_eq!(received.get(), PRODUCERS * PER);
    assert!(last.borrow().iter().all(|l| *l == Some(PER - 1)));
    // Some sends were retried (dropped) — the counter is consistent with that.
    let _ = CH.take_dropped();
}

#[test]
fn channel_is_const_constructible_in_static() {
    static CH: Channel<(u8, u16), 3> = Channel::new();
    static W: UiWaker = UiWaker::new();
    const fn build() -> Channel<u8, 1> {
        Channel::new()
    }
    fn assert_sync<T: Sync>(_: &T) {}
    let local = build();
    assert!(local.is_empty());
    CH.try_send((1, 2)).unwrap();
    assert_eq!(CH.try_recv(), Some((1, 2)));
    W.wake();
    assert!(W.take());
    assert_sync(&CH);
    assert_sync(&W);
}

#[test]
fn channel_root_waker_reaches_channels_registered_later() {
    static W: UiWaker = UiWaker::new();
    let ch: &'static Channel<u8, 2> = static_channel!(u8, 2);
    let cx = rt().create_root();
    cx.set_ui_waker(&W);
    let (count, waker) = count_waker();
    W.register(&waker);
    cx.child().child().on_message(ch, |_| {});
    ch.try_send(1).unwrap();
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    assert!(W.take());
    cx.dispose();
    assert!(cx.ui_waker().is_none());
}

#[test]
fn channel_each_root_wakes_only_its_own_waker() {
    static W1: UiWaker = UiWaker::new();
    static W2: UiWaker = UiWaker::new();
    let ch1 = static_channel!(u8, 2);
    let ch2 = static_channel!(u8, 2);
    let a = rt().create_root();
    let b = rt().create_root();
    a.set_ui_waker(&W1);
    b.set_ui_waker(&W2);
    a.on_message(ch1, |_| {});
    b.child().on_message(ch2, |_| {});
    ch1.try_send(1).unwrap();
    assert!(W1.take());
    assert!(!W2.is_set());
    ch2.try_send(1).unwrap();
    assert!(W2.take());
    assert!(!W1.is_set());
    assert!(a.ui_waker().is_some_and(|w| core::ptr::addr_eq(w, &raw const W1)));
    assert!(
        b.child()
            .ui_waker()
            .is_some_and(|w| core::ptr::addr_eq(w, &raw const W2))
    );
    a.dispose();
    b.dispose();
}

#[test]
fn channel_disposed_handler_stops_waking_its_ui() {
    static W: UiWaker = UiWaker::new();
    let ch = static_channel!(u8, 4);
    let root = rt().create_root();
    root.set_ui_waker(&W);
    let row1 = root.child();
    let row2 = root.child();
    row1.on_message(ch, |_| {});
    row2.on_message(ch, |_| {});
    // One of two handlers gone: the channel still wakes the UI.
    row1.dispose();
    ch.try_send(1).unwrap();
    assert!(W.take());
    // The last one gone: it does not.
    row2.dispose();
    ch.try_send(2).unwrap();
    assert!(!W.is_set());
    root.dispose();
}

#[test]
fn channel_set_ui_waker_on_child_scope_is_ignored() {
    static W: UiWaker = UiWaker::new();
    let root = rt().create_root();
    root.child().set_ui_waker(&W);
    assert!(root.ui_waker().is_none());
    root.dispose();
    root.set_ui_waker(&W); // dead: ignored
    assert!(root.ui_waker().is_none());
}

#[test]
fn channel_send_from_another_thread_wakes_the_owning_root() {
    static W1: UiWaker = UiWaker::new();
    static W2: UiWaker = UiWaker::new();
    static CH1: Channel<u32, 4> = Channel::new();
    static CH2: Channel<u32, 4> = Channel::new();
    let a = rt().create_root();
    let b = rt().create_root();
    a.set_ui_waker(&W1);
    b.set_ui_waker(&W2);
    a.on_message(&CH1, |_| {});
    b.on_message(&CH2, |_| {});
    // An "interrupt handler" on another thread: only `Channel` is touched there.
    std::thread::spawn(|| CH2.try_send(7).unwrap()).join().unwrap();
    assert!(W2.take());
    assert!(!W1.is_set());
    assert_eq!(rt().drain_channels(4), 1);
    a.dispose();
    b.dispose();
}

#[test]
fn waker_lease_reuses_slots_and_resets_them() {
    let first = WakerLease::STATIC_SLOTS + 2; // static slots and heap slots
    let leases: Vec<_> = (0..first).map(|_| UiWaker::lease()).collect();
    for (i, l) in leases.iter().enumerate() {
        for m in &leases[i + 1..] {
            assert!(!core::ptr::eq(l.get(), m.get()), "leases are distinct");
        }
        l.get().wake();
    }
    let ptrs: Vec<*const UiWaker> = leases.iter().map(|l| core::ptr::from_ref(l.get())).collect();
    drop(leases);
    // Returned wakers are handed out again, cleared.
    let again: Vec<_> = (0..first).map(|_| UiWaker::lease()).collect();
    for l in &again {
        assert!(!l.get().is_set());
    }
    let reused = again
        .iter()
        .filter(|l| ptrs.contains(&core::ptr::from_ref(l.get())))
        .count();
    // Other tests of this binary lease concurrently; most slots come back to us.
    assert!(reused >= 1);
}
