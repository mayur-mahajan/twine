//! Faults: the vocabulary every Twine crate uses to report a failure that the library recovered
//! from but that may leave the display wrong, stale or incomplete ([`FaultKind`], [`Faults`]).
//!
//! A fault is not an error returned to a caller: it is something that went wrong while Twine was
//! working on its own (drawing, flushing, running bindings) and that the application must be able
//! to observe. Twine provides the **mechanism** — every fault is raised, counted and reported
//! through one hook — and leaves the **policy** to the application: how severe a fault is, how to
//! react (retry, show a fail-safe screen, reset) and how to report it depends on the product and
//! the standard it follows, so Twine assigns no severities.
//!
//! The crates that detect faults record them where they occur (the engine in its fault state,
//! the reactive runtime in its own); the view layer's `Ui` merges them into one stream.
//!
//! ```
//! use twine_core::fault::{FaultKind, Faults};
//!
//! let mut f = Faults::empty();
//! f.insert(FaultKind::FlushError);
//! f.insert(FaultKind::EffectLoopCut);
//! assert!(f.contains(FaultKind::FlushError));
//! assert_eq!(f.iter().collect::<Vec<_>>(), [FaultKind::FlushError, FaultKind::EffectLoopCut]);
//! assert_eq!(format!("{f:?}"), "Faults(FlushError | EffectLoopCut)");
//! ```

use core::fmt;
use core::ops::{BitOr, BitOrAssign};

/// The kind of a fault.
///
/// Kinds are stable: a kind keeps its [`index`](Self::index) forever, new kinds are appended.
/// Match with a wildcard arm (the enum is `#[non_exhaustive]`).
///
/// Each variant documents what the fault record (`twine_engine::FaultRecord`) raised for it
/// carries: its detail `code`, its `occurrences`, and whether a display, a node or an input
/// device is attached. A `code` of `0` means "no detail".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
#[non_exhaustive]
pub enum FaultKind {
    /// A display driver failed to start a flush or to present a framebuffer (or the
    /// application reported a failed flush with `Engine::report_flush`): the area it was given
    /// may not show what was rendered.
    ///
    /// Record: `display` set; `code` = the driver's error code (`DriverErrorCode::get`, `0`
    /// when the driver does not classify its errors); one occurrence per failed flush.
    FlushError = 0,
    /// A display driver did not complete a flush (or a framebuffer swap) within
    /// `EngineConfig::flush_timeout`: the display is considered failed.
    ///
    /// Record: `display` set; `code` = the milliseconds waited (saturated to `u32::MAX`).
    FlushTimeout = 1,
    /// The reactive runtime cut an effect loop: pending bindings were dropped, so some widget
    /// properties may show outdated values.
    ///
    /// Record (forwarded by the view layer's `Ui` at each update): no display, node or input;
    /// `code` = `0`; `occurrences` = the number of cuts since the last update.
    EffectLoopCut = 2,
    /// The reactive runtime's depth guard triggered (a very deep memo chain): values stay
    /// correct, but extra work was done.
    ///
    /// Record (forwarded by `Ui` like [`EffectLoopCut`](Self::EffectLoopCut)): no display,
    /// node or input; `code` = `0`; `occurrences` = the number of triggers since the last
    /// update.
    DepthGuard = 3,
    /// A fixed capacity was reached (nodes, reactive nodes, queues): something was not created
    /// or was dropped.
    ///
    /// Record: `code` names the bound — `0` for the engine's own capacities (a full widget
    /// tree: `node` = the parent the node could not be created under, if any), or a view-layer
    /// `twine_view::CapacityFault` code (`1` = the `Ui`'s engine command queue: `display` set,
    /// `occurrences` = the number of dropped commands).
    Capacity = 4,
    /// The view layer could not build a widget it was asked to build.
    ///
    /// Record: `code` = the `twine_view::BuildFailure` code (`1` = capacity, `2` = parent gone,
    /// `3` = another engine error); `node` = the parent the widget was built under (not set
    /// when the `Ui` could not even create its root).
    BuildFailed = 5,
    /// A display uses a colour format whose renderer is not compiled in.
    ///
    /// Record: `code` = the refused [`ColorFormat`](crate::ColorFormat) as `u8`
    /// (`ColorFormat as u8`); no display (the display was not added).
    FormatDisabled = 6,
    /// An input device reported a failure through its health (`InputDevice::health`): it
    /// became degraded (some reads failed) or failed (its samples are processed as released).
    /// Raised on the transition only, not for every failed read.
    ///
    /// Record: `input` set; `code` = `1` (became degraded) or `2` (failed).
    InputDevice = 7,
    /// A channel into the UI was full: messages were dropped.
    ///
    /// Record (forwarded by `Ui`): no display, node or input; `code` = `0`; `occurrences` =
    /// the number of dropped messages since the last update.
    ChannelOverflow = 8,
    /// A draw accelerator (e.g. DMA2D) did not finish an operation within its bound; the
    /// operation was drawn in software.
    ///
    /// Record: `display` set (the display whose chunk was redrawn); `code` = `0`;
    /// `occurrences` = the number of timed-out operations in that chunk.
    AccelTimeout = 9,
}

