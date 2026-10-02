//! Batching, untracked reads, flushing and deferred effects.

use core::any::Any;

use twine_core::log::warn;

use crate::global::{Runtime, with_runtime};
use crate::runtime::{RuntimeState, ScopeKey};
use crate::scope::Scope;

/// Decrements `batch_depth` on scope exit (also on unwinding).
struct BatchGuard;

impl Drop for BatchGuard {
    fn drop(&mut self) {
        with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            inner.batch_depth = inner.batch_depth.saturating_sub(1);
        });
    }
}

/// See [`Runtime::batch`].
pub(crate) fn batch<R>(f: impl FnOnce() -> R) -> R {
    with_runtime(|rt| rt.inner.borrow_mut().batch_depth += 1);
    let guard = BatchGuard;
    let r = f();
    drop(guard);
    with_runtime(|rt| {
        let flush = {
            let inner = rt.inner.borrow();
            inner.batch_depth == 0 && !inner.running_flush && !inner.pending.is_empty()
        };
        if flush {
            rt.flush(&mut ());
        }
    });
    r
}

/// Restores the previous observer on scope exit (also on unwinding).
struct UntrackGuard(Option<crate::runtime::NodeKey>);

impl Drop for UntrackGuard {
    fn drop(&mut self) {
        let prev = self.0;
        with_runtime(|rt| rt.inner.borrow_mut().observer = prev);
    }
}

/// See [`Runtime::untrack`].
pub(crate) fn untrack<R>(f: impl FnOnce() -> R) -> R {
    let prev = with_runtime(|rt| rt.inner.borrow_mut().observer.take());
    let _guard = UntrackGuard(prev);
    f()
}

impl Runtime {
    /// Runs `f` with effects deferred until it returns; returns `f`'s result.
    ///
    /// Signal writes inside `f` mark their dependents immediately, but effects run once, after
    /// the outermost `batch` returns (unless a flush is already running, which then picks them
    /// up). If `f` panics the batch depth is restored and no effects run. Allocates nothing
    /// itself.
    ///
    /// ```
    /// use std::{cell::Cell, rc::Rc};
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let (a, b) = (cx.signal(1), cx.signal(2));
    /// let runs = Rc::new(Cell::new(0));
    /// let r = runs.clone();
    /// cx.effect(move || {
    ///     let _ = a.get() + b.get();
    ///     r.set(r.get() + 1);
    /// });
    /// let sum = rt.batch(|| {
    ///     a.set(10);
    ///     b.set(20);
    ///     a.get_untracked() + b.get_untracked()
    /// });
    /// assert_eq!(sum, 30);
    /// assert_eq!(runs.get(), 2); // initial run + one run for the batch
    /// ```
    #[inline]
    pub fn batch<R>(self, f: impl FnOnce() -> R) -> R {
        batch(f)
    }

    /// Runs `f` without dependency tracking: reads inside `f` do not subscribe the running memo
    /// or effect. Allocates nothing.
    ///
    /// ```
    /// use std::{cell::Cell, rc::Rc};
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let (a, b) = (cx.signal(1), cx.signal(1));
    /// let runs = Rc::new(Cell::new(0));
    /// let r = runs.clone();
    /// cx.effect(move || {
    ///     a.get();
    ///     rt.untrack(|| b.get());
    ///     r.set(r.get() + 1);
    /// });
    /// b.set(2); // not a dependency
    /// assert_eq!(runs.get(), 1);
    /// a.set(2);
    /// assert_eq!(runs.get(), 2);
    /// ```
    #[inline]
    pub fn untrack<R>(self, f: impl FnOnce() -> R) -> R {
        untrack(f)
    }

