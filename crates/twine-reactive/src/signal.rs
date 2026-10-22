//! Signals: [`Signal`], [`ReadSignal`], [`WriteSignal`].

use core::fmt;
use core::hash::{Hash, Hasher};

use core::panic::Location;

use crate::error::{HandleKind, ReactiveError};
use crate::global::with_runtime;
use crate::handles::{NodeHandle, with_value, with_value_mut};
use crate::memo::Memo;
use crate::scope::Scope;

const KIND: HandleKind = HandleKind::Signal;

/// Shared read path of all signal handles.
mod read {
    use super::{KIND, NodeHandle, ReactiveError, with_value};

    /// Borrows the value and calls `f` (tracked when `track`).
    ///
    /// Always inlined into the handle method (`get`, `with`, …), so a read is the runtime
    /// lookup plus the borrow at the call site, with no call layer of its own (a signal read
    /// is the hottest operation of a binding; R5 phase gate, performance fix). The lookup
    /// itself, `RuntimeState::read_node`, stays out of line and shared.
    #[inline(always)]
    pub(super) fn try_with<T: 'static, R>(
        h: NodeHandle<T>,
        track: bool,
        f: impl FnOnce(&T) -> R,
    ) -> Result<R, ReactiveError> {
        let rc = h.try_value(track, KIND)?;
        with_value::<T, R>(&rc, KIND, f)
    }

    /// [`try_with`], failing out of line (see [`ReactiveError::fail`]); inlined like it.
    #[inline(always)]
    #[track_caller]
    pub(super) fn with<T: 'static, R>(h: NodeHandle<T>, track: bool, f: impl FnOnce(&T) -> R) -> R {
        match try_with(h, track, f) {
            Ok(r) => r,
            Err(e) => h.fail(e),
        }
    }
}

/// Shared write path of all signal handles. Each write notifies only after the value cell is
/// released, and drops the old value outside it (user `Drop` may use the signal again).
mod write {
    use super::{KIND, NodeHandle, ReactiveError, with_runtime, with_value_mut};

    pub(super) fn try_update<T: 'static>(
        h: NodeHandle<T>,
        f: impl FnOnce(&mut T),
    ) -> Result<(), ReactiveError> {
        let rc = h.try_value(false, KIND)?;
        with_value_mut::<T, ()>(&rc, KIND, f)?;
        drop(rc);
        with_runtime(h.part, |rt| rt.notify(h.key));
        Ok(())
    }

    pub(super) fn try_set<T: 'static>(h: NodeHandle<T>, value: T) -> Result<(), ReactiveError> {
        let rc = h.try_value(false, KIND)?;
        let old = with_value_mut::<T, T>(&rc, KIND, |v| core::mem::replace(v, value))?;
        drop(rc);
        drop(old); // user `Drop` runs without the cell borrowed
        with_runtime(h.part, |rt| rt.notify(h.key));
        Ok(())
    }

    pub(super) fn try_set_if_changed<T: PartialEq + 'static>(
        h: NodeHandle<T>,
        value: T,
    ) -> Result<(), ReactiveError> {
        let rc = h.try_value(false, KIND)?;
        let old = with_value_mut::<T, Option<T>>(&rc, KIND, |v| {
            if *v == value {
                None
            } else {
                Some(core::mem::replace(v, value))
            }
        })?;
        drop(rc);
        if let Some(old) = old {
            drop(old);
            with_runtime(h.part, |rt| rt.notify(h.key));
        }
        Ok(())
    }

    /// The panicking (or, under `no-panic-runtime`, skipping) form of a write: the `Err` arm
    /// is out of line.
    #[inline]
    #[track_caller]
    pub(super) fn or_skip<T>(h: NodeHandle<T>, r: Result<(), ReactiveError>) {
        if let Err(e) = r {
            h.skip(e);
        }
    }
}

macro_rules! handle_impls {
    ($name:ident) => {
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T> Copy for $name<T> {}
        impl<T> PartialEq for $name<T> {
            fn eq(&self, other: &Self) -> bool {
                self.h == other.h
            }
        }
        impl<T> Eq for $name<T> {}
        impl<T> $name<T> {
            /// The partition whose runtime holds the node (for `runtime()`).
            #[inline]
            pub(crate) fn node_part(&self) -> twine_core::Partition {
                self.h.part
            }
        }
        impl<T> Hash for $name<T> {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.h.hash(state);
            }
        }
        impl<T> fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({:?})"), self.h)
            }
        }
    };
}
pub(crate) use handle_impls;

