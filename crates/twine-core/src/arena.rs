//! Generational arena ([`Arena<T>`]) with stale-handle detection ([`Id<T>`]).

use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use crate::Error;

/// A generational handle into an [`Arena<T>`]: a `u16` slot index plus a `u16` generation.
///
/// `Id`s are `Copy`, comparable and hashable regardless of `T`. A handle to a removed value
/// never resolves again, even after its slot is reused.
///
/// ```
/// use twine_core::{Arena, Id};
/// let mut a = Arena::new();
/// let id: Id<&str> = a.insert("x").unwrap();
/// assert_eq!(Id::<&str>::from_raw(id.to_raw()), id);
/// assert_eq!(id.to_string(), "#0v1");
/// ```
pub struct Id<T> {
    index: u16,
    gen_: u16,
    _m: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// A handle that never resolves (index 0, generation 0; generations start at 1).
    pub const DANGLING: Id<T> = Id::from_raw(0);

    const fn new(index: u16, gen_: u16) -> Self {
        Id {
            index,
            gen_,
            _m: PhantomData,
        }
    }

    /// Slot index.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.index
    }

    /// Generation of the slot when the handle was created (≥ 1 for real handles).
    #[must_use]
    pub const fn generation(self) -> u16 {
        self.gen_
    }

    /// Packs into `u32`: `generation << 16 | index`.
    #[must_use]
    pub const fn to_raw(self) -> u32 {
        ((self.gen_ as u32) << 16) | self.index as u32
    }

    /// Unpacks a value produced by [`Id::to_raw`].
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Id::new(raw as u16, (raw >> 16) as u16)
    }

    /// Reinterprets the handle for another element type.
    #[must_use]
    pub const fn cast<U>(self) -> Id<U> {
        Id::new(self.index, self.gen_)
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, o: &Self) -> bool {
        self.to_raw() == o.to_raw()
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl<T> Ord for Id<T> {
    /// Orders by index, then generation.
    fn cmp(&self, o: &Self) -> Ordering {
        (self.index, self.gen_).cmp(&(o.index, o.gen_))
    }
}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.to_raw().hash(h);
    }
}

impl<T> Default for Id<T> {
    /// [`Id::DANGLING`].
    fn default() -> Self {
        Self::DANGLING
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}v{}", self.index, self.gen_)
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}v{}", self.index, self.gen_)
    }
}

#[cfg(feature = "defmt")]
impl<T> defmt::Format for Id<T> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "#{}v{}", self.index, self.gen_);
    }
}

#[derive(Clone, Debug)]
struct Slot<T> {
    gen_: u16,
    value: Option<T>,
}

