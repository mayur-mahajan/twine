//! Access to the runtime of the current UI context.
//!
//! - With the `std` feature the runtime is a `thread_local!`: every thread has its own,
//!   independent runtime, so the implementation is sound without further contracts.
//! - Without `std` the runtime is a `static` guarded by a one-time, `unsafe` binding
//!   ([`bind_to_current_context`]) that states the single-context contract.
//!
//! [`with_runtime`] is the only accessor. It may be re-entered (it hands out a shared
//! `&Runtime`); rule R1 governs borrowing the runtime's `inner` cell, not this function.

use twine_core::fault::{FaultCounts, FaultKind};

use crate::runtime::{FaultHook, Runtime};
use crate::scope::Scope;
use crate::{Stats, runtime};

#[cfg(feature = "std")]
mod imp {
    use crate::runtime::Runtime;

    std::thread_local! {
        static RUNTIME: Runtime = const { Runtime::new() };
    }

    /// Runs `f` with this thread's runtime.
    pub(crate) fn with_runtime<R>(f: impl FnOnce(&Runtime) -> R) -> R {
        RUNTIME.with(f)
    }
}

#[cfg(not(feature = "std"))]
mod imp {
    use portable_atomic::{AtomicBool, Ordering};

    use crate::runtime::Runtime;

    /// The runtime in a `static`. `Runtime` holds `Rc`/`RefCell`s and is neither `Send` nor
    /// `Sync`; this wrapper asserts `Sync` under the contract below.
    struct RuntimeSlot(Runtime);

    // SAFETY: Soundness argument. The runtime is only ever touched through
    // `with_runtime`, which refuses to run (panics) until `bind_to_current_context` has been
    // called. That function is `unsafe` and its caller guarantees that from then on every
    // `twine-reactive` API that reaches the runtime — all handle methods and the free
    // functions (`create_root`, `batch`, `untrack`, `flush_effects_with`, `drain_channels`, …)
    // — is used from that one execution context only (the UI thread/task), never from an
    // interrupt handler, another core or another preempting context. Handles (`Scope`,
    // `Signal`, `Memo`, `EffectId`, …) are `!Send + !Sync`, so safe code cannot move them to
    // another thread; the free functions are covered by the binding contract. Hence the
    // `RefCell`/`Rc` state inside `Runtime` is only accessed from one context, exactly as if it
    // were a thread-local. `Channel` — the only cross-context type — never touches `RUNTIME`
    // (it uses `critical_section` and atomics), so ISRs and other tasks may use channels
    // freely. The static is never dropped, so no destructor runs in another context.
    #[allow(unsafe_code)]
    unsafe impl Sync for RuntimeSlot {}

    static RUNTIME: RuntimeSlot = RuntimeSlot(Runtime::new());
    static BOUND: AtomicBool = AtomicBool::new(false);

    /// Runs `f` with the runtime; panics if it was not bound yet.
    pub(crate) fn with_runtime<R>(f: impl FnOnce(&Runtime) -> R) -> R {
        if !BOUND.load(Ordering::Acquire) {
            unbound();
        }
        f(&RUNTIME.0)
    }

    #[cold]
    #[inline(never)]
    fn unbound() -> ! {
        panic!(
            "twine-reactive: runtime not bound; call `unsafe {{ twine_reactive::bind_to_current_context() }}` \
             once on the UI context (or enable the `std` feature)"
        )
    }

    /// See [`crate::bind_to_current_context`].
    pub(crate) fn bind() {
        BOUND.store(true, Ordering::Release);
    }
}

pub(crate) use imp::with_runtime;

/// Binds the runtime to the calling execution context.
///
/// Without the `std` feature the runtime lives in a `static`, and every function of this
/// crate panics until this has been called. Call it once, early in the UI task (typically in
/// `main` before building the `Ui`). Calling it again from the same context is harmless.
/// With `std` every thread has its own runtime and this does nothing (it exists so that code
/// written for both configurations compiles either way).
///
/// # Safety
///
/// From the first call on, every API of this crate that reaches the runtime — all methods of
/// [`Scope`], [`Signal`](crate::Signal), [`ReadSignal`](crate::ReadSignal),
/// [`WriteSignal`](crate::WriteSignal), [`Memo`](crate::Memo), [`EffectId`](crate::EffectId)
/// and the free functions ([`create_root`], [`batch`](crate::batch),
/// [`untrack`](crate::untrack), [`flush_effects_with`](crate::flush_effects_with),
/// [`drain_channels`](crate::drain_channels), [`provide_ambient`](crate::provide_ambient), …)
/// — must only be called from the execution context that made this call: never from an
/// interrupt handler, another core, or another thread/task that can preempt it.
/// [`Channel`](crate::Channel) and [`UiWaker`](crate::UiWaker) are exempt: they never touch
/// the runtime and may be used from any context.
#[allow(unsafe_code)]
pub unsafe fn bind_to_current_context() {
    #[cfg(not(feature = "std"))]
    imp::bind();
}