/// A reactive value: reading it inside a memo or effect subscribes that computation; writing
/// it re-runs the subscribers.
///
/// `Signal` is a `Copy` handle (`!Send`, `!Sync`) created with [`Scope::signal`]; the value
/// lives until its scope is disposed.
///
/// **Failures.** Using a handle after its scope was disposed, or writing it inside a read of
/// the same signal, is a programming error. The `try_*` forms ([`try_get`](Signal::try_get),
/// [`try_with`](Signal::try_with), [`try_set`](Signal::try_set),
/// [`try_update`](Signal::try_update)) return it as a [`ReactiveError`] and never panic; use
/// them (or [`is_alive`](Signal::is_alive)) where a signal is intentionally shared beyond its
/// scope. The other forms are thin wrappers over them: reads panic (with the creation location
/// in debug builds); writes panic too, except with the `no-panic-runtime` feature (implied by
/// the `safety` profile), which skips them with a `warn!` and a fault
/// ([`ReactiveError::fault_kind`]). See the crate docs § Failures.
///
/// ```
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let count = cx.signal(0);
/// count.set(1);
/// count.update(|n| *n += 1);
/// assert_eq!(count.get(), 2);
/// ```
///
/// Signals cannot be sent to another thread:
///
/// ```compile_fail
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let s = cx.signal(0u32);
/// std::thread::spawn(move || s.get_untracked());
/// ```
pub struct Signal<T> {
    h: NodeHandle<T>,
}

handle_impls!(Signal);

impl<T: 'static> Signal<T> {
    pub(crate) fn from_handle(h: NodeHandle<T>) -> Self {
        Signal { h }
    }

    /// The current value (tracked: subscribes the running memo/effect).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(3);
    /// assert_eq!(s.get(), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed, or inside [`update`](Signal::update) of the same
    /// signal (in every profile: there is no value to return; [`try_get`](Signal::try_get)
    /// returns the error instead).
    #[inline]
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        read::with(self.h, true, T::clone)
    }

    /// The current value without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(3);
    /// assert_eq!(s.get_untracked(), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Signal::get).
    #[inline]
    #[track_caller]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        read::with(self.h, false, T::clone)
    }

    /// Calls `f` with a reference to the value (tracked), without cloning it.
    ///
    /// Do not write the same signal inside `f` (the value is borrowed): use
    /// [`update`](Signal::update) instead.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.signal(vec![1, 2, 3]);
    /// assert_eq!(v.with(|v| v.len()), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Signal::get) ([`try_with`](Signal::try_with) returns the error instead).
    #[inline]
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, true, f)
    }

    /// Like [`with`](Signal::with), without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.signal(String::from("abc"));
    /// assert!(v.with_untracked(|s| s.starts_with('a')));
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Signal::get).
    #[inline]
    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, false, f)
    }

    /// The current value (tracked), or the reason there is none: the signal's scope was
    /// disposed ([`ReactiveError::Disposed`]) or the value is mutably borrowed by an enclosing
    /// [`update`](Signal::update) of the same signal ([`ReactiveError::Borrowed`]). Never
    /// panics (unless `T::clone` does) and raises no fault. For an untracked read, call it
    /// inside [`Runtime::untrack`](crate::Runtime::untrack).
    ///
    /// ```
    /// use twine_reactive::{HandleKind, ReactiveError};
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// assert_eq!(s.try_get(), Ok(1));
    /// cx.dispose();
    /// assert_eq!(s.try_get(), Err(ReactiveError::Disposed(HandleKind::Signal)));
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`], as above.
    #[inline]
    pub fn try_get(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        read::try_with(self.h, true, T::clone)
    }

    /// Calls `f` with a reference to the value (tracked), or returns the reason it cannot (as
    /// [`try_get`](Signal::try_get); `f` is then not called). Never panics (unless `f` does)
    /// and raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.signal(vec![1, 2]);
    /// assert_eq!(v.try_with(Vec::len), Ok(2));
    /// cx.dispose();
    /// assert!(v.try_with(Vec::len).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    #[inline]
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        read::try_with(self.h, true, f)
    }

    /// Whether the signal's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// assert!(s.is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }

    /// Replaces the value and notifies subscribers (always, even if equal).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// s.set(2);
    /// assert_eq!(s.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed, or if called inside [`with`](Signal::with) /
    /// [`update`](Signal::update) on the same signal. With `no-panic-runtime` it never
    /// panics: the write is skipped (`value` is dropped) with a `warn!` and a
    /// [`DisposedHandle`](crate::FaultKind::DisposedHandle) /
    /// [`ReentrantAccess`](crate::FaultKind::ReentrantAccess) fault.
    #[inline]
    #[track_caller]
    pub fn set(&self, value: T) {
        write::or_skip(self.h, write::try_set(self.h, value));
    }

    /// Like [`set`](Signal::set), but returns the reason the write was not made instead of
    /// panicking (`value` is then dropped). Never panics (unless a subscriber or `T`'s `Drop`
    /// does) and raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// assert_eq!(s.try_set(2), Ok(()));
    /// assert!(s.with(|_| s.try_set(3)).is_err()); // `s` is borrowed by `with`
    /// assert_eq!(s.get(), 2);
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] if the signal's scope was disposed,
    /// [`ReactiveError::Borrowed`] inside [`with`](Signal::with) / [`update`](Signal::update)
    /// of the same signal.
    #[inline]
    pub fn try_set(&self, value: T) -> Result<(), ReactiveError> {
        write::try_set(self.h, value)
    }

    /// Replaces the value and notifies subscribers only if it differs from the current one.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// s.set_if_changed(1); // no notification
    /// s.set_if_changed(2);
    /// assert_eq!(s.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`set`](Signal::set) (skipped instead under `no-panic-runtime`).
    #[inline]
    #[track_caller]
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        write::or_skip(self.h, write::try_set_if_changed(self.h, value));
    }

    /// Mutates the value in place and notifies subscribers (always).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.signal(vec![1]);
    /// v.update(|v| v.push(2));
    /// assert_eq!(v.get(), [1, 2]);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed, or if called inside [`with`](Signal::with) /
    /// `update` on the same signal (with `no-panic-runtime`: skipped, `f` is not called, see
    /// [`set`](Signal::set)); also if `f` reads the same signal (a read panics in every
    /// profile).
    #[inline]
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        write::or_skip(self.h, write::try_update(self.h, f));
    }

    /// Like [`update`](Signal::update), but returns the reason the update was not made (`f` is
    /// then not called) instead of panicking. Raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let n = cx.signal(1);
    /// assert_eq!(n.try_update(|n| *n += 1), Ok(()));
    /// cx.dispose();
    /// assert!(n.try_update(|n| *n += 1).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// As [`try_set`](Signal::try_set).
    #[inline]
    pub fn try_update(&self, f: impl FnOnce(&mut T)) -> Result<(), ReactiveError> {
        write::try_update(self.h, f)
    }

    /// A read-only handle to the same value.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// let r = s.read_only();
    /// s.set(2);
    /// assert_eq!(r.get(), 2);
    /// ```
    pub fn read_only(&self) -> ReadSignal<T> {
        ReadSignal { h: self.h }
    }

    /// A write-only handle to the same value.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let s = cx.signal(1);
    /// s.write_only().set(5);
    /// assert_eq!(s.get(), 5);
    /// ```
    pub fn write_only(&self) -> WriteSignal<T> {
        WriteSignal { h: self.h }
    }

    /// Read and write handles to the same value.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let (count, set_count) = cx.signal(0).split();
    /// set_count.set(3);
    /// assert_eq!(count.get(), 3);
    /// ```
    pub fn split(&self) -> (ReadSignal<T>, WriteSignal<T>) {
        (self.read_only(), self.write_only())
    }

    /// A [`Memo`] derived from this signal, owned by the signal's scope.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let n = cx.signal(3);
    /// let even = n.map(|n| n % 2 == 0);
    /// assert!(!even.get());
    /// n.set(4);
    /// assert!(even.get());
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed, like [`Scope::memo`] on a disposed scope (with
    /// `no-panic-runtime`: a memo that is never alive is returned, with a `warn!` and a fault).
    #[track_caller]
    pub fn map<U: PartialEq + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let s = *self;
        map_in_owner(self.h, KIND, move || s.with(|v| f(v)))
    }
}

