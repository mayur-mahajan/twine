//! [`Scope`]: ownership of reactive nodes, nesting, disposal and cleanups (context values:
//! `context.rs`).

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
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
use crate::runtime::{Kind, ScopeKey, ValueRc};
use crate::signal::Signal;
use crate::stored::StoredValue;

/// An owner of signals, memos, effects, child scopes, cleanups and context values.
///
/// `Scope` is a `Copy` handle (`!Send`, `!Sync`). Everything created through a scope is
/// released when the scope is [disposed](Scope::dispose); child scopes are disposed with
/// their parent.
///
/// ```
/// let root = twine_reactive::Runtime::take().unwrap().create_root();
/// let child = root.child();
/// let n = child.signal(5);
/// root.dispose(); // disposes `child` too
/// assert!(!child.is_alive());
/// assert!(!n.is_alive());
/// ```
///
/// # Context values
///
/// A scope can [`provide`](Scope::provide) values for itself and all its descendants;
/// [`use_context`](Scope::use_context) finds the nearest one of a type. Values are keyed by
/// their type, so give each meaning its own type (`struct Volume(Signal<u8>)`, not a bare
/// `Signal<u8>`); a descendant providing the same type shadows its ancestors for its subtree,
/// and a value is dropped with the scope that provided it. The usual use is an application
/// store — a `Copy` struct of signals and memos with action methods — provided by the root
/// scope and read by any component (see the facade's `twine::guide::app_state` guide).
///
/// **Lookup cost.** `use_context` walks the scope chain from the asking scope to its root:
/// per scope one arena lookup and a linear scan of the values *that scope* provides, comparing
/// [`TypeId`](core::any::TypeId)s. It **allocates nothing**: the found value is shared (`Rc`),
/// its reference count goes up and down once, and `T::clone` runs on it (a `memcpy` for a
/// `Copy` handle struct). The cost grows with the depth of the asking scope and the number of
/// values each ancestor provides — a few dozen comparisons in a typical application — so look
/// a context up **once**, where a component is built, and move the (`Copy`) result into its
/// closures rather than looking it up in bindings or frequent event handlers. The walk is one
/// non-generic function shared by every context type. `provide` allocates one `Rc` per value.
///
/// **Failure convention.** Lookups that can fail — here and in the hooks built on them
/// (`twine_view::use_navigator`, `twine_extra::i18n::use_i18n`) — come in two forms:
///
/// - `use_*` returns a `Result` whose error, [`MissingContext`](crate::MissingContext), names the
///   missing type: the form for code that must not panic (`.ok()` makes an optional context an
///   `Option`);
/// - `expect_*` ([`expect_context`](Scope::expect_context), `expect_navigator`, `expect_i18n`)
///   is a thin `#[track_caller]` wrapper that panics with the same message at the caller's
///   location: for application code where a missing context is a bug.
///
/// ```
/// use twine_reactive::{MissingContext, Runtime, Signal};
///
/// #[derive(Clone, Copy)]
/// struct Volume(Signal<u8>);
///
/// let root = Runtime::take().unwrap().create_root();
/// root.provide(Volume(root.signal(5)));
/// let page = root.child();
/// let Volume(v) = page.expect_context::<Volume>(); // found in the root
/// v.set(6);
/// assert_eq!(page.use_context::<u32>(), Err(MissingContext::of::<u32>()));
/// ```
///
/// Scopes cannot be sent to another thread:
///
/// ```compile_fail
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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
    pub(crate) fn create_node<T>(self, value: Option<ValueRc>, kind: Option<Kind>) -> NodeHandle<T> {
        let loc = Location::caller();
        match with_runtime(|rt| rt.create_node(self.key, value, kind)) {
            Some(key) => NodeHandle::new(key, loc),
            None => scope_disposed(),
        }
    }

    /// Creates a signal holding `value`, owned by this scope.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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

    /// Stores a plain, non-reactive `value` owned by this scope (see [`StoredValue`]): a
    /// `Copy` handle, dropped with the scope; reads never subscribe and writes notify nobody.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let clicks = cx.stored_value(0u32);
    /// let on_click = move || clicks.with_mut(|n| *n += 1);
    /// on_click();
    /// on_click();
    /// assert_eq!(clicks.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    pub fn stored_value<T: 'static>(self, value: T) -> StoredValue<T> {
        let rc: ValueRc = Rc::new(RefCell::new(value));
        StoredValue::from_handle(self.create_node(Some(rc), None))
    }

    /// Creates a lazily computed, cached derived value (see [`Memo`]).
    ///
    /// The closure runs on the first read and again only when a dependency changed and the
    /// memo is read (or a dependent effect runs). If the new value equals the old one,
    /// dependents are not re-run.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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
        Memo::from_handle(self.create_node(Some(rc), Some(Kind::Memo(Rc::new(compute)))))
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
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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
    /// [`Runtime::flush_effects_with`](crate::Runtime::flush_effects_with) (`&mut ()` for automatic flushes).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let a = cx.signal(1);
    /// cx.effect_with_cx(move |ctx| {
    ///     let v = a.get();
    ///     if let Some(total) = ctx.downcast_mut::<u32>() {
    ///         *total += v;
    ///     }
    /// });
    /// let mut total = 0u32;
    /// let rt = cx.runtime();
    /// rt.batch(|| {
    ///     a.set(5);
    ///     rt.flush_effects_with(&mut total);
    /// });
    /// assert_eq!(total, 5);
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed.
    #[track_caller]
    pub fn effect_with_cx(self, f: impl FnMut(&mut dyn Any) + 'static) -> EffectId {
        let h = self.create_node::<()>(None, Some(Kind::Effect(Rc::new(RefCell::new(f)))));
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
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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

    /// Whether the scope has not been disposed.
    pub fn is_alive(self) -> bool {
        with_runtime(|rt| rt.scope_alive(self.key))
    }
}
