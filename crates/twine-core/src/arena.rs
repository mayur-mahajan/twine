//! Generational arena ([`Arena<T>`]) with stale-handle detection ([`Id<T>`]).

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use crate::Error;

/// The generation of an arena slot (see [`Id::generation`]), as a plain integer.
///
/// - **Default profile: `u16`**, values `1..=`[`MAX_GENERATION`] = 65 535.
/// - **Safety profile** (feature `safety`): **`u64` holding a 48-bit value**, `1..=`
///   [`MAX_GENERATION`] = 2⁴⁸ − 2³² − 1 ≈ 2.8 × 10¹⁴ — a slot reused 100 000 times per second
///   lasts 89 years, so retirement is out of reach in practice.
///
/// Each slot serves at most `MAX_GENERATION` values, then retires (see the arena's
/// [generation budget](Arena#generation-width-and-budget)). The width is chosen by the build,
/// never at run time: the default profile pays nothing for the safety profile's handles. Step
/// a generation with [`next_generation`] (never `+ 1`: the safety profile's maximum is not
/// `u64::MAX`).
///
/// ```
/// use twine_core::arena::{Generation, MAX_GENERATION};
/// let budget = u64::from(MAX_GENERATION);
/// assert!(budget == 65_535 || budget == (1 << 48) - (1 << 32) - 1);
/// assert_eq!(budget > 65_535, twine_core::SAFETY_PROFILE);
/// let _g: Generation = MAX_GENERATION;
/// ```
#[cfg(not(feature = "safety"))]
pub type Generation = u16;
/// The generation of an arena slot (see [`Id::generation`]), as a plain integer.
///
/// - **Default profile: `u16`**, values `1..=`[`MAX_GENERATION`] = 65 535.
/// - **Safety profile** (feature `safety`): **`u64` holding a 48-bit value**, `1..=`
///   [`MAX_GENERATION`] = 2⁴⁸ − 2³² − 1 ≈ 2.8 × 10¹⁴ — a slot reused 100 000 times per second
///   lasts 89 years, so retirement is out of reach in practice.
///
/// Each slot serves at most `MAX_GENERATION` values, then retires (see the arena's
/// [generation budget](Arena#generation-width-and-budget)). The width is chosen by the build,
/// never at run time: the default profile pays nothing for the safety profile's handles. Step
/// a generation with [`next_generation`] (never `+ 1`: the safety profile's maximum is not
/// `u64::MAX`).
///
/// ```
/// use twine_core::arena::{Generation, MAX_GENERATION};
/// let budget = u64::from(MAX_GENERATION);
/// assert!(budget == 65_535 || budget == (1 << 48) - (1 << 32) - 1);
/// assert_eq!(budget > 65_535, twine_core::SAFETY_PROFILE);
/// let _g: Generation = MAX_GENERATION;
/// ```
#[cfg(feature = "safety")]
pub type Generation = u64;

/// The last [`Generation`] a slot serves: `u16::MAX` by default, `2⁴⁸ − 2³² − 1`
/// (`0xFFFE_FFFF_FFFF`) in the safety profile. The safety value leaves out the generations
/// whose top 16 bits are all ones, so the handle's niche (`Option<Id>` is as small as `Id`) costs
/// no range worth mentioning.
pub const MAX_GENERATION: Generation = repr::MAX_GENERATION;

/// The generation after `g`, or `None` when `g` is [`MAX_GENERATION`] or beyond (the slot must
/// then retire, never wrap). For tables that keep generations of their own (e.g. the
/// animation timeline). Never panics; O(1).
///
/// ```
/// use twine_core::arena::{MAX_GENERATION, next_generation};
/// assert_eq!(next_generation(1), Some(2));
/// assert_eq!(next_generation(MAX_GENERATION), None);
/// ```
#[must_use]
pub const fn next_generation(g: Generation) -> Option<Generation> {
    if g < MAX_GENERATION {
        // SAFE-ARITH: `g < MAX_GENERATION`, the type's maximum or below.
        Some(g.wrapping_add(1))
    } else {
        None
    }
}