    /// Runs all pending effects, passing `ctx` to each (see
    /// [`Scope::effect_with_cx`](crate::Scope::effect_with_cx)).
    ///
    /// Used by the view layer's `Ui` at its fixed point in the update cycle with the engine as
    /// context. A non-`()` context also re-queues effects deferred with
    /// [`defer_current_effect`](Self::defer_current_effect): those of the active root while a
    /// root is active ([`Scope::activate`]), all of them otherwise. While a root is active,
    /// effects of other roots run with the `()` context (see [`Scope::activate`]). Effects queued while flushing
    /// (including newly created ones) run in the same flush. Calling this while a flush is
    /// running is a no-op (the running flush continues). More than
    /// [`set_flush_iterations_limit`](Self::set_flush_iterations_limit) rounds of effects
    /// re-triggering each other are cut with an `error!` log and a
    /// [`FaultKind::EffectLoopCut`](crate::FaultKind::EffectLoopCut) fault.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let a = cx.signal(1);
    /// cx.effect_with_cx(move |ctx| {
    ///     let v = a.get(); // read unconditionally so the dependency is tracked
    ///     if let Some(log) = ctx.downcast_mut::<Vec<i32>>() {
    ///         log.push(v);
    ///     }
    /// });
    /// let mut log: Vec<i32> = Vec::new();
    /// rt.batch(|| {
    ///     a.set(2);
    ///     rt.flush_effects_with(&mut log);
    /// });
    /// assert_eq!(log, [2]);
    /// ```
    pub fn flush_effects_with(self, ctx: &mut dyn Any) {
        with_runtime(|rt| rt.flush(ctx));
    }

    /// Sets how many rounds of effects re-triggering each other a flush runs before cutting the
    /// loop (default 100, minimum 1: `0` is taken as `1`). Never panics.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// rt.set_flush_iterations_limit(50);
    /// ```
    pub fn set_flush_iterations_limit(self, n: u32) {
        with_runtime(|rt| rt.inner.borrow_mut().flush_iterations_limit = n.max(1));
    }

    /// Called from inside a running effect: re-queues it to run again at the next flush with a
    /// real (non-`()`) context.
    ///
    /// View-layer bindings use this when they run without an engine context (e.g. after a
    /// `set` outside `Ui::update`). When the effect's root is not the active one
    /// ([`Scope::activate`]: no update is running, or another root's), the root's waker
    /// ([`Scope::set_ui_waker`]) is woken, so the UI that can run the effect updates. Outside
    /// an effect this logs a warning and does nothing.
    ///
    /// ```
    /// struct Engine;
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let a = cx.signal(1);
    /// cx.effect_with_cx(move |ctx| {
    ///     let v = a.get();
    ///     if ctx.downcast_mut::<Engine>().is_none() {
    ///         rt.defer_current_effect();
    ///     }
    ///     let _ = v;
    /// });
    /// assert!(rt.has_pending_effects());
    /// rt.flush_effects_with(&mut Engine);
    /// assert!(!rt.has_pending_effects());
    /// ```
    pub fn defer_current_effect(self) {
        if !with_runtime(RuntimeState::defer_current_effect) {
            warn!(target: "twine::reactive", "defer_current_effect called outside an effect; ignored");
        }
    }

    /// Called from inside a running effect: disposes it (its closure is released after this
    /// run).
    ///
    /// View-layer bindings use this when the widget they write to was deleted. Outside an
    /// effect this logs a warning and does nothing.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let cx = rt.create_root();
    /// let a = cx.signal(1);
    /// let e = cx.effect(move || {
    ///     if a.get() > 1 {
    ///         rt.dispose_current_effect();
    ///     }
    /// });
    /// a.set(2);
    /// assert!(!e.is_alive());
    /// ```
    pub fn dispose_current_effect(self) {
        if !with_runtime(RuntimeState::dispose_current_effect) {
            warn!(target: "twine::reactive", "dispose_current_effect called outside an effect; ignored");
        }
    }

    /// Whether effects of any root are waiting to run (pending, or deferred until a real
    /// context). A `Ui` asks [`Scope::has_pending_effects`] for its own root instead.
    /// Allocates nothing; never panics.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// assert!(!rt.has_pending_effects());
    /// ```
    #[must_use]
    pub fn has_pending_effects(self) -> bool {
        with_runtime(|rt| {
            let inner = rt.inner.borrow();
            !inner.pending.is_empty() || !inner.deferred.is_empty()
        })
    }
}

