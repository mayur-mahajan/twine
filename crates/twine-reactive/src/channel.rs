//! [`Channel`] and [`UiWaker`]: ISR/task-safe, allocation-free delivery of values into the
//! reactive world, plus the runtime hooks the `Ui` uses to drain them.
//!
//! Neither type touches the reactive runtime: they only use `critical_section` and
//! `portable_atomic` (load/store natively, read-modify-write through the critical-section
//! fallback on targets without CAS such as thumbv6m), so they may be used from any context.
//!
//! **Wake-up routing.** Every root scope may own one `&'static UiWaker`
//! ([`Scope::set_ui_waker`]). [`Scope::on_message`] registers a task [`Waker`] over the waker
//! of the root that owns the handler in the channel ([`UiWaker::task_waker`]: no allocation,
//! clone and drop are no-ops), so a send wakes exactly that UI. The routing is set up when a
//! handler or a waker is registered and torn down when its scope is disposed; `try_send` and
//! [`drain_channels`] do no extra work for it.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::task::{RawWaker, RawWakerVTable, Waker};

use critical_section::Mutex;
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use twine_core::fault::FaultKind;
use twine_core::log::{debug, warn};

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

    /// Removes the registered task if it is `w` (or wakes the same task); returns whether it
    /// was removed. Another registered task is left in place; the flag is not touched.
    ///
    /// Interrupt-safe like [`wake`](Self::wake) (a critical section and atomics, no
    /// allocation); the removed task waker is dropped after the critical section, in the
    /// caller's context. Typically called by the task that registered itself, when it stops
    /// waiting. Never panics.
    ///
    /// ```
    /// use twine_reactive::UiWaker;
    /// static W: UiWaker = UiWaker::new();
    /// static TASK: UiWaker = UiWaker::new(); // stands in for an executor's task waker
    /// static OTHER: UiWaker = UiWaker::new();
    /// W.register(&TASK.task_waker());
    /// assert!(!W.unregister(&OTHER.task_waker())); // not the registered task: kept
    /// assert!(W.unregister(&TASK.task_waker()));
    /// W.wake();
    /// assert!(!TASK.is_set()); // no task to wake any more
    /// ```
    pub fn unregister(&self, w: &Waker) -> bool {
        let old = critical_section::with(|cs| {
            let cell = self.waker.borrow(cs);
            match cell.take() {
                Some(o) if o.will_wake(w) => Some(o),
                other => {
                    cell.set(other);
                    None
                }
            }
        });
        let removed = old.is_some();
        drop(old); // dropped outside the critical section
        removed
    }

    /// Clears the flag and removes the registered task (a waker handed to a new owner, e.g.
    /// when a pooled waker goes back to the pool).
    ///
    /// Interrupt-safe (a critical section and atomics, no allocation); the removed task waker
    /// is dropped after the critical section, in the caller's context. A `wake` racing with
    /// `reset` from an interrupt may be lost, so reset a waker only while no interrupt handler
    /// or task still uses it for the previous owner. Never panics.
    ///
    /// ```
    /// use twine_reactive::UiWaker;
    /// static W: UiWaker = UiWaker::new();
    /// static TASK: UiWaker = UiWaker::new();
    /// W.register(&TASK.task_waker());
    /// W.wake();
    /// W.reset();
    /// assert!(!W.is_set());
    /// TASK.take();
    /// W.wake();
    /// assert!(!TASK.is_set()); // the task was removed
    /// ```
    pub fn reset(&self) {
        let old = critical_section::with(|cs| self.waker.borrow(cs).take());
        self.flag.store(false, Ordering::Release);
        drop(old); // dropped outside the critical section
    }

    /// A task [`Waker`] that wakes this `UiWaker` (sets its flag and wakes the task
    /// registered in it). It allocates nothing: the waker points to `self`, so cloning and
    /// dropping it are no-ops, and it may be woken from any thread or interrupt.
    ///
    /// ```
    /// use twine_reactive::UiWaker;
    /// static W: UiWaker = UiWaker::new();
    /// let w = W.task_waker();
    /// w.clone().wake();
    /// assert!(W.take());
    /// assert!(w.will_wake(&W.task_waker()));
    /// ```
    #[must_use]
    pub fn task_waker(&'static self) -> Waker {
        let raw = RawWaker::new(core::ptr::from_ref(self).cast::<()>(), &TASK_WAKER_VTABLE);
        // SAFETY: the vtable functions uphold the `RawWaker` contract: the data pointer is a
        // `&'static UiWaker` (valid for the program's lifetime, so clones and drops need no
        // bookkeeping), and `wake`/`wake_by_ref` only call `UiWaker::wake`, which is thread-
        // and interrupt-safe (`UiWaker: Sync`).
        #[allow(unsafe_code)]
        unsafe {
            Waker::from_raw(raw)
        }
    }

    /// A `'static` waker for a new owner (a `Ui`), returned to a process-wide pool when the
    /// [`WakerLease`] is dropped. See [`WakerLease`] for where the waker lives.
    ///
    /// ```
    /// use twine_reactive::UiWaker;
    /// let lease = UiWaker::lease();
    /// let w = lease.get();
    /// assert!(!w.is_set());
    /// drop(lease); // back in the pool
    /// ```
    #[must_use]
    pub fn lease() -> WakerLease {
        WakerLease::acquire()
    }
}

