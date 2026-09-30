//! [`EngineAccess`] (the scoped "current engine") and [`EffectCx`] (the flush context).

#[cfg(debug_assertions)]
use core::cell::Cell;
use core::marker::PhantomData;
#[cfg(debug_assertions)]
use core::panic::Location;

use twine_engine::Engine;
use twine_reactive::{provide_ambient, with_ambient};

/// The scoped "current engine": lets code without an engine parameter — [`NodeRef`](crate::NodeRef)
/// `with_mut`, bindings, hooks — reach the engine while the [`Ui`](crate::Ui) runs it.
///
/// `Ui` lends the engine ([`provide`](Self::provide)) around everything it runs on behalf of
/// the application: view building, event handlers of the view layer, timer and animation
/// callbacks and the effect flush. [`with`](Self::with) borrows it **exclusively**: while one
/// borrow is active, nested calls get `None` (so there is never more than one `&mut Engine`).
/// Outside those scopes `with` returns `None`.
///
/// The slot is the reactive runtime's ambient slot ([`twine_reactive::provide_ambient`]), so
/// it shares the runtime's confinement to the UI thread / task.
///
/// # Diagnostics
///
/// The view layer's imperative calls that need the engine and cannot wait for it —
/// [`NodeRef::with_mut`](crate::NodeRef::with_mut), the [`AnimController`](crate::AnimController)
/// methods, [`ThemeHandle::set`](crate::ThemeHandle::set) — return `None` / do nothing when
/// called without it. In debug builds (`debug_assertions`) they also log one `warn!` (target
/// `twine::view`) **per call site** naming the caller's file and line, so a misplaced call is
/// visible without flooding the log from a loop. The "already warned" set is a fixed table of
/// 32 call-site addresses in a `static` guarded by a critical section (no heap, no atomics);
/// once it is full, further new sites warn on every call instead of being lost. In release builds the
/// check, the table and the `#[track_caller]` location argument are compiled out entirely: the
/// calls behave identically and silently. Calls that defer instead (bindings, timers,
/// animations and modals created or changed outside `Ui::update`) are correct and never warn.
///
/// ```
/// use twine_engine::{Engine, EngineConfig};
/// use twine_view::EngineAccess;
///
/// let mut engine = Engine::new(EngineConfig::default()).unwrap();
/// assert!(EngineAccess::with(|_| ()).is_none());
/// let nodes = EngineAccess::provide(&mut engine, || EngineAccess::with(|e| e.tree().len()));
/// assert_eq!(nodes, Some(0));
/// ```
#[derive(Debug)]
pub struct EngineAccess(());

impl EngineAccess {
    /// Makes `engine` available to [`with`](Self::with) while `f` runs.
    pub fn provide<R>(engine: &mut Engine, f: impl FnOnce() -> R) -> R {
        provide_ambient(engine, f)
    }

    /// Calls `f` with the current engine, or returns `None` (without calling `f`) when there is
    /// none: outside `Ui`, or while an enclosing `with` holds it.
    pub fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        with_ambient(|v| v.and_then(|a| a.downcast_mut::<Engine>()).map(f))
    }

    /// Whether an engine is available right now.
    #[must_use]
    pub fn available() -> bool {
        Self::with(|_| ()).is_some()
    }
}

/// The context `Ui` passes to [`twine_reactive::flush_effects_with`]: a marker telling the
/// runtime that this is a real flush (effects deferred earlier for lack of an engine are
/// re-queued). The engine itself is reached through [`EngineAccess`], which `Ui` provides for
/// the whole flush.
///
/// It is `'static` (the flush context is a `&mut dyn Any`) and can only be created by
/// [`scoped`](Self::scoped), which lends the engine for exactly the duration of the closure.
#[derive(Debug)]
pub struct EffectCx {
    _not_send: PhantomData<*const ()>,
}

impl EffectCx {
    /// Runs `f` with a flush context while `engine` is provided through [`EngineAccess`].
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_view::EffectCx;
    ///
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let n = EffectCx::scoped(&mut engine, |ecx| ecx.with_engine(|e| e.tree().len()));
    /// assert_eq!(n, Some(0));
    /// ```
    pub fn scoped<R>(engine: &mut Engine, f: impl FnOnce(&mut EffectCx) -> R) -> R {
        EngineAccess::provide(engine, || {
            f(&mut EffectCx {
                _not_send: PhantomData,
            })
        })
    }

    /// Calls `f` with the engine (see [`EngineAccess::with`]).
    pub fn with_engine<R>(&mut self, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        EngineAccess::with(f)
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
