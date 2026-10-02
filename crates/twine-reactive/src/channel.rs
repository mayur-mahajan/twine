//! [`Channel`] and [`UiWaker`]: ISR/task-safe, allocation-free delivery of values into the
//! reactive world, plus the runtime hooks the `Ui` uses to drain them ([`Scope::on_message`],
//! [`Scope::on_latest`], [`Scope::watch`]).
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
//! [`Runtime::drain_channels`] do no extra work for it.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::Cell;
use core::task::{RawWaker, RawWakerVTable, Waker};

use critical_section::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use twine_core::fault::FaultKind;
use twine_core::log::{debug, error, warn};

use crate::batch::batch;
use crate::global::{Runtime, with_runtime};
use crate::latest::Latest;
use crate::queue::{Overflow, Queue};
use crate::runtime::{Inner, ScopeKey};
use crate::scope::Scope;

/// Wakes the UI loop. A [`wake`](Self::wake) does three things, each optional for the loop
/// that runs the UI:
///
/// - sets a **flag**, polled by blocking super-loops ([`take`](Self::take),
///   [`is_set`](Self::is_set));
/// - wakes the registered async **task** [`Waker`] ([`register`](Self::register)), for the
///   embassy/async `Ui`;
/// - calls the **notify** function ([`set_notify`](Self::set_notify)): a plain `fn()` for
///   RTOS task notifications, an RTIC `pend`, a semaphore give, or a `Platform`'s notify
///   (e.g. a Cortex-M `SEV` that ends a `WFE`).
///
/// Every operation is interrupt- and thread-safe and allocation-free: an atomic flag plus a
/// `critical_section` mutex (no compare-and-swap, so it works on thumbv6m).
///
/// **Linearizable.** [`wake`](Self::wake), [`reset`](Self::reset),
/// [`set_notify`](Self::set_notify) and [`register`](Self::register) each take effect at one
/// point (inside their critical section), so a `wake` racing with a `reset` from an interrupt
/// or another thread happens either entirely before it (and is cleared with the previous
/// owner's state) or entirely after it (and is kept): a wake-up is never lost half-way.
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
    notify: Mutex<Cell<Option<NotifyFn>>>,
}

