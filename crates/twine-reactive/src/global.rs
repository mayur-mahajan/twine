//! The [`Runtime`] token and access to the runtime state of each partition.
//!
//! There is one runtime state per [`Partition`] (`Partition::COUNT` slots: 1 without the
//! `partitions` feature, 4 with it, ADR-001). Tokens and handles carry their partition, and
//! [`with_runtime`]`(p, f)` selects slot `p` (an address computation: the partition is an
//! enum below `COUNT`; without the feature the selection folds away and the code is the
//! single-runtime code).
//!
//! - With the `std` feature the slots are a `thread_local!` array: every thread has its own,
//!   independent runtimes.
//! - Without `std` the slots are one `static` array. It is sound because every path into slot
//!   `p` needs a [`Runtime`] token or a handle of partition `p` (see the `SAFETY` argument
//!   below): the first token of `p` comes from [`Runtime::take_partition`], which succeeds
//!   once per process and partition, and tokens and handles are `!Send + !Sync`, so they never
//!   leave the execution context that took it.
//!
//! [`with_runtime`] is the only accessor. It may be re-entered (it hands out a shared
//! `&RuntimeState`); rule R1 governs borrowing the state's `inner` cell, not this function.
//!
//! The interrupt probe ([`set_interrupt_probe`]) is kept apart from the runtime state, in
//! storage that any context may read (a `thread_local!` with `std`, a `critical_section`
//! mutex without), because its whole point is to be asked from the wrong context.

use core::marker::PhantomData;

use twine_core::fault::{CapacityFault, FaultCounts};
use twine_core::{Limits, Partition};

use crate::error::ReactiveError;
use crate::runtime::{FaultHook, RuntimeState};
use crate::scope::Scope;
use crate::{Stats, runtime};

// twine-core decides the partition count from its own `partitions` feature; this crate's code
// (runtime slots, `Channel`/`Latest` ownership, the debug check) follows its own feature. Cargo
// can turn on twine-core's without this crate's (a direct `twine-core/partitions` dependency);
// partitions would then exist without isolation, so the build is refused with the fix.
const _: () = assert!(
    cfg!(feature = "partitions") == (Partition::COUNT > 1),
    "twine-core's `partitions` feature is on but twine-reactive's is off: enable `twine/partitions` \
     (or `twine-reactive/partitions`); see docs/design/adr-001-partitioned-runtimes.md"
);

/// One runtime state per partition (`Partition::COUNT` slots), each knowing its partition.
const fn slots() -> [RuntimeState; Partition::COUNT] {
    #[cfg(feature = "partitions")]
    {
        let [a, b, c, d] = Partition::ALL;
        [
            RuntimeState::new(a),
            RuntimeState::new(b),
            RuntimeState::new(c),
            RuntimeState::new(d),
        ]
    }
    #[cfg(not(feature = "partitions"))]
    {
        [RuntimeState::new(Partition::MAIN)]
    }
}

#[cfg(feature = "std")]
mod imp {
    use core::cell::Cell;

    use twine_core::Partition;

    use crate::runtime::RuntimeState;

    std::thread_local! {
        /// This thread's runtime of each partition.
        static RUNTIMES: [RuntimeState; Partition::COUNT] = const { super::slots() };
        /// Whether `Runtime::take_partition` succeeded on this thread, per partition.
        static TAKEN: [Cell<bool>; Partition::COUNT] =
            const { [const { Cell::new(false) }; Partition::COUNT] };
        /// The interrupt probe of this thread's runtimes.
        static PROBE: Cell<Option<fn() -> bool>> = const { Cell::new(None) };
    }

    /// Runs `f` with this thread's runtime state of partition `p`.
    #[inline]
    pub(crate) fn with_runtime<R>(p: Partition, f: impl FnOnce(&RuntimeState) -> R) -> R {
        #[cfg(feature = "debug-checks")]
        super::check_context(p);
        RUNTIMES.with(|slots| f(p.select(slots)))
    }

    /// Claims this thread's token of partition `p`; `false` if it was claimed before.
    pub(crate) fn claim(p: Partition) -> bool {
        TAKEN.with(|t| !p.select(t).replace(true))
    }

    pub(crate) fn probe() -> Option<fn() -> bool> {
        PROBE.with(Cell::get)
    }

    pub(crate) fn set_probe(probe: Option<fn() -> bool>) {
        PROBE.with(|p| p.set(probe));
    }
}

#[cfg(not(feature = "std"))]
mod imp {
    use core::cell::Cell;