/// An [`Id`] packed into one integer ([`Id::to_raw`]): `generation << 16 | index`. **`u32`** by
/// default, **`u64`** in the safety profile (16 index bits plus its 48-bit [`Generation`]).
///
/// Layers that key data by node without depending on the arena's element type (e.g. the
/// animation timeline's `NodeKey` in `twine-anim`) store this.
///
/// ```
/// use twine_core::{Arena, Id, arena::RawId};
/// let mut a = Arena::new();
/// let id = a.insert('x').unwrap();
/// let raw: RawId = id.to_raw();
/// assert_eq!(Id::<char>::from_raw(raw), id);
/// assert_eq!(core::mem::size_of::<RawId>(), core::mem::size_of::<Id<char>>());
/// ```
#[cfg(not(feature = "safety"))]
pub type RawId = u32;
/// An [`Id`] packed into one integer ([`Id::to_raw`]): `generation << 16 | index`. **`u32`** by
/// default, **`u64`** in the safety profile (16 index bits plus its 48-bit [`Generation`]).
///
/// Layers that key data by node without depending on the arena's element type (e.g. the
/// animation timeline's `NodeKey` in `twine-anim`) store this.
///
/// ```
/// use twine_core::{Arena, Id, arena::RawId};
/// let mut a = Arena::new();
/// let id = a.insert('x').unwrap();
/// let raw: RawId = id.to_raw();
/// assert_eq!(Id::<char>::from_raw(raw), id);
/// assert_eq!(core::mem::size_of::<RawId>(), core::mem::size_of::<Id<char>>());
/// ```
#[cfg(feature = "safety")]
pub type RawId = u64;

/// Bit position of the generation in a [`RawId`].
const GEN_SHIFT: u32 = 16;

/// Storage of handles and slot generations, per profile. Both expose the same interface:
/// `Key` (an `Id`'s payload: index + generation) and `SlotGen` (what a slot stores to check a
/// key and step its generation).
#[cfg(not(feature = "safety"))]
mod repr {
    use super::{Arena, Generation};

    pub(super) const MAX_GENERATION: Generation = u16::MAX;

    /// Low bits that are zero on a multiple of `ROTATE_PERIOD`.
    const ROTATE_MASK: u16 = Arena::<()>::ROTATE_PERIOD.wrapping_sub(1);

    /// Index and generation: 4 bytes, no niche (the default profile's historic layout).
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct Key {
        index: u16,
        gen_: u16,
    }

    impl Key {
        pub(super) const INVALID: Key = Key {
            index: u16::MAX,
            gen_: 0,
        };

        /// Every `(index, generation)` is representable.
        pub(super) const fn new(index: u16, gen_: Generation) -> Key {
            Key { index, gen_ }
        }

        pub(super) const fn index(self) -> u16 {
            self.index
        }

        pub(super) const fn generation(self) -> Generation {
            self.gen_
        }
    }

    /// A slot's generation (2 bytes: the index is the slot's position).
    #[derive(Clone, Copy, Debug)]
    pub(super) struct SlotGen(u16);

    impl SlotGen {
        /// A new slot's generation (1).
        pub(super) const fn first(_index: u16) -> SlotGen {
            SlotGen(1)
        }

        /// Whether `key` (of this slot) has the slot's generation.
        #[inline(always)]
        pub(super) fn matches(self, key: Key) -> bool {
            self.0 == key.gen_
        }

        /// The key of the slot at `index`.
        #[inline(always)]
        pub(super) fn key(self, index: u16) -> Key {
            Key { index, gen_: self.0 }
        }

        /// The hot part of a removal: steps the generation unless the next one is a multiple
        /// of `ROTATE_PERIOD` or wraps (`false`: call [`step`](Self::step), cold).
        #[inline(always)]
        pub(super) fn step_within_period(&mut self) -> bool {
            let next = self.0.wrapping_add(1);
            if next & ROTATE_MASK != 0 {
                self.0 = next;
                true
            } else {
                false
            }
        }

        /// Steps the generation; `false` (unchanged) at `MAX_GENERATION`: the slot retires.
        pub(super) fn step(&mut self) -> bool {
            match self.0.checked_add(1) {
                Some(next) => {
                    self.0 = next;
                    true
                }
                None => false,
            }
        }

        #[cfg(test)]
        pub(super) fn set(&mut self, _index: u16, g: Generation) {
            self.0 = g;
        }
    }
}

/// Storage of handles and slot generations, per profile. Both expose the same interface:
/// `Key` (an `Id`'s payload: index + generation) and `SlotGen` (what a slot stores to check a
/// key and step its generation).
#[cfg(feature = "safety")]
mod repr {
    use core::num::NonZeroU16;

    use super::{Arena, Generation};

    /// Top 16 bits `0xFFFE`, low 32 bits all ones: `hi = 0xFFFF` is the niche.
    pub(super) const MAX_GENERATION: Generation = 0xFFFE_FFFF_FFFF;

    /// Low bits that are zero on a multiple of `ROTATE_PERIOD` (it divides 2³²).
    const ROTATE_MASK: u32 = (Arena::<()>::ROTATE_PERIOD as u32).wrapping_sub(1);