/// The function a [`UiWaker`] calls on every wake ([`UiWaker::set_notify`]).
type NotifyFn = fn();

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
            notify: Mutex::new(Cell::new(None)),
        }
    }

    /// Sets the flag, then wakes the registered task and calls the notify function, if any.
    ///
    /// Interrupt- and thread-safe, allocation-free, never panics (unless the notify function
    /// or the task waker does). The flag is set inside the critical section that reads the
    /// task and the notify function (the linearization point, see [`UiWaker`]); the task is
    /// woken and the notify function called after it, in the caller's context.
    ///
    /// ```
    /// use core::sync::atomic::{AtomicU32, Ordering};
    /// use twine_reactive::UiWaker;
    ///
    /// static W: UiWaker = UiWaker::new();
    /// static NOTIFIED: AtomicU32 = AtomicU32::new(0);
    /// W.set_notify(|| {
    ///     NOTIFIED.fetch_add(1, Ordering::Relaxed);
    /// });
    /// W.wake(); // e.g. from a touch interrupt
    /// assert!(W.take());
    /// assert_eq!(NOTIFIED.load(Ordering::Relaxed), 1);
    /// ```
    pub fn wake(&self) {
        let (task, notify) = critical_section::with(|cs| {
            self.flag.store(true, Ordering::Release);
            let cell = self.waker.borrow(cs);
            let w = cell.take();
            let copy = w.clone();
            cell.set(w);
            (copy, self.notify.borrow(cs).get())
        });
        if let Some(f) = notify {
            f();
        }
        if let Some(w) = task {
            w.wake();
        }
    }

    /// Sets the function every [`wake`](Self::wake) calls (replacing the previous one): the
    /// hook for run loops that sleep on something other than the flag or an async task — an
    /// RTOS task notification or semaphore, an RTIC `pend`, a `Platform`'s notify (e.g.
    /// `twine_hal::Platform::notify`, installed by `UiBuilder::platform`).
    ///
    /// `notify` runs in the context of the `wake` (possibly an interrupt handler or another
    /// thread or core), outside the waker's critical section: it must be interrupt-safe,
    /// short, and must not block. It is a plain `fn()` (no state, no allocation): a function
    /// that needs a target names a `static` (task handle, semaphore). Cleared by
    /// [`clear_notify`](Self::clear_notify) and by [`reset`](Self::reset) (a pooled waker's
    /// next owner does not inherit it). Interrupt-safe, allocation-free, never panics.
    ///
    /// ```
    /// use core::sync::atomic::{AtomicBool, Ordering};
    /// use twine_reactive::UiWaker;
    ///
    /// static W: UiWaker = UiWaker::new();
    /// /// Stands in for e.g. `xTaskNotifyGive(UI_TASK)` / `k_sem_give(&UI_SEM)`.
    /// static UI_TASK_READY: AtomicBool = AtomicBool::new(false);
    /// fn notify_ui_task() {
    ///     UI_TASK_READY.store(true, Ordering::Release);
    /// }
    /// W.set_notify(notify_ui_task);
    /// W.wake();
    /// assert!(UI_TASK_READY.load(Ordering::Acquire));
    /// ```
    pub fn set_notify(&self, notify: fn()) {
        critical_section::with(|cs| self.notify.borrow(cs).set(Some(notify)));
    }

    /// Removes the notify function (see [`set_notify`](Self::set_notify)); later wakes only
    /// set the flag and wake the task. Interrupt-safe, allocation-free, never panics.
    ///
    /// ```
    /// use core::sync::atomic::{AtomicU32, Ordering};
    /// use twine_reactive::UiWaker;
    ///
    /// static W: UiWaker = UiWaker::new();
    /// static N: AtomicU32 = AtomicU32::new(0);
    /// W.set_notify(|| {
    ///     N.fetch_add(1, Ordering::Relaxed);
    /// });
    /// W.clear_notify();
    /// W.wake();
    /// assert_eq!(N.load(Ordering::Relaxed), 0);
    /// ```
    pub fn clear_notify(&self) {
        critical_section::with(|cs| self.notify.borrow(cs).set(None));
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

    /// Clears the flag and removes the registered task and the notify function (a waker
    /// handed to a new owner, e.g. when a pooled waker goes back to the pool).
    ///
    /// Interrupt-safe (a critical section and atomics, no allocation); the removed task waker
    /// is dropped after the critical section, in the caller's context. Never panics.
    ///
    /// Atomic with respect to [`wake`](Self::wake): all three are cleared in one critical
    /// section, and `wake` sets the flag inside its own. A `wake` racing with `reset` (from an
    /// interrupt, another thread or core) is therefore either entirely before the reset —
    /// it belonged to the previous owner and is cleared with the rest (its task may still get
    /// that one wake-up) — or entirely after it, and then its flag is kept for the new owner.
    /// No interleaving leaves a wake half-applied (the flag cleared after the wake read an
    /// empty task slot).
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
        let old = critical_section::with(|cs| {
            self.flag.store(false, Ordering::Release);
            self.notify.borrow(cs).set(None);
            self.waker.borrow(cs).take()
        });
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
    /// [`WakerLease`] is dropped. See [`WakerLease`] for where the waker lives: no allocation
    /// for the first [`WakerLease::STATIC_SLOTS`] leases alive at once, a bounded number of
    /// heap wakers beyond, and a shared waker ([`WakerLease::is_shared`]) once the pool is
    /// exhausted. Callable from any context; never panics.
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
/// Heap wakers allocated so far (each allocated once, never freed).
static HEAP_ALLOCATED: Mutex<Cell<u16>> = Mutex::new(Cell::new(0));
/// How many heap wakers may be allocated ([`WakerLease::set_heap_limit`]).
static HEAP_LIMIT: Mutex<Cell<u16>> = Mutex::new(Cell::new(WakerLease::DEFAULT_HEAP_LIMIT));
/// The waker every lease shares once the pool is exhausted ([`WakerLease::is_shared`]).
static SHARED: UiWaker = UiWaker::new();

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
    /// The pool is exhausted: the shared fallback waker.
    Shared,
}

/// What [`WakerLease::acquire`] decided inside its critical section.
enum Acquired {
    Slot(Slot),
    /// A new heap waker (already counted against the limit) is to be allocated.
    Allocate,
}