    use critical_section::Mutex;
    use portable_atomic::{AtomicBool, Ordering};
    use twine_core::Partition;

    use crate::runtime::RuntimeState;

    /// An interrupt probe (`set_interrupt_probe`).
    type Probe = fn() -> bool;

    /// The partitions' runtime states in a `static` (one slot per partition). `RuntimeState`
    /// holds `Rc`/`RefCell`s and is neither `Send` nor `Sync`; this wrapper asserts `Sync`
    /// under the argument below.
    struct RuntimeSlots([RuntimeState; Partition::COUNT]);

    // SAFETY: Soundness argument, per slot (ADR-001 § 5.1; R3.S02's argument applied to each
    // partition). Slot `p` is only reached through `with_runtime(p, ..)`, and every public
    // path to `with_runtime` needs a `Runtime` token or a handle (`Scope`, `Signal`, `Memo`,
    // `EffectId`, `StoredValue`, …) and passes *its* partition: the operations of this crate
    // that reach the runtime are methods of `Runtime` or of a handle, and the only free
    // functions left (`set_interrupt_probe`, `interrupt_probe`, `in_interrupt`, and everything
    // on `Channel`, `Latest`, `UiWaker`, `WakerLease`) never touch `RUNTIMES`. A token of
    // partition `p` is made only by `Runtime::take_partition(p)`, which succeeds once per
    // process and partition (`TAKEN[p]` is checked and set inside one critical section, so two
    // contexts racing for the same partition cannot both win, and it is never cleared); further
    // tokens of `p` are copies of it or come from handles of `p` (`Scope::runtime`). A handle
    // can only be made from a scope, a root scope only from a token (`Runtime::create_root`),
    // and every handle records its creator's partition and never changes it, so every handle
    // of `p` descends from the one successful `take_partition(p)`. Tokens and handles are
    // `!Send + !Sync` (`PhantomData<*const ()>`), so safe code cannot move them to, or share
    // them with, another thread, task, core or interrupt handler (a `static` needs `Sync`;
    // spawning on another context needs `Send`). Hence the `RefCell`/`Rc` state inside slot
    // `p` is only accessed from the one execution context that took partition `p`, exactly
    // as if it were a thread-local; two partitions taken by two contexts (two cores, two
    // preempting tasks) touch two disjoint slots. The partition index selects the slot by its
    // type (`Partition::select`: an enum below `COUNT`), so a handle cannot address another
    // slot. The statics are never dropped, so no destructor runs in another context.
    #[allow(unsafe_code)]
    unsafe impl Sync for RuntimeSlots {}

    /// One runtime state per partition.
    static RUNTIMES: RuntimeSlots = RuntimeSlots(super::slots());
    /// Whether `Runtime::take_partition` succeeded, per partition (never cleared: see
    /// `Runtime::take`).
    static TAKEN: [AtomicBool; Partition::COUNT] = [const { AtomicBool::new(false) }; Partition::COUNT];
    /// The interrupt probe (readable from any context).
    static PROBE: Mutex<Cell<Option<Probe>>> = Mutex::new(Cell::new(None));

    /// Runs `f` with the runtime state of partition `p`. No check: reaching this function
    /// proves that the caller holds a token or a handle of `p` (see the soundness argument
    /// above).
    #[inline]
    pub(crate) fn with_runtime<R>(p: Partition, f: impl FnOnce(&RuntimeState) -> R) -> R {
        #[cfg(feature = "debug-checks")]
        super::check_context(p);
        f(p.select(&RUNTIMES.0))
    }

    /// Claims the process-wide token of partition `p`; `false` if it was claimed before. A
    /// critical section makes the check and the claim one step (no compare-and-swap, which
    /// thumbv6m lacks).
    pub(crate) fn claim(p: Partition) -> bool {
        let taken = p.select(&TAKEN);
        critical_section::with(|_| {
            if taken.load(Ordering::Relaxed) {
                false
            } else {
                taken.store(true, Ordering::Relaxed);
                true
            }
        })
    }

    pub(crate) fn probe() -> Option<fn() -> bool> {
        critical_section::with(|cs| PROBE.borrow(cs).get())
    }

    pub(crate) fn set_probe(probe: Option<fn() -> bool>) {
        critical_section::with(|cs| PROBE.borrow(cs).set(probe));
    }
}

pub(crate) use imp::with_runtime;

/// `debug-checks` + `partitions` + `std` (host tests, `TestUi`, `twine-sim`): the partition
/// whose flush, update or batch runs on this thread (ADR-001 § 5.4), so that a runtime access
/// through a handle of another partition meanwhile is caught.
#[cfg(all(feature = "debug-checks", feature = "partitions", feature = "std"))]
mod running {
    use core::cell::Cell;

