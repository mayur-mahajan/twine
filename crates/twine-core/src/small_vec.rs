//! [`SmallVec`]: an inline-first vector for small `Copy` elements.

use alloc::vec::Vec;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut};

#[derive(Clone)]
enum Repr<T: Copy + Default, const N: usize> {
    Inline { len: u8, buf: [T; N] },
    Heap(Vec<T>),
}

/// A vector storing up to `N` (≤ 255) elements inline, spilling to the heap when the
/// `N + 1`-th element is pushed. Once spilled it never returns to inline storage (clearing
/// keeps the heap buffer for reuse).
///
/// Out-of-range [`insert`](SmallVec::insert)/[`remove`](SmallVec::remove) are no-ops that log a
/// warning (`remove` then returns `T::default()`), never panics (P7).
///
/// ```
/// use twine_core::SmallVec;
/// let mut v: SmallVec<u16, 2> = SmallVec::new();
/// v.push(1);
/// v.push(2);
/// assert!(!v.spilled());
/// v.push(3);
/// assert!(v.spilled());
/// assert_eq!(v.as_slice(), &[1, 2, 3]);
/// assert_eq!(v.remove(9), 0); // out of range: no-op
/// ```
#[derive(Clone)]
pub struct SmallVec<T: Copy + Default, const N: usize> {
    repr: Repr<T, N>,
}

impl<T: Copy + Default, const N: usize> SmallVec<T, N> {
    const CAPACITY_OK: () = assert!(N <= 255, "SmallVec: N must be <= 255");

    /// An empty vector (no allocation).
    #[must_use]
    pub fn new() -> Self {
        let () = Self::CAPACITY_OK;
        SmallVec {
            repr: Repr::Inline {
                len: 0,
                buf: [T::default(); N],
            },
        }
    }

    /// Number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.repr {
            Repr::Inline { len, .. } => usize::from(*len),
            Repr::Heap(v) => v.len(),
        }
    }

    /// Whether there are no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the elements live on the heap.
    #[must_use]
    pub fn spilled(&self) -> bool {
        matches!(self.repr, Repr::Heap(_))
    }

    /// The elements as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        match &self.repr {
            Repr::Inline { len, buf } => &buf[..usize::from(*len)],
            Repr::Heap(v) => v,
        }
    }

    /// The elements as a mutable slice.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        match &mut self.repr {
            Repr::Inline { len, buf } => &mut buf[..usize::from(*len)],
            Repr::Heap(v) => v,
        }
    }

    /// Iterates over the elements.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// A heap copy of the elements with spare room (used when spilling).
    fn to_heap(&self) -> Vec<T> {
        let mut v = Vec::with_capacity((N * 2).max(4));
        v.extend_from_slice(self.as_slice());
        v
    }

    /// Appends an element.
    pub fn push(&mut self, v: T) {
        match &mut self.repr {
            Repr::Inline { len, buf } if usize::from(*len) < N => {
                buf[usize::from(*len)] = v;
                *len += 1;
            }
            Repr::Inline { .. } => {
                let mut h = self.to_heap();
                h.push(v);
                self.repr = Repr::Heap(h);
            }
            Repr::Heap(h) => h.push(v),
        }
    }

    /// Removes and returns the last element.
    pub fn pop(&mut self) -> Option<T> {
        match &mut self.repr {
            Repr::Inline { len, buf } => {
                if *len == 0 {
                    None
                } else {
                    *len -= 1;
                    Some(buf[usize::from(*len)])
                }
            }
            Repr::Heap(h) => h.pop(),
        }
    }

    /// Inserts `v` at `index` (`index <= len`), shifting later elements. Out of range: no-op
    /// with a warning.
    pub fn insert(&mut self, index: usize, v: T) {
        let n = self.len();
        if index > n {
            crate::warn!(target: "twine::core", "SmallVec::insert: index {} > len {}", index, n);
            return;
        }
        match &mut self.repr {
            Repr::Inline { len, buf } if n < N => {
                buf.copy_within(index..n, index + 1);
                buf[index] = v;
                *len += 1;
            }
            Repr::Inline { .. } => {
                let mut h = self.to_heap();
                h.insert(index, v);
                self.repr = Repr::Heap(h);
            }
            Repr::Heap(h) => h.insert(index, v),
        }
    }

    /// Removes the element at `index`, preserving order. Out of range: returns `T::default()`
    /// and logs a warning.
    pub fn remove(&mut self, index: usize) -> T {
        let n = self.len();
        if index >= n {
            crate::warn!(target: "twine::core", "SmallVec::remove: index {} >= len {}", index, n);
            return T::default();
        }
        match &mut self.repr {
            Repr::Inline { len, buf } => {
                let v = buf[index];
                buf.copy_within(index + 1..n, index);
                *len -= 1;
                v
            }
            Repr::Heap(h) => h.remove(index),
        }
    }

    /// Removes the element at `index`, replacing it with the last one (O(1), order not
    /// preserved). Out of range: returns `T::default()` and logs a warning.
    pub fn swap_remove(&mut self, index: usize) -> T {
        let n = self.len();
        if index >= n {
            crate::warn!(target: "twine::core", "SmallVec::swap_remove: index {} >= len {}", index, n);
            return T::default();
        }
        match &mut self.repr {
            Repr::Inline { len, buf } => {
                let v = buf[index];
                buf[index] = buf[n - 1];
                *len -= 1;
                v
            }
            Repr::Heap(h) => h.swap_remove(index),
        }
    }

    /// Keeps only the elements for which `f` returns `true`, preserving order.
    pub fn retain(&mut self, mut f: impl FnMut(&T) -> bool) {
        match &mut self.repr {
            Repr::Inline { len, buf } => {
                let mut w = 0;
                for r in 0..usize::from(*len) {
                    if f(&buf[r]) {
                        buf[w] = buf[r];
                        w += 1;
                    }
                }
                *len = w as u8;
            }
            Repr::Heap(h) => h.retain(|v| f(v)),
        }
    }

    /// Removes every element (a spilled vector keeps its heap buffer).
    pub fn clear(&mut self) {
        match &mut self.repr {
            Repr::Inline { len, .. } => *len = 0,
            Repr::Heap(h) => h.clear(),
        }
    }

    /// Whether `v` is an element.
    #[must_use]
    pub fn contains(&self, v: &T) -> bool
    where
        T: PartialEq,
    {
        self.as_slice().contains(v)
    }
}

