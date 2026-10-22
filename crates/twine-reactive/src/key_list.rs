//! [`KeyList`]: a 16-byte set of node keys (3 inline, then a boxed `Vec`).
//!
//! Every reactive node carries its subscriber list, so its size dominates the per-signal memory
//! budget (≤ 96 bytes per `u32` signal). `twine_core::SmallVec<_, 3>` is 24 bytes;
//! this list is 16 and still stores the common case (≤ 3 subscribers) without a heap allocation.
//!
//! In the safety profile a node key is 8 bytes (48-bit generations, R5.S04), so the list is 32
//! bytes on 64-bit hosts (28 on 32-bit targets) for the same three inline keys.

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::Partition;
use twine_core::alloc_guard::AllocSite;

use crate::runtime::NodeKey;

const INLINE: usize = 3;

/// An insertion-ordered list of distinct [`NodeKey`]s.
#[derive(Debug, Clone)]
pub(crate) enum KeyList {
    /// Up to three keys stored inline.
    Inline { len: u8, buf: [NodeKey; INLINE] },
    /// Spilled to the heap; never returns to inline storage. Boxed so the list stays 16 bytes
    /// (a bare `Vec` is 24).
    #[allow(clippy::box_collection)]
    Heap(Box<Vec<NodeKey>>),
}

impl Default for KeyList {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyList {
    /// An empty list.
    pub(crate) const fn new() -> Self {
        KeyList::Inline {
            len: 0,
            buf: [NodeKey::DANGLING; INLINE],
        }
    }

    /// The keys, in insertion order.
    pub(crate) fn as_slice(&self) -> &[NodeKey] {
        match self {
            // `len <= INLINE` (only `insert` and `remove` change it).
            KeyList::Inline { len, buf } => buf.get(..usize::from(*len)).unwrap_or(&[]),
            KeyList::Heap(v) => v,
        }
    }

    /// Heap bytes of a spilled list (the box and its vector), 0 inline.
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            KeyList::Inline { .. } => 0,
            KeyList::Heap(v) => core::mem::size_of::<NodeKey>()
                .saturating_mul(v.capacity())
                .saturating_add(core::mem::size_of::<Vec<NodeKey>>()),
        }
    }

    /// Number of keys.
    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Whether `k` is in the list.
    pub(crate) fn contains(&self, k: NodeKey) -> bool {
        self.as_slice().contains(&k)
    }

    /// Appends `k` unless it is already present; a spill or growth after a seal is noted in
    /// partition `part`'s seal domain (the list's runtime).
    pub(crate) fn insert(&mut self, part: Partition, k: NodeKey) {
        if self.contains(k) {
            return;
        }
        match self {
            KeyList::Inline { len, buf } => {
                if let Some(free) = buf.get_mut(usize::from(*len)) {
                    *free = k;
                    // `len < INLINE` here.
                    *len = len.saturating_add(1);
                } else {
                    twine_core::alloc_guard::grew(part, AllocSite::ReactiveEdges);
                    let mut v = Vec::with_capacity(INLINE * 2);
                    v.extend_from_slice(buf);
                    v.push(k);
                    *self = KeyList::Heap(Box::new(v));
                }
            }
            KeyList::Heap(v) => twine_core::alloc_guard::push(part, v, k, AllocSite::ReactiveEdges),
        }
    }

    /// Removes `k` (keeping the order of the others); no-op when absent.
    pub(crate) fn remove(&mut self, k: NodeKey) {
        match self {
            KeyList::Inline { len, buf } => {
                let Some(live) = buf.get_mut(..usize::from(*len)) else {
                    return;
                };
                if let Some(i) = live.iter().position(|&x| x == k) {
                    // Shift the keys after `i` down by one (at most two moves: a plain loop,
                    // not a general rotation) and clear the freed last entry.
                    let mut rest = live.iter_mut().skip(i);
                    if let Some(mut hole) = rest.next() {
                        for next in rest {
                            *hole = *next;
                            hole = next;
                        }
                        *hole = NodeKey::DANGLING;
                    }
                    *len = len.saturating_sub(1);
                }
            }
            KeyList::Heap(v) => {
                if let Some(i) = v.iter().position(|&x| x == k) {
                    v.remove(i);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(i: u16) -> NodeKey {
        NodeKey::from_raw((1 << 16) | twine_core::RawId::from(i))
    }

    #[test]
    fn key_list_is_16_bytes() {
        // Three keys inline: 4-byte keys by default, 8-byte keys in the safety profile.
        let expected = if cfg!(feature = "safety") { 32 } else { 16 };
        assert_eq!(core::mem::size_of::<KeyList>(), expected);
    }

    #[test]
    fn insert_dedupes_and_spills_in_order() {
        let mut l = KeyList::new();
        for i in 0..5 {
            l.insert(Partition::MAIN, k(i));
            l.insert(Partition::MAIN, k(i));
        }
        assert_eq!(l.len(), 5);
        assert!(matches!(l, KeyList::Heap(_)));
        let want: Vec<_> = (0..5).map(k).collect();
        assert_eq!(l.as_slice(), &want[..]);
    }

    #[test]
    fn remove_keeps_order_inline_and_heap() {
        let mut l = KeyList::new();
        for i in 0..3 {
            l.insert(Partition::MAIN, k(i));
        }
        l.remove(k(0));
        l.remove(k(9));
        assert_eq!(l.as_slice(), &[k(1), k(2)]);
        for i in 3..6 {
            l.insert(Partition::MAIN, k(i));
        }
        l.remove(k(3));
        assert_eq!(l.as_slice(), &[k(1), k(2), k(4), k(5)]);
        assert!(!l.contains(k(3)));
    }
}
