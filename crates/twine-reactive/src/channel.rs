//! [`Channel`] and [`UiWaker`]: ISR/task-safe, allocation-free delivery of values into the
//! reactive world, plus the runtime hooks the `Ui` uses to drain them.
//!
//! Neither type touches the reactive runtime: they only use `critical_section` and
//! `portable_atomic` (load/store natively, read-modify-write through the critical-section
//! fallback on targets without CAS such as thumbv6m), so they may be used from any context.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::task::Waker;

use critical_section::Mutex;
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use twine_core::log::warn;

use crate::batch::batch;
use crate::global::with_runtime;
use crate::runtime::{Inner, ScopeKey};
use crate::scope::Scope;

/// Wakes the UI loop: a flag (polled by blocking super-loops) plus an optional registered
/// async [`Waker`] (woken for the embassy/async `Ui`).
///
/// ```
/// use twine_reactive::UiWaker;
/// static W: UiWaker = UiWaker::new();
/// W.wake();
/// assert!(W.is_set());
/// assert!(W.take());
/// assert!(!W.take());
/// ```
pub struct UiWaker {
    flag: AtomicBool,
    waker: Mutex<Cell<Option<Waker>>>,
}

impl Default for UiWaker {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for UiWaker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiWaker")
            .field("set", &self.is_set())
            .finish_non_exhaustive()
    }
}

impl UiWaker {
    /// A waker with the flag cleared and no registered task.
    #[must_use]
    pub const fn new() -> Self {
        UiWaker {
            flag: AtomicBool::new(false),
            waker: Mutex::new(Cell::new(None)),
        }
    }

    /// Sets the flag and wakes the registered task, if any. ISR-safe.
    pub fn wake(&self) {
        self.flag.store(true, Ordering::Release);
        let w = critical_section::with(|cs| {
            let cell = self.waker.borrow(cs);
            let w = cell.take();
            let copy = w.clone();
            cell.set(w);
            copy
        });
        if let Some(w) = w {
            w.wake();
        }
    }

    /// Registers the task to wake (replacing a different one).
    pub fn register(&self, w: &Waker) {
        let old = critical_section::with(|cs| {
            let cell = self.waker.borrow(cs);
            let old = cell.take();
            match old {
                Some(o) if o.will_wake(w) => {
                    cell.set(Some(o));
                    None
                }
                other => {
                    cell.set(Some(w.clone()));
                    other
                }
            }
        });
        drop(old); // dropped outside the critical section
    }

    /// Reads and clears the flag.
    pub fn take(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }

    /// Whether the flag is set (not clearing it).
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// A bounded, `const`-constructible, allocation-free queue from interrupts or other tasks to
/// the UI.
///
/// [`try_send`](Channel::try_send) never blocks and wakes the UI; when the queue is full it
/// returns the value and counts it as dropped. The UI side drains registered channels with
/// [`drain_channels`] (see [`Scope::on_message`]). `Channel<T, N>` is `Sync` whenever `T` is
/// `Send` (it is built from `critical_section::Mutex` and atomics), so it can live in a
/// `static`.
///
/// ```
/// use twine_reactive::Channel;
/// static CH: Channel<u32, 4> = Channel::new();
/// assert!(CH.try_send(1).is_ok());
/// assert_eq!(CH.len(), 1);
/// assert_eq!(CH.try_recv(), Some(1));
/// assert!(CH.is_empty());
/// ```
pub struct Channel<T, const N: usize> {
    queue: Mutex<RefCell<heapless::Deque<T, N>>>,
    waker: UiWaker,
    dropped: AtomicU32,
}

impl<T, const N: usize> Default for Channel<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> core::fmt::Debug for Channel<T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Channel")
            .field("len", &self.len())
            .field("capacity", &N)
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl<T, const N: usize> Channel<T, N> {
    /// An empty channel.
    #[must_use]
    pub const fn new() -> Self {
        Channel {
            queue: Mutex::new(RefCell::new(heapless::Deque::new())),
            waker: UiWaker::new(),
            dropped: AtomicU32::new(0),
        }
    }

    /// Enqueues `v` and wakes the UI; `Err(v)` (and the dropped counter incremented) if full.
    /// ISR-safe, never blocks.
    pub fn try_send(&self, v: T) -> Result<(), T> {
        let r = critical_section::with(|cs| self.queue.borrow(cs).borrow_mut().push_back(v));
        match r {
            Ok(()) => {
                self.waker.wake();
                Ok(())
            }
            Err(v) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Err(v)
            }
        }
    }

