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
    /// A bound was reached (a [`Limits`](crate::Limits) field or a fixed capacity): something
    /// was not created, was cut or was dropped — never grown.
    ///
    /// Record: `code` is the [`CapacityFault`] code naming the bound (e.g. `0` = the widget
    /// tree, `node` = the parent the node could not be created under; `1` = a `Ui`'s engine
    /// command queue, `display` set, `occurrences` = the dropped commands; `3`/`4` = the
    /// reactive runtime's node / scope bound, forwarded by the `Ui`). See each
    /// [`CapacityFault`] variant.
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
    /// A display control (brightness, sleep, wake, rotation) requested from the engine was
    /// not applied: the driver does not support it, the driver failed, or a rotation does
    /// not fit the display's buffers. The display keeps its previous state (except that a
    /// display put to sleep stops being drawn even when its driver cannot sleep).
    ///
    /// Record: `display` set; `code` = the `twine_engine::DisplayControlFault` code (which
    /// control, and why).
    DisplayControl = 10,
    /// A reactive handle (signal, memo, stored value, effect or scope) was used after its scope
    /// was disposed, or a node was created in a disposed scope. Raised by the reactive
    /// runtime's `no-panic-runtime` feature (implied by the `safety` profile), which skips the
    /// write or creation instead of panicking: the value written is dropped, and a creation
    /// returns a handle that is not alive. A read that has no value to return still panics
    /// after raising the fault (see `twine_reactive::ReactiveError`).
    ///
    /// Record (forwarded by `Ui` like [`EffectLoopCut`](Self::EffectLoopCut)): no display,
    /// node or input; `code` = `0`; `occurrences` = the number of skipped operations since the
    /// last update.
    DisposedHandle = 11,
    /// A memo read itself, directly or through other memos (a dependency cycle). With the
    /// reactive runtime's `no-panic-runtime` feature the read returns the memo's previous value
    /// instead of panicking (a memo with no previous value still panics after raising the
    /// fault); without it, the read panics.
    ///
    /// Record (forwarded by `Ui` like [`EffectLoopCut`](Self::EffectLoopCut)): no display,
    /// node or input; `code` = `0`; `occurrences` = the number of cyclic reads since the last
    /// update.
    MemoCycle = 12,
    /// A signal or stored value was written while its value was borrowed by an enclosing
    /// access of the same value (e.g. `set` inside `with` on the same signal). With the
    /// reactive runtime's `no-panic-runtime` feature the write is skipped instead of panicking.
    ///
    /// Record (forwarded by `Ui` like [`EffectLoopCut`](Self::EffectLoopCut)): no display,
    /// node or input; `code` = `0`; `occurrences` = the number of skipped writes since the
    /// last update.
    ReentrantAccess = 13,
    /// Twine allocated heap memory after the application **sealed** it
    /// ([`alloc_guard::seal`](crate::alloc_guard::seal), `Ui::seal`): a growth point that
    /// should have been sized at start-up (from [`Limits`](crate::Limits)) grew, or something
    /// was created after start-up. The allocation was made (refusing it would lose data or
    /// break an invariant); the fault is the evidence that the "no allocation after start-up"
    /// claim did not hold.
    ///
    /// Record (raised by the view layer's `Ui` at each update, or by
    /// `Engine::take_faults`): no display, node or input; `code` = the
    /// [`AllocSite`](crate::alloc_guard::AllocSite) code of the **last** growth point;
    /// `occurrences` = the number of growths since the previous record.
    AllocAfterSeal = 14,
    /// An update used up the time of its step budget (`StepBudget::time`, e.g.
    /// `EngineConfig::step_budget`): rendering stopped after the chunk that reached it, and
    /// the rest of the frame continues in the next update (`Wake::Now`) — deferred, never
    /// dropped. Also raised when an update that deferred nothing still ended past its time
    /// (a long layout or binding run, one slow chunk). Measured with the engine's
    /// `hires_timer`; at most one record per update.
    ///
    /// Record: `display` = the display whose frame was cut short (not set when nothing was
    /// deferred); `code` = the update's time in µs when it was checked (saturated to
    /// `u32::MAX`).
    #[doc(alias = "FRAME_OVERRUN")]
    FrameOverrun = 15,
    /// The application switched a display to its fail-safe screen
    /// (`Engine::enter_fail_safe`, `Ui::enter_fail_safe`): from now on the display shows the
    /// static screen registered at start-up, and loading any other screen is refused until
    /// the application leaves the fail-safe state. Twine raises it so the switch is part of
    /// the fault stream and its evidence; the reason is the application's.
    ///
    /// Record: `display` set; `code` = the application's reason code given to
    /// `enter_fail_safe`.
    FailSafe = 16,
    /// A display-integrity check found that a region of the display does not show what was
    /// rendered: the checksum of the pixels read back from the panel (or its framebuffer)
    /// differs from the checksum of an independent off-screen re-render of the same region
    /// (`Engine::check_region`). What the application does about it (retry, fail-safe screen,
    /// reset) is its policy.
    ///
    /// Record: `display` and `node` set (the node whose region was checked); `code` = the
    /// checksum read back from the panel (the expected one is in the check's result).
    #[doc(alias = "DISPLAY_INTEGRITY")]
    DisplayIntegrity = 17,
    /// A partition tried to register for a cross-context source another partition consumes
    /// (ADR-001 § 5.5): a `Channel` or `Latest` has one consuming partition — the first one
    /// that registered a handler (`on_message`, `on_latest`, `watch`), recorded permanently —
    /// and a registration
    /// from any other partition is refused (no handler, no wake-up routing) instead of
    /// letting two runtimes share the source's waker slot or queue. Recorded in the refused
    /// partition's runtime, so only that partition's `Ui` sees it. Fan out to two partitions
    /// with one cell per consumer, set by the producer.
    ///
    /// Record: no display, no node; `occurrences` = the refused registrations (the `warn!`
    /// names the owning partition).
    #[doc(alias = "PARTITION_VIOLATION")]
    PartitionViolation = 18,
}

