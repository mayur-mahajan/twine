//! The internal typed node handle shared by [`Signal`](crate::Signal),
//! [`ReadSignal`](crate::ReadSignal), [`WriteSignal`](crate::WriteSignal),
//! [`Memo`](crate::Memo), [`StoredValue`](crate::StoredValue) and [`EffectId`](crate::EffectId),
//! and the typed access to value cells.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::panic::Location;

use twine_core::Id;

use crate::error::{HandleKind, ReactiveError};
use crate::global::with_runtime;
use crate::runtime::{NodeKey, ValueRc};

/// A `Copy`, `!Send`, `!Sync` handle to a node holding a `T`.
///
/// In debug builds it also carries the location where the node was created, used in
/// use-after-dispose messages (the node itself is gone by then).
pub(crate) struct NodeHandle<T> {
    pub(crate) key: NodeKey,
    #[cfg(debug_assertions)]
    loc: &'static Location<'static>,
    _m: Marker<T>,
}

/// Covariant in `T`, `!Send`, `!Sync`.
type Marker<T> = PhantomData<(fn() -> T, *const ())>;

// Handle sizes (R5.S04): every public handle is a `NodeHandle`, which in release builds is
// just the node's arena id — 4 bytes by default, 8 in the safety profile (48-bit generations),
// where the id's niche keeps `Option<handle>` at 8 bytes too.
const _: () = {
    use core::mem::size_of;
    assert!(size_of::<crate::Signal<u32>>() == size_of::<NodeHandle<u32>>());
    assert!(size_of::<crate::ReadSignal<u32>>() == size_of::<NodeHandle<u32>>());
    assert!(size_of::<crate::WriteSignal<u32>>() == size_of::<NodeHandle<u32>>());
    assert!(size_of::<crate::Memo<u32>>() == size_of::<NodeHandle<u32>>());
    assert!(size_of::<crate::StoredValue<u32>>() == size_of::<NodeHandle<u32>>());
    assert!(size_of::<crate::EffectId>() == size_of::<NodeHandle<()>>());
};
#[cfg(not(debug_assertions))]
const _: () = {
    use core::mem::size_of;
    assert!(size_of::<NodeHandle<u32>>() == size_of::<NodeKey>());
    assert!(size_of::<NodeKey>() == if cfg!(feature = "safety") { 8 } else { 4 });
    assert!(size_of::<Option<crate::Signal<u32>>>() == if cfg!(feature = "safety") { 8 } else { 6 });
};

impl<T> NodeHandle<T> {
    /// A handle for `key`, created at `loc`.
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    pub(crate) fn new(key: NodeKey, loc: &'static Location<'static>) -> Self {
        NodeHandle {
            key,
            #[cfg(debug_assertions)]
            loc,
            _m: PhantomData,
        }
    }

    /// A handle that is never alive ([`Id::INVALID`]): what a creation skipped under
    /// `no-panic-runtime` returns, so every later use fails like a disposed handle.
    pub(crate) fn dead(loc: &'static Location<'static>) -> Self {
        NodeHandle::new(Id::INVALID, loc)
    }

    /// Whether the node is still alive.
    pub(crate) fn is_alive(self) -> bool {
        with_runtime(|rt| rt.node_alive(self.key))
    }

    /// The failure path of a read: see [`ReactiveError::fail`] (cold, out of line).
    #[cold]
    #[inline(never)]
    #[track_caller]
    pub(crate) fn fail(self, e: ReactiveError) -> ! {
        // Where the node was created: known in debug builds only.
        #[cfg(debug_assertions)]
        let created = Some(self.loc);
        #[cfg(not(debug_assertions))]
        let created = None;
        e.fail(created)
    }

    /// The failure path of a write: see [`ReactiveError::skip`] (cold, out of line).
    #[cold]
    #[inline(never)]
    #[track_caller]
    pub(crate) fn skip(self, e: ReactiveError) {
        #[cfg(debug_assertions)]
        let created = Some(self.loc);
        #[cfg(not(debug_assertions))]
        let created = None;
        e.skip(created);
    }

    /// The value cell, tracking the read when `track`; `Disposed(kind)` if the node is dead.
    #[inline]
    pub(crate) fn try_value(self, track: bool, kind: HandleKind) -> Result<ValueRc, ReactiveError> {
        with_runtime(|rt| rt.read_node(self.key, track)).ok_or(ReactiveError::Disposed(kind))
    }
}

impl<T> Clone for NodeHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for NodeHandle<T> {}

impl<T> PartialEq for NodeHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<T> Eq for NodeHandle<T> {}

impl<T> Hash for NodeHandle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl<T> fmt::Debug for NodeHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.key)
    }
}

/// Borrows the value cell as a `V` and calls `f`; `Borrowed(kind)` if the cell is mutably
/// borrowed (a read inside `update` / `with_mut` of the same value).
///
/// The cell always holds a `V` (handles are typed and keys are never reused for another
/// node); a mismatch would be an internal bug, checked in debug builds and reported as
/// `Disposed(kind)` (the handle does not denote a live value of its type).
#[inline]
pub(crate) fn with_value<V: 'static, R>(
    rc: &ValueRc,
    kind: HandleKind,
    f: impl FnOnce(&V) -> R,
) -> Result<R, ReactiveError> {
    let b = rc.try_borrow().map_err(|_| ReactiveError::Borrowed(kind))?;
    debug_assert!(
        b.is::<V>(),
        "twine-reactive: value cell of another type (internal bug)"
    );
    b.downcast_ref::<V>().map(f).ok_or(ReactiveError::Disposed(kind))
}

/// Mutably borrows the value cell as a `V` and calls `f`; `Borrowed(kind)` if the cell is
/// borrowed (any access inside `with` / `update` of the same value). See [`with_value`].
#[inline]
pub(crate) fn with_value_mut<V: 'static, R>(
    rc: &ValueRc,
    kind: HandleKind,
    f: impl FnOnce(&mut V) -> R,
) -> Result<R, ReactiveError> {
    let mut b = rc.try_borrow_mut().map_err(|_| ReactiveError::Borrowed(kind))?;
    debug_assert!(
        b.is::<V>(),
        "twine-reactive: value cell of another type (internal bug)"
    );
    b.downcast_mut::<V>().map(f).ok_or(ReactiveError::Disposed(kind))
}