static TASK_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    task_waker_clone,
    task_waker_wake,
    task_waker_wake,
    task_waker_drop,
);

#[allow(unsafe_code)]
unsafe fn task_waker_clone(p: *const ()) -> RawWaker {
    RawWaker::new(p, &TASK_WAKER_VTABLE)
}

#[allow(unsafe_code)]
unsafe fn task_waker_wake(p: *const ()) {
    // SAFETY: every `RawWaker` with this vtable is created by `UiWaker::task_waker` (or cloned
    // from one) from a `&'static UiWaker`, so `p` points to a live `UiWaker` forever. `UiWaker`
    // is `Sync` (an atomic flag and a critical-section mutex), so waking it from any thread or
    // interrupt is sound.
    let w = unsafe { &*p.cast::<UiWaker>() };
    w.wake();
}

#[allow(unsafe_code)]
unsafe fn task_waker_drop(_p: *const ()) {}

/// Wakers kept in `static` memory: the first `STATIC_SLOTS` simultaneous leases use no heap.
static POOL: [UiWaker; WakerLease::STATIC_SLOTS] = [const { UiWaker::new() }; WakerLease::STATIC_SLOTS];
const _: () = assert!(WakerLease::STATIC_SLOTS <= 8, "POOL_USED is a u8 bitmap");
/// Bit `i` set: `POOL[i]` is leased.
static POOL_USED: Mutex<Cell<u8>> = Mutex::new(Cell::new(0));
/// Heap wakers returned by their lease, ready for the next one (intrusive list: pushing and
/// popping allocate nothing).
static HEAP_FREE: Mutex<Cell<Option<&'static HeapWaker>>> = Mutex::new(Cell::new(None));

/// A waker allocated once when the static slots are all leased; never freed (a task
/// [`Waker`] over it may still exist in another context), but reused by later leases.
struct HeapWaker {
    waker: UiWaker,
    next: Mutex<Cell<Option<&'static HeapWaker>>>,
}

/// Where a leased waker lives.
#[derive(Clone, Copy)]
enum Slot {
    Static(u8),
    Heap(&'static HeapWaker),
}

/// A `'static` [`UiWaker`] lent to one owner at a time ([`UiWaker::lease`]); returned (and
/// [`reset`](UiWaker::reset)) when dropped.
///
/// The first [`STATIC_SLOTS`](Self::STATIC_SLOTS) leases alive at the same time are served
/// from `static` memory (no heap). Beyond that a waker is allocated once and, when its lease
/// ends, kept in a free list for the next lease: the heap used for wakers is bounded by the
/// largest number of leases ever alive at once, so mounting and disposing a UI repeatedly
/// does not grow it. Wakers are never freed because a task [`Waker`] over one
/// ([`UiWaker::task_waker`]) may still be held by another context; the worst a stale one can
/// do is one spurious wake-up of the waker's next owner.
///
/// An application that wants a waker it can name from interrupt handlers before the UI
/// exists declares a `static UiWaker` and hands it to the `Ui` instead (no lease, no pool).
pub struct WakerLease {
    waker: &'static UiWaker,
    slot: Slot,
}

impl core::fmt::Debug for WakerLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WakerLease")
            .field("static", &matches!(self.slot, Slot::Static(_)))
            .field("waker", self.waker)
            .finish()
    }
}

impl WakerLease {
    /// Leases served from `static` memory before the heap is used.
    pub const STATIC_SLOTS: usize = 4;

