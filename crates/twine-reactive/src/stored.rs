//! [`StoredValue`]: a non-reactive value owned by a scope, behind a `Copy` handle.

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::error::{HandleKind, ReactiveError};
use crate::handles::{NodeHandle, with_value, with_value_mut};
use crate::signal::handle_impls;

const KIND: HandleKind = HandleKind::StoredValue;

/// A plain (non-reactive) value stored in the runtime and owned by a [`Scope`](crate::Scope),
/// behind a `Copy` handle.
///
/// Use it for state that handlers and closures share but that nothing should re-run on:
/// reading never subscribes the running memo or effect, and writing notifies nobody. It lets
/// a handle type be `Copy` (moved into any number of closures without `clone`) instead of an
/// `Rc<RefCell<T>>`. The value is dropped when its scope is disposed.
///
/// Access costs the same as [`Signal::with_untracked`](crate::Signal::with_untracked): one
/// arena lookup, no tracking. Internally a stored value is a node without subscribers (it
/// counts in [`Stats::nodes`](crate::Stats::nodes)), so it never takes part in propagation.
///
/// **Failures** as for [`Signal`](crate::Signal) (see its § Failures): using a handle whose
/// scope was disposed, or a re-entrant mutable access, fails. [`try_with`](Self::try_with),
/// [`try_with_mut`](Self::try_with_mut), [`try_get`](Self::try_get) and
/// [`try_set`](Self::try_set) return a [`ReactiveError`] and never panic (use them, or
/// [`is_alive`](Self::is_alive), for handles that may outlive their scope); the others panic
/// (with the creation location in debug builds) — except [`set`](Self::set), which the
/// `no-panic-runtime` feature skips with a `warn!` and a fault instead.
///
/// ```
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let log = cx.stored_value(Vec::new());
/// let push = move |s: &'static str| log.with_mut(|l| l.push(s));
/// push("a");
/// push("b");
/// assert_eq!(log.get(), ["a", "b"]);
/// cx.dispose();
/// assert!(log.try_get().is_err());
/// ```
///
/// Stored values cannot be sent to another thread:
///
/// ```compile_fail
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let v = cx.stored_value(0u32);
/// std::thread::spawn(move || v.get());
/// ```
pub struct StoredValue<T> {
    h: NodeHandle<T>,
}

handle_impls!(StoredValue);

impl<T: 'static> StoredValue<T> {
    pub(crate) fn from_handle(h: NodeHandle<T>) -> Self {
        StoredValue { h }
    }

    /// Calls `f` with a reference to the value.
    ///
    /// Do not access the same stored value mutably inside `f` (the value is borrowed).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value(vec![1, 2, 3]);
    /// assert_eq!(v.with(Vec::len), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// In every profile, if the value's scope was disposed, or inside a
    /// [`with_mut`](Self::with_mut) of the same stored value: there is no value to give `f`
    /// ([`try_with`](Self::try_with) returns the error instead). Nested shared accesses
    /// (`with` / [`get`](Self::get) inside `with`) are fine.
    #[inline]
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self.try_with(f) {
            Ok(r) => r,
            Err(e) => self.h.fail(e),
        }
    }

    /// Calls `f` with a mutable reference to the value (nobody is notified).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let n = cx.stored_value(1);
    /// n.with_mut(|n| *n += 1);
    /// assert_eq!(n.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// In every profile (it must return `f`'s result), if the value's scope was disposed, or
    /// on a re-entrant access: this call made inside a [`with`](Self::with) / `with_mut` of the
    /// same stored value, or `f` accessing it. [`try_with_mut`](Self::try_with_mut) returns
    /// the error instead.
    #[inline]
    #[track_caller]
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        match self.try_with_mut(f) {
            Ok(r) => r,
            Err(e) => self.h.fail(e),
        }
    }

    /// A clone of the value.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert_eq!(cx.stored_value(7).get(), 7);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`with`](Self::with) ([`try_get`](Self::try_get) returns the error instead).
    #[inline]
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// Replaces the value (nobody is notified). The old value is dropped after the value is
    /// released, so its `Drop` may use this stored value again.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value("a");
    /// v.set("b");
    /// assert_eq!(v.get(), "b");
    /// ```
    ///
    /// # Panics
    ///
    /// If the value's scope was disposed, or if called inside [`with`](Self::with) /
    /// [`with_mut`](Self::with_mut) on the same stored value. With `no-panic-runtime` it never
    /// panics: the write is skipped (`value` is dropped) with a `warn!` and a
    /// [`DisposedHandle`](crate::FaultKind::DisposedHandle) /
    /// [`ReentrantAccess`](crate::FaultKind::ReentrantAccess) fault.
    #[inline]
    #[track_caller]
    pub fn set(&self, value: T) {
        if let Err(e) = self.try_set(value) {
            self.h.skip(e);
        }
    }

    /// Like [`set`](Self::set), but returns the reason the value was not replaced (`value` is
    /// then dropped) instead of panicking. Raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value(1);
    /// assert_eq!(v.try_set(2), Ok(()));
    /// assert!(v.with(|_| v.try_set(3)).is_err()); // borrowed by `with`
    /// assert_eq!(v.get(), 2);
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] if the value's scope was disposed,
    /// [`ReactiveError::Borrowed`] inside [`with`](Self::with) / [`with_mut`](Self::with_mut)
    /// of the same stored value.
    pub fn try_set(&self, value: T) -> Result<(), ReactiveError> {
        let old = self.try_with_mut(|v| core::mem::replace(v, value))?;
        drop(old);
        Ok(())
    }

    /// Calls `f` with a reference to the value, or returns the reason it cannot (`f` is then
    /// not called): the value's scope was disposed ([`ReactiveError::Disposed`]) or the value
    /// is borrowed by an enclosing [`with_mut`](Self::with_mut) ([`ReactiveError::Borrowed`]).
    /// Never panics (unless `f` does); raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value(3);
    /// assert_eq!(v.try_with(|n| n * 2), Ok(6));
    /// cx.dispose();
    /// assert!(v.try_with(|n| n * 2).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`], as above.
    #[inline]
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        let rc = self.h.try_value(false, KIND)?;
        with_value::<T, R>(&rc, KIND, f)
    }

    /// Like [`with_mut`](Self::with_mut), or the reason it cannot (`f` is then not called):
    /// see [`try_with`](Self::try_with); a re-entrant access is also an error inside a `with`
    /// of the same value. Never panics (unless `f` does); raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value(3);
    /// assert_eq!(v.try_with_mut(|n| { *n += 1; *n }), Ok(4));
    /// cx.dispose();
    /// assert!(v.try_with_mut(|n| *n).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    #[inline]
    pub fn try_with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, ReactiveError> {
        let rc = self.h.try_value(false, KIND)?;
        with_value_mut::<T, R>(&rc, KIND, f)
    }

    /// A clone of the value, or the reason there is none (see [`try_with`](Self::try_with)).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value(1);
    /// assert_eq!(v.try_get(), Ok(1));
    /// cx.dispose();
    /// assert!(v.try_get().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Borrowed`].
    pub fn try_get(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.try_with(T::clone)
    }

    /// Whether the value's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.stored_value(());
    /// assert!(v.is_alive());
    /// cx.dispose();
    /// assert!(!v.is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }
}
