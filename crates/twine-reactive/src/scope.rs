//! [`Scope`]: ownership of reactive nodes, nesting, disposal, cleanups and context.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::{Any, TypeId};
use core::cell::RefCell;
use core::fmt;
use core::marker::PhantomData;
use core::panic::Location;

use twine_core::log::warn;

use crate::batch::batch;
use crate::effect::EffectId;
use crate::global::with_runtime;
use crate::handles::{NodeHandle, type_mismatch};
use crate::memo::Memo;
use crate::runtime::{Computation, Kind, ScopeKey, ValueRc};
use crate::signal::Signal;

/// An owner of signals, memos, effects, child scopes, cleanups and context values.
///
/// `Scope` is a `Copy` handle (`!Send`, `!Sync`). Everything created through a scope is
/// released when the scope is [disposed](Scope::dispose); child scopes are disposed with
/// their parent.
///
/// ```
/// let root = twine_reactive::create_root();
/// let child = root.child();
/// let n = child.signal(5);
/// root.dispose(); // disposes `child` too
/// assert!(!child.is_alive());
/// assert!(!n.is_alive());
/// ```
///
/// Scopes cannot be sent to another thread:
///
/// ```compile_fail
/// let cx = twine_reactive::create_root();
/// std::thread::spawn(move || cx.is_alive());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    key: ScopeKey,
    _m: PhantomData<*const ()>,
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Scope({:?})", self.key)
    }
}

/// Panics for a scope used after disposal.
#[cold]
#[inline(never)]
#[track_caller]
fn scope_disposed() -> ! {
    panic!("scope used after it was disposed")
}

impl Scope {
    pub(crate) fn from_key(key: ScopeKey) -> Self {
        Scope { key, _m: PhantomData }
    }

    pub(crate) fn key(self) -> ScopeKey {
        self.key
    }

    /// Inserts a node owned by this scope; panics if the scope is dead.
    #[track_caller]
    pub(crate) fn create_node<T>(
        self,
        value: Option<ValueRc>,
        comp: Option<Box<Computation>>,
    ) -> NodeHandle<T> {
        let loc = Location::caller();
        match with_runtime(|rt| rt.create_node(self.key, value, comp)) {
            Some(key) => NodeHandle::new(key, loc),
            None => scope_disposed(),
        }
    }

