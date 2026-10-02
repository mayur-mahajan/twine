//! `Outbox` (R3.S05): commands from the UI to a task — polled round trip, overflow policies,
//! the consumer's waker, and `recv().await` on a minimal test executor (no executor crate).

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;

use twine_reactive::{Outbox, Overflow, Runtime};

/// A waker that unparks the thread running [`block_on`] and counts its wake-ups.
struct ThreadWaker {
    thread: Thread,
    wakes: AtomicUsize,
}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// The simplest executor: polls `fut` on this thread, parking between polls. Returns the
/// output and the number of polls.
fn block_on<F: Future>(fut: F) -> (F::Output, usize) {
    let w = Arc::new(ThreadWaker {
        thread: std::thread::current(),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(w);
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    let mut polls = 0;
    loop {
        polls += 1;
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return (v, polls);
        }
        std::thread::park();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    SetTarget(i32),
    Stop,
}

#[test]
fn outbox_polled_round_trip_from_a_ui_handler() {
    // The UI sends from reactive code; the consumer polls.
    static OUT: Outbox<Cmd, 4> = Outbox::new();
    let cx = Runtime::current_thread().create_root();
    let target = cx.signal(200);
    cx.effect(move || {
        let _ = OUT.try_send(Cmd::SetTarget(target.get()));
    });
    target.set(215);
    OUT.try_send(Cmd::Stop).unwrap();
    assert_eq!(OUT.len(), 3);
    assert!(OUT.waker().take(), "sends wake the consumer");
    assert_eq!(OUT.try_recv(), Some(Cmd::SetTarget(200)));
    assert_eq!(OUT.try_recv(), Some(Cmd::SetTarget(215)));
    assert_eq!(OUT.try_recv(), Some(Cmd::Stop));
    assert_eq!(OUT.try_recv(), None);
    assert!(OUT.is_empty());
    cx.dispose();
}

#[test]
fn outbox_overflow_policies() {
    let newest: Outbox<u8, 2> = Outbox::new();
    let oldest: Outbox<u8, 2> = Outbox::new().on_full(Overflow::DropOldest);
    for i in 0..5 {
        let _ = newest.try_send(i);
        assert_eq!(oldest.try_send(i), Ok(()));
    }
    assert_eq!(newest.try_send(9), Err(9));
    assert_eq!((newest.take_dropped(), oldest.take_dropped()), (4, 3));
    assert_eq!((newest.try_recv(), newest.try_recv()), (Some(0), Some(1)));
    assert_eq!((oldest.try_recv(), oldest.try_recv()), (Some(3), Some(4)));
}

#[test]
fn outbox_notify_reaches_a_consumer_task() {
    static OUT: Outbox<u8, 2> = Outbox::new();
    static NOTIFIED: AtomicU32 = AtomicU32::new(0);
    OUT.waker().set_notify(|| {
        NOTIFIED.fetch_add(1, Ordering::SeqCst);
    });
    OUT.try_send(1).unwrap();
    OUT.try_send(2).unwrap();
    let _ = OUT.try_send(3); // refused: no notification
    assert_eq!(NOTIFIED.load(Ordering::SeqCst), 2);
}

#[test]
fn outbox_recv_ready_without_waiting() {
    static OUT: Outbox<u8, 2> = Outbox::new();
    OUT.try_send(5).unwrap();
    let (v, polls) = block_on(OUT.recv());
    assert_eq!((v, polls), (5, 1));
}

#[test]
fn outbox_recv_waits_for_a_send_from_another_thread() {
    // The UI thread (here: a spawned one) sends after the consumer started waiting; the
    // consumer task is woken through its registered waker and gets the commands in order.
    static OUT: Outbox<Cmd, 4> = Outbox::new();
    let sender = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(5));
        OUT.try_send(Cmd::SetTarget(1)).unwrap();
        OUT.try_send(Cmd::Stop).unwrap();
    });
    let (first, polls) = block_on(OUT.recv());
    assert_eq!(first, Cmd::SetTarget(1));
    assert!(polls >= 1);
    let (second, _) = block_on(async { OUT.recv().await });
    assert_eq!(second, Cmd::Stop);
    sender.join().unwrap();
}

#[test]
fn outbox_recv_is_cancel_safe() {
    static OUT: Outbox<u8, 2> = Outbox::new();
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    {
        let mut fut = pin!(OUT.recv());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
    } // dropped while waiting
    OUT.try_send(1).unwrap();
    assert_eq!(OUT.try_recv(), Some(1), "nothing was lost");
}

/// Round trips of the threaded test (Miri is ~1000x slower).
const ROUNDS: u32 = if cfg!(miri) { 20 } else { 2_000 };

#[test]
fn outbox_async_consumer_receives_every_command_in_order() {
    static OUT: Outbox<u32, 4> = Outbox::new();
    let consumer = std::thread::spawn(|| {
        block_on(async {
            let mut sum = 0u64;
            for i in 0..ROUNDS {
                let v = OUT.recv().await;
                assert_eq!(v, i, "in order");
                sum += u64::from(v);
            }
            sum
        })
        .0
    });
    for i in 0..ROUNDS {
        // DropNewest: retry while the consumer catches up (the UI would show "busy").
        let mut v = i;
        while let Err(back) = OUT.try_send(v) {
            v = back;
            std::thread::yield_now();
        }
    }
    let sum = consumer.join().unwrap();
    assert_eq!(sum, u64::from(ROUNDS) * u64::from(ROUNDS - 1) / 2);
    let _ = OUT.take_dropped(); // refusals from the retries above
}