/// A `'static` [`UiWaker`] lent to one owner at a time ([`UiWaker::lease`]); returned (and
/// [`reset`](UiWaker::reset)) when dropped.
///
/// **Where the waker lives.** The first [`STATIC_SLOTS`](Self::STATIC_SLOTS) leases alive at
/// the same time are served from `static` memory (no heap). Beyond that a waker is allocated
/// once and, when its lease ends, kept in a free list for the next lease, up to
/// [`heap_limit`](Self::heap_limit) heap wakers in total (default
/// [`DEFAULT_HEAP_LIMIT`](Self::DEFAULT_HEAP_LIMIT); `0` forbids the heap): the heap used for
/// wakers is bounded by that limit and by the largest number of leases ever alive at once, so
/// mounting and disposing a UI repeatedly does not grow it. Wakers are never freed because a
/// task [`Waker`] over one ([`UiWaker::task_waker`]) may still be held by another context;
/// the worst a stale one can do is one spurious wake-up of the waker's next owner.
///
/// **Exhausted pool.** When every static slot is leased, the free list is empty and the limit
/// is reached, the lease does not allocate: it hands out one process-wide **shared** waker
/// ([`is_shared`](Self::is_shared) is `true`) and logs an `error!`. Every owner of a shared
/// lease is woken by every other one's channels and interrupts (spurious wake-ups), and they
/// overwrite each other's task waker and notify function, so an async or notify-driven loop
/// can miss wake-ups; a polling loop that also honours its deadlines keeps working. `Ui`
/// raises a [`FaultKind::Capacity`] fault for it (code `CapacityFault::WakerPool` of
/// `twine-view`); what to do about it — raise the limit, or give each UI a `static` waker —
/// is the application's decision. A shared lease is not reset when acquired or dropped (other
/// owners still use the waker).
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
            .field("shared", &self.is_shared())
            .field("waker", self.waker)
            .finish()
    }
}

impl WakerLease {
    /// Leases served from `static` memory before the heap is used.
    pub const STATIC_SLOTS: usize = 4;

    /// The default of [`heap_limit`](Self::heap_limit): heap wakers that may be allocated
    /// beyond the [`STATIC_SLOTS`](Self::STATIC_SLOTS) (so up to 64 UIs alive at once get a
    /// waker of their own). Devices run a handful of UIs; the bound turns a leak — UIs that are
    /// never dropped — into a reported fault instead of unbounded heap growth.
    pub const DEFAULT_HEAP_LIMIT: u16 = 60;

    /// Sets how many wakers the pool may allocate on the heap in total (process-wide; `0`
    /// forbids heap wakers, e.g. for a profile without dynamic allocation after start-up).
    /// Wakers already allocated stay in the pool and are still reused; lowering the limit below
    /// their number only stops further allocations. Callable from any context (one critical
    /// section); never panics.
    ///
    /// ```
    /// use twine_reactive::{UiWaker, WakerLease};
    /// WakerLease::set_heap_limit(0); // static slots only
    /// assert_eq!(WakerLease::heap_limit(), 0);
    /// let lease = UiWaker::lease(); // one of the static slots
    /// assert!(!lease.is_shared());
    /// # drop(lease);
    /// # WakerLease::set_heap_limit(WakerLease::DEFAULT_HEAP_LIMIT);
    /// ```
    pub fn set_heap_limit(limit: u16) {
        critical_section::with(|cs| HEAP_LIMIT.borrow(cs).set(limit));
    }

    /// The limit set with [`set_heap_limit`](Self::set_heap_limit) (default
    /// [`DEFAULT_HEAP_LIMIT`](Self::DEFAULT_HEAP_LIMIT)). Never panics.
    ///
    /// ```
    /// use twine_reactive::WakerLease;
    /// assert_eq!(WakerLease::heap_limit(), WakerLease::DEFAULT_HEAP_LIMIT);
    /// ```
    #[must_use]
    pub fn heap_limit() -> u16 {
        critical_section::with(|cs| HEAP_LIMIT.borrow(cs).get())
    }

    /// The number of wakers the pool allocated on the heap so far (never decreases: heap
    /// wakers are reused, not freed). Never panics.
    ///
    /// ```
    /// use twine_reactive::{UiWaker, WakerLease};
    /// let lease = UiWaker::lease(); // a static slot: nothing allocated
    /// assert_eq!(WakerLease::heap_allocated(), 0);
    /// # drop(lease);
    /// ```
    #[must_use]
    pub fn heap_allocated() -> u16 {
        critical_section::with(|cs| HEAP_ALLOCATED.borrow(cs).get())
    }

    /// Heap bytes of the wakers the pool allocated so far
    /// ([`heap_allocated`](Self::heap_allocated) × the size of one heap waker). Process-wide,
    /// never decreases; the [`STATIC_SLOTS`](Self::STATIC_SLOTS) static wakers are not heap.
    /// Never panics.
    ///
    /// ```
    /// use twine_reactive::WakerLease;
    /// assert_eq!(WakerLease::heap_bytes() % WakerLease::HEAP_WAKER_BYTES, 0);
    /// ```
    #[must_use]
    pub fn heap_bytes() -> usize {
        usize::from(Self::heap_allocated()) * Self::HEAP_WAKER_BYTES
    }

    /// Size in bytes of one heap-allocated waker of the pool.
    pub const HEAP_WAKER_BYTES: usize = core::mem::size_of::<HeapWaker>();

