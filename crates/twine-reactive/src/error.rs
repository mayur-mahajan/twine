//! [`ReactiveError`]: why an operation on a reactive handle failed, and the out-of-line failure
//! paths shared by the panicking forms.
//!
//! Every fallible operation exists twice (see the crate docs § Failures):
//!
//! - a `try_*` form returning `Result<_, ReactiveError>`, which never panics and raises no
//!   fault (the caller decides);
//! - the usual form (`get`, `set`, `signal`, …), a thin `#[track_caller]` wrapper over the same
//!   code whose `Err` arm calls one of the `#[cold] #[inline(never)]` functions below, so the
//!   success path costs what it did before (one `match` on the result).

use core::fmt;
use core::panic::Location;

use twine_core::fault::FaultKind;

/// The kind of reactive handle a [`ReactiveError`] is about.
///
/// ```
/// use twine_reactive::{HandleKind, ReactiveError};
/// assert_eq!(HandleKind::StoredValue.name(), "stored value");
/// assert_eq!(
///     ReactiveError::Disposed(HandleKind::Signal).to_string(),
///     "signal used after its scope was disposed"
/// );
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum HandleKind {
    /// A [`Signal`](crate::Signal), [`ReadSignal`](crate::ReadSignal) or
    /// [`WriteSignal`](crate::WriteSignal).
    Signal,
    /// A [`Memo`](crate::Memo).
    Memo,
    /// A [`StoredValue`](crate::StoredValue).
    StoredValue,
    /// An effect ([`EffectId`](crate::EffectId)).
    Effect,
    /// A [`Scope`](crate::Scope).
    Scope,
}

impl HandleKind {
    /// The kind's name as messages spell it (`"signal"`, `"memo"`, `"stored value"`,
    /// `"effect"`, `"scope"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            HandleKind::Signal => "signal",
            HandleKind::Memo => "memo",
            HandleKind::StoredValue => "stored value",
            HandleKind::Effect => "effect",
            HandleKind::Scope => "scope",
        }
    }
}

impl fmt::Display for HandleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why an operation on a reactive handle failed: the error of the `try_*` forms
/// ([`Signal::try_set`](crate::Signal::try_set), [`Memo::try_get`](crate::Memo::try_get),
/// [`Scope::try_signal`](crate::Scope::try_signal), …).
///
/// Every variant is a programming error in the application (a handle kept beyond the life of
/// its scope, a write inside a read of the same value, a memo that reads itself) or an
/// exhausted fixed capacity — never a consequence of user input. The usual, panicking forms
/// panic with this error's message; with the `no-panic-runtime` feature (implied by the
/// `safety` profile) the operations that can be skipped are skipped instead, after a `warn!`
/// and the fault [`fault_kind`](Self::fault_kind) names (see the crate docs § Failures).
///
/// ```
/// use twine_reactive::{HandleKind, ReactiveError, Runtime};
///
/// let cx = Runtime::take().unwrap().create_root();
/// let s = cx.signal(1);
/// cx.dispose();
/// assert_eq!(s.try_set(2), Err(ReactiveError::Disposed(HandleKind::Signal)));
/// assert_eq!(cx.try_signal(0).err(), Some(ReactiveError::Disposed(HandleKind::Scope)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum ReactiveError {
    /// The handle's scope was disposed (for [`HandleKind::Scope`]: the scope itself, e.g. when
    /// creating a node in it). Handles returned by a creation that was skipped under
    /// `no-panic-runtime` are never alive and fail the same way.
    Disposed(HandleKind),
    /// The value is borrowed by an enclosing access of the same value: a write inside `with`,
    /// or any access inside `update` / `with_mut`, on the same handle.
    Borrowed(HandleKind),
    /// The runtime's bound is reached: [`Limits::max_scopes`](twine_core::Limits::max_scopes)
    /// live scopes ([`HandleKind::Scope`]) or
    /// [`Limits::max_reactive_nodes`](twine_core::Limits::max_reactive_nodes) live nodes
    /// (signals, memos, stored values and effects together); both at most 65 535 (see
    /// [`Runtime::set_limits`](crate::Runtime::set_limits)).
    Capacity(HandleKind),
    /// A memo read itself, directly or through other memos (a dependency cycle).
    Cycle,
}

