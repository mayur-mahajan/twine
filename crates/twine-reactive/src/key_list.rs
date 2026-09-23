//! [`KeyList`]: a 16-byte set of node keys (3 inline, then a boxed `Vec`).
//!
//! Every reactive node carries its subscriber list, so its size dominates the per-signal memory
//! budget (design 03 §5: ≤ 96 bytes per `u32` signal). `twine_core::SmallVec<_, 3>` is 24 bytes;
//! this list is 16 and still stores the common case (≤ 3 subscribers) without a heap allocation.

use alloc::boxed::Box;
use alloc::vec::Vec;

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
            KeyList::Inline { len, buf } => &buf[..usize::from(*len)],
            KeyList::Heap(v) => v,
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

    /// Appends `k` unless it is already present.
    pub(crate) fn insert(&mut self, k: NodeKey) {
        if self.contains(k) {
            return;
        }
        match self {
            KeyList::Inline { len, buf } => {
                let n = usize::from(*len);
                if n < INLINE {
                    buf[n] = k;
                    *len += 1;
                } else {
                    let mut v = Vec::with_capacity(INLINE * 2);
                    v.extend_from_slice(buf);
                    v.push(k);
                    *self = KeyList::Heap(Box::new(v));
                }
            }
            KeyList::Heap(v) => v.push(k),
        }
    }

    /// Removes `k` (keeping the order of the others); no-op when absent.
    pub(crate) fn remove(&mut self, k: NodeKey) {
        match self {
            KeyList::Inline { len, buf } => {
                let n = usize::from(*len);
                if let Some(i) = buf[..n].iter().position(|&x| x == k) {
                    buf.copy_within(i + 1..n, i);
                    buf[n - 1] = NodeKey::DANGLING;
                    *len -= 1;
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

    fn k(i: u32) -> NodeKey {
        NodeKey::from_raw((1 << 16) | i)
    }

    #[test]
    fn key_list_is_16_bytes() {
        assert_eq!(core::mem::size_of::<KeyList>(), 16);
    }

    #[test]
    fn insert_dedupes_and_spills_in_order() {
        let mut l = KeyList::new();
        for i in 0..5 {
            l.insert(k(i));
            l.insert(k(i));
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
            l.insert(k(i));
        }
        l.remove(k(0));
        l.remove(k(9));
        assert_eq!(l.as_slice(), &[k(1), k(2)]);
        for i in 3..6 {
            l.insert(k(i));
        }
        l.remove(k(3));
        assert_eq!(l.as_slice(), &[k(1), k(2), k(4), k(5)]);
        assert!(!l.contains(k(3)));
    }
}