    /// Dequeues the oldest value.
    pub fn try_recv(&self) -> Option<T> {
        critical_section::with(|cs| self.queue.borrow(cs).borrow_mut().pop_front())
    }

    /// Number of queued values.
    pub fn len(&self) -> usize {
        critical_section::with(|cs| self.queue.borrow(cs).borrow().len())
    }

    /// Whether no value is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The waker signalled by [`try_send`](Channel::try_send).
    pub fn waker(&self) -> &UiWaker {
        &self.waker
    }

    /// Returns and resets the number of values dropped because the queue was full.
    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::Relaxed)
    }
}

/// Type-erased view of a channel used by the runtime hooks.
pub(crate) trait ChannelSource {
    fn pending_len(&self) -> usize;
    fn ui_waker(&self) -> &UiWaker;
}

impl<T, const N: usize> ChannelSource for Channel<T, N> {
    fn pending_len(&self) -> usize {
        self.len()
    }

    fn ui_waker(&self) -> &UiWaker {
        &self.waker
    }
}

/// Drains up to `max` messages; returns how many were handled.
type DrainFn = Box<dyn FnMut(usize) -> usize>;

/// An `on_message` registration, removed when its scope is disposed.
pub(crate) struct ChannelReg {
    /// Monotonic id (registrations are kept sorted by it).
    id: u64,
    scope: ScopeKey,
    chan: &'static dyn ChannelSource,
    /// `None` while the handler is running (taken out of the runtime, R1).
    drain: Option<DrainFn>,
}

/// Removes the registrations of scope `s`; the caller drops them after releasing the borrow.
pub(crate) fn take_scope_channels(inner: &mut Inner, s: ScopeKey) -> Vec<ChannelReg> {
    let mut removed = Vec::new();
    let mut i = 0;
    while i < inner.channels.len() {
        if inner.channels[i].scope == s {
            removed.push(inner.channels.remove(i));
        } else {
            i += 1;
        }
    }
    removed
}

impl Scope {
    /// Calls `f` for every value received on `ch` while this scope is alive.
    ///
    /// Values are delivered by [`drain_channels`] (called by `Ui::update` inside one
    /// [`batch`](crate::batch)). The registration is removed when the scope is disposed. On a
    /// dead scope this is a no-op with a warning.
    ///
    /// ```
    /// use twine_reactive::{Channel, drain_channels};
    /// static TEMP: Channel<i16, 8> = Channel::new();
    /// let cx = twine_reactive::create_root();
    /// let temp = cx.signal(0);
    /// cx.on_message(&TEMP, move |t| temp.set(t));
    /// TEMP.try_send(215).unwrap(); // from an ISR or another task
    /// assert_eq!(drain_channels(16), 1);
    /// assert_eq!(temp.get(), 215);
    /// ```
    pub fn on_message<T: 'static, const N: usize>(
        self,
        ch: &'static Channel<T, N>,
        mut f: impl FnMut(T) + 'static,
    ) {
        let drain: DrainFn = Box::new(move |max: usize| {
            let dropped = ch.take_dropped();
            if dropped > 0 {
                warn!(target: "twine::reactive", "channel dropped {} messages", dropped);
            }
            let mut n = 0;
            while n < max {
                let Some(v) = ch.try_recv() else {
                    break;
                };
                f(v);
                n += 1;
            }
            n
        });
        let scope = self.key();
        let mut waker = None;
        let rejected = with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            if !inner.scopes.contains(scope) {
                return Some(drain);
            }
            waker.clone_from(&inner.ui_waker);
            let id = inner.next_channel_id;
            inner.next_channel_id = id.wrapping_add(1);
            inner.channels.push(ChannelReg {
                id,
                scope,
                chan: ch,
                drain: Some(drain),
            });
            None
        });
        if let Some(d) = rejected {
            warn!(target: "twine::reactive", "on_message on disposed scope {:?}; ignored", scope);
            drop(d);
        } else if let Some(w) = waker {
            ch.waker().register(&w);
        }
    }
}