    fn acquire() -> WakerLease {
        let got = critical_section::with(|cs| {
            let used = POOL_USED.borrow(cs);
            let bits = used.get();
            let free = bits.trailing_ones() as usize;
            if free < Self::STATIC_SLOTS {
                used.set(bits | (1 << free));
                // `free < STATIC_SLOTS <= 8`: fits a `u8`.
                return Acquired::Slot(Slot::Static(free as u8));
            }
            let head = HEAP_FREE.borrow(cs);
            if let Some(h) = head.get() {
                head.set(h.next.borrow(cs).take());
                return Acquired::Slot(Slot::Heap(h));
            }
            let allocated = HEAP_ALLOCATED.borrow(cs);
            if allocated.get() < HEAP_LIMIT.borrow(cs).get() {
                // Counted now, allocated after the critical section.
                allocated.set(allocated.get() + 1);
                Acquired::Allocate
            } else {
                Acquired::Slot(Slot::Shared)
            }
        });
        let slot = match got {
            Acquired::Slot(Slot::Shared) => return Self::shared(),
            Acquired::Slot(slot) => slot,
            Acquired::Allocate => Self::allocate(),
        };
        let waker = match slot {
            Slot::Static(i) => &POOL[usize::from(i)],
            Slot::Heap(h) => &h.waker,
            Slot::Shared => &SHARED,
        };
        waker.reset();
        WakerLease { waker, slot }
    }

    /// A new heap waker (cold: only beyond the static slots, once per waker).
    #[cold]
    #[inline(never)]
    fn allocate() -> Slot {
        debug!(
            target: "twine::reactive",
            "waker pool: {} static slots leased; allocating a waker",
            Self::STATIC_SLOTS
        );
        Slot::Heap(Box::leak(Box::new(HeapWaker {
            waker: UiWaker::new(),
            next: Mutex::new(Cell::new(None)),
        })))
    }

    /// The shared fallback lease of an exhausted pool (cold).
    #[cold]
    #[inline(never)]
    fn shared() -> WakerLease {
        error!(
            target: "twine::reactive",
            "waker pool exhausted ({} static, heap limit {}): sharing one waker (no per-UI wake-up routing)",
            Self::STATIC_SLOTS,
            Self::heap_limit()
        );
        WakerLease {
            waker: &SHARED,
            slot: Slot::Shared,
        }
    }

    /// The leased waker. The reference outlives the lease (it is `'static`), but once the
    /// lease is dropped the waker may belong to another owner.
    #[must_use]
    pub fn get(&self) -> &'static UiWaker {
        self.waker
    }

    /// Whether this is the shared fallback of an exhausted pool (see [`WakerLease`] §
    /// Exhausted pool): the waker is not this owner's alone. Never panics.
    ///
    /// ```
    /// let lease = twine_reactive::UiWaker::lease();
    /// assert!(!lease.is_shared()); // the pool has room
    /// ```
    #[must_use]
    pub fn is_shared(&self) -> bool {
        matches!(self.slot, Slot::Shared)
    }
}

impl Drop for WakerLease {
    fn drop(&mut self) {
        if self.is_shared() {
            return; // other owners still use it
        }
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
            Slot::Shared => {}
        });
    }
}

/// A bounded, `const`-constructible, allocation-free FIFO queue from interrupts, tasks,
/// threads or other cores **to the UI**.
///
/// [`try_send`](Channel::try_send) never blocks and wakes the UI. When the queue is full, the
/// [`Overflow`] policy chosen with [`on_full`](Channel::on_full) decides which value is lost:
/// the new one ([`Overflow::DropNewest`], the default: `try_send` returns it) or the oldest
/// queued one ([`Overflow::DropOldest`], for telemetry). Lost values are counted and reported
/// as a [`FaultKind::ChannelOverflow`] fault when the UI drains the channel. The UI side
/// handles values with [`Scope::on_message`], delivered by [`Runtime::drain_channels`]
/// (`Ui::update` does it).
///
/// Use a `Channel` when every value matters in order (events, commands, samples). For "the
/// current reading" — where only the newest value matters and nothing should ever be dropped —
/// use [`Latest`](crate::Latest); for commands from the UI to a task, [`Outbox`](crate::Outbox).
///
/// `Channel<T, N>` is `Sync` whenever `T` is `Send` (it is built from `critical_section::Mutex`
/// and `portable-atomic`, no compare-and-swap, so it works on thumbv6m), so it can live in a
/// `static`, or be owned by the application's ports (see [`Scope::on_message`] § Ports).
/// Sending, receiving and draining allocate nothing.
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
    queue: Queue<T, N>,
    waker: UiWaker,
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
            .field("on_full", &self.queue.overflow())
            .field("dropped", &self.queue.peek_dropped())
            .finish_non_exhaustive()
    }
}