    use twine_core::Partition;

    std::thread_local! {
        static RUNNING: Cell<Option<Partition>> = const { Cell::new(None) };
    }

    /// The partition running on this thread, if any.
    pub(super) fn get() -> Option<Partition> {
        RUNNING.with(Cell::get)
    }

    /// Restores the previously running partition when dropped (also on unwinding).
    pub(crate) struct Running(Option<Partition>);

    impl Drop for Running {
        fn drop(&mut self) {
            let prev = self.0;
            RUNNING.with(|r| r.set(prev));
        }
    }

    /// Marks `p` as running until the returned guard is dropped.
    pub(crate) fn enter(p: Partition) -> Running {
        Running(RUNNING.with(|r| r.replace(Some(p))))
    }
}

/// Without the cross-partition check: nothing to record (zero-sized, no code).
#[cfg(not(all(feature = "debug-checks", feature = "partitions", feature = "std")))]
mod running {
    use twine_core::Partition;

    /// Zero-sized: there is no check to feed.
    pub(crate) struct Running;

    #[inline(always)]
    pub(crate) fn enter(_p: Partition) -> Running {
        Running
    }
}

pub(crate) use running::enter as enter_partition;

/// `debug-checks`: panics when the runtime is reached from interrupt context according to the
/// registered probe (see [`set_interrupt_probe`]), and — with `partitions` and `std` — when
/// partition `p`'s runtime is reached while another partition's flush, update or batch runs
/// on this thread (a handle shared across partitions, ADR-001 § 5.4). Out of line: one call
/// per runtime access in builds that opt in, nothing otherwise.
#[cfg(feature = "debug-checks")]
#[inline(never)]
fn check_context(p: Partition) {
    if let Some(probe) = imp::probe() {
        if probe() {
            context_violation(
                "the reactive runtime was used from interrupt context; only `Channel` and \
                 `UiWaker` may be used there",
            );
        }
    }
    #[cfg(all(feature = "partitions", feature = "std"))]
    if let Some(running) = running::get() {
        if running != p {
            context_violation(
                "a handle of one partition was used while another partition's flush, update or \
                 batch was running; pass data between partitions through `Latest`, `Channel` \
                 or `Outbox`",
            );
        }
    }
    #[cfg(not(all(feature = "partitions", feature = "std")))]
    let _ = p;
}

/// The failure path of [`check_context`]: logs, then panics with `what`.
#[cfg(feature = "debug-checks")]
#[cold]
#[inline(never)]
fn context_violation(what: &'static str) -> ! {
    twine_core::log::error!(target: "twine::reactive", "twine-reactive: {} (debug-checks)", what);
    // PANIC: `debug-checks` only (a debugging aid): the runtime reached from interrupt context
    // through `unsafe` code would be a data race, and a handle used across partitions breaks
    // their isolation; stopping is the only sound reaction.
    panic!("twine-reactive: {what} (debug-checks)")
}