/// Delivers queued channel messages to their [`Scope::on_message`] handlers, at most
/// `max_per_channel` per registration, inside one [`batch`](crate::batch). Returns the number
/// of messages handled.
///
/// Registrations added by a handler are served from the next call. Dropped-message counts are
/// logged at `warn!` on every call that finds some; rate limiting is the `Ui`'s job.
///
/// ```
/// use twine_reactive::{Channel, drain_channels};
/// static CH: Channel<u8, 4> = Channel::new();
/// let cx = twine_reactive::create_root();
/// let last = cx.signal(0u8);
/// cx.on_message(&CH, move |v| last.set(v));
/// CH.try_send(1).unwrap();
/// CH.try_send(2).unwrap();
/// assert_eq!(drain_channels(1), 1);
/// assert_eq!(last.get(), 1);
/// assert_eq!(drain_channels(8), 1);
/// assert_eq!(last.get(), 2);
/// ```
pub fn drain_channels(max_per_channel: usize) -> usize {
    batch(|| {
        with_runtime(|rt| {
            let Some(last_id) = rt.inner.borrow().channels.last().map(|r| r.id) else {
                return 0;
            };
            let mut total = 0;
            let mut i = 0;
            loop {
                let (id, drain) = {
                    let mut inner = rt.inner.borrow_mut();
                    let Some(reg) = inner.channels.get_mut(i) else {
                        break;
                    };
                    if reg.id > last_id {
                        break;
                    }
                    (reg.id, reg.drain.take())
                };
                if let Some(mut d) = drain {
                    total += d(max_per_channel);
                    let leftover = {
                        let mut inner = rt.inner.borrow_mut();
                        match inner.channels.binary_search_by_key(&id, |r| r.id) {
                            Ok(p) => {
                                inner.channels[p].drain = Some(d);
                                None
                            }
                            Err(_) => Some(d), // unregistered by its own handler
                        }
                    };
                    drop(leftover);
                }
                // Registrations are sorted by id; resume after `id` even if some were removed.
                i = rt.inner.borrow().channels.partition_point(|r| r.id <= id);
            }
            total
        })
    })
}

/// Registers `w` in the [`UiWaker`] of every channel with an `on_message` registration, so a
/// `try_send` on any of them wakes the UI task. The runtime keeps `w` and also registers it in
/// the channels of later `on_message` calls.
///
/// ```
/// use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
/// use std::task::{Wake, Waker};
/// struct Count(AtomicUsize);
/// impl Wake for Count {
///     fn wake(self: Arc<Self>) { self.0.fetch_add(1, Ordering::SeqCst); }
/// }
/// static CH: twine_reactive::Channel<u8, 2> = twine_reactive::Channel::new();
/// let cx = twine_reactive::create_root();
/// cx.on_message(&CH, |_| {});
/// let count = Arc::new(Count(AtomicUsize::new(0)));
/// twine_reactive::register_waker(&Waker::from(count.clone()));
/// CH.try_send(1).unwrap();
/// assert_eq!(count.0.load(Ordering::SeqCst), 1);
/// ```
pub fn register_waker(w: &Waker) {
    let old = with_runtime(|rt| rt.inner.borrow_mut().ui_waker.replace(w.clone()));
    drop(old); // outside the borrow (R1: a waker's drop is foreign code)
    let mut i = 0;
    // The registration list is re-read each step: nothing is borrowed while `register` runs.
    while let Some(chan) = with_runtime(|rt| rt.inner.borrow().channels.get(i).map(|r| r.chan)) {
        chan.ui_waker().register(w);
        i += 1;
    }
}

/// Whether any channel with an `on_message` registration has queued messages.
///
/// ```
/// assert!(!twine_reactive::any_channel_pending());
/// ```
pub fn any_channel_pending() -> bool {
    with_runtime(|rt| {
        rt.inner
            .borrow()
            .channels
            .iter()
            .any(|r| r.chan.pending_len() > 0)
    })
}