impl<T, const N: usize> Channel<T, N> {
    /// An empty channel with room for `N` values; a full channel refuses new values
    /// ([`Overflow::DropNewest`]; see [`on_full`](Self::on_full)).
    #[must_use]
    pub const fn new() -> Self {
        Channel {
            queue: Queue::new(Overflow::DropNewest),
            waker: UiWaker::new(),
        }
    }

    /// The channel with overflow policy `policy` (see [`Overflow`]); `const`, so a `static`
    /// states its policy where it is declared. Default: [`Overflow::DropNewest`].
    ///
    /// ```
    /// use twine_reactive::{Channel, Overflow};
    /// static TELEMETRY: Channel<i16, 8> = Channel::new().on_full(Overflow::DropOldest);
    /// assert_eq!(TELEMETRY.overflow(), Overflow::DropOldest);
    /// ```
    #[must_use]
    pub const fn on_full(mut self, policy: Overflow) -> Self {
        self.queue.set_overflow(policy);
        self
    }

    /// The overflow policy (see [`on_full`](Self::on_full)). Never panics.
    ///
    /// ```
    /// use twine_reactive::{Channel, Overflow};
    /// let ch: Channel<u8, 2> = Channel::new();
    /// assert_eq!(ch.overflow(), Overflow::DropNewest); // the default
    /// ```
    #[must_use]
    pub const fn overflow(&self) -> Overflow {
        self.queue.overflow()
    }

    /// Queues `v` and wakes the UI. Interrupt- and thread-safe; never blocks, never allocates,
    /// never panics (unless a notify function or task waker does, see [`UiWaker::wake`]).
    ///
    /// # Errors
    ///
    /// `Err(v)` when the channel is full and its policy is [`Overflow::DropNewest`] (the UI is
    /// not woken). With [`Overflow::DropOldest`] a full channel evicts its oldest value
    /// instead (dropped in the caller's context) and `try_send` succeeds. Either way the lost
    /// value is counted ([`take_dropped`](Self::take_dropped)).
    ///
    /// ```
    /// use twine_reactive::Channel;
    /// static CH: Channel<u8, 1> = Channel::new();
    /// assert_eq!(CH.try_send(1), Ok(()));
    /// assert_eq!(CH.try_send(2), Err(2)); // full: the new value is refused
    /// assert_eq!(CH.take_dropped(), 1);
    /// ```
    #[inline]
    pub fn try_send(&self, v: T) -> Result<(), T> {
        self.queue.push(v)?;
        self.waker.wake();
        Ok(())
    }

    /// Dequeues the oldest value (`None` when empty). Any context; allocation-free; never
    /// panics. The UI normally receives through [`Scope::on_message`] instead.
    ///
    /// ```
    /// use twine_reactive::Channel;
    /// let ch: Channel<u8, 2> = Channel::new();
    /// ch.try_send(7).unwrap();
    /// assert_eq!(ch.try_recv(), Some(7));
    /// assert_eq!(ch.try_recv(), None);
    /// ```
    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.queue.pop()
    }

    /// Number of queued values.
    ///
    /// ```
    /// let ch: twine_reactive::Channel<u8, 2> = twine_reactive::Channel::new();
    /// ch.try_send(1).unwrap();
    /// assert_eq!(ch.len(), 1);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether no value is queued.
    ///
    /// ```
    /// let ch: twine_reactive::Channel<u8, 2> = twine_reactive::Channel::new();
    /// assert!(ch.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The waker signalled by [`try_send`](Channel::try_send) (the routing to the UI's waker is
    /// set up by [`Scope::on_message`]).
    #[must_use]
    pub fn waker(&self) -> &UiWaker {
        &self.waker
    }

    /// Returns and resets the number of values lost because the channel was full (refused or
    /// evicted, see [`Overflow`]). The `on_message` drain calls it and reports the count as a
    /// [`FaultKind::ChannelOverflow`] fault; call it yourself only for a channel without a
    /// handler.
    ///
    /// ```
    /// use twine_reactive::{Channel, Overflow};
    /// let ch: Channel<u8, 1> = Channel::new().on_full(Overflow::DropOldest);
    /// ch.try_send(1).unwrap();
    /// ch.try_send(2).unwrap(); // evicts 1
    /// assert_eq!(ch.take_dropped(), 1);
    /// assert_eq!(ch.take_dropped(), 0);
    /// ```
    pub fn take_dropped(&self) -> u32 {
        self.queue.take_dropped()
    }
}

