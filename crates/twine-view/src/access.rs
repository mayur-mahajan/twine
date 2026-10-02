//! [`EngineAccess`] (the scoped "current engine") and [`EffectCx`] (the flush context).

#[cfg(debug_assertions)]
use core::cell::Cell;
#[cfg(debug_assertions)]
use core::panic::Location;

use twine_engine::Engine;
use twine_reactive::Runtime;

/// The scoped "current engine": lets code without an engine parameter — [`NodeRef`](crate::NodeRef)
/// `with_mut`, bindings, hooks — reach the engine while the [`Ui`](crate::Ui) runs it.
///
/// `Ui` lends the engine ([`provide`](Self::provide)) around everything it runs on behalf of
/// the application: view building, event handlers of the view layer, timer and animation
/// callbacks and the effect flush. [`with`](Self::with) borrows it **exclusively**: while one
/// borrow is active, nested calls get `None` (so there is never more than one `&mut Engine`).
/// Outside those scopes `with` returns `None`.
///
/// The slot is the reactive runtime's ambient slot ([`Runtime::provide_ambient`]), so it
/// shares the runtime's confinement to the UI thread / task: every call takes the
/// [`Runtime`] token — or anything that converts into one, such as the [`Scope`] a handler
/// or effect already captured — as proof that it runs there (free: the token is
/// zero-sized).
///
/// [`Scope`]: twine_reactive::Scope
///
/// # Diagnostics
///
/// The view layer's calls that must return what the engine holds —
/// [`NodeRef::with_mut`](crate::NodeRef::with_mut) and
/// [`AnimController::is_playing`](crate::AnimController::is_playing) — return `None` / `false`
/// when called without it. In debug builds (`debug_assertions`) they also log one `warn!` (target
/// `twine::view`) **per call site** naming the caller's file and line, so a misplaced call is
/// visible without flooding the log from a loop. The "already warned" set is a fixed table of
/// 32 call-site addresses in a `static` guarded by a critical section (no heap, no atomics);
/// once it is full, further new sites warn on every call instead of being lost. In release builds the
/// check, the table and the `#[track_caller]` location argument are compiled out entirely: the
/// calls behave identically and silently. Calls that defer instead (bindings, timers,
/// animations and modals created outside `Ui::update`) or are queued on the `Ui` (scope
/// cleanups, the other [`AnimController`](crate::AnimController) methods,
/// [`ThemeHandle::set`](crate::ThemeHandle::set), [`ThemeHandle::set_mode`](crate::ThemeHandle::set_mode),
/// [`MotionHandle::set`](crate::MotionHandle::set): see [`UiCore`](crate::UiCore) § Engine
/// commands) are correct and never warn.
///
/// ```
/// use twine_engine::{Engine, EngineConfig};
/// use twine_reactive::Runtime;
/// use twine_view::EngineAccess;
///
/// let rt = Runtime::take().unwrap();
/// let mut engine = Engine::new(EngineConfig::default()).unwrap();
/// assert!(EngineAccess::with(rt, |_| ()).is_none());
/// let nodes = EngineAccess::provide(rt, &mut engine, || EngineAccess::with(rt, |e| e.tree().len()));
/// assert_eq!(nodes, Some(0));
/// ```
#[derive(Debug)]
pub struct EngineAccess(());

impl EngineAccess {
    /// Makes `engine` available to [`with`](Self::with) while `f` runs (also when `f`
    /// panics, the previous state is restored). `rt` is the runtime of the calling context
    /// (a [`Runtime`] or a [`Scope`](twine_reactive::Scope)). Allocates nothing.
    ///
    /// # Panics
    ///
    /// Never by itself; a panic of `f` propagates after the previous state is restored.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_reactive::Runtime;
    /// use twine_view::EngineAccess;
    ///
    /// let rt = Runtime::take().unwrap();
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// // What a host loop that drives its own engine does around application code:
    /// let available = EngineAccess::provide(rt, &mut engine, || EngineAccess::available(rt));
    /// assert!(available);
    /// assert!(!EngineAccess::available(rt)); // lent only while `f` runs
    /// ```
    pub fn provide<R>(rt: impl Into<Runtime>, engine: &mut Engine, f: impl FnOnce() -> R) -> R {
        rt.into().provide_ambient(engine, f)
    }

    /// Calls `f` with the current engine, or returns `None` (without calling `f`) when there is
    /// none: outside `Ui`, or while an enclosing `with` holds it. `rt` is the runtime of the
    /// calling context (a [`Runtime`] or a [`Scope`](twine_reactive::Scope)). Allocates
    /// nothing; never panics (unless `f` does).
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_reactive::Runtime;
    /// use twine_view::EngineAccess;
    ///
    /// let rt = Runtime::take().unwrap();
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// EngineAccess::provide(rt, &mut engine, || {
    ///     let outer = EngineAccess::with(rt, |_e| {
    ///         // The borrow is exclusive: a nested `with` gets nothing.
    ///         EngineAccess::with(rt, |_| ()).is_none()
    ///     });
    ///     assert_eq!(outer, Some(true));
    /// });
    /// ```
    pub fn with<R>(rt: impl Into<Runtime>, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        rt.into().with_ambient(|v| as_engine(v).map(f))
    }

