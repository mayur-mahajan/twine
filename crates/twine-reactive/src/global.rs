//! Access to the runtime of the current UI context (design 03 §2).
//!
//! - With the `std` feature the runtime is a `thread_local!`: every thread has its own,
//!   independent runtime, so the implementation is sound without further contracts.
//! - Without `std` the runtime is a `static` guarded by a one-time, `unsafe` binding
//!   ([`bind_to_current_context`]) that states the single-context contract.
//!
//! [`with_runtime`] is the only accessor. It may be re-entered (it hands out a shared
//! `&Runtime`); rule R1 governs borrowing the runtime's `inner` cell, not this function.

use crate::runtime::Runtime;
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

    // SAFETY: Soundness argument (design 03 §2). The runtime is only ever touched through
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

/// Binds the `no_std` runtime to the calling execution context (only without the `std`
/// feature).
///
/// Without `std`, the runtime lives in a `static`; every function of this crate panics until
/// this has been called. Call it once, early in the UI task (typically in `main` before
/// building the `Ui`). Calling it again from the same context is harmless.
///
/// # Safety
///
/// From the first call on, every API of this crate that reaches the runtime — all methods of
/// [`Scope`], [`Signal`](crate::Signal), [`ReadSignal`](crate::ReadSignal),
/// [`WriteSignal`](crate::WriteSignal), [`Memo`](crate::Memo), [`EffectId`](crate::EffectId)
/// and the free functions ([`create_root`], [`batch`](crate::batch),
/// [`untrack`](crate::untrack), [`flush_effects_with`](crate::flush_effects_with),
/// [`drain_channels`](crate::drain_channels), …) — must only be called from the execution
/// context that made this call: never from an interrupt handler, another core, or another
/// thread/task that can preempt it. [`Channel`](crate::Channel) and
/// [`UiWaker`](crate::UiWaker) are exempt: they never touch the runtime and may be used from
/// any context.
#[cfg(not(feature = "std"))]
#[allow(unsafe_code)]
pub unsafe fn bind_to_current_context() {
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

/// Live object counts and counters of this thread's runtime (tests and diagnostics).
#[doc(hidden)]
#[must_use]
pub fn debug_stats() -> Stats {
    with_runtime(|rt| {
        let inner = rt.inner.borrow();
        let runtime::Counters {
            writes,
            effect_runs,
            memo_runs,
            depth_guard_hits,
            loop_cuts,
        } = inner.counters;
        Stats {
            nodes: inner.nodes.len(),
            scopes: inner.scopes.len(),
            pending: inner.pending.len(),
            deferred: inner.deferred.len(),
            channels: inner.channels.len(),
            writes,
            effect_runs,
            memo_runs,
            depth_guard_hits,
            loop_cuts,
        }
    })
}