/// Type-erased view of a registered source ([`Channel`] or [`Latest`]) used by the runtime
/// hooks.
pub(crate) trait Source {
    /// Whether a drain would find work.
    fn pending(&self) -> bool;
    /// The waker the source signals; the routing points it at the UI's waker.
    fn ui_waker(&self) -> &UiWaker;
}

impl<T, const N: usize> Source for Channel<T, N> {
    fn pending(&self) -> bool {
        !self.is_empty()
    }

    fn ui_waker(&self) -> &UiWaker {
        &self.waker
    }
}

/// Reports `dropped` values lost by a full channel (cold: only when a channel overflowed).
#[cold]
#[inline(never)]
fn report_overflow(dropped: u32) {
    warn!(target: "twine::reactive", "channel dropped {} messages", dropped);
    crate::global::record_fault(FaultKind::ChannelOverflow, dropped);
}

/// Drains up to `max` messages; returns how many were handled.
type DrainFn = Box<dyn FnMut(usize) -> usize>;

/// An `on_message` / `on_latest` / `watch` registration, removed when its scope is disposed.
pub(crate) struct ChannelReg {
    /// Monotonic id (registrations are kept sorted by it).
    id: u64,
    scope: ScopeKey,
    /// The root of `scope`: the UI whose update delivers this registration's messages.
    root: ScopeKey,
    chan: &'static dyn Source,
    /// The waker of the root scope owning the handler, registered in the channel (`None`
    /// while that root has none).
    waker: Option<&'static UiWaker>,
    /// `None` while the handler is running (taken out of the runtime, R1).
    drain: Option<DrainFn>,
}

