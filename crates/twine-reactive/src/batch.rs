//! Batching, untracked reads, flushing and deferred effects.

use core::any::Any;

use twine_core::log::warn;

use crate::global::with_runtime;
use crate::runtime::Runtime;

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

/// Runs `f` with effects deferred until it returns; returns `f`'s result.
///
/// Signal writes inside `f` mark their dependents immediately, but effects run once, after
/// the outermost `batch` returns (unless a flush is already running, which then picks them
/// up). If `f` panics the batch depth is restored and no effects run.
///
/// ```
/// use std::{cell::Cell, rc::Rc};
/// let cx = twine_reactive::create_root();
/// let (a, b) = (cx.signal(1), cx.signal(2));
/// let runs = Rc::new(Cell::new(0));
/// let r = runs.clone();
/// cx.effect(move || {
///     let _ = a.get() + b.get();
///     r.set(r.get() + 1);
/// });
/// let sum = twine_reactive::batch(|| {
///     a.set(10);
///     b.set(20);
///     a.get_untracked() + b.get_untracked()
/// });
/// assert_eq!(sum, 30);
/// assert_eq!(runs.get(), 2); // initial run + one run for the batch
/// ```
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
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

/// Runs `f` without dependency tracking: reads inside `f` do not subscribe the running memo
/// or effect.
///
/// ```
/// use std::{cell::Cell, rc::Rc};
/// let cx = twine_reactive::create_root();
/// let (a, b) = (cx.signal(1), cx.signal(1));
/// let runs = Rc::new(Cell::new(0));
/// let r = runs.clone();
/// cx.effect(move || {
///     a.get();
///     twine_reactive::untrack(|| b.get());
///     r.set(r.get() + 1);
/// });
/// b.set(2); // not a dependency
/// assert_eq!(runs.get(), 1);
/// a.set(2);
/// assert_eq!(runs.get(), 2);
/// ```
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    let prev = with_runtime(|rt| rt.inner.borrow_mut().observer.take());
    let _guard = UntrackGuard(prev);
    f()
}

/// Runs all pending effects, passing `ctx` to each (see
/// [`Scope::effect_with_cx`](crate::Scope::effect_with_cx)).
///
/// Used by the view layer's `Ui` at its fixed point in the update cycle with the engine as
/// context. A non-`()` context also re-queues effects deferred with
/// [`defer_current_effect`]. Effects queued while flushing (including newly created ones) run
/// in the same flush. Calling this while a flush is running is a no-op (the running flush
/// continues). More than [`set_flush_iterations_limit`] rounds of effects re-triggering each
/// other are cut with an `error!` log.
///
/// ```
/// let cx = twine_reactive::create_root();
/// let a = cx.signal(1);
/// cx.effect_with_cx(move |ctx| {
///     let v = a.get(); // read unconditionally so the dependency is tracked
///     if let Some(log) = ctx.downcast_mut::<Vec<i32>>() {
///         log.push(v);
///     }
/// });
/// let mut log: Vec<i32> = Vec::new();
/// twine_reactive::batch(|| {
///     a.set(2);
///     twine_reactive::flush_effects_with(&mut log);
/// });
/// assert_eq!(log, [2]);
/// ```
pub fn flush_effects_with(ctx: &mut dyn Any) {
    with_runtime(|rt| rt.flush(ctx));
}

/// Sets how many rounds of effects re-triggering each other a flush runs before cutting the
/// loop (default 100, minimum 1).
///
/// ```
/// twine_reactive::set_flush_iterations_limit(50);
/// ```
pub fn set_flush_iterations_limit(n: u32) {
    with_runtime(|rt| rt.inner.borrow_mut().flush_iterations_limit = n.max(1));
}

/// Called from inside a running effect: re-queues it to run again at the next flush with a
/// real (non-`()`) context.
///
/// View-layer bindings use this when they run without an engine context (e.g. after a `set`
/// outside `Ui::update`). Outside an effect this logs a warning and does nothing.
///
/// ```
/// struct Engine;
/// let cx = twine_reactive::create_root();
/// let a = cx.signal(1);
/// cx.effect_with_cx(move |ctx| {
///     let v = a.get();
///     if ctx.downcast_mut::<Engine>().is_none() {
///         twine_reactive::defer_current_effect();
///     }
///     let _ = v;
/// });
/// assert!(twine_reactive::has_pending_effects());
/// twine_reactive::flush_effects_with(&mut Engine);
/// assert!(!twine_reactive::has_pending_effects());
/// ```
pub fn defer_current_effect() {
    if !with_runtime(Runtime::defer_current_effect) {
        warn!(target: "twine::reactive", "defer_current_effect called outside an effect; ignored");
    }
}

/// Called from inside a running effect: disposes it (its closure is released after this run).
///
/// View-layer bindings use this when the widget they write to was deleted. Outside an effect
/// this logs a warning and does nothing.
///
/// ```
/// let cx = twine_reactive::create_root();
/// let a = cx.signal(1);
/// let e = cx.effect(move || {
///     if a.get() > 1 {
///         twine_reactive::dispose_current_effect();
///     }
/// });
/// a.set(2);
/// assert!(!e.is_alive());
/// ```
pub fn dispose_current_effect() {
    if !with_runtime(Runtime::dispose_current_effect) {
        warn!(target: "twine::reactive", "dispose_current_effect called outside an effect; ignored");
    }
}

/// Whether effects are waiting to run (pending, or deferred until a real context).
///
/// ```
/// assert!(!twine_reactive::has_pending_effects());
/// ```
pub fn has_pending_effects() -> bool {
    with_runtime(|rt| {
        let inner = rt.inner.borrow();
        !inner.pending.is_empty() || !inner.deferred.is_empty()
    })
}