/// The scope owning node `h`, or `Disposed(kind)` if the node is dead.
pub(crate) fn owner_scope<T>(h: NodeHandle<T>, kind: HandleKind) -> Result<Scope, ReactiveError> {
    with_runtime(h.part, |rt| rt.node_scope(h.key))
        .map(|k| Scope::from_key(k, h.part))
        .ok_or(ReactiveError::Disposed(kind))
}

/// `map` of signals and memos: a memo computing `f`, owned by the scope owning `h` (failing
/// like [`Scope::memo`] on a disposed scope when `h` is dead).
#[track_caller]
pub(crate) fn map_in_owner<T, U: PartialEq + 'static>(
    h: NodeHandle<T>,
    kind: HandleKind,
    f: impl Fn() -> U + 'static,
) -> Memo<U> {
    match owner_scope(h, kind) {
        Ok(cx) => cx.memo(f),
        Err(e) => {
            h.skip(e);
            Memo::from_handle(NodeHandle::dead(h.part, Location::caller()))
        }
    }
}

/// A read-only view of a [`Signal`] (see [`Signal::read_only`], [`Signal::split`]).
///
/// Fails like [`Signal`] (see its § Failures).
///
/// ```
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let (r, w) = cx.signal(1).split();
/// w.set(2);
/// assert_eq!(r.get(), 2);
/// ```
pub struct ReadSignal<T> {
    h: NodeHandle<T>,
}