/// Whether `a` and `b` are the same channel.
fn same_channel(a: &'static dyn Source, b: &'static dyn Source) -> bool {
    core::ptr::addr_eq(a, b)
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
    /// Values are delivered in order by [`Runtime::drain_channels`] (called by `Ui::update`
    /// inside one [`batch`](Runtime::batch), at most `messages_per_channel` per update — a
    /// `UiBuilder` setting). Values lost to the channel's [`Overflow`] policy are reported as
    /// one [`FaultKind::ChannelOverflow`] fault per drain (with the count). A send wakes the
    /// waker of this scope's root ([`set_ui_waker`](Scope::set_ui_waker)), i.e. the `Ui` the
    /// handler belongs to. A channel wakes one UI: if handlers of several roots listen to the
    /// same channel, the most recently registered one's. The registration is removed when the
    /// scope is disposed. On a dead scope this is a no-op with a warning. Registration
    /// allocates the boxed handler once; delivering allocates nothing.
    ///
    /// # Ports: where the channel lives
    ///
    /// The channel must be `'static`: the registration lives in the runtime until the scope is
    /// disposed, which the borrow checker cannot tie to a shorter lifetime, and the producer —
    /// an interrupt handler, another task or core — needs a `'static` reference anyway. It need
    /// not be a global the application names, though: declare the cross-context objects in
    /// `main` (or let the board support code own them) and hand the application a `Copy`
    /// **ports** struct of references. The application then depends only on its parameters,
    /// and a test gives every UI its own channels (`TestUi::channel`, which leaks a fresh one)
    /// instead of sharing a global between tests running in parallel.
    ///
    /// ```
    /// use twine_reactive::{Channel, Runtime, Scope, Signal};
    ///
    /// /// What the application talks to; built by `main`, a test, or the simulator.
    /// #[derive(Clone, Copy)]
    /// struct Ports {
    ///     temp: &'static Channel<i16, 8>,
    /// }
    ///
    /// fn app(cx: Scope, ports: Ports) -> Signal<i16> {
    ///     let temp = cx.signal(0);
    ///     cx.on_message(ports.temp, move |t| temp.set(t));
    ///     temp
    /// }
    ///
    /// static TEMP: Channel<i16, 8> = Channel::new(); // owned by `main`, not named by `app`
    /// let rt = Runtime::take().unwrap();
    /// let temp = app(rt.create_root(), Ports { temp: &TEMP });
    /// TEMP.try_send(215).unwrap(); // from an ISR or another task
    /// assert_eq!(rt.drain_channels(16), 1);
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
                report_overflow(dropped);
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
        self.register_source(ch, drain);
    }

    /// Calls `f` with the newest value of `latest` whenever it changed since the last update,
    /// while this scope is alive: the "current reading" counterpart of
    /// [`on_message`](Self::on_message). Values set before this call are not delivered (read
    /// [`Latest::get`] for the current one, or use [`watch`](Self::watch)); several sets
    /// between two updates are delivered once, with the newest value. Nothing is ever dropped
    /// or reported as a fault: a [`Latest`] has no queue.
    ///
    /// Routing, lifetime (`'static`, see [`on_message`](Self::on_message) § Ports) and disposal
    /// as for `on_message`. Delivering clones the value once inside a critical section (see
    /// [`Latest::get`]) and allocates nothing for heap-free `T`.
    ///
    /// ```
    /// use twine_reactive::{Latest, Runtime};
    /// static LEVEL: Latest<u8> = Latest::new(0);
    /// let rt = Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let seen = cx.signal(Vec::new());
    /// cx.on_latest(&LEVEL, move |v| seen.update(|s| s.push(v)));
    /// LEVEL.set(3);
    /// LEVEL.set(4); // overwrites 3 before the UI looked
    /// rt.drain_channels(16);
    /// assert_eq!(seen.get(), vec![4]);
    /// ```
    pub fn on_latest<T: Clone + 'static>(self, latest: &'static Latest<T>, f: impl FnMut(T) + 'static) {
        self.register_latest(latest, latest.version(), f);
    }

    /// A signal that follows `latest`: it starts with the current value and is set to the
    /// newest one in every update after a [`Latest::set`] (from an interrupt, a task or
    /// another thread). No closure, no queue, nothing dropped; the UI re-reads on wake-up, so
    /// a sensor writing faster than the display refreshes costs one update per frame, not
    /// one per sample. See [`on_latest`](Self::on_latest) for routing, lifetime and cost.
    ///
    /// The signal belongs to this scope (disposed with it). It is set whenever the cell's
    /// version changed, even to an equal value; derive a [`Memo`](crate::Memo) to filter equal
    /// values.
    ///
    /// # Panics
    ///
    /// On a disposed scope, like [`signal`](Self::signal).
    ///
    /// ```
    /// use twine_reactive::{Latest, Runtime};
    /// static TEMP: Latest<i16> = Latest::new(200);
    /// let rt = Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let temp = cx.watch(&TEMP);
    /// assert_eq!(temp.get(), 200);
    /// TEMP.set(215); // from an ISR or another task
    /// rt.drain_channels(16); // `Ui::update` does this
    /// assert_eq!(temp.get(), 215);
    /// ```
    #[track_caller]
    pub fn watch<T: Clone + 'static>(self, latest: &'static Latest<T>) -> crate::ReadSignal<T> {
        let (value, version) = latest.snapshot();
        let s = self.signal(value);
        self.register_latest(latest, version, move |v| s.set(v));
        s.read_only()
    }

    /// Registers `f` for the versions of `latest` after `seen`.
    fn register_latest<T: Clone + 'static>(
        self,
        latest: &'static Latest<T>,
        mut seen: u32,
        mut f: impl FnMut(T) + 'static,
    ) {
        let drain: DrainFn = Box::new(move |_max: usize| match latest.get_if_newer(&mut seen) {
            Some(v) => {
                f(v);
                1
            }
            None => 0,
        });
        self.register_source(latest, drain);
    }

    /// Adds a registration of this scope for `src` and routes `src`'s waker to the root's.
    fn register_source(self, src: &'static dyn Source, drain: DrainFn) {
        let scope = self.key();
        let mut waker = None;
        let rejected = with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            let Some(root) = inner.root_of(scope) else {
                return Some(drain);
            };
            waker = inner.root_waker(root);
            let id = inner.next_channel_id;
            inner.next_channel_id = id.wrapping_add(1);
            inner.channels.push(ChannelReg {
                id,
                scope,
                root,
                chan: src,
                waker,
                drain: Some(drain),
            });
            None
        });
        if let Some(d) = rejected {
            warn!(target: "twine::reactive", "message handler on disposed scope {:?}; ignored", scope);
            drop(d);
        } else if let Some(w) = waker {
            src.ui_waker().register(&w.task_waker());
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
    /// let root = twine_reactive::Runtime::take().unwrap().create_root();
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
            let mut targets: Vec<&'static dyn Source> = Vec::new();
            for reg in inner.channels.iter_mut().filter(|r| r.root == root) {
                reg.waker = Some(waker);
                targets.push(reg.chan);
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
            inner.root_of(self.key()).and_then(|r| inner.root_waker(r))
        })
    }
}

