//! [`EngineAccess`] (the scoped "current engine") and [`EffectCx`] (the flush context).

use core::marker::PhantomData;

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