/// The proof that the calling execution context owns a reactive runtime — a **partition**'s
/// runtime: the token every runtime-wide operation needs (creating a root scope, batching,
/// flushing, draining channels, the ambient slot, statistics and faults).
///
/// `Runtime` is a `Copy`, `!Send` and `!Sync` value naming its [`Partition`] (zero-sized
/// without the `partitions` feature, one byte with it). The first token of a partition comes
/// from [`Runtime::take_partition`] ([`Runtime::take`] for [`Partition::MAIN`]); copies of it,
/// and the one any handle of that partition returns ([`Scope::runtime`]), are equally valid.
/// Because no token can leave the context that took it — and every handle (`Scope`, `Signal`,
/// …) descends from a token and carries its partition — each partition's runtime is only ever
/// used from that one context, without `unsafe` at the call site and without a run-time check.
/// An interrupt handler, another core or another preempting task cannot reach it; they talk to
/// the UI through [`Channel`](crate::Channel), [`Latest`](crate::Latest) and
/// [`UiWaker`](crate::UiWaker), which need no token.
///
/// - **`std`**: every thread has its own runtimes; [`take_partition`](Self::take_partition)
///   succeeds once per thread and partition, and `Runtime::current_thread` /
///   `current_thread_partition` (`std` only) return the calling thread's token at any time
///   (tests, tools, host code).
/// - **without `std`**: one runtime per partition and process; `take_partition` succeeds once
///   per partition.
///
/// # Partitions
///
/// With the `partitions` feature there are [`Partition::COUNT`] = 4 independent runtimes
/// (ADR-001, `docs/design/adr-001-partitioned-runtimes.md`). A partition is one runtime and
/// every `Ui` built on its token; two partitions share no state — arenas, effect queues and
/// loop cut, [`Limits`], faults and fault hook, seal domain — so a safety-critical display
/// and a non-critical one stay independent: an effect loop, an exhausted bound or a fault in
/// one never touches the other, and they may run in different, preempting contexts (two
/// cores, two RTOS tasks). Every handle carries its partition and reaches only that
/// partition's runtime. Pass data between partitions only through `Latest`, `Channel` or
/// `Outbox`; with `debug-checks` on the host, using a handle of one partition while another
/// partition's update runs panics. Without the feature `take_partition` of anything but
/// `MAIN` returns `None` and the runtime compiles to a single state, as before.
///
/// ```
/// use twine_reactive::Runtime;
///
/// let rt = Runtime::take().expect("runtime already taken");
/// assert!(Runtime::take().is_none()); // one owner
/// let cx = rt.create_root();
/// let n = cx.signal(1);
/// rt.batch(|| n.set(2));
/// assert_eq!(n.get(), 2);
/// assert_eq!(cx.runtime(), rt); // every handle leads back to its runtime
/// cx.dispose();
/// ```
///
/// The token cannot be sent to another thread (so neither can anything made from it), in any
/// partition:
///
/// ```compile_fail
/// let rt = twine_reactive::Runtime::take().unwrap();
/// std::thread::spawn(move || rt.create_root());
/// ```
///
/// ```compile_fail
/// fn assert_send<T: Send>(_: T) {}
/// assert_send(twine_reactive::Runtime::take().unwrap());
/// ```
///
/// ```compile_fail
/// use twine_core::Partition;
/// fn assert_send<T: Send>(_: T) {}
/// let p = Partition::ALL[Partition::COUNT - 1];
/// assert_send(twine_reactive::Runtime::take_partition(p).unwrap());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Runtime {
    part: Partition,
    _context: PhantomData<*const ()>,
}

// Without `partitions` the token is zero-sized (ADR-001 § 6.2: zero cost); with it, one byte.
// Keyed on `Partition` (twine-core decides its size from its own `partitions` feature, which can
// be on without this crate's), so the check holds under any feature unification.
const _: () = assert!(core::mem::size_of::<Runtime>() == core::mem::size_of::<Partition>());

impl Runtime {
    /// A token of partition `p`; only for the constructors below and for handles (which
    /// descend from one).
    pub(crate) const fn token(p: Partition) -> Runtime {
        Runtime {
            part: p,
            _context: PhantomData,
        }
    }

    /// Takes the runtime of [`Partition::MAIN`] for the calling execution context: `Some` the
    /// first time, `None` afterwards (without `std`: once per process; with `std`: once per
    /// thread). The same as `take_partition(Partition::MAIN)`: an application with one
    /// partition never names one.
    ///
    /// Call it once, early in the UI task (typically in `main`), and hand the token to the
    /// `Ui` (`UiBuilder::runtime`); copy it for anything else that needs it. The claim is
    /// never released, not even when every copy is dropped: handles made from the token may
    /// still exist in this context, so handing the runtime to another context later would
    /// not be sound.
    ///
    /// Allocates nothing; never panics. Without `std` it takes one critical section.
    ///
    /// ```
    /// use twine_reactive::Runtime;
    /// let rt = Runtime::take().expect("runtime already taken");
    /// assert!(Runtime::take().is_none());
    /// let _copy = rt; // `Copy`: still the same runtime
    /// ```
    #[must_use]
    pub fn take() -> Option<Runtime> {
        Self::take_partition(Partition::MAIN)
    }

    /// Takes the runtime of partition `p` for the calling execution context: `Some` the first
    /// time, `None` afterwards — once per partition and process without `std`, once per
    /// partition and thread with it. Each partition can be taken by a different context (two
    /// cores, two RTOS tasks, two executors), or several by one context (a super-loop stepping
    /// both UIs). Without the `partitions` feature only [`Partition::MAIN`] exists, so every
    /// other partition is `None` (it cannot be named: `Partition::new(1)` is `None` too).
    ///
    /// The claim is never released (see [`take`](Self::take)). Allocates nothing; never
    /// panics. Without `std` it takes one critical section.
    ///
    /// ```
    /// use twine_core::Partition;
    /// use twine_reactive::Runtime;
    ///
    /// let main = Runtime::take_partition(Partition::MAIN).expect("main partition free");
    /// assert_eq!(main.partition(), Partition::MAIN);
    /// assert!(Runtime::take().is_none()); // `take` claims the same partition
    /// if let Some(p) = Partition::new(1) {
    ///     // `partitions` feature: a second, independent runtime.
    ///     let other = Runtime::take_partition(p).unwrap();
    ///     assert_ne!(other, main);
    /// }
    /// ```
    #[must_use]
    pub fn take_partition(p: Partition) -> Option<Runtime> {
        imp::claim(p).then(|| Runtime::token(p))
    }