impl FaultKind {
    /// Every kind, in [`index`](Self::index) order.
    pub const ALL: [FaultKind; 10] = [
        FaultKind::FlushError,
        FaultKind::FlushTimeout,
        FaultKind::EffectLoopCut,
        FaultKind::DepthGuard,
        FaultKind::Capacity,
        FaultKind::BuildFailed,
        FaultKind::FormatDisabled,
        FaultKind::InputDevice,
        FaultKind::ChannelOverflow,
        FaultKind::AccelTimeout,
    ];

    /// The number of kinds (the length of per-kind tables such as fault counters).
    pub const COUNT: usize = Self::ALL.len();

    /// The kind's stable index (`0..COUNT`), e.g. for per-kind tables or diagnostic codes.
    ///
    /// ```
    /// use twine_core::fault::FaultKind;
    /// assert_eq!(FaultKind::FlushError.index(), 0);
    /// assert_eq!(FaultKind::from_index(FaultKind::ChannelOverflow.index()), Some(FaultKind::ChannelOverflow));
    /// ```
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The kind with `index`, `None` if out of range.
    /// ```
    /// use twine_core::fault::FaultKind;
    /// assert_eq!(FaultKind::from_index(4), Some(FaultKind::Capacity));
    /// assert_eq!(FaultKind::from_index(FaultKind::COUNT), None);
    /// ```
    #[must_use]
    pub const fn from_index(index: usize) -> Option<FaultKind> {
        if index < Self::COUNT {
            Some(Self::ALL[index])
        } else {
            None
        }
    }

    /// The kind's name, as written in the source (`"FlushError"`, …).
    /// ```
    /// use twine_core::fault::FaultKind;
    /// assert_eq!(FaultKind::InputDevice.name(), "InputDevice");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            FaultKind::FlushError => "FlushError",
            FaultKind::FlushTimeout => "FlushTimeout",
            FaultKind::EffectLoopCut => "EffectLoopCut",
            FaultKind::DepthGuard => "DepthGuard",
            FaultKind::Capacity => "Capacity",
            FaultKind::BuildFailed => "BuildFailed",
            FaultKind::FormatDisabled => "FormatDisabled",
            FaultKind::InputDevice => "InputDevice",
            FaultKind::ChannelOverflow => "ChannelOverflow",
            FaultKind::AccelTimeout => "AccelTimeout",
        }
    }

    const fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

impl fmt::Display for FaultKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// `Faults` is a `u32` bit set: adding a 33rd kind must widen it.
const _: () = assert!(FaultKind::COUNT <= 32);

/// A set of [`FaultKind`]s (e.g. the faults raised since they were last taken).
///
/// ```
/// use twine_core::fault::{FaultKind, Faults};
///
/// let f = Faults::from(FaultKind::Capacity) | FaultKind::BuildFailed;
/// assert_eq!(f.len(), 2);
/// assert!(!f.is_empty());
/// assert!(Faults::empty().is_empty());
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Faults(u32);

impl Faults {
    /// No faults.
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// assert!(Faults::empty().is_empty());
    /// assert_eq!(Faults::empty(), Faults::default());
    /// ```
    #[must_use]
    pub const fn empty() -> Faults {
        Faults(0)
    }