handle_impls!(ReadSignal);

impl<T: 'static> ReadSignal<T> {
    /// The current value (tracked). Panics like [`Signal::get`].
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert_eq!(cx.signal(1).read_only().get(), 1);
    /// ```
    #[inline]
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        read::with(self.h, true, T::clone)
    }

    /// The current value without subscribing. Panics like [`Signal::get`].
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert_eq!(cx.signal(1).read_only().get_untracked(), 1);
    /// ```
    #[inline]
    #[track_caller]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        read::with(self.h, false, T::clone)
    }

    /// Calls `f` with a reference to the value (tracked). See [`Signal::with`].
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let r = cx.signal(vec![1, 2]).read_only();
    /// assert_eq!(r.with(Vec::len), 2);
    /// ```
    #[inline]
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, true, f)
    }

    /// Like [`with`](ReadSignal::with), without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let r = cx.signal(vec![1, 2]).read_only();
    /// assert_eq!(r.with_untracked(Vec::len), 2);
    /// ```
    #[inline]
    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, false, f)
    }

    /// The current value (tracked), or the reason there is none (see [`Signal::try_get`]).
    /// Never panics; raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let r = cx.signal(1).read_only();
    /// cx.dispose();
    /// assert!(r.try_get().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    #[inline]
    pub fn try_get(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        read::try_with(self.h, true, T::clone)
    }

    /// Calls `f` with a reference to the value (tracked), or returns the reason it cannot (see
    /// [`Signal::try_with`]). Never panics (unless `f` does); raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let r = cx.signal(vec![1, 2, 3]).read_only();
    /// assert_eq!(r.try_with(Vec::len), Ok(3));
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    #[inline]
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        read::try_with(self.h, true, f)
    }

    /// Whether the signal's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert!(cx.signal(1).read_only().is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }

    /// A [`Memo`] derived from this signal (see [`Signal::map`]).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let r = cx.signal(2).read_only();
    /// assert_eq!(r.map(|n| n * 10).get(), 20);
    /// ```
    #[track_caller]
    pub fn map<U: PartialEq + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let s = *self;
        map_in_owner(self.h, KIND, move || s.with(|v| f(v)))
    }
}

/// A write-only view of a [`Signal`] (see [`Signal::write_only`], [`Signal::split`]).
///
/// Fails like [`Signal`] (see its § Failures).
///
/// ```
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let s = cx.signal(1);
/// let w = s.write_only();
/// w.update(|n| *n += 1);
/// assert_eq!(s.get(), 2);
/// ```
pub struct WriteSignal<T> {
    h: NodeHandle<T>,
}

handle_impls!(WriteSignal);

impl<T: 'static> WriteSignal<T> {
    /// Replaces the value and notifies (see [`Signal::set`]).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let (r, w) = cx.signal(1).split();
    /// w.set(9);
    /// assert_eq!(r.get(), 9);
    /// ```
    #[inline]
    #[track_caller]
    pub fn set(&self, value: T) {
        write::or_skip(self.h, write::try_set(self.h, value));
    }

    /// Like [`set`](WriteSignal::set), returning the reason the write was not made (see
    /// [`Signal::try_set`]). Never panics; raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let w = cx.signal(1).write_only();
    /// assert_eq!(w.try_set(2), Ok(()));
    /// cx.dispose();
    /// assert!(w.try_set(3).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    #[inline]
    pub fn try_set(&self, value: T) -> Result<(), ReactiveError> {
        write::try_set(self.h, value)
    }

    /// Replaces the value and notifies only if it changed (see [`Signal::set_if_changed`]).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let (r, w) = cx.signal(1).split();
    /// w.set_if_changed(2);
    /// assert_eq!(r.get(), 2);
    /// ```
    #[inline]
    #[track_caller]
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        write::or_skip(self.h, write::try_set_if_changed(self.h, value));
    }

    /// Mutates the value in place and notifies (see [`Signal::update`]).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let (r, w) = cx.signal(1).split();
    /// w.update(|n| *n *= 3);
    /// assert_eq!(r.get(), 3);
    /// ```
    #[inline]
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        write::or_skip(self.h, write::try_update(self.h, f));
    }

    /// Like [`update`](WriteSignal::update), returning the reason the update was not made (see
    /// [`Signal::try_update`]). Never panics (unless `f` does); raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let (r, w) = cx.signal(1).split();
    /// assert_eq!(w.try_update(|n| *n += 1), Ok(()));
    /// assert_eq!(r.get(), 2);
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    #[inline]
    pub fn try_update(&self, f: impl FnOnce(&mut T)) -> Result<(), ReactiveError> {
        write::try_update(self.h, f)
    }

    /// Whether the signal's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert!(cx.signal(1).write_only().is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }
}
