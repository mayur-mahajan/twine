//! Context values: [`Scope::provide`], [`Scope::use_context`], [`Scope::expect_context`] and
//! the [`MissingContext`] error.
//!
//! The user-facing documentation (lookup cost, failure convention) is on [`Scope`], section
//! "Context values".

// NOTE(R5.S02): the `safety` profile removes or gates the panicking `expect_*` forms (thin
// `#[track_caller]` wrappers over the `Result` forms, which stay).

use alloc::rc::Rc;
use core::any::{Any, TypeId, type_name};
use core::fmt;

use twine_core::log::warn;

use crate::global::with_runtime;
use crate::scope::Scope;

/// No value of the asked type was provided by the scope or any of its ancestors (or the
/// scope was disposed: a disposed scope provides nothing). Returned by
/// [`Scope::use_context`]; [`type_name`](Self::type_name) names the missing type.
///
/// ```
/// use twine_reactive::{MissingContext, Runtime};
///
/// #[derive(Clone)]
/// struct Settings;
///
/// let cx = Runtime::take().unwrap().create_root();
/// let Err(err) = cx.use_context::<Settings>() else { unreachable!() };
/// let _: MissingContext = err;
/// assert!(err.type_name().ends_with("Settings"));
/// assert_eq!(err.to_string(), format!("context {} not provided", err.type_name()));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MissingContext {
    type_name: &'static str,
}

impl MissingContext {
    /// The error for a missing `T`.
    #[inline]
    #[must_use]
    pub fn of<T: ?Sized + 'static>() -> Self {
        MissingContext {
            type_name: type_name::<T>(),
        }
    }

    /// The name of the missing type, as [`core::any::type_name`] spells it (meant for
    /// diagnostics: the exact text is not stable across compiler versions).
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Panics with this error's message at the caller's location (the shared, out-of-line
    /// panic path of [`Scope::expect_context`] and the `expect_*` hooks).
    ///
    /// # Panics
    ///
    /// Always, with `"context {type_name} not provided"`.
    #[cold]
    #[inline(never)]
    #[track_caller]
    pub fn panic(self) -> ! {
        panic!("{}", self)
    }
}

impl fmt::Display for MissingContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "context {} not provided", self.type_name)
    }
}

impl core::error::Error for MissingContext {}

impl Scope {
    /// Makes `value` available to this scope and its descendants via
    /// [`use_context`](Scope::use_context) — typically an application's store (a `Copy`
    /// struct of signals and memos) provided once by the root scope. A descendant providing the
    /// same type shadows this value for its own subtree; providing the same type again in the
    /// same scope replaces the value (the old one is dropped). The value is dropped when the
    /// scope is disposed.
    ///
    /// Allocates the shared cell holding the value (one `Rc`), and grows the scope's context
    /// list the first time a type is provided.
    ///
    /// Never panics: on a disposed scope this is a no-op with a warning.
    ///
    /// ```
    /// #[derive(Clone, PartialEq, Debug)]
    /// struct Theme(&'static str);
    /// let root = twine_reactive::Runtime::take().unwrap().create_root();
    /// root.provide(Theme("dark"));
    /// let child = root.child();
    /// assert_eq!(child.use_context::<Theme>(), Ok(Theme("dark")));
    /// child.provide(Theme("light")); // shadows the root's value below `child`
    /// assert_eq!(child.child().expect_context::<Theme>(), Theme("light"));
    /// assert_eq!(root.expect_context::<Theme>(), Theme("dark"));
    /// ```
    pub fn provide<T: Clone + 'static>(self, value: T) {
        let value: Rc<dyn Any> = Rc::new(value);
        let old = with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            let Some(data) = inner.scopes.get_mut(self.key()) else {
                return Err(value);
            };
            let id = TypeId::of::<T>();
            if let Some(slot) = data.contexts.iter_mut().find(|(t, _)| *t == id) {
                Ok(Some(core::mem::replace(&mut slot.1, value)))
            } else {
                data.contexts.push((id, value));
                Ok(None)
            }
        });
        match old {
            Ok(old) => drop(old), // dropped outside the borrow (R1)
            Err(value) => {
                warn!(target: "twine::reactive", "provide on disposed scope {:?}; ignored", self.key());
                drop(value);
            }
        }
    }

    /// The nearest value of type `T` provided by this scope or an ancestor (a clone of it), or
    /// [`MissingContext`] naming `T` when there is none (also on a disposed scope). Use
    /// `.ok()` for a context that is optional, [`expect_context`](Scope::expect_context) where
    /// a missing one is a programming error.
    ///
    /// **Cost:** a walk up the scope chain comparing [`TypeId`]s, without allocation (see the
    /// [`Scope`] § Context values); look contexts up once while building, not in
    /// bindings. `T::clone` runs without the runtime borrowed, so it may use the runtime.
    ///
    /// Never panics.
    ///
    /// ```
    /// use twine_reactive::{Runtime, Signal};
    ///
    /// #[derive(Clone, Copy)]
    /// struct Volume(Signal<u8>);
    ///
    /// let root = Runtime::take().unwrap().create_root();
    /// root.provide(Volume(root.signal(3)));
    /// let page = root.child().child();
    /// let Ok(Volume(v)) = page.use_context::<Volume>() else { panic!("provided by the root") };
    /// v.set(7);
    /// assert_eq!(root.expect_context::<Volume>().0.get(), 7);
    /// assert!(page.use_context::<u8>().is_err());
    /// ```
    #[inline]
    pub fn use_context<T: Clone + 'static>(self) -> Result<T, MissingContext> {
        match self.find_context(TypeId::of::<T>()) {
            Some(rc) => {
                // Always a `T`: values are stored under their own type id by `provide`.
                let v = rc.downcast_ref::<T>();
                debug_assert!(v.is_some(), "context stored under the wrong type id");
                // `T::clone` is user code: called without the runtime borrowed (R1).
                v.cloned().ok_or_else(MissingContext::of::<T>)
            }
            None => Err(MissingContext::of::<T>()),
        }
    }

    /// Like [`use_context`](Scope::use_context), but panics if no `T` was provided: a thin
    /// `#[track_caller]` wrapper (the panic reports the caller's location), for application
    /// code where a missing context is a bug.
    ///
    /// ```
    /// let root = twine_reactive::Runtime::take().unwrap().create_root();
    /// root.provide(42u32);
    /// assert_eq!(root.child().expect_context::<u32>(), 42);
    /// ```
    ///
    /// # Panics
    ///
    /// With `"context {type_name} not provided"` if neither this scope nor an ancestor
    /// provides a `T` (see [`MissingContext`]).
    #[inline]
    #[track_caller]
    pub fn expect_context<T: Clone + 'static>(self) -> T {
        match self.use_context::<T>() {
            Ok(v) => v,
            Err(e) => e.panic(),
        }
    }

    /// The value with type id `id` nearest to this scope (shared, not cloned). Not generic, so
    /// one copy serves every context type. Allocation-free.
    fn find_context(self, id: TypeId) -> Option<Rc<dyn Any>> {
        with_runtime(|rt| {
            let inner = rt.inner.borrow();
            let mut cur = Some(self.key());
            while let Some(s) = cur {
                let data = inner.scopes.get(s)?;
                if let Some((_, v)) = data.contexts.iter().find(|(t, _)| *t == id) {
                    return Some(Rc::clone(v));
                }
                cur = data.parent;
            }
            None
        })
    }
}