/// Storage for up to 65 535 live values, addressed by generational [`Id`]s.
///
/// Freed slots are reused (LIFO) with a bumped generation, so stale ids are detected. A slot
/// whose generation would wrap past `u16::MAX` is **retired** (never reused) to rule out ABA.
/// Insert and remove are O(1).
///
/// ```
/// use twine_core::Arena;
/// let mut a = Arena::new();
/// let x = a.insert(10).unwrap();
/// assert_eq!(a.get(x), Some(&10));
/// assert_eq!(a.remove(x), Some(10));
/// assert_eq!(a.get(x), None); // stale
/// let y = a.insert(20).unwrap();
/// assert_eq!(y.index(), x.index());
/// assert_ne!(y, x);
/// ```
#[derive(Clone, Debug)]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u16>,
    len: u16,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    /// Maximum number of live values and of slots.
    pub const MAX_LEN: usize = u16::MAX as usize;

    /// An empty arena (no allocation).
    #[must_use]
    pub const fn new() -> Self {
        Arena {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }

    /// An empty arena with room for `n` values.
    #[must_use]
    pub fn with_capacity(n: u16) -> Self {
        Arena {
            slots: Vec::with_capacity(usize::from(n)),
            free: Vec::new(),
            len: 0,
        }
    }

    /// Stores `v`, returning its handle; [`Error::CapacityExceeded`] when 65 535 values are
    /// live (or every slot is retired).
    pub fn insert(&mut self, v: T) -> Result<Id<T>, Error> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[usize::from(index)];
            debug_assert!(slot.value.is_none());
            slot.value = Some(v);
            self.len += 1;
            return Ok(Id::new(index, slot.gen_));
        }
        if self.slots.len() >= Self::MAX_LEN {
            crate::warn!(target: "twine::core", "Arena::insert: capacity exceeded ({} live)", self.len);
            return Err(Error::CapacityExceeded);
        }
        let index = self.slots.len() as u16;
        self.slots.push(Slot {
            gen_: 1,
            value: Some(v),
        });
        self.len += 1;
        Ok(Id::new(index, 1))
    }

    fn slot(&self, id: Id<T>) -> Option<&Slot<T>> {
        self.slots
            .get(usize::from(id.index))
            .filter(|s| s.gen_ == id.gen_ && s.value.is_some())
    }

    /// Removes and returns the value of `id`; `None` if the id is stale or unknown.
    pub fn remove(&mut self, id: Id<T>) -> Option<T> {
        let slot = self.slots.get_mut(usize::from(id.index))?;
        if slot.gen_ != id.gen_ {
            return None;
        }
        let v = slot.value.take()?;
        self.len -= 1;
        if slot.gen_ == u16::MAX {
            crate::debug!(target: "twine::core", "Arena: slot {} retired", id.index);
        } else {
            slot.gen_ += 1;
            self.free.push(id.index);
        }
        Some(v)
    }

    /// The value of `id`, if it is live.
    #[must_use]
    pub fn get(&self, id: Id<T>) -> Option<&T> {
        self.slot(id).and_then(|s| s.value.as_ref())
    }

    /// Mutable access to the value of `id`, if it is live.
    #[must_use]
    pub fn get_mut(&mut self, id: Id<T>) -> Option<&mut T> {
        self.slots
            .get_mut(usize::from(id.index))
            .filter(|s| s.gen_ == id.gen_)
            .and_then(|s| s.value.as_mut())
    }

    /// Mutable access to two values at once. If `a == b` the result is `(Some, None)`.
    pub fn get2_mut(&mut self, a: Id<T>, b: Id<T>) -> (Option<&mut T>, Option<&mut T>) {
        if a == b {
            return (self.get_mut(a), None);
        }
        let valid = |slots: &[Slot<T>], id: Id<T>| {
            slots
                .get(usize::from(id.index))
                .is_some_and(|s| s.gen_ == id.gen_ && s.value.is_some())
        };
        match (valid(&self.slots, a), valid(&self.slots, b)) {
            (false, false) => (None, None),
            (true, false) => (self.get_mut(a), None),
            (false, true) => (None, self.get_mut(b)),
            (true, true) => {
                // Both live, so their indices differ (one slot holds one value).
                let (ia, ib) = (usize::from(a.index), usize::from(b.index));
                let (lo, hi) = (ia.min(ib), ia.max(ib));
                let (left, right) = self.slots.split_at_mut(hi);
                let (slot_lo, slot_hi) = (&mut left[lo], &mut right[0]);
                let (sa, sb) = if ia < ib {
                    (slot_lo, slot_hi)
                } else {
                    (slot_hi, slot_lo)
                };
                (sa.value.as_mut(), sb.value.as_mut())
            }
        }
    }

    /// Whether `id` refers to a live value.
    #[must_use]
    pub fn contains(&self, id: Id<T>) -> bool {
        self.slot(id).is_some()
    }

    /// Number of live values.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// Whether no value is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Live values with their ids, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.value.as_ref().map(|v| (Id::new(i as u16, s.gen_), v)))
    }

    /// Live values with their ids, mutably, in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Id<T>, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(i, s)| s.value.as_mut().map(|v| (Id::new(i as u16, s.gen_), v)))
    }

    /// Removes every value. All existing ids become stale (slots are kept and their
    /// generations bumped, so no id can ever resolve to a later value).
    pub fn clear(&mut self) {
        for i in 0..self.slots.len() {
            let gen_ = self.slots[i].gen_;
            if self.slots[i].value.is_some() {
                self.remove(Id::new(i as u16, gen_));
            }
        }
        debug_assert_eq!(self.len, 0);
    }

    /// Number of slots allocated (live, free and retired).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_id_after_remove_is_none() {
        let mut a = Arena::new();
        let id = a.insert('a').unwrap();
        assert!(a.contains(id));
        assert_eq!(a.remove(id), Some('a'));
        assert_eq!(a.get(id), None);
        assert_eq!(a.get_mut(id), None);
        assert_eq!(a.remove(id), None);
        assert!(!a.contains(id));
        assert!(a.is_empty());
        assert_eq!(a.get(Id::DANGLING), None);
        assert_eq!(a.get(Id::from_raw(0xFFFF_FFFF)), None);
    }

    #[test]
    fn slot_reuse_bumps_generation() {
        let mut a = Arena::new();
        let x = a.insert(1).unwrap();
        let y = a.insert(2).unwrap();
        a.remove(x);
        a.remove(y);
        // LIFO reuse: y's slot first.
        let z = a.insert(3).unwrap();
        assert_eq!((z.index(), z.generation()), (y.index(), 2));
        let w = a.insert(4).unwrap();
        assert_eq!((w.index(), w.generation()), (x.index(), 2));
        assert_eq!(a.get(y), None);
        assert_eq!(a.get(z), Some(&3));
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn retired_slot_never_reused() {
        let mut a = Arena::new();
        let mut id = a.insert(0u8).unwrap();
        for _ in 0..65_534 {
            a.remove(id).unwrap();
            id = a.insert(0).unwrap();
            assert_eq!(id.index(), 0);
        }
        assert_eq!(id.generation(), u16::MAX);
        a.remove(id).unwrap();
        let next = a.insert(1).unwrap();
        assert_eq!(next.index(), 1, "retired slot 0 must not be reused");
        assert_eq!(a.get(id), None);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn capacity_error_at_limit() {
        let mut a = Arena::with_capacity(16);
        for _ in 0..65_535 {
            a.insert(()).unwrap();
        }
        assert_eq!(a.len(), 65_535);
        assert_eq!(a.insert(()), Err(Error::CapacityExceeded));
        let first = a.iter().next().unwrap().0;
        a.remove(first);
        assert!(a.insert(()).is_ok());
    }

    #[test]
    fn iter_skips_free_slots() {
        let mut a = Arena::new();
        let ids: alloc::vec::Vec<_> = (0..5).map(|i| a.insert(i).unwrap()).collect();
        a.remove(ids[1]);
        a.remove(ids[3]);
        let vals: alloc::vec::Vec<i32> = a.iter().map(|(_, v)| *v).collect();
        assert_eq!(vals, [0, 2, 4]);
        for (_, v) in a.iter_mut() {
            *v *= 10;
        }
        assert_eq!(a.get(ids[4]), Some(&40));
        assert_eq!(
            a.iter().map(|(id, _)| id).collect::<alloc::vec::Vec<_>>(),
            [ids[0], ids[2], ids[4]]
        );
        a.clear();
        assert!(a.is_empty());
        assert_eq!(a.get(ids[0]), None);
        let n = a.insert(9).unwrap();
        assert!(!ids.contains(&n));
        assert!(a.capacity() >= 5);
    }

    #[test]
    fn get2_mut_distinct() {
        let mut a = Arena::new();
        let x = a.insert(1).unwrap();
        let y = a.insert(2).unwrap();
        let (p, q) = a.get2_mut(x, y);
        core::mem::swap(p.unwrap(), q.unwrap());
        assert_eq!((a.get(x), a.get(y)), (Some(&2), Some(&1)));
        let (p, q) = a.get2_mut(y, x);
        assert_eq!((p.copied(), q.copied()), (Some(1), Some(2)));
        let (p, q) = a.get2_mut(x, x);
        assert_eq!((p.copied(), q), (Some(2), None));
        a.remove(x);
        let x2 = a.insert(7).unwrap();
        let (p, q) = a.get2_mut(x, x2);
        assert_eq!((p, q.copied()), (None, Some(7)));
        let (p, q) = a.get2_mut(x2, x);
        assert_eq!((p.copied(), q), (Some(7), None));
        let (p, q) = a.get2_mut(Id::from_raw(0x0001_0100), y);
        assert_eq!((p, q.copied()), (None, Some(1)));
    }

    #[test]
    fn id_traits() {
        let a: Id<u8> = Id::from_raw(0x0002_0005);
        assert_eq!((a.index(), a.generation()), (5, 2));
        assert_eq!(alloc::format!("{a:?} {a}"), "#5v2 #5v2");
        let b: Id<u32> = a.cast();
        assert_eq!(b.to_raw(), a.to_raw());
        assert!(Id::<u8>::from_raw(0x0001_0001) < a);
        assert_eq!(Id::<u8>::default(), Id::DANGLING);
    }
}