    /// Index and 48-bit generation in 8 bytes (align 4, no padding): the generation's low 32
    /// bits, its high 16 bits **inverted** (so `NonZeroU16`: high bits `0xFFFF` — above
    /// `MAX_GENERATION` — are the `None` of `Option<Id>`), the slot index.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct Key {
        lo: u32,
        hi_inv: NonZeroU16,
        index: u16,
    }

    impl Key {
        /// Index `u16::MAX`, generation 0 (high bits 0, stored inverted).
        pub(super) const INVALID: Key = Key {
            lo: 0,
            hi_inv: NonZeroU16::MAX,
            index: u16::MAX,
        };

        /// A generation above `MAX_GENERATION` (only reachable through `Id::from_raw` /
        /// `Id::from_parts` with a foreign value) gives `INVALID`, which never resolves.
        pub(super) const fn new(index: u16, gen_: Generation) -> Key {
            // Truncating casts on purpose: each field takes its own bits.
            let hi = (gen_ >> 32) as u16;
            match NonZeroU16::new(!hi) {
                Some(hi_inv) if gen_ <= MAX_GENERATION => Key {
                    lo: gen_ as u32,
                    hi_inv,
                    index,
                },
                _ => Key::INVALID,
            }
        }

        pub(super) const fn index(self) -> u16 {
            self.index
        }

        pub(super) const fn generation(self) -> Generation {
            ((!self.hi_inv.get() as u64) << 32) | self.lo as u64
        }
    }

    /// A slot's generation, stored as the slot's whole key (8 bytes: a check is one key
    /// compare, and a 6-byte generation would be padded to 8 anyway).
    #[derive(Clone, Copy, Debug)]
    pub(super) struct SlotGen(Key);

    impl core::fmt::Debug for Key {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "#{}v{}", self.index, self.generation())
        }
    }

    impl SlotGen {
        /// A new slot's generation (1).
        pub(super) const fn first(index: u16) -> SlotGen {
            SlotGen(Key::new(index, 1))
        }

        /// Whether `key` (of this slot) has the slot's generation.
        #[inline(always)]
        pub(super) fn matches(self, key: Key) -> bool {
            self.0 == key
        }

        /// The key of the slot at `index` (stored whole).
        #[inline(always)]
        pub(super) fn key(self, _index: u16) -> Key {
            self.0
        }

        /// The hot part of a removal: steps the low 32 bits unless the next generation is a
        /// multiple of `ROTATE_PERIOD` (which includes every carry into the high bits and the
        /// step past `MAX_GENERATION`: `false`, call [`step`](Self::step), cold). Same code as
        /// a 32-bit generation.
        #[inline(always)]
        pub(super) fn step_within_period(&mut self) -> bool {
            let next = self.0.lo.wrapping_add(1);
            if next & ROTATE_MASK != 0 {
                self.0.lo = next;
                true
            } else {
                false
            }
        }

        /// Steps the generation; `false` (unchanged) at `MAX_GENERATION`: the slot retires.
        pub(super) fn step(&mut self) -> bool {
            match super::next_generation(self.0.generation()) {
                Some(next) => {
                    self.0 = Key::new(self.0.index, next);
                    true
                }
                None => false,
            }
        }

        #[cfg(test)]
        pub(super) fn set(&mut self, index: u16, g: Generation) {
            self.0 = Key::new(index, g);
        }
    }
}

/// A generational handle into an [`Arena<T>`]: a `u16` slot index plus a [`Generation`]
/// (16-bit by default, 48-bit in the safety profile).
///
/// `Id`s are `Copy`, comparable and hashable regardless of `T`. A handle to a removed value
/// never resolves again, even after its slot is reused.
///
/// | Profile | Generation | `size_of::<Id<T>>()` | `size_of::<Option<Id<T>>>()` | [`RawId`] |
/// |---------|------------|----------------------|------------------------------|-----------|
/// | default | 16-bit (`u16`) | 4 | 6 | `u32` |
/// | `safety` | 48-bit (in a `u64`) | 8 (align 4) | 8 | `u64` |
///
/// ```
/// use twine_core::{Arena, Id};
/// let mut a = Arena::new();
/// let id: Id<&str> = a.insert("x").unwrap();
/// assert_eq!(Id::<&str>::from_raw(id.to_raw()), id);
/// assert_eq!(id.to_string(), "#0v1");
/// ```
pub struct Id<T> {
    key: repr::Key,
    _m: PhantomData<fn() -> T>,
}

// The sizes the table above promises (and that the engine's node and the reactive handles
// are budgeted with).
#[cfg(not(feature = "safety"))]
const _: () = {
    assert!(core::mem::size_of::<Id<()>>() == 4);
    assert!(core::mem::size_of::<Option<Id<()>>>() == 6);
};
#[cfg(feature = "safety")]
const _: () = {
    assert!(core::mem::size_of::<Id<()>>() == 8);
    assert!(core::mem::align_of::<Id<()>>() == 4);
    assert!(core::mem::size_of::<Option<Id<()>>>() == 8);
    // `MAX_GENERATION + 1` is a multiple of `ROTATE_PERIOD`, so the step past it takes the
    // cold path of `remove` (see `SlotGen::step_within_period`).
    assert!((MAX_GENERATION + 1) % (Arena::<()>::ROTATE_PERIOD as u64) == 0);
};
const _: () = assert!(core::mem::size_of::<RawId>() == core::mem::size_of::<Id<()>>());