    /// The partition this token is the runtime of. Free; never panics.
    ///
    /// ```
    /// use twine_core::Partition;
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// assert_eq!(rt.partition(), Partition::MAIN);
    /// assert_eq!(rt.create_root().partition(), Partition::MAIN);
    /// ```
    #[must_use]
    #[inline]
    pub const fn partition(self) -> Partition {
        self.part
    }

    /// The calling thread's runtime of [`Partition::MAIN`] (`std` only), whether or not
    /// [`take`](Self::take) was called on this thread; it does not claim it either. With `std`
    /// every thread has its own runtimes, so a token for them can be handed out at any time:
    /// this is the convenience for tests, test harnesses, simulators and host tools. Portable
    /// code uses [`take`](Self::take). Allocates nothing; never panics.
    ///
    /// ```
    /// use twine_reactive::Runtime;
    /// let rt = Runtime::current_thread();
    /// let cx = rt.create_root();
    /// assert!(cx.is_alive());
    /// assert!(Runtime::take().is_some()); // not claimed by `current_thread`
    /// cx.dispose();
    /// ```
    #[cfg(feature = "std")]
    #[must_use]
    pub fn current_thread() -> Runtime {
        Runtime::token(Partition::MAIN)
    }

    /// The calling thread's runtime of partition `p` (`std` only), without claiming it: like
    /// [`current_thread`](Self::current_thread) for another partition (tests that run two
    /// partitions on one thread, `TestUi` on a chosen partition, the simulator). Allocates
    /// nothing; never panics.
    ///
    /// ```
    /// use twine_core::Partition;
    /// use twine_reactive::Runtime;
    /// let last = Partition::ALL[Partition::COUNT - 1];
    /// let rt = Runtime::current_thread_partition(last);
    /// assert_eq!(rt.partition(), last);
    /// assert!(Runtime::take_partition(last).is_some()); // not claimed
    /// ```
    #[cfg(feature = "std")]
    #[must_use]
    pub fn current_thread_partition(p: Partition) -> Runtime {
        Runtime::token(p)
    }

    /// Creates a new root scope.
    ///
    /// Multiple roots may coexist (each `Ui` owns one). A root lives until it is
    /// [disposed](Scope::dispose). Allocates the scope's arena slot (amortized).
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let n = cx.signal(1);
    /// assert_eq!(n.get(), 1);
    /// cx.dispose();
    /// ```
    ///
    /// # Panics
    ///
    /// Only when the runtime already holds [`Limits::max_scopes`](twine_core::Limits::max_scopes) live scopes. With `no-panic-runtime` it
    /// never panics: it returns a scope that is never alive after a `warn!` and a
    /// [`Capacity`](crate::FaultKind::Capacity) fault.
    /// [`try_create_root`](Self::try_create_root) returns the error instead.
    #[must_use]
    #[inline]
    #[track_caller]
    pub fn create_root(self) -> Scope {
        match try_create_root(self.part) {
            Ok(s) => s,
            Err(e) => crate::scope::dead_scope(self.part, e),
        }
    }

    /// Like [`create_root`](Self::create_root), returning
    /// [`ReactiveError::Capacity`] (`HandleKind::Scope`) when the runtime already holds `Limits::max_scopes`
    /// live scopes instead of panicking. Raises no fault.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.try_create_root().expect("fewer than 65 535 scopes");
    /// assert!(cx.is_alive());
    /// cx.dispose();
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Capacity`], as above.
    pub fn try_create_root(self) -> Result<Scope, ReactiveError> {
        try_create_root(self.part)
    }

    /// Drops every node, scope, channel registration and pending effect of the runtime.
    /// **Tests only**: cleanups do not run and all existing handles become dead.
    #[doc(hidden)]
    pub fn reset(self) {
        with_runtime(self.part, RuntimeState::reset);
    }

    /// Live object counts and counters of the runtime (diagnostics, tests and fault analysis).
    /// Allocates nothing; never panics.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let _a = cx.signal(1);
    /// assert!(rt.stats().nodes >= 1);
    /// cx.dispose();
    /// ```
    #[must_use]
    pub fn stats(self) -> Stats {
        with_runtime(self.part, |rt| {
            let inner = rt.inner.borrow();
            let runtime::Counters {
                writes,
                effect_runs,
                memo_runs,
            } = inner.counters;
            let faults = rt.faults.borrow().total;
            Stats {
                nodes: inner.nodes.len(),
                scopes: inner.scopes.len(),
                pending: inner.pending.len(),
                deferred: inner.deferred.len(),
                channels: inner.channels.len(),
                writes,
                effect_runs,
                memo_runs,
                faults,
                retired_slots: inner.nodes.retired().saturating_add(inner.scopes.retired()),
            }
        })
    }