impl ReactiveError {
    /// The fault the `no-panic-runtime` feature raises when it skips an operation that failed
    /// with this error: [`FaultKind::DisposedHandle`], [`FaultKind::ReentrantAccess`],
    /// [`FaultKind::Capacity`] or [`FaultKind::MemoCycle`].
    ///
    /// ```
    /// use twine_reactive::{FaultKind, HandleKind, ReactiveError};
    /// assert_eq!(ReactiveError::Cycle.fault_kind(), FaultKind::MemoCycle);
    /// assert_eq!(ReactiveError::Capacity(HandleKind::Scope).fault_kind(), FaultKind::Capacity);
    /// ```
    #[must_use]
    pub const fn fault_kind(self) -> FaultKind {
        match self {
            ReactiveError::Disposed(_) => FaultKind::DisposedHandle,
            ReactiveError::Borrowed(_) => FaultKind::ReentrantAccess,
            ReactiveError::Capacity(_) => FaultKind::Capacity,
            ReactiveError::Cycle => FaultKind::MemoCycle,
        }
    }

    /// Logs the error (`warn!`, with the caller's location) and records its fault in the
    /// runtime (counted, reported to the fault hook, kept for `take_faults`). Called without
    /// any borrow of the runtime held (rule R1: the hook is foreign code).
    #[cfg(feature = "no-panic-runtime")]
    #[cold]
    #[inline(never)]
    fn report(self, at: &'static Location<'static>) {
        twine_core::log::warn!(
            target: "twine::reactive",
            "{} at {}:{}",
            self,
            at.file(),
            at.line()
        );
        crate::global::with_runtime(|rt| match self {
            ReactiveError::Capacity(HandleKind::Scope) => {
                rt.record_capacity(twine_core::fault::CapacityFault::ReactiveScopes, 1);
            }
            ReactiveError::Capacity(_) => {
                rt.record_capacity(twine_core::fault::CapacityFault::ReactiveNodes, 1);
            }
            _ => rt.record_fault(self.fault_kind(), 1),
        });
    }

    /// The failure path of an operation that can be skipped (a write, a creation): with
    /// `no-panic-runtime`, reports the error ([`report`](Self::report)) and returns, so the
    /// caller skips the operation; otherwise panics with the error's message.
    /// `created` is where the handle was created (debug builds), for the message.
    #[cold]
    #[inline(never)]
    #[track_caller]
    pub(crate) fn skip(self, created: Option<&'static Location<'static>>) {
        #[cfg(feature = "no-panic-runtime")]
        {
            let _ = created;
            self.report(Location::caller());
        }
        #[cfg(not(feature = "no-panic-runtime"))]
        self.fail(created);
    }

    /// The failure path of an operation that has no value to continue with (a read of a
    /// disposed or borrowed value, a memo's first computation reading itself): with
    /// `no-panic-runtime`, reports the error first (so the application's fault hook runs
    /// before its panic handler); then panics with the error's message.
    ///
    /// This is the reactive runtime's one panic site in every profile (see
    /// `docs/safety/panics.md`): a read must return a `T`, and no `T` exists — returning
    /// another value would show wrong data silently. Code that must not panic uses the
    /// `try_*` forms.
    #[cold]
    #[inline(never)]
    #[track_caller]
    pub(crate) fn fail(self, created: Option<&'static Location<'static>>) -> ! {
        #[cfg(feature = "no-panic-runtime")]
        self.report(Location::caller());
        // PANIC: a read must return a value and none exists (disposed handle, borrowed value,
        // cycle on a memo's first computation); inventing one would show wrong data
        // silently. A programming error, never input data; `try_*` forms avoid it.
        panic!("twine-reactive: {self}{}", CreatedAt(created))
    }
}

/// ` (created at file:line:col)` when the creation location is known (debug builds), nothing
/// otherwise: the suffix of [`ReactiveError::fail`]'s message.
struct CreatedAt(Option<&'static Location<'static>>);

impl fmt::Display for CreatedAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(at) => write!(f, " (created at {at})"),
            None => Ok(()),
        }
    }
}

impl fmt::Display for ReactiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReactiveError::Disposed(HandleKind::Scope) => f.write_str("scope used after it was disposed"),
            ReactiveError::Disposed(k) => write!(f, "{k} used after its scope was disposed"),
            ReactiveError::Borrowed(k) => {
                write!(
                    f,
                    "{k} already borrowed (accessed inside an access of the same value)"
                )
            }
            ReactiveError::Capacity(HandleKind::Scope) => {
                f.write_str("too many reactive scopes (Limits::max_scopes)")
            }
            ReactiveError::Capacity(_) => f.write_str("too many reactive nodes (Limits::max_reactive_nodes)"),
            ReactiveError::Cycle => {
                f.write_str("cycle detected (a memo reads itself, directly or indirectly)")
            }
        }
    }
}

impl core::error::Error for ReactiveError {}
