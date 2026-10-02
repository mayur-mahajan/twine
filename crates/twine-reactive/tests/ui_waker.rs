//! `UiWaker`: the notify function (`set_notify`) and linearizable `reset`/`wake` (R3.S01).

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use twine_reactive::{Channel, Runtime, UiWaker};

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

#[test]
fn notify_is_called_on_every_wake_and_cleared_by_reset() {
    static W: UiWaker = UiWaker::new();
    static N: AtomicU32 = AtomicU32::new(0);
    fn count() {
        N.fetch_add(1, Ordering::SeqCst);
    }
    W.set_notify(count);
    W.wake();
    W.wake();
    assert_eq!(N.load(Ordering::SeqCst), 2);
    W.clear_notify();
    W.wake();
    assert_eq!(N.load(Ordering::SeqCst), 2, "cleared");
    W.set_notify(count);
    W.reset();
    W.wake();
    assert_eq!(N.load(Ordering::SeqCst), 2, "reset removes the notify function");
}

#[test]
fn a_pooled_waker_does_not_pass_its_notify_to_the_next_owner() {
    static N: AtomicU32 = AtomicU32::new(0);
    let lease = UiWaker::lease();
    let w = lease.get();
    w.set_notify(|| {
        N.fetch_add(1, Ordering::SeqCst);
    });
    drop(lease);
    w.wake(); // a stale reference: at most a spurious flag for the next owner
    assert_eq!(N.load(Ordering::SeqCst), 0);
    w.reset();
}

#[test]
fn channel_send_from_another_thread_calls_the_notify_function() {
    static UI: UiWaker = UiWaker::new();
    static CH: Channel<u32, 4> = Channel::new();
    static NOTIFIED: AtomicU32 = AtomicU32::new(0);
    static GOT: AtomicU32 = AtomicU32::new(0);
    // e.g. an RTOS task notification or semaphore that the UI task sleeps on.
    UI.set_notify(|| {
        NOTIFIED.fetch_add(1, Ordering::SeqCst);
    });
    let root = rt().create_root();
    root.set_ui_waker(&UI);
    root.on_message(&CH, |v| GOT.store(v, Ordering::SeqCst));
    thread::spawn(|| CH.try_send(42).unwrap()).join().unwrap();
    assert_eq!(
        NOTIFIED.load(Ordering::SeqCst),
        1,
        "the sender's context ran the notify"
    );
    assert!(UI.take(), "and set the UI's flag");
    assert_eq!(rt().drain_channels(16), 1);
    assert_eq!(GOT.load(Ordering::SeqCst), 42);
    root.dispose();
}

/// The R2 docs-pass follow-up: a `wake` that ran entirely inside `reset` (between its removal
/// of the task and its clearing of the flag) was lost — flag cleared, no task woken. Now each
/// is one critical section: here the `wake` (another thread, standing in for an interrupt)
/// starts while `reset` holds the critical section, so it must take effect after the reset.
#[test]
fn regression_wake_racing_with_reset_is_not_lost() {
    static W: UiWaker = UiWaker::new();
    // A few rounds: before the fix the waking thread set the flag outside the critical section
    // and the reset then cleared it, every time the thread got to run within the pause.
    for _ in 0..if cfg!(miri) { 1 } else { 5 } {
        W.reset();
        let waker = critical_section::with(|_| {
            let waker = thread::spawn(|| W.wake());
            // Let the thread reach `wake` and block on the critical section held here.
            thread::sleep(Duration::from_millis(5));
            W.reset(); // nested: same thread, same critical section
            waker
        });
        waker.join().unwrap();
        assert!(W.take(), "the wake that came after the reset was lost");
    }
}

/// Stress: `reset` and `wake` from two threads at the same time, many times. Whatever the
/// interleaving, the outcome is one of the two linear orders: the wake before the reset (it
/// reached the previous owner's task, then was cleared) or after it (its flag is kept). Never
/// neither.
// Not built under Miri: thousands of thread rendezvous (hours under Miri); the deterministic
// regression above covers the same interleaving there.
#[cfg(not(miri))]
#[test]
fn reset_and_wake_are_linearizable_under_contention() {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Barrier};

    static W: UiWaker = UiWaker::new();
    static TASK: UiWaker = UiWaker::new(); // the previous owner's task
    let rounds = 2000;
    let barrier = Arc::new(Barrier::new(2));
    let stop = Arc::new(AtomicBool::new(false));
    let waker = {
        let barrier = barrier.clone();
        let stop = stop.clone();
        thread::spawn(move || {
            loop {
                barrier.wait();
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                W.wake();
                barrier.wait();
            }
        })
    };
    for round in 0..rounds {
        W.register(&TASK.task_waker());
        TASK.take();
        barrier.wait();
        W.reset();
        barrier.wait(); // the wake has returned
        let after_reset = W.take();
        let before_reset = TASK.take();
        assert!(
            after_reset || before_reset,
            "round {round}: the wake neither survived the reset nor reached the previous task"
        );
    }
    stop.store(true, Ordering::SeqCst);
    barrier.wait();
    waker.join().unwrap();
}
