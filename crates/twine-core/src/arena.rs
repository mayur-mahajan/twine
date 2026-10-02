//! Generational arena ([`Arena<T>`]) with stale-handle detection ([`Id<T>`]).

use alloc::collections::VecDeque;
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

    /// A handle that no [`Arena`] ever hands out, whatever it stored before: its index
    /// (`u16::MAX`) is outside the slot range (an arena has at most [`Arena::MAX_LEN`] =
    /// 65 535 slots, indices `0..=65 534`) **and** its generation is 0 (generations start at 1
    /// and never wrap: a slot retires instead). So it never resolves, is never equal to a live
    /// or stale handle, and — unlike [`DANGLING`](Self::DANGLING), which only its generation
    /// separates from slot 0 — it stays invalid under any generation scheme.
    ///
    /// Layers above use it as a "this was never created" marker (e.g. the view layer's dead
    /// node for a widget whose creation failed). Looking it up costs one bounds check.
    ///
    /// ```
    /// use twine_core::{Arena, Id};
    /// let a: Arena<u8> = Arena::new();
    /// assert!(!a.contains(Id::INVALID));
    /// assert_eq!(Id::<u8>::INVALID.index(), u16::MAX);
    /// assert_ne!(Id::<u8>::INVALID, Id::DANGLING);
    /// ```
    pub const INVALID: Id<T> = Id::new(u16::MAX, 0);

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

// `remove` tests `gen & (ROTATE_PERIOD - 1)`.
const _: () = assert!(Arena::<()>::ROTATE_PERIOD.is_power_of_two());

