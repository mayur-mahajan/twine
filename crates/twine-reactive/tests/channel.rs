//! P07.S07: `Channel`, `UiWaker`, `Scope::on_message`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};

use twine_reactive::{
    Channel, UiWaker, any_channel_pending, create_root, debug_stats, drain_channels, register_waker,
};

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
    let cx = create_root();
    let temp = cx.signal(0);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.effect(move || s.borrow_mut().push(temp.get()));
    cx.on_message(ch, move |v| temp.set(v));
    ch.try_send(20).unwrap();
    ch.try_send(21).unwrap();
    assert!(any_channel_pending());
    assert_eq!(drain_channels(16), 2);
    assert!(!any_channel_pending());
    // Both messages were handled inside one batch: the effect ran once, with the last value.
    assert_eq!(*seen.borrow(), [0, 21]);
    assert_eq!(drain_channels(16), 0);
}

#[test]
fn channel_drain_respects_max_per_channel() {
    let ch1 = static_channel!(u8, 8);
    let ch2 = static_channel!(u8, 8);
    let cx = create_root();
    let got = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    cx.on_message(ch1, move |v| g.borrow_mut().push(v));
    let g = got.clone();
    cx.on_message(ch2, move |v| g.borrow_mut().push(v + 100));
    for i in 0..5 {
        ch1.try_send(i).unwrap();
        ch2.try_send(i).unwrap();
    }
    assert_eq!(drain_channels(2), 4);
    assert_eq!(*got.borrow(), [0, 1, 100, 101]);
    assert_eq!(drain_channels(10), 6);
    assert_eq!(got.borrow().len(), 10);
}

#[test]
fn channel_dispose_scope_unregisters_channel() {
    let ch = static_channel!(u8, 4);
    let root = create_root();
    let child = root.child();
    let n = Rc::new(Cell::new(0));
    let c = n.clone();
    child.on_message(ch, move |_| c.set(c.get() + 1));
    assert_eq!(debug_stats().channels, 1);
    child.dispose();
    assert_eq!(debug_stats().channels, 0);
    ch.try_send(1).unwrap();
    assert_eq!(drain_channels(8), 0);
    assert_eq!(n.get(), 0);
    assert!(!any_channel_pending());
    // Registering on a dead scope is ignored.
    child.on_message(ch, |_| panic!("never"));
    assert_eq!(debug_stats().channels, 0);
}

#[test]
fn channel_drain_reentrancy_registering_inside_handler() {
    let ch = static_channel!(u8, 4);
    let ch2 = static_channel!(u8, 4);
    let root = create_root();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let registered = Rc::new(Cell::new(false));
    let child = root.child();
    child.on_message(ch, move |v| {
        l.borrow_mut().push(format!("ch {v}"));
        if !registered.get() {
            registered.set(true);
            // Nested drain is safe (the running handler is skipped).
            assert_eq!(drain_channels(8), 0);
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
    assert_eq!(drain_channels(8), 1, "the new registration is served next call");
    assert_eq!(drain_channels(8), 1);
    assert_eq!(*log.borrow(), ["ch 1", "ch2 7"]);
    ch.try_send(9).unwrap();
    ch.try_send(10).unwrap();
    assert_eq!(drain_channels(1), 1);
    assert!(!child.is_alive());
    assert_eq!(debug_stats().channels, 1);
    assert_eq!(drain_channels(8), 0, "ch is no longer registered");
}

#[test]
fn channel_register_waker_reaches_every_registered_channel() {
    let ch1 = static_channel!(u8, 2);
    let ch2 = static_channel!(u8, 2);
    let cx = create_root();
    cx.on_message(ch1, |_| {});
    cx.on_message(ch2, |_| {});
    let (count, waker) = count_waker();
    register_waker(&waker);
    ch1.try_send(1).unwrap();
    ch2.try_send(1).unwrap();
    assert_eq!(count.0.load(Ordering::SeqCst), 2);
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
    let cx = create_root();
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
        if drain_channels(16) == 0 {
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
