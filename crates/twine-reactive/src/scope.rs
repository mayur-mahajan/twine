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
use crate::error::{HandleKind, ReactiveError};
use crate::global::with_runtime;
use crate::handles::NodeHandle;
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
///   location: for application code where a missing context is a bug. The `safety` profile
///   removes `expect_context` and `expect_navigator` (no panicking lookups).
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

impl Scope {
    pub(crate) fn from_key(key: ScopeKey) -> Self {
        Scope { key, _m: PhantomData }
    }

    /// A scope that is never alive (what a creation skipped under `no-panic-runtime`
    /// returns).
    pub(crate) fn dead() -> Self {
        Scope::from_key(ScopeKey::INVALID)
    }

    pub(crate) fn key(self) -> ScopeKey {
        self.key
    }

    /// Inserts a node owned by this scope (`what`: the kind of handle made from it, for the
    /// error); the value and closure of a refused node are dropped here, outside the runtime's
    /// borrow.
    #[track_caller]
    pub(crate) fn try_create_node<T>(
        self,
        value: Option<ValueRc>,
        kind: Option<Kind>,
        what: HandleKind,
    ) -> Result<NodeHandle<T>, ReactiveError> {
        let loc = Location::caller();
        match with_runtime(|rt| rt.create_node(self.key, value, kind, what)) {
            Ok(key) => Ok(NodeHandle::new(key, loc)),
            Err((e, value, kind)) => {
                drop((value, kind)); // user values and closures, outside the borrow (R1)
                Err(e)
            }
        }
    }

