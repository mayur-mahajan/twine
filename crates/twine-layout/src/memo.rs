//! The measurement memo of one layout call: the size [`resolve_node`](crate::size::resolve_node)
//! found for a node whose size depends on its content, reused instead of measuring the node's
//! subtree again.
//!
//! Without it, measuring nested content-sized nodes is exponential in their nesting: a
//! content-sized node measures its children once per axis, each child measures its own
//! children once per axis, and so on (×2 per level), and every level of the arrangement
//! measures its subtree again. With it, each node is measured a bounded number of times per
//! layout call (linear time).
//!
//! **Exactness.** A node's resolution reads styles (constant during a layout call), the
//! `parent` size it is given (only through `Pct` lengths: recorded per entry), the `ovr`
//! sizes forced by a flex/grid parent (part of the key) and the widgets' content sizes, which
//! may depend on the sizes of the node's own and its parent's content areas
//! ([`LayoutTree::content_size`]). Rectangles change only when the layout places a node, and
//! [`place`](crate::tree::place) forgets the entries of the placed node and of its children,
//! so a memo hit returns exactly what measuring again would. Entries live for one
//! [`layout_subtree_with`](crate::layout_subtree_with) / [`layout_children_with`](crate::layout_children_with)
//! call.
//!
//! **Storage.** Indexed by [`LayoutTree::slot`]: 8 bytes per slot up to the largest slot
//! measured, plus one entry per content-measured node; kept (and reused) by
//! [`LayoutScratch`](crate::LayoutScratch), cleared in O(1) per call.

use alloc::vec::Vec;

use twine_core::Size;

use crate::size::Resolved;

/// One remembered resolution.
#[derive(Clone, Copy)]
struct Val {
    /// The sizes forced by the parent's layout.
    ovr: [Option<i32>; 2],
    /// The parent content size it was resolved in, when the result depends on it (a `Pct`
    /// length); `None`: valid for any parent size.
    parent: Option<Size>,
    r: Resolved,
}

/// The remembered resolutions of one node: a content-sized node is typically resolved with
/// two keys (its parent measures it once per axis), so two are kept.
#[derive(Clone, Copy)]
struct Entry {
    vals: [Option<Val>; 2],
    /// The value a third key replaces.
    next: u8,
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct Memo {
    /// The current layout call (entries of other calls are stale). Never `0`, the epoch of a
    /// slot that was never used.
    epoch: u32,
    /// Per slot: `(epoch, entry index)`.
    slots: Vec<(u32, u32)>,
    entries: Vec<Entry>,
    /// Content measurements made (for tests: the cost of a layout call).
    measured: u32,
}

impl core::fmt::Debug for Entry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Entry").finish_non_exhaustive()
    }
}

impl Memo {
    pub(crate) const fn new() -> Self {
        Self {
            epoch: 1,
            slots: Vec::new(),
            entries: Vec::new(),
            measured: 0,
        }
    }

    /// Starts a layout call: every entry becomes stale (O(1)).
    pub(crate) fn begin(&mut self) {
        self.entries.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            // Wrapped: stale slots could carry the new epoch.
            self.slots.iter_mut().for_each(|s| s.0 = 0);
            self.epoch = 1;
        }
    }

    /// Counts one content measurement (see [`measured`](Self::measured)).
    #[inline]
    pub(crate) fn count_measurement(&mut self) {
        self.measured = self.measured.wrapping_add(1);
    }

    /// Content measurements made since the scratch was created.
    pub(crate) const fn measured(&self) -> u32 {
        self.measured
    }

    fn entry(&self, slot: u32) -> Option<usize> {
        let &(epoch, idx) = self.slots.get(usize::try_from(slot).ok()?)?;
        (epoch == self.epoch).then_some(idx as usize)
    }

    /// The remembered resolution of `slot` for `parent` and `ovr`.
    #[inline]
    pub(crate) fn get(&self, slot: u32, parent: Size, ovr: [Option<i32>; 2]) -> Option<Resolved> {
        if self.entries.is_empty() {
            return None;
        }
        let e = self.entries.get(self.entry(slot)?)?;
        e.vals
            .iter()
            .flatten()
            .find(|v| v.ovr == ovr && v.parent.is_none_or(|p| p == parent))
            .map(|v| v.r)
    }

    /// Remembers `r` as the resolution of `slot` for `ovr` (and `parent`, when
    /// `parent_dependent`).
    pub(crate) fn put(
        &mut self,
        slot: u32,
        parent: Size,
        parent_dependent: bool,
        ovr: [Option<i32>; 2],
        r: Resolved,
    ) {
        let Ok(s) = usize::try_from(slot) else {
            return;
        };
        let val = Val {
            ovr,
            parent: parent_dependent.then_some(parent),
            r,
        };
        if let Some(idx) = self.entry(slot) {
            if let Some(e) = self.entries.get_mut(idx) {
                let i = match e.vals {
                    [None, _] => 0,
                    [_, None] => 1,
                    _ => usize::from(e.next),
                };
                if let Some(v) = e.vals.get_mut(i) {
                    *v = Some(val);
                }
                e.next = u8::from(i == 0);
            }
            return;
        }
        if s >= self.slots.len() {
            self.slots.resize(s.saturating_add(1), (0, 0));
        }
        let Ok(idx) = u32::try_from(self.entries.len()) else {
            return;
        };
        self.entries.push(Entry {
            vals: [Some(val), None],
            next: 1,
        });
        if let Some(sl) = self.slots.get_mut(s) {
            *sl = (self.epoch, idx);
        }
    }

    /// Forgets what is remembered for `slot` (its rectangle, or its parent's, changed).
    #[inline]
    pub(crate) fn forget(&mut self, slot: u32) {
        if let Some(idx) = self.entry(slot) {
            if let Some(e) = self.entries.get_mut(idx) {
                e.vals = [None, None];
            }
        }
    }

    /// Whether nothing is remembered (no forgetting needed).
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
