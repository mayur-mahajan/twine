//! Signals: [`Signal`], [`ReadSignal`], [`WriteSignal`] (design 02 §2).

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::global::with_runtime;
use crate::handles::{NodeHandle, with_value, with_value_mut};
use crate::memo::Memo;
use crate::scope::Scope;

const WHAT: &str = "signal";

/// Shared read path of all signal handles.
mod read {
    use super::{NodeHandle, WHAT, with_value};

    #[track_caller]
    pub(super) fn with<T: 'static, R>(h: NodeHandle<T>, track: bool, f: impl FnOnce(&T) -> R) -> R {
        let rc = h.value(track, WHAT);
        with_value::<T, R>(&rc, f)
    }

    pub(super) fn try_get<T: Clone + 'static>(h: NodeHandle<T>) -> Option<T> {
        let rc = crate::global::with_runtime(|rt| rt.read_node(h.key, true))?;
        Some(with_value::<T, T>(&rc, T::clone))
    }
}

/// Shared write path of all signal handles.
mod write {
    use super::{NodeHandle, WHAT, with_runtime, with_value_mut};

    #[track_caller]
    pub(super) fn update<T: 'static>(h: NodeHandle<T>, f: impl FnOnce(&mut T)) {
        let rc = h.value(false, WHAT);
        with_value_mut::<T, ()>(&rc, f);
        drop(rc);
        with_runtime(|rt| rt.notify(h.key));
    }

    #[track_caller]
    pub(super) fn set<T: 'static>(h: NodeHandle<T>, value: T) {
        let rc = h.value(false, WHAT);
        let old = with_value_mut::<T, T>(&rc, |v| core::mem::replace(v, value));
        drop(rc);
        drop(old); // user `Drop` runs without the cell borrowed
        with_runtime(|rt| rt.notify(h.key));
    }

    #[track_caller]
    pub(super) fn set_if_changed<T: PartialEq + 'static>(h: NodeHandle<T>, value: T) {
        let rc = h.value(false, WHAT);
        let old = with_value_mut::<T, Option<T>>(&rc, |v| {
            if *v == value {
                None
            } else {
                Some(core::mem::replace(v, value))
            }
        });
        drop(rc);
        if let Some(old) = old {
            drop(old);
            with_runtime(|rt| rt.notify(h.key));
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
/// lives until its scope is disposed. Using a handle after that panics (with the creation
/// location in debug builds); use [`try_get`](Signal::try_get) / [`is_alive`](Signal::is_alive)
/// where a signal is intentionally shared across scopes.
///
/// ```
/// let cx = twine_reactive::create_root();
/// let count = cx.signal(0);
/// count.set(1);
/// count.update(|n| *n += 1);
/// assert_eq!(count.get(), 2);
/// ```
///
/// Signals cannot be sent to another thread:
///
/// ```compile_fail
/// let cx = twine_reactive::create_root();
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
    /// let cx = twine_reactive::create_root();
    /// let s = cx.signal(3);
    /// assert_eq!(s.get(), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed.
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
    /// let cx = twine_reactive::create_root();
    /// let s = cx.signal(3);
    /// assert_eq!(s.get_untracked(), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed.
    #[track_caller]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        read::with(self.h, false, T::clone)
    }

    /// Calls `f` with a reference to the value (tracked), without cloning it.
    ///
    /// Do not write the same signal inside `f` (the value is borrowed; the write panics with a
    /// `RefCell` borrow error) — use [`update`](Signal::update) instead.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.signal(vec![1, 2, 3]);
    /// assert_eq!(v.with(|v| v.len()), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed.
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, true, f)
    }

    /// Like [`with`](Signal::with), without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.signal(String::from("abc"));
    /// assert!(v.with_untracked(|s| s.starts_with('a')));
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed.
    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, false, f)
    }

    /// The current value (tracked), or `None` if the signal was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let s = cx.signal(1);
    /// assert_eq!(s.try_get(), Some(1));
    /// cx.dispose();
    /// assert_eq!(s.try_get(), None);
    /// ```
    pub fn try_get(&self) -> Option<T>
    where
        T: Clone,
    {
        read::try_get(self.h)
    }

    /// Whether the signal's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let s = cx.signal(1);
    /// assert!(s.is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }

    /// Replaces the value and notifies subscribers (always, even if equal).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let s = cx.signal(1);
    /// s.set(2);
    /// assert_eq!(s.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed, or if called inside [`with`](Signal::with) on the
    /// same signal.
    #[track_caller]
    pub fn set(&self, value: T) {
        write::set(self.h, value);
    }

    /// Replaces the value and notifies subscribers only if it differs from the current one.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let s = cx.signal(1);
    /// s.set_if_changed(1); // no notification
    /// s.set_if_changed(2);
    /// assert_eq!(s.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed.
    #[track_caller]
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        write::set_if_changed(self.h, value);
    }

    /// Mutates the value in place and notifies subscribers (always).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let v = cx.signal(vec![1]);
    /// v.update(|v| v.push(2));
    /// assert_eq!(v.get(), [1, 2]);
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed, or if `f` accesses the same signal.
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        write::update(self.h, f);
    }

    /// A read-only handle to the same value.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
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
    /// let cx = twine_reactive::create_root();
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
    /// let cx = twine_reactive::create_root();
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
    /// let cx = twine_reactive::create_root();
    /// let n = cx.signal(3);
    /// let even = n.map(|n| n % 2 == 0);
    /// assert!(!even.get());
    /// n.set(4);
    /// assert!(even.get());
    /// ```
    ///
    /// # Panics
    ///
    /// If the signal's scope was disposed.
    #[track_caller]
    pub fn map<U: PartialEq + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let s = *self;
        owner_scope(self.h, WHAT).memo(move || s.with(|v| f(v)))
    }
}