/// Storage for up to 65 535 live values, addressed by generational [`Id`]s.
///
/// Freed slots are reused with a bumped generation, so stale ids are detected. A slot whose
/// generation would wrap past `u16::MAX` is **retired** (never reused) to rule out ABA; the
/// number of retired slots is [`retired`](Self::retired). Insert and remove are O(1)
/// worst case apart from the amortized growth of the storage (slots and free-index lists
/// grow geometrically; never in steady state).
///
/// # Reuse order and generation budget
///
/// Freed slots go on a **stack** and the most recently freed one is reused first, so churn
/// stays on cache-hot slots. Every [`ROTATE_PERIOD`](Self::ROTATE_PERIOD) (4 096) generations
/// a slot is instead parked at the back of a FIFO **reserve**, and the reserve is used only
/// when the stack is empty. A hot slot thus serves at most 4 096 values in a row before every
/// parked slot has had its turn, which spreads the 16-bit generations over all free slots. The
/// only cost on the common path is one bit test in `remove`.
///
/// Each slot serves at most 65 535 values (generations `1..=u16::MAX`), so the total budget is
/// *slots × 65 535* insertions (≈ 4.29 × 10⁹ for a full-size arena). With `S` slots, never more
/// than `L < S` values live, `P` = `ROTATE_PERIOD` and `K = ⌊65 534 / P⌋` = 15 parkings per
/// slot lifetime, **no slot retires before `(P − 1) × (K × (S − L + 1) + 1)` insertions**
/// ≈ `(S − L) × 61 425` — within one rotation period per slot of the ideal `(S − L) × 65 534`
/// (e.g. ≥ 3.5 × 10⁶ for 64 slots with at most 8 live). Proof sketch: the reserve is used
/// only when the stack is empty, i.e. when at most `L − 1` slots are outside the reserve, so a
/// parked slot leaves the reserve only after at least `S − L` other slots were parked behind
/// it; a slot must be reused `P − 1` times between two parkings, and a slot that retires was
/// parked `K` times. Past that point slots retire one by one and are replaced by new ones:
/// memory grows by one slot per 65 535 insertions into it, never faster. Wider generations
/// are a later option (R5.S04).
///
/// Looking a value up ([`get`](Self::get), [`get_mut`](Self::get_mut),
/// [`contains`](Self::contains)) is one bounds check plus one generation compare, whatever
/// the reuse order.
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
    /// Indices of free slots, reused LIFO (most recently freed first: cache-hot).
    free: Vec<u16>,
    /// Free slots parked after `ROTATE_PERIOD` generations, reused FIFO when `free` is empty.
    reserve: VecDeque<u16>,
    len: u16,
    /// Slots whose generation reached `u16::MAX` and that are never reused.
    retired: u16,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    /// Maximum number of live values and of slots. Slot indices are `0..MAX_LEN`, so index
    /// `u16::MAX` is never used (see [`Id::INVALID`]).
    pub const MAX_LEN: usize = u16::MAX as usize;

    /// A slot is parked in the FIFO reserve each time its generation reaches a multiple of
    /// this (a power of two), so it serves at most this many values in a row (see
    /// [reuse order](Self#reuse-order-and-generation-budget)).
    pub const ROTATE_PERIOD: u16 = 4096;

    /// An empty arena (no allocation).
    #[must_use]
    pub const fn new() -> Self {
        Arena {
            slots: Vec::new(),
            free: Vec::new(),
            reserve: VecDeque::new(),
            len: 0,
            retired: 0,
        }
    }

    /// An empty arena with room for `n` values.
    #[must_use]
    pub fn with_capacity(n: u16) -> Self {
        Arena {
            slots: Vec::with_capacity(usize::from(n)),
            free: Vec::new(),
            reserve: VecDeque::new(),
            len: 0,
            retired: 0,
        }
    }

    /// Stores `v`, returning its handle; [`Error::CapacityExceeded`] when 65 535 values are
    /// live (or every slot is retired).
    pub fn insert(&mut self, v: T) -> Result<Id<T>, Error> {
        // Hot path: the most recently freed slot (same code as a plain LIFO free list).
        if let Some(index) = self.free.pop() {
            return Ok(self.fill(index, v));
        }
        // The stack is empty: the oldest parked slot, else a new slot.
        if let Some(index) = self.reserve.pop_front() {
            return Ok(self.fill(index, v));
        }
        if self.slots.len() >= Self::MAX_LEN {
            crate::warn!(target: "twine::core", "Arena::insert: capacity exceeded ({} live)", self.len);
            return Err(Error::CapacityExceeded);
        }
        let index = self.slots.len() as u16;
        if self.slots.len() == self.slots.capacity() {
            // Grow by a quarter (at least 8 slots), not by doubling: slots can be large (the
            // engine's nodes) and an arena half empty after a doubling wastes RAM on small
            // devices. Still geometric, so pushes stay amortized O(1).
            let grow = (self.slots.len() / 4).max(8);
            self.slots.reserve_exact(grow);
        }
        self.slots.push(Slot {
            gen_: 1,
            value: Some(v),
        });
        self.len += 1;
        Ok(Id::new(index, 1))
    }

    /// Stores `v` in the free slot `index`.
    #[inline(always)]
    fn fill(&mut self, index: u16, v: T) -> Id<T> {
        let slot = &mut self.slots[usize::from(index)];
        debug_assert!(slot.value.is_none());
        slot.value = Some(v);
        self.len += 1;
        Id::new(index, slot.gen_)
    }

    fn slot(&self, id: Id<T>) -> Option<&Slot<T>> {
        self.slots
            .get(usize::from(id.index))
            .filter(|s| s.gen_ == id.gen_ && s.value.is_some())
    }

    /// Removes and returns the value of `id`; `None` if the id is stale or unknown.
    // `#[inline]`: without the hint the rotation branch tips LLVM into an out-of-line call at
    // every call site (+5 % on the reactive runtime's scope create/dispose bench).
    #[inline]
    pub fn remove(&mut self, id: Id<T>) -> Option<T> {
        let slot = self.slots.get_mut(usize::from(id.index))?;
        if slot.gen_ != id.gen_ {
            return None;
        }
        let v = slot.value.take()?;
        self.len -= 1;
        // One add and one bit test, as cheap as the old `gen == u16::MAX` check: a generation
        // that wraps to 0 (retirement) or hits a multiple of ROTATE_PERIOD (parking) takes the
        // cold path.
        let next = slot.gen_.wrapping_add(1);
        if next & (Self::ROTATE_PERIOD - 1) != 0 {
            slot.gen_ = next;
            self.free.push(id.index);
        } else {
            self.park_or_retire(id.index);
        }
        Some(v)
    }

    /// `remove` of a slot whose generation reaches a multiple of `ROTATE_PERIOD` (parked in
    /// the reserve) or would wrap (retired).
    #[cold]
    #[inline(never)]
    fn park_or_retire(&mut self, index: u16) {
        let slot = &mut self.slots[usize::from(index)];
        if slot.gen_ == u16::MAX {
            self.retired += 1;
            crate::debug!(target: "twine::core", "Arena: slot {} retired", index);
        } else {
            slot.gen_ += 1;
            self.reserve.push_back(index);
        }
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

    /// Number of slots in use or ever used (live, free and retired): the high-water mark of
    /// the slot storage, at most [`capacity`](Self::capacity).
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Heap bytes the arena's own storage reserves: the slot vector (each slot holds a `T`
    /// inline plus its generation) and the two free-index lists, by capacity. Memory a `T`
    /// owns elsewhere (a `Box` inside it) is not included, nor is allocator overhead: this is
    /// what the arena asked the allocator for.
    ///
    /// Allocates nothing; never panics; O(1).
    ///
    /// ```
    /// use twine_core::Arena;
    /// let mut a: Arena<u64> = Arena::with_capacity(10);
    /// let reserved = a.bytes_reserved();
    /// assert!(reserved >= 10 * core::mem::size_of::<u64>());
    /// a.insert(1).unwrap();
    /// assert_eq!(a.bytes_reserved(), reserved); // within capacity: unchanged
    /// ```
    #[must_use]
    pub fn bytes_reserved(&self) -> usize {
        self.slots.capacity() * core::mem::size_of::<Slot<T>>()
            + (self.free.capacity() + self.reserve.capacity()) * core::mem::size_of::<u16>()
    }

    /// Number of **retired** slots: slots that served all 65 535 generations and are never
    /// reused (see the [generation budget](Self#generation-budget)). Each one is dead memory
    /// until the arena is dropped; a non-zero value on a device means the arena's working set
    /// is too small for its churn.
    ///
    /// ```
    /// use twine_core::Arena;
    /// let mut a = Arena::new();
    /// let mut id = a.insert(()).unwrap();
    /// while id.generation() < u16::MAX {
    ///     a.remove(id);
    ///     id = a.insert(()).unwrap();
    /// }
    /// assert_eq!(a.retired(), 0);
    /// a.remove(id);
    /// assert_eq!(a.retired(), 1);
    /// ```
    #[doc(alias = "wear")]
    #[must_use]
    pub fn retired(&self) -> usize {
        usize::from(self.retired)
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
        // LIFO reuse: y's slot (freed last) first.
        let z = a.insert(3).unwrap();
        assert_eq!((z.index(), z.generation()), (y.index(), 2));
        let w = a.insert(4).unwrap();
        assert_eq!((w.index(), w.generation()), (x.index(), 2));
        assert_eq!(a.get(x), None);
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
        assert_eq!(a.retired(), 0);
        a.remove(id).unwrap();
        assert_eq!(a.retired(), 1);
        let next = a.insert(1).unwrap();
        assert_eq!(next.index(), 1, "retired slot 0 must not be reused");
        assert_eq!((a.retired(), a.slot_count()), (1, 2));
        assert_eq!(a.get(id), None);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn invalid_id_is_never_handed_out() {
        // Fill every slot, churn some of them (bumping generations), and check that no handle
        // ever equals `Id::INVALID` and that it never resolves.
        let mut a: Arena<()> = Arena::new();
        let mut ids = Vec::with_capacity(Arena::<()>::MAX_LEN);
        while let Ok(id) = a.insert(()) {
            assert_ne!(id, Id::INVALID);
            ids.push(id);
        }
        assert_eq!(ids.len(), Arena::<()>::MAX_LEN);
        assert!(ids.iter().all(|id| id.index() < u16::MAX && id.generation() >= 1));
        assert!(!a.contains(Id::INVALID));
        assert!(a.get(Id::INVALID).is_none());
        assert!(a.get_mut(Id::INVALID).is_none());
        assert!(a.remove(Id::INVALID).is_none());
        for &id in ids.iter().rev().take(64) {
            a.remove(id);
            let again = a.insert(()).unwrap();
            assert_ne!(again, Id::INVALID);
            assert!(again.generation() >= 1);
        }
        assert_eq!(a.len(), Arena::<()>::MAX_LEN);
        assert!(!a.contains(Id::INVALID));
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
