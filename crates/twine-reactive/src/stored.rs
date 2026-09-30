//! [`StoredValue`]: a non-reactive value owned by a scope, behind a `Copy` handle.

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::global::with_runtime;
use crate::handles::{NodeHandle, with_value, with_value_mut};
use crate::signal::handle_impls;

const WHAT: &str = "stored value";

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
/// Panic policy as for [`Signal`](crate::Signal): using a handle whose scope was disposed
/// panics (with the creation location in debug builds); [`try_with`](Self::try_with),
/// [`try_with_mut`](Self::try_with_mut), [`try_get`](Self::try_get) and
/// [`is_alive`](Self::is_alive) exist for handles that may outlive their scope.
///
/// ```
/// let cx = twine_reactive::create_root();
/// let log = cx.stored_value(Vec::new());
/// let push = move |s: &'static str| log.with_mut(|l| l.push(s));
/// push("a");
/// push("b");
/// assert_eq!(log.get(), ["a", "b"]);
/// cx.dispose();
/// assert_eq!(log.try_get(), None);
/// ```
///
/// Stored values cannot be sent to another thread:
///
/// ```compile_fail
/// let cx = twine_reactive::create_root();
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
    /// Do not access the same stored value mutably inside `f` (the value is borrowed; that
    /// panics with a `RefCell` borrow error).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.stored_value(vec![1, 2, 3]);
    /// assert_eq!(v.with(Vec::len), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// If the value's scope was disposed.
    #[inline]
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let rc = self.h.value(false, WHAT);
        with_value::<T, R>(&rc, f)
    }

    /// Calls `f` with a mutable reference to the value (nobody is notified).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let n = cx.stored_value(1);
    /// n.with_mut(|n| *n += 1);
    /// assert_eq!(n.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// If the value's scope was disposed, or if `f` accesses the same stored value.
    #[inline]
    #[track_caller]
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let rc = self.h.value(false, WHAT);
        with_value_mut::<T, R>(&rc, f)
    }

    /// A clone of the value.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// assert_eq!(cx.stored_value(7).get(), 7);
    /// ```
    ///
    /// # Panics
    ///
    /// If the value's scope was disposed.
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
    /// let cx = twine_reactive::create_root();
    /// let v = cx.stored_value("a");
    /// v.set("b");
    /// assert_eq!(v.get(), "b");
    /// ```
    ///
    /// # Panics
    ///
    /// If the value's scope was disposed, or if called inside [`with`](Self::with) /
    /// [`with_mut`](Self::with_mut) on the same stored value.
    #[track_caller]
    pub fn set(&self, value: T) {
        let old = self.with_mut(|v| core::mem::replace(v, value));
        drop(old);
    }

    /// Calls `f` with a reference to the value, or returns `None` if the value's scope was
    /// disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.stored_value(3);
    /// assert_eq!(v.try_with(|n| n * 2), Some(6));
    /// cx.dispose();
    /// assert_eq!(v.try_with(|n| n * 2), None);
    /// ```
    #[inline]
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        let rc = with_runtime(|rt| rt.read_node(self.h.key, false))?;
        Some(with_value::<T, R>(&rc, f))
    }

    /// Like [`with_mut`](Self::with_mut), or `None` (without calling `f`) if the value's scope
    /// was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.stored_value(3);
    /// assert_eq!(v.try_with_mut(|n| { *n += 1; *n }), Some(4));
    /// cx.dispose();
    /// assert_eq!(v.try_with_mut(|n| *n), None);
    /// ```
    #[inline]
    pub fn try_with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let rc = with_runtime(|rt| rt.read_node(self.h.key, false))?;
        Some(with_value_mut::<T, R>(&rc, f))
    }

    /// A clone of the value, or `None` if the value's scope was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.stored_value(1);
    /// assert_eq!(v.try_get(), Some(1));
    /// cx.dispose();
    /// assert_eq!(v.try_get(), None);
    /// ```
    pub fn try_get(&self) -> Option<T>
    where
        T: Clone,
    {
        self.try_with(T::clone)
    }

    /// Whether the value's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.stored_value(());
    /// assert!(v.is_alive());
    /// cx.dispose();
    /// assert!(!v.is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }
}
