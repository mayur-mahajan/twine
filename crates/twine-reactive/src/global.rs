//! The [`Runtime`] token and access to the runtime state of the current UI context.
//!
//! - With the `std` feature the state is a `thread_local!`: every thread has its own,
//!   independent runtime.
//! - Without `std` the state is one `static`. It is sound because every path into it needs a
//!   [`Runtime`] token or a handle derived from one (see the `SAFETY` argument below): the
//!   first token comes from [`Runtime::take`], which succeeds once per process, and tokens and
//!   handles are `!Send + !Sync`, so they never leave the execution context that took it.
//!
//! [`with_runtime`] is the only accessor. It may be re-entered (it hands out a shared
//! `&RuntimeState`); rule R1 governs borrowing the state's `inner` cell, not this function.
//!
//! The interrupt probe ([`set_interrupt_probe`]) is kept apart from the runtime state, in
//! storage that any context may read (a `thread_local!` with `std`, a `critical_section`
//! mutex without), because its whole point is to be asked from the wrong context.

use core::marker::PhantomData;

use twine_core::fault::FaultCounts;

use crate::runtime::{FaultHook, RuntimeState};
use crate::scope::Scope;
use crate::{Stats, runtime};

#[cfg(feature = "std")]
mod imp {
    use core::cell::Cell;

    use crate::runtime::RuntimeState;

    std::thread_local! {
        static RUNTIME: RuntimeState = const { RuntimeState::new() };
        /// Whether `Runtime::take` succeeded on this thread.
        static TAKEN: Cell<bool> = const { Cell::new(false) };
        /// The interrupt probe of this thread's runtime.
        static PROBE: Cell<Option<fn() -> bool>> = const { Cell::new(None) };
    }

    /// Runs `f` with this thread's runtime state.
    #[inline]
    pub(crate) fn with_runtime<R>(f: impl FnOnce(&RuntimeState) -> R) -> R {
        #[cfg(feature = "debug-checks")]
        super::check_context();
        RUNTIME.with(f)
    }