    /// The panicking (under `no-panic-runtime`: skipping) form of a creation: on `Err`, a
    /// handle that is never alive, after [`ReactiveError::skip`] (out of line).
    #[inline]
    #[track_caller]
    fn or_dead<T>(r: Result<NodeHandle<T>, ReactiveError>) -> NodeHandle<T> {
        match r {
            Ok(h) => h,
            Err(e) => dead_node(e),
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
    /// If the scope was disposed, or if the runtime already holds `Limits::max_reactive_nodes` live nodes. With
    /// `no-panic-runtime` it never panics: it returns a signal that is never alive (writes
    /// are skipped, reads fail) after a `warn!` and a
    /// [`DisposedHandle`](crate::FaultKind::DisposedHandle) /
    /// [`Capacity`](crate::FaultKind::Capacity) fault. [`try_signal`](Scope::try_signal)
    /// returns the error instead.
    #[inline]
    #[track_caller]
    pub fn signal<T: 'static>(self, value: T) -> Signal<T> {
        Signal::from_handle(Self::or_dead(self.signal_node(value)))
    }

    /// Like [`signal`](Scope::signal), but returns the reason the signal could not be created
    /// (`value` is then dropped) instead of panicking. Raises no fault.
    ///
    /// ```
    /// use twine_reactive::{HandleKind, ReactiveError};
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.try_signal(1).expect("scope alive");
    /// assert_eq!(s.get(), 1);
    /// cx.dispose();
    /// assert_eq!(cx.try_signal(2).err(), Some(ReactiveError::Disposed(HandleKind::Scope)));
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] (`HandleKind::Scope`) if the scope was disposed;
    /// [`ReactiveError::Capacity`] if the runtime already holds `Limits::max_reactive_nodes` live nodes.
    #[inline]
    #[track_caller]
    pub fn try_signal<T: 'static>(self, value: T) -> Result<Signal<T>, ReactiveError> {
        self.signal_node(value).map(Signal::from_handle)
    }

    #[track_caller]
    fn signal_node<T: 'static>(self, value: T) -> Result<NodeHandle<T>, ReactiveError> {
        let rc: ValueRc = Rc::new(RefCell::new(value));
        self.try_create_node(Some(rc), None, HandleKind::Signal)
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
    /// Like [`signal`](Scope::signal) (with `no-panic-runtime`: a stored value that is never
    /// alive, and a fault). [`try_stored_value`](Scope::try_stored_value) returns the error.
    #[inline]
    #[track_caller]
    pub fn stored_value<T: 'static>(self, value: T) -> StoredValue<T> {
        StoredValue::from_handle(Self::or_dead(self.stored_node(value)))
    }

    /// Like [`stored_value`](Scope::stored_value), returning the reason the value could not be
    /// stored (`value` is then dropped). Raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert_eq!(cx.try_stored_value(5).map(|v| v.get()), Ok(5));
    /// cx.dispose();
    /// assert!(cx.try_stored_value(5).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// As [`try_signal`](Scope::try_signal).
    #[inline]
    #[track_caller]
    pub fn try_stored_value<T: 'static>(self, value: T) -> Result<StoredValue<T>, ReactiveError> {
        self.stored_node(value).map(StoredValue::from_handle)
    }

    #[track_caller]
    fn stored_node<T: 'static>(self, value: T) -> Result<NodeHandle<T>, ReactiveError> {
        let rc: ValueRc = Rc::new(RefCell::new(value));
        self.try_create_node(Some(rc), None, HandleKind::StoredValue)
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
    /// Like [`signal`](Scope::signal) (with `no-panic-runtime`: a memo that is never alive,
    /// and a fault; `f` is dropped). [`try_memo`](Scope::try_memo) returns the error.
    #[inline]
    #[track_caller]
    pub fn memo<T: PartialEq + 'static>(self, f: impl Fn() -> T + 'static) -> Memo<T> {
        Memo::from_handle(Self::or_dead(self.memo_node(f)))
    }

    /// Like [`memo`](Scope::memo), returning the reason the memo could not be created (`f` is
    /// then dropped). Raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.try_memo(|| 6 * 7).expect("scope alive");
    /// assert_eq!(m.get(), 42);
    /// cx.dispose();
    /// assert!(cx.try_memo(|| 0).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// As [`try_signal`](Scope::try_signal).
    #[inline]
    #[track_caller]
    pub fn try_memo<T: PartialEq + 'static>(
        self,
        f: impl Fn() -> T + 'static,
    ) -> Result<Memo<T>, ReactiveError> {
        self.memo_node(f).map(Memo::from_handle)
    }

    #[track_caller]
    fn memo_node<T: PartialEq + 'static>(
        self,
        f: impl Fn() -> T + 'static,
    ) -> Result<NodeHandle<T>, ReactiveError> {
        let rc: ValueRc = Rc::new(RefCell::new(None::<T>));
        let compute = move |cell: &RefCell<dyn Any>| -> Option<bool> {
            let new = f();
            // Borrowed only by a `with` of this memo further up the stack (see `recompute`).
            let mut b = cell.try_borrow_mut().ok()?;
            debug_assert!(
                b.is::<Option<T>>(),
                "twine-reactive: memo cell of another type (internal bug)"
            );
            let slot = b.downcast_mut::<Option<T>>()?;
            if slot.as_ref() == Some(&new) {
                return Some(false);
            }
            let old = slot.replace(new);
            drop(b);
            drop(old); // user `Drop` runs without the cell borrowed
            Some(true)
        };
        self.try_create_node(Some(rc), Some(Kind::Memo(Rc::new(compute))), HandleKind::Memo)
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
    /// Like [`signal`](Scope::signal) (with `no-panic-runtime`: `f` is dropped without
    /// running and a dead [`EffectId`] is returned, with a fault).
    /// [`try_effect`](Scope::try_effect) returns the error.
    #[inline]
    #[track_caller]
    pub fn effect(self, mut f: impl FnMut() + 'static) -> EffectId {
        self.effect_with_cx(move |_| f())
    }

    /// Like [`effect`](Scope::effect), returning the reason the effect could not be created
    /// (`f` is then dropped without running). Raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert!(cx.try_effect(|| {}).is_ok());
    /// cx.dispose();
    /// assert!(cx.try_effect(|| {}).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// As [`try_signal`](Scope::try_signal).
    #[inline]
    #[track_caller]
    pub fn try_effect(self, mut f: impl FnMut() + 'static) -> Result<EffectId, ReactiveError> {
        self.effect_node(move |_| f())
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
    /// Like [`effect`](Scope::effect).
    #[inline]
    #[track_caller]
    pub fn effect_with_cx(self, f: impl FnMut(&mut dyn Any) + 'static) -> EffectId {
        match self.effect_node(f) {
            Ok(id) => id,
            Err(e) => EffectId::from_handle(dead_node(e)),
        }
    }

    #[track_caller]
    fn effect_node(self, f: impl FnMut(&mut dyn Any) + 'static) -> Result<EffectId, ReactiveError> {
        let h = self.try_create_node::<()>(
            None,
            Some(Kind::Effect(Rc::new(RefCell::new(f)))),
            HandleKind::Effect,
        )?;
        with_runtime(|rt| rt.run_new_effect(h.key));
        Ok(EffectId::from_handle(h))
    }

    /// Creates a child scope, disposed together with this one.
    ///
    /// ```
    /// let root = twine_reactive::Runtime::take().unwrap().create_root();
    /// let page = root.child();
    /// root.dispose();
    /// assert!(!page.is_alive());
    /// ```
    ///
    /// # Panics
    ///
    /// If the scope was disposed, or if the runtime already holds `Limits::max_scopes` live scopes. With
    /// `no-panic-runtime` it never panics: it returns a scope that is never alive (everything
    /// created in it fails the same way) after a `warn!` and a fault.
    /// [`try_child`](Scope::try_child) returns the error instead.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn child(self) -> Scope {
        match self.try_child() {
            Ok(s) => s,
            Err(e) => dead_scope(e),
        }
    }

    /// Like [`child`](Scope::child), returning the reason the scope could not be created.
    /// Raises no fault.
    ///
    /// ```
    /// use twine_reactive::{HandleKind, ReactiveError};
    /// let root = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert!(root.try_child().is_ok());
    /// root.dispose();
    /// assert_eq!(root.try_child(), Err(ReactiveError::Disposed(HandleKind::Scope)));
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] (`HandleKind::Scope`) if this scope was disposed;
    /// [`ReactiveError::Capacity`] (`HandleKind::Scope`) if the runtime already holds `Limits::max_scopes`
    /// live scopes.
    pub fn try_child(self) -> Result<Scope, ReactiveError> {
        with_runtime(|rt| rt.create_scope(Some(self.key))).map(Scope::from_key)
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

/// The failure path of a node creation: [`ReactiveError::skip`], then a handle that is never
/// alive (reached only under `no-panic-runtime`; otherwise `skip` panics). Out of line.
#[cold]
#[inline(never)]
#[track_caller]
fn dead_node<T>(e: ReactiveError) -> NodeHandle<T> {
    e.skip(None);
    NodeHandle::dead(Location::caller())
}

/// The failure path of a scope creation, like [`dead_node`].
#[cold]
#[inline(never)]
#[track_caller]
pub(crate) fn dead_scope(e: ReactiveError) -> Scope {
    e.skip(None);
    Scope::dead()
}