    /// Creates a signal holding `value`, owned by this scope.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let name = cx.signal(String::from("twine"));
    /// assert_eq!(name.with(|s| s.len()), 5);
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    pub fn signal<T: 'static>(self, value: T) -> Signal<T> {
        let rc: ValueRc = Rc::new(RefCell::new(value));
        Signal::from_handle(self.create_node(Some(rc), None))
    }

    /// Creates a lazily computed, cached derived value (see [`Memo`]).
    ///
    /// The closure runs on the first read and again only when a dependency changed and the
    /// memo is read (or a dependent effect runs). If the new value equals the old one,
    /// dependents are not re-run.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let a = cx.signal(2);
    /// let double = cx.memo(move || a.get() * 2);
    /// assert_eq!(double.get(), 4);
    /// a.set(5);
    /// assert_eq!(double.get(), 10);
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    pub fn memo<T: PartialEq + 'static>(self, f: impl Fn() -> T + 'static) -> Memo<T> {
        let rc: ValueRc = Rc::new(RefCell::new(None::<T>));
        let compute = move |cell: &RefCell<dyn Any>| -> bool {
            let new = f();
            let mut b = cell.borrow_mut();
            let Some(slot) = b.downcast_mut::<Option<T>>() else {
                type_mismatch()
            };
            if slot.as_ref() == Some(&new) {
                return false;
            }
            let old = slot.replace(new);
            drop(b);
            drop(old); // user `Drop` runs without the cell borrowed
            true
        };
        let comp = Box::new(Computation {
            kind: Kind::Memo(Rc::new(compute)),
            sources: twine_core::SmallVec::new(),
        });
        Memo::from_handle(self.create_node(Some(rc), Some(comp)))
    }

    /// Creates an effect: `f` runs now (collecting its dependencies) and again after any of
    /// them changes.
    ///
    /// Created outside a flush, the effect runs immediately (with a `()` context). Created
    /// while effects are being flushed (e.g. by another effect), it runs later in the same
    /// flush with that flush's context.
    ///
    /// ```
    /// use std::{cell::Cell, rc::Rc};
    /// let cx = twine_reactive::create_root();
    /// let a = cx.signal(1);
    /// let seen = Rc::new(Cell::new(0));
    /// let s = seen.clone();
    /// cx.effect(move || s.set(a.get()));
    /// assert_eq!(seen.get(), 1);
    /// a.set(7);
    /// assert_eq!(seen.get(), 7);
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    pub fn effect(self, mut f: impl FnMut() + 'static) -> EffectId {
        self.effect_with_cx(move |_| f())
    }

    /// Like [`effect`](Scope::effect), but `f` receives the flush context passed to
    /// [`flush_effects_with`](crate::flush_effects_with) (`&mut ()` for automatic flushes).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let a = cx.signal(1);
    /// cx.effect_with_cx(move |ctx| {
    ///     let v = a.get();
    ///     if let Some(total) = ctx.downcast_mut::<u32>() {
    ///         *total += v;
    ///     }
    /// });
    /// let mut total = 0u32;
    /// twine_reactive::batch(|| {
    ///     a.set(5);
    ///     twine_reactive::flush_effects_with(&mut total);
    /// });
    /// assert_eq!(total, 5);
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    pub fn effect_with_cx(self, f: impl FnMut(&mut dyn Any) + 'static) -> EffectId {
        let comp = Box::new(Computation {
            kind: Kind::Effect(Rc::new(RefCell::new(f))),
            sources: twine_core::SmallVec::new(),
        });
        let h = self.create_node::<()>(None, Some(comp));
        with_runtime(|rt| rt.run_new_effect(h.key));
        EffectId::from_handle(h)
    }

    /// Creates a child scope, disposed together with this one.
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    #[must_use]
    pub fn child(self) -> Scope {
        match with_runtime(|rt| rt.create_scope(Some(self.key))) {
            Some(k) => Scope::from_key(k),
            None => scope_disposed(),
        }
    }

    /// Disposes the scope: child scopes (depth-first, most recent first), then cleanups (most
    /// recent first), then its nodes, then its context values. Signal writes made by cleanups
    /// are batched. Disposing a dead scope is a no-op.
    ///
    /// ```
    /// use std::{cell::RefCell, rc::Rc};
    /// let cx = twine_reactive::create_root();
    /// let log = Rc::new(RefCell::new(Vec::new()));
    /// let l = log.clone();
    /// cx.on_cleanup(move || l.borrow_mut().push("cleanup"));
    /// cx.dispose();
    /// cx.dispose(); // no-op
    /// assert_eq!(*log.borrow(), ["cleanup"]);
    /// ```
    pub fn dispose(self) {
        batch(|| with_runtime(|rt| rt.dispose_scope(self.key)));
    }

    /// Registers `f` to run when the scope is disposed. On a dead scope `f` is dropped with a
    /// warning.
    pub fn on_cleanup(self, f: impl FnOnce() + 'static) {
        if let Err(f) = with_runtime(|rt| rt.add_cleanup(self.key, Box::new(f))) {
            warn!(target: "twine::reactive", "on_cleanup on disposed scope {:?}; ignored", self.key);
            drop(f);
        }
    }

    /// Makes `value` available to this scope and its descendants via
    /// [`use_context`](Scope::use_context). Providing the same type again in the same scope
    /// replaces the value. On a dead scope this is a no-op with a warning.
    ///
    /// ```
    /// #[derive(Clone, PartialEq, Debug)]
    /// struct Theme(&'static str);
    /// let root = twine_reactive::create_root();
    /// root.provide(Theme("dark"));
    /// let child = root.child();
    /// assert_eq!(child.use_context::<Theme>(), Some(Theme("dark")));
    /// assert_eq!(child.use_context::<u8>(), None);
    /// ```
    pub fn provide<T: Clone + 'static>(self, value: T) {
        let value: Rc<dyn Any> = Rc::new(value);
        let old = with_runtime(|rt| {
            let mut inner = rt.inner.borrow_mut();
            let Some(data) = inner.scopes.get_mut(self.key) else {
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
                warn!(target: "twine::reactive", "provide on disposed scope {:?}; ignored", self.key);
                drop(value);
            }
        }
    }

    /// The nearest value of type `T` provided by this scope or an ancestor.
    pub fn use_context<T: Clone + 'static>(self) -> Option<T> {
        let id = TypeId::of::<T>();
        let found: Option<Rc<dyn Any>> = with_runtime(|rt| {
            let inner = rt.inner.borrow();
            let mut cur = Some(self.key);
            while let Some(s) = cur {
                let data = inner.scopes.get(s)?;
                if let Some((_, v)) = data.contexts.iter().find(|(t, _)| *t == id) {
                    return Some(v.clone());
                }
                cur = data.parent;
            }
            None
        });
        // `T::clone` is user code: called without the runtime borrowed (R1).
        found.map(|rc| match rc.downcast_ref::<T>() {
            Some(v) => v.clone(),
            None => type_mismatch(),
        })
    }

    /// Like [`use_context`](Scope::use_context), but panics if no `T` was provided.
    ///
    /// # Panics
    ///
    /// With `"context {type_name} not provided"` if neither this scope nor an ancestor
    /// provides a `T`.
    #[track_caller]
    pub fn expect_context<T: Clone + 'static>(self) -> T {
        match self.use_context::<T>() {
            Some(v) => v,
            None => panic!("context {} not provided", core::any::type_name::<T>()),
        }
    }

    /// Whether the scope has not been disposed.
    pub fn is_alive(self) -> bool {
        with_runtime(|rt| rt.scope_alive(self.key))
    }
}