    /// Claims this thread's token; `false` if it was claimed before.
    pub(crate) fn claim() -> bool {
        TAKEN.with(|t| !t.replace(true))
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

    use crate::runtime::RuntimeState;

    /// An interrupt probe (`set_interrupt_probe`).
    type Probe = fn() -> bool;

    /// The runtime state in a `static`. `RuntimeState` holds `Rc`/`RefCell`s and is neither
    /// `Send` nor `Sync`; this wrapper asserts `Sync` under the argument below.
    struct RuntimeSlot(RuntimeState);

    // SAFETY: Soundness argument. The state is only reached through `with_runtime`, and every
    // public path to `with_runtime` needs a `Runtime` token or a handle (`Scope`, `Signal`,
    // `Memo`, `EffectId`, `StoredValue`, …): the operations of this crate that reach the
    // runtime are methods of `Runtime` or of a handle, and the only free functions left
    // (`set_interrupt_probe`, `interrupt_probe`, `in_interrupt`, and everything on `Channel`,
    // `UiWaker`, `WakerLease`) never touch `RUNTIME`. A handle can only be made from a scope,
    // and a root scope only from a token (`Runtime::create_root`), so every handle descends
    // from a token. The first token is made by `Runtime::take`, which succeeds once per
    // process (`TAKEN` is checked and set inside one critical section, so two contexts racing
    // for it cannot both win, and it is never cleared); further tokens are copies of it or
    // come from handles (`Scope::runtime`). Tokens and handles are `!Send + !Sync`
    // (`PhantomData<*const ()>`), so safe code cannot move them to, or share them with,
    // another thread, task, core or interrupt handler (a `static` needs `Sync`; spawning on
    // another context needs `Send`). Hence the `RefCell`/`Rc` state inside `RuntimeState` is
    // only accessed from the one execution context that took the token, exactly as if it were
    // a thread-local. The static is never dropped, so no destructor runs in another context.
    #[allow(unsafe_code)]
    unsafe impl Sync for RuntimeSlot {}

    static RUNTIME: RuntimeSlot = RuntimeSlot(RuntimeState::new());
    /// Whether `Runtime::take` succeeded (never cleared: see `Runtime::take`).
    static TAKEN: AtomicBool = AtomicBool::new(false);
    /// The interrupt probe (readable from any context).
    static PROBE: Mutex<Cell<Option<Probe>>> = Mutex::new(Cell::new(None));

    /// Runs `f` with the runtime state. No check: reaching this function proves that the
    /// caller holds a token or a handle (see the soundness argument above).
    #[inline]
    pub(crate) fn with_runtime<R>(f: impl FnOnce(&RuntimeState) -> R) -> R {
        #[cfg(feature = "debug-checks")]
        super::check_context();
        f(&RUNTIME.0)
    }

    /// Claims the process-wide token; `false` if it was claimed before. A critical section
    /// makes the check and the claim one step (no compare-and-swap, which thumbv6m lacks).
    pub(crate) fn claim() -> bool {
        critical_section::with(|_| {
            if TAKEN.load(Ordering::Relaxed) {
                false
            } else {
                TAKEN.store(true, Ordering::Relaxed);
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

/// `debug-checks`: panics when the runtime is reached from interrupt context according to the
/// registered probe (see [`set_interrupt_probe`]). Out of line: one call per runtime access in
/// builds that opt in, nothing otherwise.
#[cfg(feature = "debug-checks")]
#[inline(never)]
fn check_context() {
    if let Some(probe) = imp::probe() {
        if probe() {
            used_from_interrupt();
        }
    }
}

#[cfg(feature = "debug-checks")]
#[cold]
#[inline(never)]
fn used_from_interrupt() -> ! {
    twine_core::log::error!(
        target: "twine::reactive",
        "reactive runtime used from interrupt context (send a message through a Channel instead)"
    );
    panic!(
        "twine-reactive: the reactive runtime was used from interrupt context; only `Channel` and \
         `UiWaker` may be used there (debug-checks)"
    )
}

/// The proof that the calling execution context owns the reactive runtime: the token every
/// runtime-wide operation needs (creating a root scope, batching, flushing, draining channels,
/// the ambient slot, statistics and faults).
///
/// `Runtime` is a zero-sized, `Copy`, `!Send` and `!Sync` value. The first one comes from
/// [`Runtime::take`]; copies of it, and the one any [`Scope`] returns
/// ([`Scope::runtime`]), are equally valid. Because no token can leave the context that took
/// it — and every handle (`Scope`, `Signal`, …) descends from a token — the runtime is only
/// ever used from that one context, without `unsafe` at the call site and without a run-time
/// check. An interrupt handler, another core or another preempting task cannot reach it; they
/// talk to the UI through [`Channel`](crate::Channel) and [`UiWaker`](crate::UiWaker), which
/// need no token.
///
/// - **`std`**: every thread has its own runtime; [`take`](Self::take) succeeds once per
///   thread, and `Runtime::current_thread` (`std` only) returns the calling thread's token
///   at any time (tests, tools, host code).
/// - **without `std`**: one runtime per process; [`take`](Self::take) succeeds once.
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
/// The token cannot be sent to another thread (so neither can anything made from it):
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
// NOTE(R5.S07): the token becomes the runtime identity (a partition id); handles made from a
// token then carry it, and `with_runtime` selects the partition's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Runtime {
    _context: PhantomData<*const ()>,
}

impl Runtime {
    /// A token; only for the constructors below and for handles (which descend from one).
    pub(crate) const fn token() -> Runtime {
        Runtime {
            _context: PhantomData,
        }
    }

    /// Takes the runtime for the calling execution context: `Some` the first time, `None`
    /// afterwards (without `std`: once per process; with `std`: once per thread).
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
        imp::claim().then(Runtime::token)
    }

    /// The calling thread's runtime (`std` only), whether or not [`take`](Self::take) was
    /// called on this thread; it does not claim it either. With `std` every thread has its own
    /// runtime, so a token for it can be handed out at any time: this is the convenience for
    /// tests, test harnesses, simulators and host tools. Portable code uses
    /// [`take`](Self::take). Allocates nothing; never panics.
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
        Runtime::token()
    }

    /// Creates a new root scope.
    ///
    /// Multiple roots may coexist (each `Ui` owns one). A root lives until it is
    /// [disposed](Scope::dispose). Allocates the scope's arena slot (amortized); panics only
    /// when the arena already holds 65 535 live scopes.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let n = cx.signal(1);
    /// assert_eq!(n.get(), 1);
    /// cx.dispose();
    /// ```
    #[must_use]
    pub fn create_root(self) -> Scope {
        create_root()
    }

    /// Drops every node, scope, channel registration and pending effect of the runtime.
    /// **Tests only**: cleanups do not run and all existing handles become dead.
    #[doc(hidden)]
    pub fn reset(self) {
        with_runtime(RuntimeState::reset);
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
        with_runtime(|rt| {
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
                retired_slots: inner.nodes.retired() + inner.scopes.retired(),
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
        with_runtime(|rt| rt.inner.borrow().memory())
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
        with_runtime(|rt| core::mem::take(&mut rt.faults.borrow_mut().new))
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
        with_runtime(|rt| rt.faults.borrow_mut().hook = hook);
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
        Runtime::token()
    }
}

/// Creates a new root scope (lazily initializing the runtime); see [`Runtime::create_root`].
pub(crate) fn create_root() -> Scope {
    with_runtime(|rt| match rt.create_scope(None) {
        Some(key) => Scope::from_key(key),
        None => unreachable!("a root scope has no parent that could be dead"),
    })
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
/// - **Per runtime**: with the `std` feature one per thread, without it one per process. A
///   new probe replaces the previous one; it stays set when a `Ui` is dropped (it describes
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

/// Records `n` occurrences of `kind` in the current runtime (crate-internal fault sites that
/// have no `&RuntimeState` at hand).
pub(crate) fn record_fault(kind: twine_core::fault::FaultKind, n: u32) {
    with_runtime(|rt| rt.record_fault(kind, n));
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
                Runtime::token()
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