/// The scope owning node `h`; panics (use-after-dispose) if it is dead.
#[track_caller]
pub(crate) fn owner_scope<T>(h: NodeHandle<T>, what: &str) -> Scope {
    match with_runtime(|rt| rt.node_scope(h.key)) {
        Some(s) => Scope::from_key(s),
        None => h.disposed(what),
    }
}

/// A read-only view of a [`Signal`] (see [`Signal::read_only`], [`Signal::split`]).
///
/// ```
/// let cx = twine_reactive::create_root();
/// let (r, w) = cx.signal(1).split();
/// w.set(2);
/// assert_eq!(r.get(), 2);
/// ```
pub struct ReadSignal<T> {
    h: NodeHandle<T>,
}

handle_impls!(ReadSignal);

impl<T: 'static> ReadSignal<T> {
    /// The current value (tracked). Panics if the signal's scope was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// assert_eq!(cx.signal(1).read_only().get(), 1);
    /// ```
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        read::with(self.h, true, T::clone)
    }

    /// The current value without subscribing. Panics if the signal's scope was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// assert_eq!(cx.signal(1).read_only().get_untracked(), 1);
    /// ```
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
    /// let cx = twine_reactive::create_root();
    /// let r = cx.signal(vec![1, 2]).read_only();
    /// assert_eq!(r.with(Vec::len), 2);
    /// ```
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, true, f)
    }

    /// Like [`with`](ReadSignal::with), without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let r = cx.signal(vec![1, 2]).read_only();
    /// assert_eq!(r.with_untracked(Vec::len), 2);
    /// ```
    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        read::with(self.h, false, f)
    }

    /// The current value (tracked), or `None` if the signal was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let r = cx.signal(1).read_only();
    /// cx.dispose();
    /// assert_eq!(r.try_get(), None);
    /// ```
    pub fn try_get(&self) -> Option<T>
    where
        T: Clone,
    {
        read::try_get(self.h)
    }

    /// Whether the signal's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// assert!(cx.signal(1).read_only().is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }

    /// A [`Memo`] derived from this signal (see [`Signal::map`]).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let r = cx.signal(2).read_only();
    /// assert_eq!(r.map(|n| n * 10).get(), 20);
    /// ```
    #[track_caller]
    pub fn map<U: PartialEq + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let s = *self;
        owner_scope(self.h, WHAT).memo(move || s.with(|v| f(v)))
    }
}

/// A write-only view of a [`Signal`] (see [`Signal::write_only`], [`Signal::split`]).
///
/// ```
/// let cx = twine_reactive::create_root();
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
    /// let cx = twine_reactive::create_root();
    /// let (r, w) = cx.signal(1).split();
    /// w.set(9);
    /// assert_eq!(r.get(), 9);
    /// ```
    #[track_caller]
    pub fn set(&self, value: T) {
        write::set(self.h, value);
    }

    /// Replaces the value and notifies only if it changed (see [`Signal::set_if_changed`]).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let (r, w) = cx.signal(1).split();
    /// w.set_if_changed(2);
    /// assert_eq!(r.get(), 2);
    /// ```
    #[track_caller]
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        write::set_if_changed(self.h, value);
    }

    /// Mutates the value in place and notifies (see [`Signal::update`]).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let (r, w) = cx.signal(1).split();
    /// w.update(|n| *n *= 3);
    /// assert_eq!(r.get(), 3);
    /// ```
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        write::update(self.h, f);
    }

    /// Whether the signal's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// assert!(cx.signal(1).write_only().is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }
}