    /// What the runtime's heap is used for, by part ([`RuntimeMemory`](crate::RuntimeMemory):
    /// units and what is not included). Allocates nothing; never panics; O(nodes + scopes),
    /// so call it on demand (diagnostics, budgeting, tests), not every frame.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let _a = cx.signal([0u8; 64]);
    /// assert!(rt.memory().values >= 64);
    /// cx.dispose();
    /// ```
    #[must_use]
    pub fn memory(self) -> crate::RuntimeMemory {
        with_runtime(self.part, |rt| rt.inner.borrow().memory())
    }

    /// The faults the runtime recorded since the last call (occurrences per kind), clearing
    /// them. Allocates nothing; never panics.
    ///
    /// The runtime records [`FaultKind::EffectLoopCut`](crate::FaultKind::EffectLoopCut),
    /// [`FaultKind::DepthGuard`](crate::FaultKind::DepthGuard) and
    /// [`FaultKind::ChannelOverflow`](crate::FaultKind::ChannelOverflow). `Ui` takes them at
    /// every update and forwards them to the engine, so applications using `Ui` read all faults
    /// there; this method is for code using the runtime on its own. Lifetime totals are in
    /// [`stats`](Self::stats).
    ///
    /// ```
    /// use twine_core::fault::FaultKind;
    ///
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// rt.set_flush_iterations_limit(3);
    /// let a = cx.signal(0u32);
    /// cx.effect(move || a.set(a.get() + 1)); // an effect loop, cut by the runtime
    /// let f = rt.take_faults();
    /// assert_eq!(f.get(FaultKind::EffectLoopCut), 1);
    /// assert!(rt.take_faults().kinds().is_empty());
    /// cx.dispose();
    /// ```
    pub fn take_faults(self) -> FaultCounts {
        with_runtime(self.part, |rt| core::mem::take(&mut rt.faults.borrow_mut().new))
    }

    /// Which bound the runtime's [`FaultKind::Capacity`](crate::FaultKind::Capacity) faults
    /// since the last call were about: the occurrences per bound
    /// ([`CapacityFault::ReactiveNodes`], [`ReactiveScopes`](CapacityFault::ReactiveScopes),
    /// [`PendingEffects`](CapacityFault::PendingEffects)), clearing them. Their sum is the
    /// `Capacity` count of [`take_faults`](Self::take_faults); `Ui` forwards each as a record
    /// with its code. Allocates nothing; never panics.
    ///
    /// ```
    /// use twine_core::Limits;
    /// use twine_core::fault::CapacityFault;
    ///
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// rt.set_limits(&Limits::DEFAULT.max_scopes(1).preallocated()).unwrap();
    /// let cx = rt.create_root();
    /// assert!(cx.try_child().is_err()); // `try_*`: an error, no fault
    /// let counts = rt.take_capacity_faults();
    /// assert!(counts.iter().all(|&(_, n)| n == 0));
    /// assert_eq!(counts[1].0, CapacityFault::ReactiveScopes);
    /// cx.dispose();
    /// ```
    pub fn take_capacity_faults(self) -> [(CapacityFault, u32); 3] {
        let counts = with_runtime(self.part, |rt| {
            core::mem::take(&mut rt.faults.borrow_mut().capacity)
        });
        let [a, b, c] = counts;
        let [ka, kb, kc] = runtime::RUNTIME_BOUNDS;
        [(ka, a), (kb, b), (kc, c)]
    }