/// Restores the previously active root when [`Scope::activate`] returns (also on unwinding).
struct ActiveGuard(Option<ScopeKey>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let prev = self.0;
        with_runtime(|rt| rt.inner.borrow_mut().active_root = prev);
    }
}

impl Scope {
    /// Runs `f` with this scope's **root** as the runtime's *active root*: the update of one
    /// UI among several sharing the runtime (one `Ui` per root). While it is active:
    ///
    /// - effects of the active root run as usual (with the flush context and the
    ///   [ambient](Runtime::provide_ambient) value — the UI's engine);
    /// - effects of **other** roots that a signal write makes run meanwhile see neither: they
    ///   run with the `()` context and an empty ambient slot, so a view-layer binding of
    ///   another UI can never write into this UI's engine (it defers itself, and its root's
    ///   waker is woken to run it in its own update); a plain effect runs normally;
    /// - a flush with a context re-queues only the active root's deferred effects.
    ///
    /// Nested calls shadow the outer root until they return; the previous one is restored
    /// also when `f` panics. Without any active root (outside every UI update) nothing is
    /// filtered. Cost: two writes of the runtime state; per effect run one comparison.
    /// Allocates nothing. A disposed scope activates no root (nothing is filtered).
    ///
    /// # Panics
    ///
    /// Never by itself. A panic of `f` propagates to the caller unchanged, after the
    /// previously active root has been restored (the restore runs during unwinding). With the
    /// `debug-checks` feature, like every runtime access, it panics when called from interrupt
    /// context according to the registered [`set_interrupt_probe`](crate::set_interrupt_probe).
    ///
    /// ```
    /// use std::{cell::Cell, rc::Rc};
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let (ui_a, ui_b) = (rt.create_root(), rt.create_root());
    /// let shared = ui_a.signal(0);
    /// let ctx_seen_by_b = Rc::new(Cell::new(None));
    /// let seen = ctx_seen_by_b.clone();
    /// ui_b.effect_with_cx(move |ctx| {
    ///     let v = shared.get();
    ///     seen.set(Some((v, ctx.is::<u32>())));
    /// });
    /// let mut engine_a = 7u32; // UI A's context
    /// ui_a.activate(|| {
    ///     rt.batch(|| {
    ///         shared.set(1);
    ///         rt.flush_effects_with(&mut engine_a);
    ///     })
    /// });
    /// // B's effect ran, but not with A's context.
    /// assert_eq!(ctx_seen_by_b.get(), Some((1, false)));
    /// ```
    pub fn activate<R>(self, f: impl FnOnce() -> R) -> R {
        let prev = with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            let root = inner.root_of(self.key());
            core::mem::replace(&mut inner.active_root, root)
        });
        let _restore = ActiveGuard(prev);
        f()
    }

    /// Whether effects of this scope's **root** are waiting to run: pending, or deferred until
    /// the root's next flush with a context ([`Runtime::defer_current_effect`]). What a `Ui`
    /// asks for its own root (another UI's work does not keep it busy). `false` for a disposed
    /// scope. Allocates nothing; scans the effect queues only when they are not empty; never
    /// panics.
    ///
    /// ```
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let (ui_a, ui_b) = (rt.create_root(), rt.create_root());
    /// let a = ui_a.signal(1);
    /// ui_a.effect_with_cx(move |ctx| {
    ///     let _ = a.get();
    ///     if !ctx.is::<u32>() {
    ///         rt.defer_current_effect(); // waits for A's context
    ///     }
    /// });
    /// assert!(ui_a.has_pending_effects());
    /// assert!(!ui_b.has_pending_effects());
    /// ```
    #[must_use]
    pub fn has_pending_effects(self) -> bool {
        with_runtime(|rt| {
            let inner = rt.inner.borrow();
            if inner.pending.is_empty() && inner.deferred.is_empty() {
                return false;
            }
            inner
                .root_of(self.key())
                .is_some_and(|root| inner.has_pending_effects_of(root))
        })
    }
}