    fn acquire() -> WakerLease {
        let slot = critical_section::with(|cs| {
            let used = POOL_USED.borrow(cs);
            let bits = used.get();
            let free = bits.trailing_ones() as usize;
            if free < Self::STATIC_SLOTS {
                used.set(bits | (1 << free));
                // `free < STATIC_SLOTS <= 8`: fits a `u8`.
                return Some(Slot::Static(free as u8));
            }
            let head = HEAP_FREE.borrow(cs);
            let h = head.get()?;
            head.set(h.next.borrow(cs).take());
            Some(Slot::Heap(h))
        });
        let slot = slot.unwrap_or_else(|| {
            debug!(
                target: "twine::reactive",
                "waker pool: {} static slots leased; allocating a waker",
                Self::STATIC_SLOTS
            );
            Slot::Heap(Box::leak(Box::new(HeapWaker {
                waker: UiWaker::new(),
                next: Mutex::new(Cell::new(None)),
            })))
        });
        let waker = match slot {
            Slot::Static(i) => &POOL[usize::from(i)],
            Slot::Heap(h) => &h.waker,
        };
        waker.reset();
        WakerLease { waker, slot }
    }

    /// The leased waker. The reference outlives the lease (it is `'static`), but once the
    /// lease is dropped the waker may belong to another owner.
    #[must_use]
    pub fn get(&self) -> &'static UiWaker {
        self.waker
    }
}

impl Drop for WakerLease {
    fn drop(&mut self) {
        self.waker.reset();
        critical_section::with(|cs| match self.slot {
            Slot::Static(i) => {
                let used = POOL_USED.borrow(cs);
                used.set(used.get() & !(1 << i));
            }
            Slot::Heap(h) => {
                let head = HEAP_FREE.borrow(cs);
                h.next.borrow(cs).set(head.get());
                head.set(Some(h));
            }
        });
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
    /// The waker of the root scope owning the handler, registered in the channel (`None`
    /// while that root has none).
    waker: Option<&'static UiWaker>,
    /// `None` while the handler is running (taken out of the runtime, R1).
    drain: Option<DrainFn>,
}

/// Whether `a` and `b` are the same channel.
fn same_channel(a: &'static dyn ChannelSource, b: &'static dyn ChannelSource) -> bool {
    core::ptr::addr_eq(a, b)
}

/// The root of scope `s` (walking up the parents), or `None` if `s` is dead.
pub(crate) fn root_of(inner: &Inner, s: ScopeKey) -> Option<ScopeKey> {
    let mut cur = s;
    loop {
        match inner.scopes.get(cur)?.parent {
            Some(p) => cur = p,
            None => return Some(cur),
        }
    }
}

/// The waker set on root scope `root` ([`Scope::set_ui_waker`]).
fn root_waker(inner: &Inner, root: ScopeKey) -> Option<&'static UiWaker> {
    inner
        .root_wakers
        .iter()
        .find(|(k, _)| *k == root)
        .map(|&(_, w)| w)
}

/// Removes the registrations of scope `s`; the caller passes them to [`release_channels`]
/// after releasing the borrow.
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

/// Undoes the wake-up routing of removed registrations (the runtime must not be borrowed):
/// a channel still handled elsewhere wakes the UI of its remaining handler, any other channel
/// stops waking the UI of the removed one. Drops the registrations.
pub(crate) fn release_channels(removed: Vec<ChannelReg>) {
    for reg in removed {
        let Some(w) = reg.waker else {
            continue;
        };
        let other = with_runtime(|rt| {
            rt.inner
                .borrow()
                .channels
                .iter()
                .rev()
                .filter(|r| same_channel(r.chan, reg.chan))
                .find_map(|r| r.waker)
        });
        match other {
            Some(o) => reg.chan.ui_waker().register(&o.task_waker()),
            None => {
                reg.chan.ui_waker().unregister(&w.task_waker());
            }
        }
        drop(reg); // the handler is user code: dropped without the runtime borrowed (R1)
    }
}

