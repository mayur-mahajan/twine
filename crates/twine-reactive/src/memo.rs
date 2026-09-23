//! [`Memo`]: lazily computed, cached, equality-checked derived values (design 03 §3).

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::global::with_runtime;
use crate::handles::{NodeHandle, type_mismatch, with_value};
use crate::signal::{handle_impls, owner_scope};

const WHAT: &str = "memo";

/// A derived value computed from signals and other memos (created with
/// [`Scope::memo`](crate::Scope::memo) or `map`).
///
/// - **Lazy**: the closure first runs when the memo is read, not when it is created.
/// - **Cached**: reads return the stored value until a dependency changes.
/// - **Glitch-free**: before recomputing, the memo brings its own sources up to date, so it
///   never observes a mix of old and new values.
/// - **Short-circuit**: if the new value equals the old one (`PartialEq`), dependents are not
///   re-run.
///
/// ```
/// use std::{cell::Cell, rc::Rc};
/// let cx = twine_reactive::create_root();
/// let n = cx.signal(1);
/// let runs = Rc::new(Cell::new(0));
/// let r = runs.clone();
/// let parity = cx.memo(move || {
///     r.set(r.get() + 1);
///     n.get() % 2
/// });
/// assert_eq!(runs.get(), 0); // lazy
/// assert_eq!(parity.get(), 1);
/// assert_eq!(parity.get(), 1);
/// assert_eq!(runs.get(), 1); // cached
/// ```
pub struct Memo<T> {
    h: NodeHandle<T>,
}

handle_impls!(Memo);

impl<T: 'static> Memo<T> {
    pub(crate) fn from_handle(h: NodeHandle<T>) -> Self {
        Memo { h }
    }

    /// Brings the memo up to date, then borrows its value.
    #[track_caller]
    fn read<R>(self, track: bool, f: impl FnOnce(&T) -> R) -> R {
        let Some(rc) = with_runtime(|rt| {
            rt.update_if_necessary(self.h.key, &mut ());
            rt.read_node(self.h.key, track)
        }) else {
            self.h.disposed(WHAT)
        };
        with_value::<Option<T>, R>(&rc, |v| match v {
            Some(v) => f(v),
            None => type_mismatch(),
        })
    }

    /// The current value (tracked).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let m = cx.memo(|| 40 + 2);
    /// assert_eq!(m.get(), 42);
    /// ```
    ///
    /// # Panics
    ///
    /// If the memo's scope was disposed, or on a dependency cycle (the memo reads itself).
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.read(true, T::clone)
    }

    /// The current value without subscribing (still recomputes if stale).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let m = cx.memo(|| 1);
    /// assert_eq!(m.get_untracked(), 1);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Memo::get).
    #[track_caller]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        self.read(false, T::clone)
    }

    /// Calls `f` with a reference to the current value (tracked).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let m = cx.memo(|| vec![1, 2, 3]);
    /// assert_eq!(m.with(Vec::len), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Memo::get).
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.read(true, f)
    }

    /// Like [`with`](Memo::with), without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let m = cx.memo(|| String::from("hi"));
    /// assert_eq!(m.with_untracked(String::len), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Memo::get).
    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.read(false, f)
    }

    /// The current value (tracked), or `None` if the memo was disposed.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let m = cx.memo(|| 7);
    /// assert_eq!(m.try_get(), Some(7));
    /// cx.dispose();
    /// assert_eq!(m.try_get(), None);
    /// ```
    pub fn try_get(&self) -> Option<T>
    where
        T: Clone,
    {
        if self.is_alive() {
            Some(self.read(true, T::clone))
        } else {
            None
        }
    }

    /// Whether the memo's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// assert!(cx.memo(|| 1).is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }

    /// A memo derived from this one, owned by this memo's scope.
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let a = cx.signal(3);
    /// let sq = cx.memo(move || a.get() * a.get());
    /// let label = sq.map(|v| format!("{v}"));
    /// assert_eq!(label.get(), "9");
    /// ```
    ///
    /// # Panics
    ///
    /// If the memo's scope was disposed.
    #[track_caller]
    pub fn map<U: PartialEq + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let m = *self;
        owner_scope(self.h, WHAT).memo(move || m.with(|v| f(v)))
    }
}