impl Runtime {
    /// Delivers queued channel messages to their [`Scope::on_message`] handlers — of **every**
    /// root —, at most `max_per_channel` per registration, inside one [`batch`](Runtime::batch).
    /// Returns the number of messages handled.
    ///
    /// For a program with one consumer of the runtime. A `Ui` delivers only its own handlers'
    /// messages ([`Scope::drain_channels`] on its root), so two UIs sharing a runtime never run
    /// each other's handlers.
    ///
    /// Registrations added by a handler are served from the next call. Dropped-message counts
    /// are logged at `warn!` and recorded as
    /// [`FaultKind::ChannelOverflow`](crate::FaultKind::ChannelOverflow) on every call that
    /// finds some; rate limiting is the `Ui`'s job. Allocates nothing.
    ///
    /// ```
    /// use twine_reactive::Channel;
    /// static CH: Channel<u8, 4> = Channel::new();
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let last = cx.signal(0u8);
    /// cx.on_message(&CH, move |v| last.set(v));
    /// CH.try_send(1).unwrap();
    /// CH.try_send(2).unwrap();
    /// assert_eq!(rt.drain_channels(1), 1);
    /// assert_eq!(last.get(), 1);
    /// assert_eq!(rt.drain_channels(8), 1);
    /// assert_eq!(last.get(), 2);
    /// ```
    pub fn drain_channels(self, max_per_channel: usize) -> usize {
        drain_channels(None, max_per_channel)
    }

    /// Whether any channel with an `on_message` registration (of any root) has queued
    /// messages. Allocates nothing; never panics. A `Ui` asks
    /// [`Scope::any_channel_pending`] for its own root instead.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// assert!(!rt.any_channel_pending());
    /// ```
    #[must_use]
    pub fn any_channel_pending(self) -> bool {
        with_runtime(|rt| rt.inner.borrow().channels.iter().any(|r| r.chan.pending()))
    }
}

impl Scope {
    /// [`Runtime::drain_channels`] for the registrations of this scope's **root** only (the
    /// handlers registered anywhere in the root's tree): what a `Ui`'s update calls on its
    /// root scope, so the handlers of another UI sharing the runtime run only in that UI's
    /// update — with that UI's engine lent, never with this one's. Returns the number of
    /// messages handled; `0` for a disposed scope. Allocates nothing; never panics (unless a
    /// handler does).
    ///
    /// ```
    /// use twine_reactive::Channel;
    /// static A: Channel<u8, 4> = Channel::new();
    /// static B: Channel<u8, 4> = Channel::new();
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let (ui_a, ui_b) = (rt.create_root(), rt.create_root());
    /// let (got_a, got_b) = (ui_a.signal(0u8), ui_b.signal(0u8));
    /// ui_a.child().on_message(&A, move |v| got_a.set(v));
    /// ui_b.on_message(&B, move |v| got_b.set(v));
    /// A.try_send(1).unwrap();
    /// B.try_send(2).unwrap();
    /// assert_eq!(ui_a.drain_channels(16), 1); // only A's handler ran
    /// assert_eq!((got_a.get(), got_b.get()), (1, 0));
    /// assert!(ui_b.any_channel_pending());
    /// assert_eq!(ui_b.drain_channels(16), 1);
    /// assert_eq!(got_b.get(), 2);
    /// ```
    pub fn drain_channels(self, max_per_channel: usize) -> usize {
        match with_runtime(|rt| rt.inner.borrow().root_of(self.key())) {
            Some(root) => drain_channels(Some(root), max_per_channel),
            None => 0,
        }
    }

    /// Whether a channel with a registration in this scope's **root** tree has queued
    /// messages (see [`drain_channels`](Self::drain_channels)); `false` for a disposed scope.
    /// One comparison per registration; allocates nothing; never panics.
    ///
    /// ```
    /// use twine_reactive::Channel;
    /// static CH: Channel<u8, 4> = Channel::new();
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let (ui_a, ui_b) = (rt.create_root(), rt.create_root());
    /// ui_a.on_message(&CH, |_| {});
    /// CH.try_send(1).unwrap();
    /// assert!(ui_a.any_channel_pending());
    /// assert!(!ui_b.any_channel_pending()); // not B's channel
    /// ```
    #[must_use]
    pub fn any_channel_pending(self) -> bool {
        with_runtime(|rt| {
            let inner = rt.inner.borrow();
            let Some(root) = inner.root_of(self.key()) else {
                return false;
            };
            inner.channels.iter().any(|r| r.root == root && r.chan.pending())
        })
    }
}

/// See [`Runtime::drain_channels`] (`root: None`) and [`Scope::drain_channels`] (`Some`: only
/// the registrations of that root).
fn drain_channels(root: Option<ScopeKey>, max_per_channel: usize) -> usize {
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
                    let mine = root.is_none_or(|r| r == reg.root);
                    (reg.id, if mine { reg.drain.take() } else { None })
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
