//! The internal typed node handle shared by [`Signal`](crate::Signal),
//! [`ReadSignal`](crate::ReadSignal), [`WriteSignal`](crate::WriteSignal),
//! [`Memo`](crate::Memo) and [`EffectId`](crate::EffectId).

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::panic::Location;

use twine_core::log::error;

use crate::global::with_runtime;
use crate::runtime::{NodeKey, ValueRc};

/// A `Copy`, `!Send`, `!Sync` handle to a node holding a `T`.
///
/// In debug builds it also carries the location where the node was created, used in
/// use-after-dispose panics (the node itself is gone by then).
pub(crate) struct NodeHandle<T> {
    pub(crate) key: NodeKey,
    #[cfg(debug_assertions)]
    loc: &'static Location<'static>,
    _m: Marker<T>,
}

/// Covariant in `T`, `!Send`, `!Sync`.
type Marker<T> = PhantomData<(fn() -> T, *const ())>;

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

    /// Whether the node is still alive.
    pub(crate) fn is_alive(self) -> bool {
        with_runtime(|rt| rt.node_alive(self.key))
    }

    /// Panics with the use-after-dispose message (`what` = "signal", "memo", …).
    #[cold]
    #[inline(never)]
    #[track_caller]
    pub(crate) fn disposed(self, what: &str) -> ! {
        #[cfg(debug_assertions)]
        panic!(
            "{what} used after its scope was disposed (created at {})",
            self.loc
        );
        #[cfg(not(debug_assertions))]
        panic!("{what} used after its scope was disposed");
    }

    /// The value cell, tracking the read when `track`; panics if the node is dead.
    #[track_caller]
    pub(crate) fn value(self, track: bool, what: &str) -> ValueRc {
        match with_runtime(|rt| rt.read_node(self.key, track)) {
            Some(v) => v,
            None => self.disposed(what),
        }
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

/// Panics for a value cell holding an unexpected type (cannot happen through the public API).
#[cold]
#[inline(never)]
pub(crate) fn type_mismatch() -> ! {
    error!(target: "twine::reactive", "type mismatch (internal bug)");
    panic!("twine-reactive: type mismatch (internal bug)")
}

/// Borrows the value cell as a `V` and calls `f`.
pub(crate) fn with_value<V: 'static, R>(rc: &ValueRc, f: impl FnOnce(&V) -> R) -> R {
    let b = rc.borrow();
    match b.downcast_ref::<V>() {
        Some(v) => f(v),
        None => type_mismatch(),
    }
}

/// Mutably borrows the value cell as a `V` and calls `f`.
pub(crate) fn with_value_mut<V: 'static, R>(rc: &ValueRc, f: impl FnOnce(&mut V) -> R) -> R {
    let mut b = rc.borrow_mut();
    match b.downcast_mut::<V>() {
        Some(v) => f(v),
        None => type_mismatch(),
    }
}