    /// Applies the runtime's share of `limits` to this partition's runtime (other partitions
    /// keep their own bounds; per thread with `std`): [`max_reactive_nodes`](Limits::max_reactive_nodes),
    /// [`max_scopes`](Limits::max_scopes) and
    /// [`max_pending_effects`](Limits::max_pending_effects) bound creations and the effect
    /// queues from now on (a creation beyond them fails with
    /// [`ReactiveError::Capacity`]; an effect beyond the queue bound is not queued and raises
    /// a `Capacity` fault with code [`CapacityFault::PendingEffects`]). With
    /// [`preallocate`](Limits::preallocate) it also reserves the node and scope arenas, both
    /// effect queues and the propagation stack, so staying within the bounds allocates
    /// nothing there. Lowering a bound below the current count refuses further creations
    /// until enough are disposed. The `Ui` builders call it with `AppConfig::limits`.
    ///
    /// Allocates (the reservation) only with `preallocate`; never panics.
    ///
    /// # Errors
    /// [`twine_core::Error::InvalidArgument`] from [`Limits::validate`]; nothing is applied.
    ///
    /// ```
    /// use twine_core::Limits;
    /// use twine_reactive::{HandleKind, ReactiveError};
    ///
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// rt.set_limits(&Limits::DEFAULT.max_reactive_nodes(2).preallocated()).unwrap();
    /// let cx = rt.create_root();
    /// let _a = cx.signal(1);
    /// let _b = cx.signal(2);
    /// assert_eq!(cx.try_signal(3).err(), Some(ReactiveError::Capacity(HandleKind::Signal)));
    /// cx.dispose();
    /// rt.set_limits(&Limits::DEFAULT.preallocated()).unwrap();
    /// ```
    pub fn set_limits(self, limits: &Limits) -> Result<(), twine_core::Error> {
        limits.validate()?;
        with_runtime(self.part, |rt| rt.inner.borrow_mut().set_limits(limits));
        Ok(())
    }

    /// Sets the function called for every fault the runtime records (`None` removes it), with
    /// the kind and the number of occurrences. It runs where the fault is detected — possibly
    /// in the middle of an effect flush — so keep it short: record, count or signal a
    /// supervisor. A new hook replaces the previous one.
    ///
    /// - **Re-entry:** the hook is called after the runtime's fault bookkeeping is released, so
    ///   it may call [`take_faults`](Self::take_faults), [`stats`](Self::stats) or
    ///   `set_fault_hook` itself without panicking. Since it can run in the middle of an
    ///   effect flush, it should not write signals or create reactive nodes: defer such work
    ///   (e.g. through a `static` flag the main loop reads).
    /// - Allocates nothing; never panics.
    ///
    /// ```
    /// use core::sync::atomic::{AtomicU32, Ordering};
    /// use twine_core::fault::FaultKind;
    ///
    /// static LOOP_CUTS: AtomicU32 = AtomicU32::new(0);
    /// fn on_fault(kind: FaultKind, n: u32) {
    ///     if kind == FaultKind::EffectLoopCut {
    ///         LOOP_CUTS.fetch_add(n, Ordering::Relaxed);
    ///     }
    /// }
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// rt.set_fault_hook(Some(on_fault));
    /// let cx = rt.create_root();
    /// rt.set_flush_iterations_limit(3);
    /// let a = cx.signal(0u32);
    /// cx.effect(move || a.set(a.get() + 1));
    /// assert_eq!(LOOP_CUTS.load(Ordering::Relaxed), 1);
    /// rt.set_fault_hook(None);
    /// cx.dispose();
    /// ```
    pub fn set_fault_hook(self, hook: Option<FaultHook>) {
        with_runtime(self.part, |rt| rt.faults.borrow_mut().hook = hook);
    }
}

impl From<Scope> for Runtime {
    /// The runtime `scope` belongs to (see [`Scope::runtime`]).
    #[inline]
    fn from(scope: Scope) -> Runtime {
        scope.runtime()
    }
}

impl Scope {
    /// The runtime this scope belongs to: a token as good as the one [`Runtime::take`]
    /// returned (a scope only exists in the context that owns its runtime). Free: the token
    /// is zero-sized. Never panics, also on a disposed scope.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let n = cx.signal(1);
    /// cx.runtime().batch(|| n.set(2)); // e.g. inside a handler that captured `cx`
    /// assert_eq!(n.get(), 2);
    /// cx.dispose();
    /// ```
    #[must_use]
    #[inline]
    pub fn runtime(self) -> Runtime {
        Runtime::token(self.partition())
    }
}

/// Creates a new root scope in partition `p` (lazily initializing the runtime); see
/// [`Runtime::try_create_root`].
pub(crate) fn try_create_root(p: Partition) -> Result<Scope, ReactiveError> {
    with_runtime(p, |rt| rt.create_scope(None)).map(|k| Scope::from_key(k, p))
}