/// Creates a new root scope (lazily initializing the runtime).
///
/// Multiple roots may coexist (each `Ui` owns one). A root lives until it is
/// [disposed](Scope::dispose).
///
/// ```
/// let cx = twine_reactive::create_root();
/// let n = cx.signal(1);
/// assert_eq!(n.get(), 1);
/// cx.dispose();
/// ```
#[must_use]
pub fn create_root() -> Scope {
    with_runtime(|rt| match rt.create_scope(None) {
        Some(key) => Scope::from_key(key),
        None => unreachable!("a root scope has no parent that could be dead"),
    })
}

/// Drops every node, scope, channel registration and pending effect of this thread's
/// runtime. **Tests only**: cleanups do not run and all existing handles become dead.
#[doc(hidden)]
pub fn reset() {
    with_runtime(Runtime::reset);
}

/// Live object counts and counters of the current context's runtime (diagnostics, tests and
/// fault analysis).
///
/// ```
/// let cx = twine_reactive::create_root();
/// let _a = cx.signal(1);
/// assert!(twine_reactive::runtime_stats().nodes >= 1);
/// cx.dispose();
/// ```
#[must_use]
pub fn runtime_stats() -> Stats {
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

/// The faults the runtime recorded since the last call (occurrences per kind), clearing them.
///
/// The runtime records [`FaultKind::EffectLoopCut`], [`FaultKind::DepthGuard`] and
/// [`FaultKind::ChannelOverflow`]. `Ui` takes them at every update and forwards them to the
/// engine, so applications using `Ui` read all faults there; this function is for code using the
/// runtime on its own. Lifetime totals are in [`runtime_stats`].
///
/// ```
/// use twine_core::fault::FaultKind;
///
/// let cx = twine_reactive::create_root();
/// twine_reactive::set_flush_iterations_limit(3);
/// let a = cx.signal(0u32);
/// cx.effect(move || a.set(a.get() + 1)); // an effect loop, cut by the runtime
/// let f = twine_reactive::take_faults();
/// assert_eq!(f.get(FaultKind::EffectLoopCut), 1);
/// assert!(twine_reactive::take_faults().kinds().is_empty());
/// cx.dispose();
/// ```
pub fn take_faults() -> FaultCounts {
    with_runtime(|rt| core::mem::take(&mut rt.faults.borrow_mut().new))
}

/// Sets the function called for every fault the runtime records (`None` removes it), with the
/// kind and the number of occurrences. It runs where the fault is detected — possibly in the
/// middle of an effect flush — so keep it short: record, count or signal a supervisor.
///
/// - **Per runtime:** the hook belongs to the current runtime — with the `std` feature one per
///   thread (set it on the UI thread), without it the single bound runtime. A new hook
///   replaces the previous one.
/// - **Re-entry:** the hook is called after the runtime's fault bookkeeping is released, so it
///   may call [`take_faults`], [`runtime_stats`] or `set_fault_hook` itself without
///   panicking. Since it can run in the middle of an effect flush, it should not write signals
///   or create reactive nodes: defer such work (e.g. through a `static` flag the main loop
///   reads).
/// - Allocates nothing; never panics (like every runtime function without `std`, it requires
///   the runtime to be bound: see [`bind_to_current_context`](crate::bind_to_current_context)).
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
/// twine_reactive::set_fault_hook(Some(on_fault));
/// let cx = twine_reactive::create_root();
/// twine_reactive::set_flush_iterations_limit(3);
/// let a = cx.signal(0u32);
/// cx.effect(move || a.set(a.get() + 1));
/// assert_eq!(LOOP_CUTS.load(Ordering::Relaxed), 1);
/// twine_reactive::set_fault_hook(None);
/// cx.dispose();
/// ```
pub fn set_fault_hook(hook: Option<FaultHook>) {
    with_runtime(|rt| rt.faults.borrow_mut().hook = hook);
}

/// Records `n` occurrences of `kind` in the current runtime (crate-internal fault sites that have
/// no `&Runtime` at hand).
pub(crate) fn record_fault(kind: FaultKind, n: u32) {
    with_runtime(|rt| rt.record_fault(kind, n));
}