    /// Whether an engine is available right now (one read of the ambient slot). `rt` is the
    /// runtime of the calling context. Never panics.
    ///
    /// ```
    /// use twine_reactive::Runtime;
    /// use twine_view::EngineAccess;
    ///
    /// let rt = Runtime::take().unwrap();
    /// assert!(!EngineAccess::available(rt)); // outside every `Ui` update
    /// ```
    #[must_use]
    pub fn available(rt: impl Into<Runtime>) -> bool {
        rt.into().ambient_is::<Engine>()
    }
}

/// Whether the engine is available to the running effect; defers the effect (not evaluated:
/// it runs, and re-subscribes, at the next flush) when it is not. The guard of every effect
/// that needs the engine (bindings, model sync, modals): one shared out-of-line copy, a single
/// read of the ambient slot ([`Runtime::ambient_is`]).
#[inline(never)]
pub(crate) fn engine_ready(rt: Runtime) -> bool {
    let ready = EngineAccess::available(rt);
    if !ready {
        rt.defer_current_effect();
    }
    ready
}

/// The engine in the ambient slot, if that is what it holds. Out of line: [`EngineAccess::with`]
/// is instantiated once per closure (event handlers, `NodeRef::with_mut`, model sync, …), the
/// downcast is shared. Binding runs do not go through it (see `bind::binding_target`).
#[inline(never)]
fn as_engine(v: Option<&mut dyn core::any::Any>) -> Option<&mut Engine> {
    v.and_then(|a| a.downcast_mut::<Engine>())
}

/// The context `Ui` passes to [`Runtime::flush_effects_with`]: a marker telling the
/// runtime that this is a real flush (effects deferred earlier for lack of an engine are
/// re-queued). The engine itself is reached through [`EngineAccess`], which `Ui` provides for
/// the whole flush.
///
/// It is `'static` (the flush context is a `&mut dyn Any`) and can only be created by
/// [`scoped`](Self::scoped), which lends the engine for exactly the duration of the closure.
#[derive(Debug)]
pub struct EffectCx {
    rt: Runtime,
}

impl EffectCx {
    /// Runs `f` with a flush context while `engine` is provided through [`EngineAccess`].
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_reactive::Runtime;
    /// use twine_view::EffectCx;
    ///
    /// let rt = Runtime::take().unwrap();
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let n = EffectCx::scoped(rt, &mut engine, |ecx| ecx.with_engine(|e| e.tree().len()));
    /// assert_eq!(n, Some(0));
    /// ```
    pub fn scoped<R>(rt: Runtime, engine: &mut Engine, f: impl FnOnce(&mut EffectCx) -> R) -> R {
        EngineAccess::provide(rt, engine, || f(&mut EffectCx { rt }))
    }

    /// Calls `f` with the engine (see [`EngineAccess::with`]).
    pub fn with_engine<R>(&mut self, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        EngineAccess::with(self.rt, f)
    }
}

/// Number of call sites [`no_engine`] remembers (debug builds only).
#[cfg(debug_assertions)]
const WARNED_SITES: usize = 32;

/// The addresses of the call sites that already warned (`0`: free slot). Guarded by a
/// critical section (the `critical-section` implementation the application already supplies
/// for the reactive runtime): several `Ui`s on several threads (hosted tests) may warn at the
/// same time, and load/store atomics alone could lose an entry.
#[cfg(debug_assertions)]
static WARNED: critical_section::Mutex<Cell<[usize; WARNED_SITES]>> =
    critical_section::Mutex::new(Cell::new([0; WARNED_SITES]));

/// Records `loc` and returns `true` the first time it is seen (and always once the table is
/// full). A call site duplicated into several code-generation units may have several
/// `Location`s and warn once per copy.
#[cfg(debug_assertions)]
fn first_time(loc: &'static Location<'static>) -> bool {
    let key = core::ptr::from_ref(loc) as usize;
    critical_section::with(|cs| {
        let cell = WARNED.borrow(cs);
        let mut sites = cell.get();
        if sites.contains(&key) {
            return false;
        }
        if let Some(free) = sites.iter_mut().find(|k| **k == 0) {
            *free = key;
            cell.set(sites);
        }
        true
    })
}

/// Reports that `what` (e.g. `"NodeRef::with_mut"`) ran without the engine and was ignored:
/// one `warn!` per call site of the `#[track_caller]` chain that leads here (see
/// [`EngineAccess`] § Diagnostics). Compiled out in release builds.
#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
#[track_caller]
pub(crate) fn no_engine(what: &'static str) {
    let loc = Location::caller();
    if first_time(loc) {
        twine_core::warn!(
            target: "twine::view",
            "{} called at {}:{} without the engine (outside a Ui handler, effect, timer or \
             build, or while the engine is borrowed); ignored. Write a signal or post a \
             message instead. Warned once per call site.",
            what,
            loc.file(),
            loc.line()
        );
    }
}

/// Release builds: no diagnostics (see [`EngineAccess`] § Diagnostics).
#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn no_engine(_what: &'static str) {}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn first_time_once_per_location() {
        let a = Location::caller();
        let b = Location::caller();
        assert!(first_time(a));
        assert!(!first_time(a));
        assert!(first_time(b));
        assert!(!first_time(b));
    }
}
