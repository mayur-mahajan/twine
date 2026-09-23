//! [`EffectId`]: handle to an effect created with [`Scope::effect`](crate::Scope::effect).

use core::fmt;

use crate::global::with_runtime;
use crate::handles::NodeHandle;

/// Handle to an effect (`Copy`, `!Send`, `!Sync`).
///
/// Effects are disposed with their scope; [`dispose`](EffectId::dispose) stops one earlier.
///
/// ```
/// use std::{cell::Cell, rc::Rc};
/// let cx = twine_reactive::create_root();
/// let a = cx.signal(0);
/// let runs = Rc::new(Cell::new(0));
/// let r = runs.clone();
/// let e = cx.effect(move || {
///     a.get();
///     r.set(r.get() + 1);
/// });
/// e.dispose();
/// a.set(1);
/// assert_eq!(runs.get(), 1); // only the initial run
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectId {
    h: NodeHandle<()>,
}

impl fmt::Debug for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EffectId({:?})", self.h)
    }
}

impl EffectId {
    pub(crate) fn from_handle(h: NodeHandle<()>) -> Self {
        EffectId { h }
    }

    /// Stops the effect and releases its closure. Disposing twice is a no-op.
    pub fn dispose(self) {
        with_runtime(|rt| rt.dispose_node(self.h.key));
    }

    /// Whether the effect is still alive (not disposed, scope alive).
    ///
    /// ```
    /// let cx = twine_reactive::create_root();
    /// let e = cx.effect(|| {});
    /// assert!(e.is_alive());
    /// cx.dispose();
    /// assert!(!e.is_alive());
    /// ```
    pub fn is_alive(self) -> bool {
        self.h.is_alive()
    }
}