/// Sets the function that tells whether the caller runs in interrupt context (`None` removes
/// it): the platform's answer (`twine_hal::Platform::in_interrupt`, installed by
/// `UiBuilder::platform`), typically reading an interrupt-status register.
///
/// The runtime is confined to the context that took its [`Runtime`] token, which the type
/// system enforces for safe code. The probe is the run-time safety net behind it: with the
/// `debug-checks` feature every access to the runtime asks the probe and **panics** when it
/// answers `true`. A reactive API reached from an interrupt handler — through `unsafe` code
/// or an interrupt-registration API that does not require `Send` — would be a data race, so
/// a fault-and-continue is not an option. Without `debug-checks` the runtime never calls the
/// probe: no cost.
///
/// - **Per execution context, for every partition**: with the `std` feature one per thread,
///   without it one per process (the probe reads per-core hardware state, so one function
///   answers correctly on several cores). A new probe replaces the previous one; it stays set when a `Ui` is dropped (it describes
///   the platform, not the `Ui`).
/// - **Any context**: the probe is stored apart from the runtime (a `thread_local!` with
///   `std`, a `critical_section` mutex without), so this function, [`interrupt_probe`] and
///   [`in_interrupt`] need no token and may be called from interrupt handlers.
/// - The probe must be cheap, side-effect free and callable from any context. A UI that runs
///   in an interrupt-driven context itself (an RTIC task, an interrupt-priority executor)
///   needs a probe that answers `false` there.
/// - Allocates nothing; never panics.
///
/// ```
/// fn never_in_interrupt() -> bool {
///     false
/// }
/// twine_reactive::set_interrupt_probe(Some(never_in_interrupt));
/// assert!(!twine_reactive::in_interrupt());
/// twine_reactive::set_interrupt_probe(None);
/// assert!(twine_reactive::interrupt_probe().is_none());
/// ```
pub fn set_interrupt_probe(probe: Option<fn() -> bool>) {
    imp::set_probe(probe);
}

/// The probe set with [`set_interrupt_probe`], if any. Callable from any context; allocates
/// nothing; never panics.
///
/// ```
/// assert!(twine_reactive::interrupt_probe().is_none()); // none by default
/// ```
#[must_use]
pub fn interrupt_probe() -> Option<fn() -> bool> {
    imp::probe()
}

/// Whether the caller runs in interrupt context according to the probe set with
/// [`set_interrupt_probe`] (`false` without one). Callable from any context; allocates
/// nothing; never panics.
///
/// ```
/// twine_reactive::set_interrupt_probe(Some(|| true)); // e.g. a mock platform in a handler
/// assert!(twine_reactive::in_interrupt());
/// twine_reactive::set_interrupt_probe(None);
/// assert!(!twine_reactive::in_interrupt());
/// ```
#[must_use]
pub fn in_interrupt() -> bool {
    interrupt_probe().is_some_and(|probe| probe())
}

/// Records `n` occurrences of `kind` in partition `p`'s runtime (crate-internal fault sites
/// that have no `&RuntimeState` at hand).
pub(crate) fn record_fault(p: Partition, kind: twine_core::fault::FaultKind, n: u32) {
    with_runtime(p, |rt| rt.record_fault(kind, n));
}

/// `runtime()` for a handle type: every handle exists only in its runtime's context, so it
/// can hand out that runtime's token (free: zero-sized).
macro_rules! handle_runtime {
    ($($ty:ident),*) => {$(
        impl<T> crate::$ty<T> {
            /// The runtime this handle belongs to (see [`Scope::runtime`]): lets code that only
            /// captured a handle reach the runtime-wide operations. Free (the token is
            /// zero-sized); never panics, also for a disposed node.
            ///
            /// ```
            /// let rt = twine_reactive::Runtime::take().unwrap();
            /// let cx = rt.create_root();
            #[doc = concat!("let h = cx.", handle_runtime!(@ctor $ty), ";")]
            /// assert_eq!(h.runtime(), rt);
            /// cx.dispose();
            /// ```
            #[must_use]
            #[inline]
            pub fn runtime(&self) -> Runtime {
                Runtime::token(self.node_part())
            }
        }
    )*};
    (@ctor Signal) => { "signal(1)" };
    (@ctor ReadSignal) => { "signal(1).read_only()" };
    (@ctor WriteSignal) => { "signal(1).write_only()" };
    (@ctor Memo) => { "memo(|| 1)" };
    (@ctor StoredValue) => { "stored_value(1)" };
}

handle_runtime!(Signal, ReadSignal, WriteSignal, Memo, StoredValue);