    /// Whether the set holds no kind.
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// assert!(Faults::empty().is_empty());
    /// assert!(!Faults::from(FaultKind::Capacity).is_empty());
    /// ```
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The number of kinds in the set.
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// assert_eq!((Faults::from(FaultKind::Capacity) | FaultKind::FlushError).len(), 2);
    /// ```
    #[must_use]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Whether `kind` is in the set.
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// let f = Faults::from(FaultKind::FlushTimeout);
    /// assert!(f.contains(FaultKind::FlushTimeout));
    /// assert!(!f.contains(FaultKind::FlushError));
    /// ```
    #[must_use]
    pub const fn contains(self, kind: FaultKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Adds `kind`.
    /// Idempotent: inserting a kind already in the set changes nothing.
    ///
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// let mut f = Faults::empty();
    /// f.insert(FaultKind::Capacity);
    /// f.insert(FaultKind::Capacity);
    /// assert_eq!(f.len(), 1);
    /// ```
    pub fn insert(&mut self, kind: FaultKind) {
        self.0 |= kind.bit();
    }

    /// Removes `kind`.
    /// Removing a kind that is not in the set changes nothing.
    ///
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// let mut f = Faults::from(FaultKind::Capacity) | FaultKind::BuildFailed;
    /// f.remove(FaultKind::Capacity);
    /// assert_eq!(f, Faults::from(FaultKind::BuildFailed));
    /// ```
    pub fn remove(&mut self, kind: FaultKind) {
        self.0 &= !kind.bit();
    }

    /// The kinds of both sets.
    /// Same as `self | other`.
    ///
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// let a = Faults::from(FaultKind::Capacity);
    /// let b = Faults::from(FaultKind::FlushError);
    /// assert_eq!(a.union(b), a | b);
    /// assert_eq!(a.union(b).len(), 2);
    /// ```
    #[must_use]
    pub const fn union(self, other: Faults) -> Faults {
        Faults(self.0 | other.0)
    }

    /// The kinds in the set, in [`FaultKind::index`] order.
    /// ```
    /// use twine_core::fault::{FaultKind, Faults};
    /// let f = Faults::from(FaultKind::ChannelOverflow) | FaultKind::FlushError;
    /// let kinds: Vec<_> = f.iter().collect();
    /// assert_eq!(kinds, [FaultKind::FlushError, FaultKind::ChannelOverflow]);
    /// ```
    pub fn iter(self) -> impl Iterator<Item = FaultKind> {
        FaultKind::ALL.into_iter().filter(move |k| self.contains(*k))
    }
}

impl From<FaultKind> for Faults {
    fn from(kind: FaultKind) -> Faults {
        Faults(kind.bit())
    }
}

impl BitOr for Faults {
    type Output = Faults;
    fn bitor(self, rhs: Faults) -> Faults {
        self.union(rhs)
    }
}

impl BitOr<FaultKind> for Faults {
    type Output = Faults;
    fn bitor(self, rhs: FaultKind) -> Faults {
        self.union(rhs.into())
    }
}

impl BitOrAssign for Faults {
    fn bitor_assign(&mut self, rhs: Faults) {
        *self = self.union(rhs);
    }
}

impl BitOrAssign<FaultKind> for Faults {
    fn bitor_assign(&mut self, rhs: FaultKind) {
        self.insert(rhs);
    }
}

impl FromIterator<FaultKind> for Faults {
    fn from_iter<I: IntoIterator<Item = FaultKind>>(iter: I) -> Faults {
        let mut f = Faults::empty();
        for k in iter {
            f.insert(k);
        }
        f
    }
}

impl fmt::Debug for Faults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Faults(")?;
        for (i, k) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(" | ")?;
            }
            f.write_str(k.name())?;
        }
        f.write_str(")")
    }
}

/// Saturating per-kind occurrence counters (how often each fault happened since start-up).
///
/// ```
/// use twine_core::fault::{FaultCounts, FaultKind};
///
/// let mut c = FaultCounts::new();
/// c.add(FaultKind::FlushError, 2);
/// c.add(FaultKind::FlushError, u32::MAX); // saturates
/// assert_eq!(c.get(FaultKind::FlushError), u32::MAX);
/// assert_eq!(c.get(FaultKind::Capacity), 0);
/// assert_eq!(c.total(), u32::MAX);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FaultCounts([u32; FaultKind::COUNT]);

