//! [`Partition`]: the identity of a partitioned runtime (ADR-001, `docs/design/adr-001-partitioned-runtimes.md`).
//!
//! A **partition** is one reactive runtime and every `Ui` (engine, displays, inputs) built on
//! it. Two partitions in one image share no mutable state: each has its own reactive arenas,
//! effect queue and loop cut, [`Limits`](crate::Limits), fault stream, seal domain
//! ([`alloc_guard`](crate::alloc_guard)), step budget, heartbeat, frame observer and fail-safe
//! screen. A safety-critical display and a non-critical one become independent: a loop cut,
//! an exhausted bound or a fault in one cannot change what the other shows or when.
//!
//! The type lives in `twine-core` because both the engine and the reactive runtime need it
//! (like [`Limits`](crate::Limits)). The runtime's token is the partition's capability
//! (`twine_reactive::Runtime::take_partition`); this type only names it.
//!
//! # Feature `partitions`
//!
//! Partitions are opt-in. Without the feature [`Partition::COUNT`] is 1, the type is
//! zero-sized and every per-partition table has one entry, so an image with one partition
//! compiles to the code it had before partitions existed (no byte per handle, no slot
//! selection). With it, [`Partition::COUNT`] is 2 and a partition is one byte.
//!
//! ```
//! use twine_core::Partition;
//!
//! assert_eq!(Partition::MAIN.index(), 0);
//! assert_eq!(Partition::new(0), Some(Partition::MAIN));
//! assert_eq!(Partition::new(Partition::COUNT as u8), None); // out of range
//! let table = [10u32; Partition::COUNT];
//! assert_eq!(*Partition::MAIN.select(&table), 10);
//! ```

/// The index of a partition: an enum, so it is provably below [`Partition::COUNT`] (a slot
/// selection needs no bounds check, and `Option<Partition>` and handles holding one keep a
/// niche).
#[cfg(feature = "partitions")]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
enum Index {
    P0 = 0,
    P1 = 1,
}

/// Without `partitions`: one partition, zero-sized.
#[cfg(not(feature = "partitions"))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum Index {
    P0 = 0,
}

/// A partition: one reactive runtime and the UIs built on it (see the [module docs](self)).
///
/// `Copy`, compared and hashed by index. One byte with the `partitions` feature, zero-sized
/// without. [`Partition::MAIN`] (index 0) always exists; with `partitions` there are
/// [`Partition::COUNT`] = 2 (a critical and a non-critical partition: each per-partition table
/// costs RAM in every image with the feature, ADR-001 § 6.2).
///
/// A `Partition` value is only a name: holding one grants no access. Access to a partition's
/// runtime needs its token (`twine_reactive::Runtime::take_partition`), which is claimed once.
///
/// ```
/// use twine_core::Partition;
///
/// let all: Vec<u8> = Partition::ALL.iter().map(|p| p.index()).collect();
/// assert_eq!(all.len(), Partition::COUNT);
/// assert_eq!(all[0], 0);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Partition(Index);

impl Partition {
    /// The number of partitions: 2 with the `partitions` feature, 1 without. A power of two.
    pub const COUNT: usize = if cfg!(feature = "partitions") { 2 } else { 1 };

    /// The main partition (index 0): what `Runtime::take()` claims and what a single-partition
    /// application uses everywhere.
    pub const MAIN: Partition = Partition(Index::P0);

    /// Every partition, in index order.
    #[cfg(feature = "partitions")]
    pub const ALL: [Partition; Partition::COUNT] = [Partition(Index::P0), Partition(Index::P1)];

    /// Every partition, in index order.
    #[cfg(not(feature = "partitions"))]
    pub const ALL: [Partition; Partition::COUNT] = [Partition(Index::P0)];

    /// The partition with `index`, or `None` when `index >= COUNT` (without the `partitions`
    /// feature: `Some` for 0 only). Never panics.
    ///
    /// ```
    /// use twine_core::Partition;
    /// assert_eq!(Partition::new(0), Some(Partition::MAIN));
    /// assert_eq!(Partition::new(200), None);
    /// ```
    #[must_use]
    pub const fn new(index: u8) -> Option<Partition> {
        #[cfg(feature = "partitions")]
        let p = match index {
            0 => Index::P0,
            1 => Index::P1,
            _ => return None,
        };
        #[cfg(not(feature = "partitions"))]
        let p = match index {
            0 => Index::P0,
            _ => return None,
        };
        Some(Partition(p))
    }

    /// The partition's index (`0..COUNT`). Never panics.
    ///
    /// ```
    /// assert_eq!(twine_core::Partition::MAIN.index(), 0);
    /// ```
    #[must_use]
    #[inline]
    pub const fn index(self) -> u8 {
        self.0 as u8
    }

    /// This partition's entry of a per-partition table (`[T; COUNT]`): how the runtime selects
    /// its state slot and the allocation guard its seal record. The index is an enum below
    /// `COUNT`, so this compiles to an address computation (no bounds check, no panic); without
    /// the `partitions` feature it is the table's only entry.
    ///
    /// ```
    /// use twine_core::Partition;
    /// let names = ["main"; Partition::COUNT];
    /// assert_eq!(*Partition::MAIN.select(&names), "main");
    /// ```
    #[must_use]
    #[inline(always)]
    pub fn select<T>(self, table: &[T; Partition::COUNT]) -> &T {
        let [first, ..] = table;
        // The index is below `COUNT` by its type: the fallback is never taken and folds away.
        table.get(usize::from(self.index())).unwrap_or(first)
    }
}

impl Default for Partition {
    /// [`Partition::MAIN`].
    fn default() -> Self {
        Partition::MAIN
    }
}

impl core::fmt::Display for Partition {
    /// `partition <index>`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "partition {}", self.index())
    }
}

// Sizes (ADR-001 § 6.2): zero-sized without the feature, one byte with it; `COUNT` is a power
// of two in both configurations.
const _: () = {
    assert!(Partition::COUNT.is_power_of_two());
    assert!(core::mem::size_of::<Partition>() == if cfg!(feature = "partitions") { 1 } else { 0 });
    assert!(core::mem::size_of::<Option<Partition>>() == 1);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_round_trip_and_out_of_range_is_none() {
        for (i, p) in Partition::ALL.iter().enumerate() {
            assert_eq!(usize::from(p.index()), i);
            assert_eq!(Partition::new(p.index()), Some(*p));
        }
        for i in Partition::COUNT..=255 {
            assert_eq!(Partition::new(i as u8), None);
        }
        let table: [usize; Partition::COUNT] = core::array::from_fn(|i| i * 10);
        for p in Partition::ALL {
            assert_eq!(*p.select(&table), usize::from(p.index()) * 10);
        }
        assert_eq!(Partition::default(), Partition::MAIN);
    }
}