impl FaultKind {
    /// Every kind, in [`index`](Self::index) order.
    pub const ALL: [FaultKind; 19] = [
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
        FaultKind::DisplayControl,
        FaultKind::DisposedHandle,
        FaultKind::MemoCycle,
        FaultKind::ReentrantAccess,
        FaultKind::AllocAfterSeal,
        FaultKind::FrameOverrun,
        FaultKind::FailSafe,
        FaultKind::DisplayIntegrity,
        FaultKind::PartitionViolation,
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
        match Self::ALL.split_at_checked(index) {
            Some((_, [kind, ..])) => Some(*kind),
            _ => None,
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
            FaultKind::DisplayControl => "DisplayControl",
            FaultKind::DisposedHandle => "DisposedHandle",
            FaultKind::MemoCycle => "MemoCycle",
            FaultKind::ReentrantAccess => "ReentrantAccess",
            FaultKind::AllocAfterSeal => "AllocAfterSeal",
            FaultKind::FrameOverrun => "FrameOverrun",
            FaultKind::FailSafe => "FailSafe",
            FaultKind::DisplayIntegrity => "DisplayIntegrity",
            FaultKind::PartitionViolation => "PartitionViolation",
        }
    }

    const fn bit(self) -> u32 {
        // SHIFT: the discriminants are the dense indices `0..COUNT` (const assertion below; every
        // variant is in `ALL`: test `every_variant_is_listed_in_all`) and `COUNT ≤ 32`.
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
// The discriminants are the dense indices `0..COUNT` (`FaultKind::bit` shifts by them).
const _: () = {
    let mut i = 0;
    let mut rest: &[FaultKind] = &FaultKind::ALL;
    while let [kind, tail @ ..] = rest {
        assert!(
            *kind as usize == i,
            "FaultKind::ALL must list the kinds in index order"
        );
        i += 1;
        rest = tail;
    }
};

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

/// The bound named by a [`FaultKind::Capacity`] record (its `code`): which limit was reached.
///
/// Codes are stable (a bound keeps its code forever; new bounds are appended). Most bounds are
/// fields of [`Limits`](crate::Limits); the others are fixed capacities of the crate that
/// raises the fault. In every case the operation was refused, cut or dropped — Twine never
/// grows past a bound.
///
/// ```
/// use twine_core::fault::CapacityFault;
///
/// assert_eq!(CapacityFault::EngineQueue.code(), 1);
/// assert_eq!(CapacityFault::from_code(4), Some(CapacityFault::ReactiveScopes));
/// assert_eq!(CapacityFault::from_code(9999), None);
/// assert_eq!(CapacityFault::TreeDepth.name(), "TreeDepth");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u32)]
#[non_exhaustive]
pub enum CapacityFault {
    /// The widget tree is full ([`Limits::max_nodes`](crate::Limits::max_nodes), at most
    /// 65 535): the node was not created. `node` = the parent, if any.
    Nodes = 0,
    /// A `Ui`'s engine command queue was full ([`Limits::engine_queue`](crate::Limits::engine_queue)):
    /// engine side effects issued without the engine (a scope disposed or a handle called
    /// outside `Ui::update`) were dropped. `display` set; `occurrences` = their number.
    EngineQueue = 1,
    /// The process-wide waker pool was exhausted when a `Ui` was mounted
    /// ([`Limits::max_heap_wakers`](crate::Limits::max_heap_wakers)): it got the shared
    /// fallback waker instead of its own, so channel and interrupt wake-ups are no longer
    /// routed to this UI alone. `display` set.
    WakerPool = 2,
    /// The reactive runtime's node bound was reached
    /// ([`Limits::max_reactive_nodes`](crate::Limits::max_reactive_nodes): signals, memos,
    /// stored values and effects): with `no-panic-runtime` the creation was skipped and
    /// returned a handle that is never alive (without it, the creation panics). Forwarded by
    /// the `Ui`; `occurrences` = the skipped creations.
    ReactiveNodes = 3,
    /// The reactive runtime's scope bound was reached
    /// ([`Limits::max_scopes`](crate::Limits::max_scopes)), as for
    /// [`ReactiveNodes`](Self::ReactiveNodes).
    ReactiveScopes = 4,
    /// More effects waited to run than
    /// [`Limits::max_pending_effects`](crate::Limits::max_pending_effects): the effects beyond
    /// it were not queued (like an effect-loop cut, the bindings concerned may show outdated
    /// values until their sources change again). Forwarded by the `Ui`; `occurrences` = the
    /// effects not queued.
    PendingEffects = 5,
    /// A node would have been deeper than [`Limits::max_tree_depth`](crate::Limits::max_tree_depth)
    /// below its root: the creation or move was refused. `node` = the parent.
    TreeDepth = 6,
    /// A widget text was longer than [`Limits::max_text_len`](crate::Limits::max_text_len)
    /// bytes: it was cut at the last character boundary that fits. `node` = the widget.
    TextLen = 7,
    /// The engine's event handlers reached [`Limits::max_handlers`](crate::Limits::max_handlers):
    /// the handler was not added. `node` = the node it was for.
    Handlers = 8,
    /// The engine's timers reached [`Limits::max_timers`](crate::Limits::max_timers): the timer
    /// was not added (its id never resolves).
    Timers = 9,
    /// The engine's running animations reached [`Limits::max_anims`](crate::Limits::max_anims):
    /// the animation was not started (its id never resolves).
    Animations = 10,
    /// More focus groups than the engine's fixed `MAX_GROUPS`: the group was not created.
    FocusGroups = 11,
    /// A navigator's route stack reached [`Limits::max_nav_depth`](crate::Limits::max_nav_depth):
    /// the push was refused (the current page stays).
    NavDepth = 12,
    /// The heap Twine reports for a `Ui` (`MemoryReport::heap_bytes`) exceeded
    /// [`Limits::heap_budget`](crate::Limits::heap_budget) when the `Ui` was sealed.
    /// `occurrences` = 1; nothing was refused (the memory is in use), the record is evidence.
    HeapBudget = 13,
    /// A keyed collection had more rows to show than its bound
    /// ([`Limits::max_list_items`](crate::Limits::max_list_items), or a smaller per-collection
    /// bound): the items beyond it were not shown (a `for_each` shows the first ones, a
    /// `virtual_list` the first visible ones). `node` = the collection.
    ListItems = 14,
    /// The nodes with a theme class of their own reached
    /// [`Limits::max_theme_classes`](crate::Limits::max_theme_classes): the variant or role was
    /// not set and the node keeps its widget class's look. `node` = the node.
    ThemeClasses = 15,
}

impl CapacityFault {
    /// Every bound, in code order.
    pub const ALL: [CapacityFault; 16] = [
        CapacityFault::Nodes,
        CapacityFault::EngineQueue,
        CapacityFault::WakerPool,
        CapacityFault::ReactiveNodes,
        CapacityFault::ReactiveScopes,
        CapacityFault::PendingEffects,
        CapacityFault::TreeDepth,
        CapacityFault::TextLen,
        CapacityFault::Handlers,
        CapacityFault::Timers,
        CapacityFault::Animations,
        CapacityFault::FocusGroups,
        CapacityFault::NavDepth,
        CapacityFault::HeapBudget,
        CapacityFault::ListItems,
        CapacityFault::ThemeClasses,
    ];

    /// The stable numeric code (the fault record's `code`).
    ///
    /// ```
    /// use twine_core::fault::CapacityFault;
    /// assert_eq!(CapacityFault::Nodes.code(), 0);
    /// ```
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// The bound with code `code` (`None` for an unknown code).
    ///
    /// ```
    /// use twine_core::fault::CapacityFault;
    /// assert_eq!(CapacityFault::from_code(1), Some(CapacityFault::EngineQueue));
    /// ```
    #[must_use]
    pub const fn from_code(code: u32) -> Option<CapacityFault> {
        Some(match code {
            0 => CapacityFault::Nodes,
            1 => CapacityFault::EngineQueue,
            2 => CapacityFault::WakerPool,
            3 => CapacityFault::ReactiveNodes,
            4 => CapacityFault::ReactiveScopes,
            5 => CapacityFault::PendingEffects,
            6 => CapacityFault::TreeDepth,
            7 => CapacityFault::TextLen,
            8 => CapacityFault::Handlers,
            9 => CapacityFault::Timers,
            10 => CapacityFault::Animations,
            11 => CapacityFault::FocusGroups,
            12 => CapacityFault::NavDepth,
            13 => CapacityFault::HeapBudget,
            14 => CapacityFault::ListItems,
            15 => CapacityFault::ThemeClasses,
            _ => return None,
        })
    }

    /// The bound's name, as written in the source (`"Nodes"`, …).
    ///
    /// ```
    /// use twine_core::fault::CapacityFault;
    /// assert_eq!(CapacityFault::HeapBudget.name(), "HeapBudget");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            CapacityFault::Nodes => "Nodes",
            CapacityFault::EngineQueue => "EngineQueue",
            CapacityFault::WakerPool => "WakerPool",
            CapacityFault::ReactiveNodes => "ReactiveNodes",
            CapacityFault::ReactiveScopes => "ReactiveScopes",
            CapacityFault::PendingEffects => "PendingEffects",
            CapacityFault::TreeDepth => "TreeDepth",
            CapacityFault::TextLen => "TextLen",
            CapacityFault::Handlers => "Handlers",
            CapacityFault::Timers => "Timers",
            CapacityFault::Animations => "Animations",
            CapacityFault::FocusGroups => "FocusGroups",
            CapacityFault::NavDepth => "NavDepth",
            CapacityFault::HeapBudget => "HeapBudget",
            CapacityFault::ListItems => "ListItems",
            CapacityFault::ThemeClasses => "ThemeClasses",
        }
    }
}