impl<T> Id<T> {
    /// A handle that never resolves (index 0, generation 0; generations start at 1).
    pub const DANGLING: Id<T> = Id::from_raw(0);

    /// A handle that no [`Arena`] ever hands out, whatever it stored before: its index
    /// (`u16::MAX`) is outside the slot range (an arena has at most [`Arena::MAX_LEN`] =
    /// 65 535 slots, indices `0..=65 534`) **and** its generation is 0 (generations start at 1
    /// and never wrap: a slot retires instead). So it never resolves, is never equal to a live
    /// or stale handle, and — unlike [`DANGLING`](Self::DANGLING), which only its generation
    /// separates from slot 0 — it stays invalid whatever the [`Generation`] width.
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
    pub const INVALID: Id<T> = Id::from_key(repr::Key::INVALID);

    const fn from_key(key: repr::Key) -> Self {
        Id { key, _m: PhantomData }
    }

    /// The handle of slot `index` at `generation`, for tables that keep generational slots of
    /// their own (e.g. `twine-anim`'s timers and animations) and want `Id`'s size, niche and
    /// traits. A generation above [`MAX_GENERATION`] gives [`INVALID`](Self::INVALID). Never
    /// panics.
    ///
    /// ```
    /// use twine_core::Id;
    /// let id: Id<()> = Id::from_parts(3, 7);
    /// assert_eq!((id.index(), id.generation()), (3, 7));
    /// ```
    #[must_use]
    pub const fn from_parts(index: u16, generation: Generation) -> Self {
        Id::from_key(repr::Key::new(index, generation))
    }

    /// Slot index.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.key.index()
    }

    /// Generation of the slot when the handle was created (≥ 1 for real handles; see
    /// [`Generation`] for its width).
    #[must_use]
    pub const fn generation(self) -> Generation {
        self.key.generation()
    }

    /// Packs into a [`RawId`]: `generation << 16 | index`.
    #[must_use]
    pub const fn to_raw(self) -> RawId {
        // Widening casts: the generation fits above the 16 index bits in both profiles.
        ((self.generation() as RawId) << GEN_SHIFT) | self.index() as RawId
    }

    /// Unpacks a value produced by [`Id::to_raw`]. In the safety profile a value whose
    /// generation is above [`MAX_GENERATION`] (never produced by `to_raw`) gives
    /// [`INVALID`](Self::INVALID).
    #[must_use]
    pub const fn from_raw(raw: RawId) -> Self {
        // Truncating casts on purpose: each field takes its own bits.
        Id::from_parts(raw as u16, (raw >> GEN_SHIFT) as Generation)
    }

    /// Reinterprets the handle for another element type.
    #[must_use]
    pub const fn cast<U>(self) -> Id<U> {
        Id::from_key(self.key)
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
        self.key == o.key
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
        (self.index(), self.generation()).cmp(&(o.index(), o.generation()))
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
        write!(f, "#{}v{}", self.index(), self.generation())
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}v{}", self.index(), self.generation())
    }
}

#[cfg(feature = "defmt")]
impl<T> defmt::Format for Id<T> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "#{}v{}", self.index(), self.generation());
    }
}

#[derive(Clone, Debug)]
struct Slot<T> {
    gen_: repr::SlotGen,
    value: Option<T>,
}

const _: () = assert!(Arena::<()>::ROTATE_PERIOD.is_power_of_two());