impl<T: Copy + Default, const N: usize> Default for SmallVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Default, const N: usize> Deref for SmallVec<T, N> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: Copy + Default, const N: usize> DerefMut for SmallVec<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T: Copy + Default, const N: usize> Extend<T> for SmallVec<T, N> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for v in iter {
            self.push(v);
        }
    }
}

impl<T: Copy + Default, const N: usize> FromIterator<T> for SmallVec<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut v = Self::new();
        v.extend(iter);
        v
    }
}

impl<'a, T: Copy + Default, const N: usize> IntoIterator for &'a SmallVec<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: Copy + Default, const N: usize> IntoIterator for &'a mut SmallVec<T, N> {
    type Item = &'a mut T;
    type IntoIter = core::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}

impl<T: Copy + Default + fmt::Debug, const N: usize> fmt::Debug for SmallVec<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Copy + Default + PartialEq, const N: usize> PartialEq for SmallVec<T, N> {
    /// Compares elements only (inline and spilled vectors with equal contents are equal).
    fn eq(&self, o: &Self) -> bool {
        self.as_slice() == o.as_slice()
    }
}

impl<T: Copy + Default + Eq, const N: usize> Eq for SmallVec<T, N> {}

impl<T: Copy + Default + Hash, const N: usize> Hash for SmallVec<T, N> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.as_slice().hash(h);
    }
}

#[cfg(feature = "defmt")]
impl<T: Copy + Default + defmt::Format, const N: usize> defmt::Format for SmallVec<T, N> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "{}", self.as_slice());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_spills_after_n() {
        let mut v: SmallVec<u8, 3> = SmallVec::new();
        for i in 0..3 {
            v.push(i);
            assert!(!v.spilled());
        }
        v.push(3);
        assert!(v.spilled());
        assert_eq!(&*v, &[0, 1, 2, 3]);
        v.clear();
        assert!(v.spilled() && v.is_empty());
        let mut z: SmallVec<u8, 0> = SmallVec::new();
        z.push(1);
        assert!(z.spilled());
        assert_eq!(z.pop(), Some(1));
        assert_eq!(z.pop(), None);
    }

    #[test]
    fn remove_preserves_order() {
        let mut v: SmallVec<i32, 8> = (0..6).collect();
        assert_eq!(v.remove(1), 1);
        assert_eq!(v.as_slice(), &[0, 2, 3, 4, 5]);
        assert_eq!(v.remove(10), 0);
        v.insert(0, 9);
        v.insert(6, 8);
        v.insert(100, 7);
        assert_eq!(v.as_slice(), &[9, 0, 2, 3, 4, 5, 8]);
        let mut h: SmallVec<i32, 2> = (0..6).collect();
        assert_eq!(h.remove(1), 1);
        h.insert(1, 11);
        assert_eq!(h.as_slice(), &[0, 11, 2, 3, 4, 5]);
        let mut full: SmallVec<i32, 2> = (0..2).collect();
        full.insert(1, 5);
        assert!(full.spilled());
        assert_eq!(full.as_slice(), &[0, 5, 1]);
    }

    #[test]
    fn swap_remove() {
        let mut v: SmallVec<i32, 8> = (0..5).collect();
        assert_eq!(v.swap_remove(1), 1);
        assert_eq!(v.as_slice(), &[0, 4, 2, 3]);
        assert_eq!(v.swap_remove(9), 0);
        let mut h: SmallVec<i32, 1> = (0..5).collect();
        assert_eq!(h.swap_remove(0), 0);
        assert_eq!(h.as_slice(), &[4, 1, 2, 3]);
    }

    #[test]
    fn retain_inline_and_heap() {
        let mut v: SmallVec<i32, 8> = (0..8).collect();
        v.retain(|x| x % 2 == 0);
        assert_eq!(v.as_slice(), &[0, 2, 4, 6]);
        assert!(!v.spilled());
        let mut h: SmallVec<i32, 2> = (0..8).collect();
        h.retain(|x| x % 3 == 0);
        assert_eq!(h.as_slice(), &[0, 3, 6]);
        assert!(h.contains(&3) && !h.contains(&4));
    }

    #[test]
    fn traits() {
        let a: SmallVec<i32, 4> = (0..3).collect();
        let mut b: SmallVec<i32, 4> = SmallVec::default();
        b.extend([0, 1, 2, 3, 4]);
        b.pop();
        b.pop();
        assert_eq!(a, b); // inline == spilled with the same contents
        assert_eq!(alloc::format!("{a:?}"), "[0, 1, 2]");
        let mut c = a.clone();
        for x in &mut c {
            *x += 1;
        }
        c[0] = 10;
        assert_eq!((&c).into_iter().copied().collect::<Vec<_>>(), [10, 2, 3]);
        assert_eq!(c.as_mut_slice().len(), 3);
    }
}
