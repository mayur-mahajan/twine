//! [`Memo`]: lazily computed, cached, equality-checked derived values.

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::error::{HandleKind, ReactiveError};
use crate::global::with_runtime;
use crate::handles::{NodeHandle, with_value};
use crate::signal::{handle_impls, map_in_owner};

const KIND: HandleKind = HandleKind::Memo;

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
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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
///
/// **Failures** as for [`Signal`](crate::Signal) (see its § Failures): reading a memo whose
/// scope was disposed, or a memo that reads itself (a dependency cycle), fails. The `try_*`
/// forms return a [`ReactiveError`]; the others panic — except that with the
/// `no-panic-runtime` feature a cyclic read returns the memo's previous value, with a `warn!`
/// and a [`MemoCycle`](crate::FaultKind::MemoCycle) fault, when it has one.
pub struct Memo<T> {
    h: NodeHandle<T>,
}

handle_impls!(Memo);

impl<T: 'static> Memo<T> {
    pub(crate) fn from_handle(h: NodeHandle<T>) -> Self {
        Memo { h }
    }

    /// Brings the memo up to date, then calls `f` with its value.
    ///
    /// `stale_on_cycle` (the panicking forms under `no-panic-runtime`): when the memo is part
    /// of a cycle, use its previous value if it has one, reporting the cycle as a fault.
    #[inline]
    #[track_caller]
    fn read_with<R>(
        self,
        track: bool,
        stale_on_cycle: bool,
        f: impl FnOnce(&T) -> R,
    ) -> Result<R, ReactiveError> {
        let (up, rc) = with_runtime(|rt| {
            let up = rt.update_if_necessary(self.h.key, &mut ());
            (up, rt.read_node(self.h.key, track))
        });
        let rc = rc.ok_or(ReactiveError::Disposed(KIND))?;
        let stale = match up {
            Ok(()) => false,
            Err(_) if stale_on_cycle => true,
            Err(e) => return Err(e),
        };
        // `None`: the memo never finished a computation (a cycle on its first one).
        match with_value::<Option<T>, Option<R>>(&rc, KIND, |v| v.as_ref().map(f))? {
            Some(r) => {
                if stale {
                    self.h.skip(ReactiveError::Cycle);
                }
                Ok(r)
            }
            None => Err(ReactiveError::Cycle),
        }
    }

    /// The panicking form of [`read_with`](Self::read_with) (failure out of line).
    #[inline]
    #[track_caller]
    fn read<R>(self, track: bool, f: impl FnOnce(&T) -> R) -> R {
        match self.read_with(track, cfg!(feature = "no-panic-runtime"), f) {
            Ok(r) => r,
            Err(e) => self.h.fail(e),
        }
    }

    /// The current value (tracked).
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.memo(|| 40 + 2);
    /// assert_eq!(m.get(), 42);
    /// ```
    ///
    /// # Panics
    ///
    /// If the memo's scope was disposed, or on a dependency cycle (the memo reads itself) —
    /// with `no-panic-runtime`, a cycle returns the previous value (and raises
    /// [`MemoCycle`](crate::FaultKind::MemoCycle)) when there is one. Also when the value is
    /// mutably borrowed (cannot happen through this API: memo values are never borrowed
    /// mutably while user code runs). [`try_get`](Memo::try_get) returns the error instead.
    #[inline]
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
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.memo(|| 1);
    /// assert_eq!(m.get_untracked(), 1);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Memo::get).
    #[inline]
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
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.memo(|| vec![1, 2, 3]);
    /// assert_eq!(m.with(Vec::len), 3);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Memo::get).
    #[inline]
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.read(true, f)
    }

    /// Like [`with`](Memo::with), without subscribing.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.memo(|| String::from("hi"));
    /// assert_eq!(m.with_untracked(String::len), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// Like [`get`](Memo::get).
    #[inline]
    #[track_caller]
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.read(false, f)
    }

    /// The current value (tracked), or the reason there is none: the memo's scope was disposed
    /// ([`ReactiveError::Disposed`]) or the memo is part of a dependency cycle
    /// ([`ReactiveError::Cycle`]: it reads itself, directly or through other memos). Never
    /// panics (unless the memo's computation or `T::clone` does); raises no fault.
    ///
    /// ```
    /// use twine_reactive::{HandleKind, ReactiveError};
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.memo(|| 7);
    /// assert_eq!(m.try_get(), Ok(7));
    /// cx.dispose();
    /// assert_eq!(m.try_get(), Err(ReactiveError::Disposed(HandleKind::Memo)));
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Cycle`].
    #[inline]
    pub fn try_get(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.read_with(true, false, T::clone)
    }

    /// Calls `f` with a reference to the current value (tracked), or returns the reason it
    /// cannot (as [`try_get`](Memo::try_get); `f` is then not called). Raises no fault.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let m = cx.memo(|| vec![1, 2, 3]);
    /// assert_eq!(m.try_with(Vec::len), Ok(3));
    /// ```
    ///
    /// # Errors
    ///
    /// [`ReactiveError::Disposed`] or [`ReactiveError::Cycle`].
    #[inline]
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        self.read_with(true, false, f)
    }

    /// Whether the memo's scope is still alive.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// assert!(cx.memo(|| 1).is_alive());
    /// ```
    pub fn is_alive(&self) -> bool {
        self.h.is_alive()
    }

    /// A memo derived from this one, owned by this memo's scope.
    ///
    /// ```
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let a = cx.signal(3);
    /// let sq = cx.memo(move || a.get() * a.get());
    /// let label = sq.map(|v| format!("{v}"));
    /// assert_eq!(label.get(), "9");
    /// ```
    ///
    /// # Panics
    ///
    /// If the memo's scope was disposed, like [`Signal::map`](crate::Signal::map) (with
    /// `no-panic-runtime`: a memo that is never alive is returned, with a fault).
    #[track_caller]
    pub fn map<U: PartialEq + 'static>(&self, f: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let m = *self;
        map_in_owner(self.h, KIND, move || m.with(|v| f(v)))
    }
}