/// Storage for up to 65 535 live values, addressed by generational [`Id`]s.
///
/// Freed slots are reused with a bumped generation, so stale ids are detected. A slot whose
/// generation would pass [`MAX_GENERATION`] is **retired** (never reused) to rule out
/// ABA; the number of retired slots is [`retired`](Self::retired). Insert and remove are O(1)
/// worst case apart from the amortized growth of the storage (slots and free-index lists grow
/// geometrically; never in steady state).
///
/// # Reuse order
///
/// Freed slots go on a **stack** and the most recently freed one is reused first, so churn
/// stays on cache-hot slots. Every [`ROTATE_PERIOD`](Self::ROTATE_PERIOD) (4 096) generations
/// a slot is instead parked at the back of a FIFO **reserve**, and the reserve is used only
/// when the stack is empty. A hot slot thus serves at most 4 096 values in a row before every
/// parked slot has had its turn, which spreads the generations over all free slots. The only
/// cost on the common path is one bit test in `remove`.
///
/// # Generation width and budget
///
/// Each slot serves at most `G =` [`MAX_GENERATION`] values (generations `1..=G`):
/// **65 535** by default (16-bit), **2⁴⁸ − 2³² − 1 ≈ 2.8 × 10¹⁴** in the safety profile
/// (feature `safety`, 48-bit; see [`Generation`]). With `S` slots, never more than `L < S`
/// values live, `P` = `ROTATE_PERIOD` and `K = ⌊(G − 1) / P⌋` parkings per slot lifetime (15
/// by default, ≈ 6.9 × 10¹⁰ in the safety profile), **no slot retires before
/// `(P − 1) × (K × (S − L + 1) + 1)` insertions** ≈ `(S − L + 1) × G`. Proof sketch: the
/// reserve is used only when the stack is empty, i.e. when at most `L − 1` slots are outside
/// the reserve, so a parked slot leaves the reserve only after at least `S − L` other slots
/// were parked behind it; a slot must be reused `P − 1` times between two parkings, and a
/// slot that retires was parked `K` times.
///
/// | `S − L` (free slots) | default: first retirement after | at 100 insertions/s | safety: first retirement after | at 1 000 insertions/s | at 100 000 insertions/s |
/// |------:|------:|------:|------:|------:|------:|
/// | 0 (one slot, no rotation) | 6.6 × 10⁴ | 11 min | 2.8 × 10¹⁴ | 8 900 years | 89 years |
/// | 1  | 1.3 × 10⁵ | 21 min | 5.6 × 10¹⁴ | 17 800 years | 178 years |
/// | 15 | 9.9 × 10⁵ | 2.7 h | 4.5 × 10¹⁵ | 143 000 years | 1 400 years |
///
/// (Insertions into this one arena, sustained without pause.) So by default a busy arena
/// retires slots within hours, while in the safety profile no slot retires within any
/// product's lifetime, even at rates far beyond UI churn. Retirement stays **correct** in both
/// profiles: past the bound slots retire one by one and are replaced by new ones, so memory
/// grows by one slot per `G` insertions into it, never faster, and a slot inside a
/// [`reserve`](Self::reserve)d capacity allocates nothing.
///
/// Looking a value up ([`get`](Self::get), [`get_mut`](Self::get_mut),
/// [`contains`](Self::contains)) is one bounds check plus one generation compare, whatever
/// the reuse order and the generation width.
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
    /// Live values: at most `MAX_LEN` (one per slot).
    len: u16,
    /// Slots whose generation reached `MAX_GENERATION` and that are never reused: at most
    /// `MAX_LEN` (one per slot).
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
    /// [reuse order](Self#reuse-order)).
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
            return self.fill(index, v);
        }
        // The stack is empty: the oldest parked slot, else a new slot.
        if let Some(index) = self.reserve.pop_front() {
            return self.fill(index, v);
        }
        let Ok(index) = u16::try_from(self.slots.len()) else {
            return Err(self.full());
        };
        if index == u16::MAX {
            return Err(self.full());
        }
        if self.slots.len() == self.slots.capacity() {
            self.grow_slots();
        }
        let gen_ = repr::SlotGen::first(index);
        self.slots.push(Slot { gen_, value: Some(v) });
        // SAFE-ARITH: `len` < `slots.len()` ≤ `MAX_LEN` before the push (every slot holds at
        // most one value and this one is new), so it stays ≤ `u16::MAX`.
        self.len = self.len.wrapping_add(1);
        Ok(Id::from_key(gen_.key(index)))
    }

    /// The `CapacityExceeded` path of `insert` (cold: logs).
    #[cold]
    #[inline(never)]
    fn full(&self) -> Error {
        crate::warn!(target: "twine::core", "Arena::insert: capacity exceeded ({} live)", self.len);
        Error::CapacityExceeded
    }

    /// Grows the slot storage by a quarter (at least 8 slots), not by doubling: slots can be
    /// large (the engine's nodes) and an arena half empty after a doubling wastes RAM on small
    /// devices. Still geometric, so pushes stay amortized O(1). Cold: never reached within a
    /// [`reserve`](Self::reserve)d capacity.
    #[cold]
    #[inline(never)]
    fn grow_slots(&mut self) {
        crate::alloc_guard::grew(crate::alloc_guard::AllocSite::ArenaSlots);
        let grow = (self.slots.len() >> 2).max(8);
        self.slots.reserve_exact(grow);
    }

    /// Reserves storage for `n` slots in total — the slot vector and both free lists — so that
    /// inserting and removing values never allocates while at most `n` slots are in use (no
    /// retirement). Used to preallocate an arena from a [`Limits`](crate::Limits) bound. Does
    /// nothing for parts already that large; `n` is capped at [`MAX_LEN`](Self::MAX_LEN).
    /// Allocates (once); never panics.
    ///
    /// ```
    /// use twine_core::Arena;
    /// let mut a: Arena<u32> = Arena::new();
    /// a.reserve(100);
    /// assert!(a.capacity() >= 100);
    /// let before = a.bytes_reserved();
    /// let ids: Vec<_> = (0..100).map(|i| a.insert(i).unwrap()).collect();
    /// for id in ids {
    ///     a.remove(id);
    /// }
    /// assert_eq!(a.bytes_reserved(), before); // no growth
    /// ```
    pub fn reserve(&mut self, n: u16) {
        let n = usize::from(n).min(Self::MAX_LEN);
        self.slots.reserve_exact(n.saturating_sub(self.slots.len()));
        self.free.reserve_exact(n.saturating_sub(self.free.len()));
        self.reserve.reserve_exact(n.saturating_sub(self.reserve.len()));
    }

    /// Stores `v` in the free slot `index`.
    #[inline(always)]
    fn fill(&mut self, index: u16, v: T) -> Result<Id<T>, Error> {
        // The free lists hold indices of existing slots only; `None` is unreachable (no
        // panic branch: the bounds check is needed either way).
        let Some(slot) = self.slots.get_mut(usize::from(index)) else {
            return Err(Self::lost_slot(index));
        };
        debug_assert!(slot.value.is_none());
        slot.value = Some(v);
        // SAFE-ARITH: a free slot existed, so fewer than `slots.len()` ≤ `MAX_LEN` values
        // were live.
        self.len = self.len.wrapping_add(1);
        Ok(Id::from_key(slot.gen_.key(index)))
    }

    /// A free-list index outside the slots (an internal invariant violation; unreachable).
    #[cold]
    #[inline(never)]
    fn lost_slot(index: u16) -> Error {
        debug_assert!(false, "Arena: free slot {index} out of range");
        crate::error!(target: "twine::core", "Arena: free slot {} out of range", index);
        Error::OutOfBounds
    }

    fn slot(&self, id: Id<T>) -> Option<&Slot<T>> {
        self.slots
            .get(usize::from(id.index()))
            .filter(|s| s.gen_.matches(id.key) && s.value.is_some())
    }

    /// Removes and returns the value of `id`; `None` if the id is stale or unknown.
    // `#[inline]`: without the hint the rotation branch tips LLVM into an out-of-line call at
    // every call site (+5 % on the reactive runtime's scope create/dispose bench).
    #[inline]
    pub fn remove(&mut self, id: Id<T>) -> Option<T> {
        let index = id.index();
        let slot = self.slots.get_mut(usize::from(index))?;
        if !slot.gen_.matches(id.key) {
            return None;
        }
        let v = slot.value.take()?;
        // SAFE-ARITH: the slot held a value, so `len` ≥ 1.
        self.len = self.len.wrapping_sub(1);
        // One add and one bit test, as cheap as a `gen == MAX` check: a generation that
        // would pass MAX_GENERATION (retirement) or hits a multiple of ROTATE_PERIOD (parking)
        // takes the cold path.
        if slot.gen_.step_within_period() {
            crate::alloc_guard::push(
                &mut self.free,
                index,
                crate::alloc_guard::AllocSite::ArenaFreeList,
            );
        } else {
            self.park_or_retire(index);
        }
        Some(v)
    }

    /// `remove` of a slot whose generation reaches a multiple of `ROTATE_PERIOD` (parked in
    /// the reserve) or would pass `MAX_GENERATION` (retired: never reused).
    #[cold]
    #[inline(never)]
    fn park_or_retire(&mut self, index: u16) {
        let Some(slot) = self.slots.get_mut(usize::from(index)) else {
            return;
        };
        if slot.gen_.step() {
            crate::alloc_guard::push_back(
                &mut self.reserve,
                index,
                crate::alloc_guard::AllocSite::ArenaFreeList,
            );
        } else {
            // At most one retirement per slot, and there are at most `MAX_LEN` slots.
            self.retired = self.retired.saturating_add(1);
            crate::debug!(target: "twine::core", "Arena: slot {} retired", index);
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
            .get_mut(usize::from(id.index()))
            .filter(|s| s.gen_.matches(id.key))
            .and_then(|s| s.value.as_mut())
    }

    /// Mutable access to two values at once. If `a == b` the result is `(Some, None)`.
    pub fn get2_mut(&mut self, a: Id<T>, b: Id<T>) -> (Option<&mut T>, Option<&mut T>) {
        if a == b {
            return (self.get_mut(a), None);
        }
        let valid = |slots: &[Slot<T>], id: Id<T>| {
            slots
                .get(usize::from(id.index()))
                .is_some_and(|s| s.gen_.matches(id.key) && s.value.is_some())
        };
        match (valid(&self.slots, a), valid(&self.slots, b)) {
            (false, false) => (None, None),
            (true, false) => (self.get_mut(a), None),
            (false, true) => (None, self.get_mut(b)),
            (true, true) => {
                // Both live, so their indices differ (one slot holds one value) and both are
                // in range: the `else` arm is unreachable.
                let (ia, ib) = (usize::from(a.index()), usize::from(b.index()));
                let (lo, hi) = (ia.min(ib), ia.max(ib));
                let Some((left, right)) = self.slots.split_at_mut_checked(hi) else {
                    return (None, None);
                };
                let (Some(slot_lo), Some(slot_hi)) = (left.get_mut(lo), right.first_mut()) else {
                    return (None, None);
                };
                let (sa, sb) = if ia < ib {
                    (slot_lo, slot_hi)
                } else {
                    (slot_hi, slot_lo)
                };
                (sa.value.as_mut(), sb.value.as_mut())
            }
        }
    }

    /// Whether [`insert`](Self::insert) would fail with [`Error::CapacityExceeded`]: no free
    /// slot is left and the arena already has [`MAX_LEN`](Self::MAX_LEN) slots. Lets a caller
    /// refuse a value *before* building it (`insert` drops the value it cannot store, which
    /// matters when its `Drop` must not run in the caller's context). Never panics.
    ///
    /// ```
    /// use twine_core::Arena;
    /// let mut a: Arena<u8> = Arena::new();
    /// assert!(!a.is_full());
    /// a.insert(1).unwrap();
    /// assert!(!a.is_full());
    /// ```
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.free.is_empty() && self.reserve.is_empty() && self.slots.len() >= Self::MAX_LEN
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
        // Slot indices fit `u16` (at most `MAX_LEN` slots).
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.value.as_ref().map(|v| (Id::from_key(s.gen_.key(i as u16)), v)))
    }

    /// Live values with their ids, mutably, in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Id<T>, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| {
            let id = Id::from_key(s.gen_.key(i as u16));
            s.value.as_mut().map(|v| (id, v))
        })
    }

    /// Removes every value. All existing ids become stale (slots are kept and their
    /// generations bumped, so no id can ever resolve to a later value).
    pub fn clear(&mut self) {
        // Slot indices fit `u16` (at most `MAX_LEN` slots).
        let end = u16::try_from(self.slots.len()).unwrap_or(u16::MAX);
        for index in 0..end {
            let live = self
                .slots
                .get(usize::from(index))
                .filter(|s| s.value.is_some())
                .map(|s| s.gen_.key(index));
            if let Some(key) = live {
                self.remove(Id::from_key(key));
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
    /// inline plus its [`Generation`]) and the two free-index lists, by capacity. Memory a `T`
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
        let slots = self
            .slots
            .capacity()
            .saturating_mul(core::mem::size_of::<Slot<T>>());
        let free = self
            .free
            .capacity()
            .saturating_add(self.reserve.capacity())
            .saturating_mul(core::mem::size_of::<u16>());
        slots.saturating_add(free)
    }

    /// Number of **retired** slots: slots that served all [`MAX_GENERATION`] generations
    /// (65 535 by default, ≈ 2.8 × 10¹⁴ in the safety profile) and are never reused (see the
    /// [generation budget](Self#generation-width-and-budget)). Each one is dead memory until
    /// the arena is dropped; a non-zero value on a device means the arena's working set is too
    /// small for its churn (by default) — in the safety profile it stays 0 in practice.
    ///
    /// ```
    /// use twine_core::Arena;
    /// let mut a = Arena::new();
    /// let mut id = a.insert(()).unwrap();
    /// for _ in 0..1_000 {
    ///     a.remove(id);
    ///     id = a.insert(()).unwrap();
    /// }
    /// assert_eq!(a.retired(), 0); // far below the budget of one slot
    /// assert_eq!(a.slot_count(), 1);
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

    /// Gives slot `index` generation `g` (the slot must be free): reaches the end of the
    /// generation range directly, in both widths.
    fn age_slot<T>(a: &mut Arena<T>, index: u16, g: Generation) {
        let slot = a.slots.get_mut(usize::from(index)).unwrap();
        assert!(slot.value.is_none());
        slot.gen_.set(index, g);
    }

    /// The retirement path, in the width of this build: the last generation is handed out,
    /// then the slot retires instead of wrapping, and later values get a new slot.
    #[test]
    fn last_generation_retires_the_slot() {
        let mut a = Arena::new();
        let first = a.insert(0u8).unwrap();
        a.remove(first).unwrap();
        age_slot(&mut a, 0, MAX_GENERATION - 1);
        let before_last = a.insert(1).unwrap();
        assert_eq!(before_last.generation(), MAX_GENERATION - 1);
        a.remove(before_last).unwrap();
        let last = a.insert(2).unwrap();
        assert_eq!((last.index(), last.generation()), (0, MAX_GENERATION));
        assert_eq!(a.retired(), 0);
        a.remove(last).unwrap();
        assert_eq!(a.retired(), 1);
        let next = a.insert(3).unwrap();
        assert_eq!(next.index(), 1, "retired slot 0 must not be reused");
        assert_eq!((a.retired(), a.slot_count(), a.len()), (1, 2, 1));
        // No id of the retired slot resolves again, whatever its generation.
        for id in [first, before_last, last, Id::from_raw(0)] {
            assert_eq!(a.get(id), None);
            assert_eq!(a.remove(id), None);
        }
        // `clear` skips the retired slot.
        a.clear();
        assert_eq!((a.len(), a.retired()), (0, 1));
    }

    /// A generation that reaches a multiple of `ROTATE_PERIOD` near the end of the range is
    /// parked, not retired: only `MAX_GENERATION` retires.
    #[test]
    fn rotation_before_the_last_generation_parks() {
        let mut a = Arena::new();
        let x = a.insert(0u8).unwrap();
        let y = a.insert(0u8).unwrap();
        a.remove(x).unwrap();
        a.remove(y).unwrap();
        // Slot 0's next generation is the last multiple of ROTATE_PERIOD below the wrap.
        let period = Generation::from(Arena::<u8>::ROTATE_PERIOD);
        let parked_at = MAX_GENERATION - (period - 1);
        assert_eq!(parked_at % period, 0);
        age_slot(&mut a, 0, parked_at - 1);
        // LIFO: slot 1 (freed last) first, then slot 0.
        let other = a.insert(1).unwrap();
        let old = a.insert(2).unwrap();
        assert_eq!((other.index(), old.index()), (1, 0));
        assert_eq!(old.generation(), parked_at - 1);
        a.remove(other).unwrap();
        a.remove(old).unwrap(); // generation `parked_at`: parked in the reserve
        assert_eq!(a.retired(), 0);
        // The stack holds slot 1; slot 0 comes back from the reserve once the stack is empty.
        let p = a.insert(3).unwrap();
        let q = a.insert(4).unwrap();
        assert_eq!((p.index(), q.index()), (1, 0));
        assert_eq!(q.generation(), parked_at);
        assert_eq!(a.slot_count(), 2);
    }

    /// The default profile's full churn: 65 534 reuses of one slot reach `u16::MAX`, the next
    /// removal retires it (the loop is the real path, not a shortcut).
    #[cfg(not(feature = "safety"))]
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

    /// The safety profile's wide generations: the same churn that retires a slot by default
    /// leaves it in service, past the 16-bit range.
    #[cfg(feature = "safety")]
    #[test]
    fn wide_generations_outlive_the_16_bit_range() {
        let mut a = Arena::new();
        let first = a.insert(0u8).unwrap();
        let mut id = first;
        for _ in 0..70_000 {
            a.remove(id).unwrap();
            id = a.insert(0).unwrap();
            assert_eq!(id.index(), 0);
        }
        assert_eq!(id.generation(), 70_001);
        assert_eq!((a.retired(), a.slot_count()), (0, 1));
        assert_eq!(a.get(first), None);
        // A handle whose generation matches the live one only in its low 16 bits is stale.
        let low_bits_alias = Id::<u8>::from_raw(RawId::from(id.generation() & 0xFFFF) << 16);
        assert_ne!(low_bits_alias, id);
        assert_eq!(a.get(low_bits_alias), None);
    }

    /// The safety profile's generation crosses 2³² (the carry from the stored low half into
    /// the high half, on the cold path): the new handle resolves, and a stale handle equal
    /// to it in the low 32 bits does not.
    #[cfg(feature = "safety")]
    #[test]
    fn generation_carries_past_32_bits() {
        let mut a = Arena::new();
        let x = a.insert(0u8).unwrap();
        a.remove(x).unwrap();
        let below = (1u64 << 32) - 1;
        age_slot(&mut a, 0, below);
        let old = a.insert(1).unwrap();
        assert_eq!(old.generation(), below);
        a.remove(old).unwrap(); // next = 2³²: a multiple of ROTATE_PERIOD, parked
        let new = a.insert(2).unwrap();
        assert_eq!(new.generation(), 1 << 32);
        assert_eq!(a.get(new), Some(&2));
        assert_eq!(a.get(old), None);
        let alias = Id::<u8>::from_parts(0, 0); // same low 32 bits as 2³²
        assert_eq!(a.get(alias), None);
        // The niche: generations above `MAX_GENERATION` do not exist as handles.
        assert_eq!(Id::<u8>::from_parts(0, MAX_GENERATION + 1), Id::INVALID);
        assert_eq!(Id::<u8>::from_raw(RawId::MAX), Id::INVALID);
        assert!(Some(new).is_some() && core::mem::size_of_val(&Some(new)) == 8);
    }

    #[test]
    fn raw_round_trip_keeps_the_full_generation() {
        let id: Id<u8> = Id::from_parts(7, MAX_GENERATION);
        assert_eq!(id.generation(), MAX_GENERATION);
        assert_eq!(Id::<u8>::from_raw(id.to_raw()), id);
        assert_eq!(id.to_raw() >> 16, RawId::from(MAX_GENERATION));
        assert_eq!(next_generation(MAX_GENERATION), None);
        assert_eq!(next_generation(MAX_GENERATION - 1), Some(MAX_GENERATION));
        assert_eq!(Id::<u8>::INVALID.to_raw(), 0xFFFF);
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