impl fmt::Display for CapacityFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
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
        // SAFE-INDEX: `kind.index() < COUNT`, the array's length (dense discriminants, asserted
        // at compile time); the fallback is never taken.
        match self.0.split_at_checked(kind.index()) {
            Some((_, [c, ..])) => *c,
            _ => 0,
        }
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
        // SAFE-INDEX: `kind.index() < COUNT`, the array's length; the fallback is never taken.
        if let Some(c) = self.0.get_mut(kind.index()) {
            *c = c.saturating_add(n);
        }
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

    /// Every variant is listed in `ALL`, so the dense-index assertion covers every discriminant
    /// and `FaultKind::bit` shifts by less than `COUNT ≤ 32`.
    #[test]
    fn every_variant_is_listed_in_all() {
        let src = include_str!("fault.rs");
        let body = &src[src.find("pub enum FaultKind {").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        let variants = body
            .lines()
            .map(str::trim)
            .filter(|l| l.ends_with(',') && l.starts_with(|c: char| c.is_ascii_uppercase()))
            .count();
        assert_eq!(variants, FaultKind::COUNT);
    }

    #[test]
    fn indices_are_stable_and_dense() {
        for (i, k) in FaultKind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
            assert_eq!(FaultKind::from_index(i), Some(*k));
        }
        assert_eq!(FaultKind::from_index(FaultKind::COUNT), None);
        for (i, c) in CapacityFault::ALL.iter().enumerate() {
            assert_eq!(c.code() as usize, i);
            assert_eq!(CapacityFault::from_code(c.code()), Some(*c));
        }
        assert_eq!(CapacityFault::from_code(CapacityFault::ALL.len() as u32), None);
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