impl FaultCounts {
    /// All counters at zero.
    /// ```
    /// use twine_core::fault::{FaultCounts, FaultKind};
    /// assert_eq!(FaultCounts::new().total(), 0);
    /// ```
    #[must_use]
    pub const fn new() -> FaultCounts {
        FaultCounts([0; FaultKind::COUNT])
    }

    /// The occurrences of `kind`.
    /// ```
    /// use twine_core::fault::{FaultCounts, FaultKind};
    /// let mut c = FaultCounts::new();
    /// c.add(FaultKind::Capacity, 3);
    /// assert_eq!(c.get(FaultKind::Capacity), 3);
    /// assert_eq!(c.get(FaultKind::FlushError), 0);
    /// ```
    #[must_use]
    pub const fn get(&self, kind: FaultKind) -> u32 {
        self.0[kind.index()]
    }

    /// Adds `n` occurrences of `kind` (saturating).
    /// Never panics: a counter stops at `u32::MAX`.
    ///
    /// ```
    /// use twine_core::fault::{FaultCounts, FaultKind};
    /// let mut c = FaultCounts::new();
    /// c.add(FaultKind::FlushError, u32::MAX - 1);
    /// c.add(FaultKind::FlushError, 5);
    /// assert_eq!(c.get(FaultKind::FlushError), u32::MAX);
    /// ```
    pub fn add(&mut self, kind: FaultKind, n: u32) {
        let c = &mut self.0[kind.index()];
        *c = c.saturating_add(n);
    }

    /// The occurrences of every kind together (saturating).
    /// ```
    /// use twine_core::fault::{FaultCounts, FaultKind};
    /// let mut c = FaultCounts::new();
    /// c.add(FaultKind::Capacity, 2);
    /// c.add(FaultKind::InputDevice, 1);
    /// assert_eq!(c.total(), 3);
    /// ```
    #[must_use]
    pub fn total(&self) -> u32 {
        self.0.iter().fold(0u32, |a, &b| a.saturating_add(b))
    }

    /// The kinds that occurred at least once.
    /// ```
    /// use twine_core::fault::{FaultCounts, FaultKind, Faults};
    /// let mut c = FaultCounts::new();
    /// c.add(FaultKind::Capacity, 2);
    /// c.add(FaultKind::FlushError, 0); // zero occurrences: not listed
    /// assert_eq!(c.kinds(), Faults::from(FaultKind::Capacity));
    /// ```
    #[must_use]
    pub fn kinds(&self) -> Faults {
        FaultKind::ALL.into_iter().filter(|k| self.get(*k) > 0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec::Vec;

    #[test]
    fn indices_are_stable_and_dense() {
        for (i, k) in FaultKind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
            assert_eq!(FaultKind::from_index(i), Some(*k));
        }
        assert_eq!(FaultKind::from_index(FaultKind::COUNT), None);
    }

    #[test]
    fn set_operations() {
        let mut f = Faults::empty();
        assert!(f.is_empty());
        f |= FaultKind::ChannelOverflow;
        f |= Faults::from(FaultKind::FlushError);
        assert_eq!(f.len(), 2);
        assert_eq!(
            f.iter().collect::<Vec<_>>(),
            [FaultKind::FlushError, FaultKind::ChannelOverflow]
        );
        f.remove(FaultKind::FlushError);
        assert!(!f.contains(FaultKind::FlushError));
        assert_eq!(f, Faults::from(FaultKind::ChannelOverflow));
        let all: Faults = FaultKind::ALL.into_iter().collect();
        assert_eq!(all.len(), FaultKind::COUNT);
    }

    #[test]
    fn formatting() {
        assert_eq!(format!("{:?}", Faults::empty()), "Faults()");
        assert_eq!(format!("{}", FaultKind::DepthGuard), "DepthGuard");
        let f = Faults::from(FaultKind::Capacity) | FaultKind::BuildFailed;
        assert_eq!(format!("{f:?}"), "Faults(Capacity | BuildFailed)");
    }

    #[test]
    fn counts() {
        let mut c = FaultCounts::new();
        c.add(FaultKind::Capacity, 3);
        c.add(FaultKind::InputDevice, 1);
        assert_eq!(c.get(FaultKind::Capacity), 3);
        assert_eq!(c.total(), 4);
        assert_eq!(
            c.kinds(),
            Faults::from(FaultKind::Capacity) | FaultKind::InputDevice
        );
    }
}