impl Scope {
    /// Calls `f` for every value received on `ch` while this scope is alive.
    ///
    /// Values are delivered by [`drain_channels`] (called by `Ui::update` inside one
    /// [`batch`](crate::batch)). A send wakes the waker of this scope's root
    /// ([`set_ui_waker`](Scope::set_ui_waker)), i.e. the `Ui` the handler belongs to. A
    /// channel wakes one UI: if handlers of several roots listen to the same channel, the
    /// most recently registered one's. The registration is removed when the scope is
    /// disposed. On a dead scope this is a no-op with a warning.
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
                crate::global::record_fault(FaultKind::ChannelOverflow, dropped);
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
            let Some(root) = root_of(&inner, scope) else {
                return Some(drain);
            };
            waker = root_waker(&inner, root);
            let id = inner.next_channel_id;
            inner.next_channel_id = id.wrapping_add(1);
            inner.channels.push(ChannelReg {
                id,
                scope,
                chan: ch,
                waker,
                drain: Some(drain),
            });
            None
        });
        if let Some(d) = rejected {
            warn!(target: "twine::reactive", "on_message on disposed scope {:?}; ignored", scope);
            drop(d);
        } else if let Some(w) = waker {
            ch.waker().register(&w.task_waker());
        }
    }

    /// Makes `waker` the waker of this root scope: every channel with an
    /// [`on_message`](Scope::on_message) handler in the scope's tree (now or later) wakes it
    /// on a send. Replaces a previous waker of the root. The waker is forgotten when the root
    /// is disposed.
    ///
    /// Each `Ui` sets its own waker on its own root, so two `Ui`s sharing a runtime are woken
    /// only by their own channels. A no-op with a warning on a dead or non-root scope.
    ///
    /// ```
    /// use twine_reactive::{Channel, UiWaker};
    /// static CH: Channel<u8, 2> = Channel::new();
    /// static W: UiWaker = UiWaker::new();
    /// let root = twine_reactive::create_root();
    /// root.child().on_message(&CH, |_| {});
    /// root.set_ui_waker(&W);
    /// assert_eq!(root.ui_waker().map(|w| w as *const UiWaker), Some(&W as *const UiWaker));
    /// CH.try_send(1).unwrap(); // e.g. from an interrupt
    /// assert!(W.take());
    /// ```
    pub fn set_ui_waker(self, waker: &'static UiWaker) {
        let root = self.key();
        let targets = with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            match inner.scopes.get(root) {
                Some(d) if d.parent.is_none() => {}
                Some(_) => return Err("not a root"),
                None => return Err("disposed"),
            }
            match inner.root_wakers.iter_mut().find(|(k, _)| *k == root) {
                Some(e) => e.1 = waker,
                None => inner.root_wakers.push((root, waker)),
            }
            // Registrations of this root's tree: point them (and their channels) at `waker`.
            let mut targets: Vec<&'static dyn ChannelSource> = Vec::new();
            let Inner { channels, scopes, .. } = &mut *inner;
            for reg in channels.iter_mut() {
                let mut cur = Some(reg.scope);
                while let Some(s) = cur {
                    if s == root {
                        reg.waker = Some(waker);
                        targets.push(reg.chan);
                        break;
                    }
                    cur = scopes.get(s).and_then(|d| d.parent);
                }
            }
            Ok(targets)
        });
        match targets {
            Ok(targets) => {
                let w = waker.task_waker();
                for chan in targets {
                    chan.ui_waker().register(&w);
                }
            }
            Err(why) => {
                warn!(target: "twine::reactive", "set_ui_waker on {} scope {:?}; ignored", why, root);
            }
        }
    }

    /// The waker of this scope's root ([`set_ui_waker`](Scope::set_ui_waker)); `None` if it
    /// has none or the scope is dead.
    #[must_use]
    pub fn ui_waker(self) -> Option<&'static UiWaker> {
        with_runtime(|rt| {
            let inner = rt.inner.borrow();
            root_of(&inner, self.key()).and_then(|r| root_waker(&inner, r))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_waker_sets_the_flag_through_clones() {
        static W: UiWaker = UiWaker::new();
        let w: &'static UiWaker = &W;
        let waker = w.task_waker();
        let clone = waker.clone();
        drop(waker);
        clone.wake_by_ref();
        assert!(w.take());
        clone.wake();
        assert!(w.take());
        assert!(!w.is_set());
    }

    #[test]
    fn unregister_removes_only_the_given_task() {
        static A: UiWaker = UiWaker::new();
        static B: UiWaker = UiWaker::new();
        let ch: Channel<u8, 2> = Channel::new();
        ch.waker().register(&A.task_waker());
        assert!(!ch.waker().unregister(&B.task_waker()));
        ch.try_send(1).unwrap();
        assert!(A.take());
        assert!(ch.waker().unregister(&A.task_waker()));
        ch.try_send(2).unwrap();
        assert!(!A.is_set());
    }
}
