//! The allocation guard: detects heap allocation by Twine after start-up ("seal").
//!
//! A safety case for a fixed-memory product claims that after start-up the UI allocates
//! nothing: everything it needs was sized from [`Limits`](crate::Limits) and reserved while
//! the `Ui` was built. Sealing makes that claim checkable at run time:
//!
//! 1. The application builds its UI, then calls `Ui::seal()` (or [`seal`] directly), which
//!    seals the UI's [`Partition`].
//! 2. From then on, every **growth point** of Twine that is about to allocate — a vector at
//!    capacity, a new reactive node, a new widget — calls [`note`] (or [`grew`]), which counts
//!    the allocation and remembers its [`AllocSite`].
//! 3. The `Ui` turns the count into a
//!    [`FaultKind::AllocAfterSeal`](crate::fault::FaultKind::AllocAfterSeal) record at its next
//!    update (`code` = the last site), through the normal fault stream.
//!
//! The allocation itself still happens: refusing it would lose data or break an invariant.
//! The fault is the evidence that the claim did not hold, and where.
//!
//! # Why a guard instead of an allocator hook
//!
//! Twine does not choose the global allocator (platform rule 3), so it cannot see allocations.
//! The guard is Twine-internal and independent of the allocator: each growth point checks
//! whether it is about to allocate (`len == capacity`, or a creation that boxes a value) and
//! only then reads the seal flag, in a `#[cold]` function. Paths that stay within capacity
//! cost nothing extra (the capacity comparison is the one `Vec::push` makes anyway).
//!
//! # Coverage
//!
//! Covered (each reports its [`AllocSite`]):
//!
//! | Crate | Growth points |
//! |-------|---------------|
//! | core | [`Arena`](crate::Arena) slots and free lists; [`SmallVec`](crate::SmallVec) spills and growth |
//! | reactive | node, scope, cleanup and context creation; effect queues; dependency edges; channel handler registration; scope bookkeeping; the propagation stack; heap wakers (disposal allocates nothing: it walks the scope tree's links and releases one node at a time) |
//! | anim | animation and timer tables |
//! | style | a style's property list (`StyleBuf`, a property set for the first time) |
//! | engine | widget creation; event handlers; focus groups; posted events; invalidation, layout, scroll-into-view and delete scratch; style transitions |
//! | widgets | widget text buffers (label with its password-mode and shaped copies, checkbox, span, button-matrix button and popover, dropdown/roller options); button-matrix tables and a static map's owned copy; a span group's spans and line layout |
//! | view | engine-command ring (re)allocation; navigator stacks; keyed collections' row lists and reconcile scratch; dynamic-text format buffers |
//!
//! Not covered (documented assumptions of use): allocations made by **application code**
//! (closures, `Clone`/`Drop` of application values, application `String`s and `Vec`s handed to
//! Twine — e.g. an `OwnedMap`, grid tracks, a list's items), by third-party crates (decoders),
//! by caches whose budgets are `EngineConfig` bounds (glyphs, images, shapes, layers: filled
//! on first use, then evicting), by `log`/`defmt` formatting, by `alloc::format!` in diagnostics, and by the
//! `debug-checks` instrumentation (invalidation logs; not in a product). The test helper
//! `twine_testing::assert_no_alloc_after_seal` closes the gap in tests: it counts **every**
//! allocation with a counting global allocator and also checks this guard, so a test proves
//! the claim for the scenario it runs.
//!
//! # Scope of the seal: one domain per partition
//!
//! Each [`Partition`] has its own seal record (ADR-001 § 5.3): sealing one partition does
//! not seal another, and growth in partition B after partition A sealed is B's evidence, never
//! A's. Every growth point therefore names its owner's partition: the reactive runtime its
//! slot, the engine the partition it was given (`Engine::set_partition`, applied by
//! `AppConfig::apply_limits`), which it hands to its tree, animation and timer tables and
//! widgets, and the view layer the partition of its `Runtime` token. Without the
//! `partitions` feature there is one record and `Partition` is zero-sized.
//!
//! The records live in a process-wide table without `std` (one product, one start-up) and in
//! a per-thread table with `std` (like the reactive runtime: tests and host tools run
//! independent UIs on their threads). They use plain atomic loads and stores (no
//! compare-and-swap, so they work on `thumbv6m`); a partition's growth points are reached
//! from that partition's context only, so the read-modify-write of its counter is not
//! contended — two partitions on two cores touch two records.
//!
//! ```
//! use twine_core::Partition;
//! use twine_core::alloc_guard::{self, AllocSite};
//!
//! let p = Partition::MAIN;
//! alloc_guard::seal(p);
//! let mut v: Vec<u32> = Vec::with_capacity(1);
//! alloc_guard::push(p, &mut v, 1, AllocSite::EngineScratch); // within capacity: nothing
//! alloc_guard::push(p, &mut v, 2, AllocSite::EngineScratch); // grows: counted
//! let report = alloc_guard::take(p).unwrap();
//! assert_eq!((report.count, report.last), (1, AllocSite::EngineScratch));
//! assert_eq!(alloc_guard::take(p), None); // taken
//! alloc_guard::unseal(p);
//! ```

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::Partition;

/// A growth point of Twine: where an allocation after [`seal`] happened (the `code` of a
/// [`FaultKind::AllocAfterSeal`](crate::fault::FaultKind::AllocAfterSeal) record).
///
/// Codes are stable and grouped by crate (`1..` core, `10..` reactive, `20..` anim and
/// engine, `30..` widgets, `40..` view).
///
/// ```
/// use twine_core::alloc_guard::AllocSite;
///
/// assert_eq!(AllocSite::from_code(AllocSite::ReactiveNode.code()), Some(AllocSite::ReactiveNode));
/// assert_eq!(AllocSite::from_code(0), None);
/// assert_eq!(AllocSite::Widget.name(), "Widget");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
#[non_exhaustive]
pub enum AllocSite {
    /// An [`Arena`](crate::Arena)'s slot storage grew.
    ArenaSlots = 1,
    /// An [`Arena`](crate::Arena)'s free lists grew (a value was removed).
    ArenaFreeList = 2,
    /// A [`SmallVec`](crate::SmallVec) spilled to the heap or its heap buffer grew.
    SmallVec = 3,
    /// A reactive node (signal, memo, stored value, effect) was created.
    ReactiveNode = 10,
    /// A reactive scope was created, or a scope's child/node list grew.
    ReactiveScope = 11,
    /// A cleanup was registered on a scope.
    ReactiveCleanup = 12,
    /// A context value was provided on a scope.
    ReactiveContext = 13,
    /// An effect queue (pending or deferred) grew.
    EffectQueue = 14,
    /// A dependency edge list (subscribers, sources) grew.
    ReactiveEdges = 15,
    /// A channel handler was registered (`on_message`).
    ChannelHandler = 16,
    /// The runtime's propagation stack grew.
    ReactiveScratch = 17,
    /// The waker pool allocated a heap waker.
    Waker = 18,
    /// An animation was started into a full animation table.
    Animation = 20,
    /// A timer was added into a full timer table.
    Timer = 21,
    /// A widget (engine node) was created.
    Widget = 22,
    /// An event handler was registered.
    Handler = 23,
    /// A focus group was created or a group's member list grew.
    FocusGroup = 24,
    /// An engine scratch or queue grew (posted events, invalidations, layout, scroll into
    /// view, style transitions).
    EngineScratch = 25,
    /// The delete scratch (post-order of a deleted subtree) grew.
    DeleteScratch = 26,
    /// A style's property list (`StyleBuf`: a node's local style, a shared style) grew: a
    /// property was set on it for the first time.
    StyleProps = 27,
    /// A widget's text buffer grew (label — with its password-mode and shaped copies —,
    /// checkbox, span, button-matrix button and popover, owned dropdown/roller options).
    Text = 30,
    /// A widget's private table grew, or a static table became an owned copy (button-matrix
    /// buttons, a span group's spans and line layout).
    WidgetBuffer = 31,
    /// A `Ui`'s engine-command ring was (re)allocated.
    EngineQueue = 40,
    /// A navigator's route or page stack grew.
    NavStack = 41,
    /// A keyed collection's (`for_each`, `virtual_list`) row list or reconcile scratch grew
    /// (kept between changes; reserved for `Limits::max_list_items` with `preallocate`).
    Reconcile = 42,
}

impl AllocSite {
    /// The stable numeric code.
    ///
    /// ```
    /// use twine_core::alloc_guard::AllocSite;
    /// assert_eq!(AllocSite::ArenaSlots.code(), 1);
    /// ```
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// The site with `code` (`None` for an unknown code).
    ///
    /// ```
    /// use twine_core::alloc_guard::AllocSite;
    /// assert_eq!(AllocSite::from_code(22), Some(AllocSite::Widget));
    /// ```
    #[must_use]
    pub const fn from_code(code: u32) -> Option<AllocSite> {
        Some(match code {
            1 => AllocSite::ArenaSlots,
            2 => AllocSite::ArenaFreeList,
            3 => AllocSite::SmallVec,
            10 => AllocSite::ReactiveNode,
            11 => AllocSite::ReactiveScope,
            12 => AllocSite::ReactiveCleanup,
            13 => AllocSite::ReactiveContext,
            14 => AllocSite::EffectQueue,
            15 => AllocSite::ReactiveEdges,
            16 => AllocSite::ChannelHandler,
            17 => AllocSite::ReactiveScratch,
            18 => AllocSite::Waker,
            20 => AllocSite::Animation,
            21 => AllocSite::Timer,
            22 => AllocSite::Widget,
            23 => AllocSite::Handler,
            24 => AllocSite::FocusGroup,
            25 => AllocSite::EngineScratch,
            26 => AllocSite::DeleteScratch,
            27 => AllocSite::StyleProps,
            30 => AllocSite::Text,
            31 => AllocSite::WidgetBuffer,
            40 => AllocSite::EngineQueue,
            41 => AllocSite::NavStack,
            42 => AllocSite::Reconcile,
            _ => return None,
        })
    }

    /// The site's name, as written in the source.
    ///
    /// ```
    /// use twine_core::alloc_guard::AllocSite;
    /// assert_eq!(AllocSite::Reconcile.name(), "Reconcile");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            AllocSite::ArenaSlots => "ArenaSlots",
            AllocSite::ArenaFreeList => "ArenaFreeList",
            AllocSite::SmallVec => "SmallVec",
            AllocSite::ReactiveNode => "ReactiveNode",
            AllocSite::ReactiveScope => "ReactiveScope",
            AllocSite::ReactiveCleanup => "ReactiveCleanup",
            AllocSite::ReactiveContext => "ReactiveContext",
            AllocSite::EffectQueue => "EffectQueue",
            AllocSite::ReactiveEdges => "ReactiveEdges",
            AllocSite::ChannelHandler => "ChannelHandler",
            AllocSite::ReactiveScratch => "ReactiveScratch",
            AllocSite::Waker => "Waker",
            AllocSite::Animation => "Animation",
            AllocSite::Timer => "Timer",
            AllocSite::Widget => "Widget",
            AllocSite::Handler => "Handler",
            AllocSite::FocusGroup => "FocusGroup",
            AllocSite::EngineScratch => "EngineScratch",
            AllocSite::DeleteScratch => "DeleteScratch",
            AllocSite::StyleProps => "StyleProps",
            AllocSite::Text => "Text",
            AllocSite::WidgetBuffer => "WidgetBuffer",
            AllocSite::EngineQueue => "EngineQueue",
            AllocSite::NavStack => "NavStack",
            AllocSite::Reconcile => "Reconcile",
        }
    }
}

impl core::fmt::Display for AllocSite {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Allocations noted since the last [`take`]: how many, and the last growth point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AllocReport {
    /// Allocations noted (saturating).
    pub count: u32,
    /// The growth point of the last one.
    pub last: AllocSite,
}

/// One partition's seal record.
#[derive(Clone, Copy)]
struct State {
    sealed: bool,
    /// Allocations noted since the last [`take`].
    pending: u32,
    /// The code of the last site noted.
    last: u32,
    /// Allocations noted since the last [`seal`].
    total: u32,
}

impl State {
    const NEW: State = State {
        sealed: false,
        pending: 0,
        last: 0,
        total: 0,
    };
}

#[cfg(any(feature = "std", test))]
mod imp {
    use core::cell::Cell;
    use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

    use super::State;
    use crate::Partition;

    std::thread_local! {
        /// One seal record per partition (per thread: tests and host tools run independent
        /// UIs on their threads).
        static STATES: [Cell<State>; Partition::COUNT] =
            const { [const { Cell::new(State::NEW) }; Partition::COUNT] };
    }

    /// Whether any record of any thread was ever sealed (set by `update`, never cleared). A
    /// process-wide pre-check: until something is sealed — every run that does not test the
    /// seal — a growth point's check is this one load instead of a thread-local lookup (a
    /// call on some hosts), as cheap as the `static` records without `std`. Only ever stored
    /// `true` (no read-modify-write).
    static EVER_SEALED: AtomicBool = AtomicBool::new(false);

    #[inline]
    pub(super) fn sealed(p: Partition) -> bool {
        EVER_SEALED.load(Relaxed) && sealed_record(p)
    }

    /// The thread-local half of [`sealed`]: out of line, so that the thread-local address
    /// is not computed (a call on some hosts) ahead of the pre-check.
    #[inline(never)]
    fn sealed_record(p: Partition) -> bool {
        STATES.try_with(|s| p.select(s).get().sealed).unwrap_or(false)
    }

    #[inline]
    pub(super) fn pending(p: Partition) -> u32 {
        // Allocations are only noted in a sealed record.
        if EVER_SEALED.load(Relaxed) {
            pending_record(p)
        } else {
            0
        }
    }

    /// The thread-local half of [`pending`] (out of line, like [`sealed_record`]).
    #[inline(never)]
    fn pending_record(p: Partition) -> u32 {
        STATES.try_with(|s| p.select(s).get().pending).unwrap_or(0)
    }

    /// Runs `f` on the partition's record of this thread (during thread teardown, when the
    /// slot is gone, on a cleared copy that is then dropped).
    pub(super) fn update<R>(p: Partition, f: impl FnOnce(&mut State) -> R) -> R {
        let mut v = STATES.try_with(|s| p.select(s).get()).unwrap_or(State::NEW);
        let r = f(&mut v);
        if v.sealed {
            EVER_SEALED.store(true, Relaxed);
        }
        let _ = STATES.try_with(|s| p.select(s).set(v));
        r
    }
}

#[cfg(not(any(feature = "std", test)))]
mod imp {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

    use super::State;
    use crate::Partition;

    /// One partition's record in atomics (load/store only: no compare-and-swap, so it works
    /// on `thumbv6m`). Each partition's growth points run in that partition's context only,
    /// so the read-modify-write of `update` is not contended (see the module docs).
    struct Slot {
        sealed: AtomicBool,
        pending: AtomicU32,
        last: AtomicU32,
        total: AtomicU32,
    }

    impl Slot {
        const fn new() -> Self {
            Slot {
                sealed: AtomicBool::new(false),
                pending: AtomicU32::new(0),
                last: AtomicU32::new(0),
                total: AtomicU32::new(0),
            }
        }
    }

    static STATES: [Slot; Partition::COUNT] = [const { Slot::new() }; Partition::COUNT];

    #[inline]
    pub(super) fn sealed(p: Partition) -> bool {
        p.select(&STATES).sealed.load(Relaxed)
    }

    #[inline]
    pub(super) fn pending(p: Partition) -> u32 {
        p.select(&STATES).pending.load(Relaxed)
    }

    /// Runs `f` on the partition's record.
    pub(super) fn update<R>(p: Partition, f: impl FnOnce(&mut State) -> R) -> R {
        let slot = p.select(&STATES);
        let mut v = State {
            sealed: slot.sealed.load(Relaxed),
            pending: slot.pending.load(Relaxed),
            last: slot.last.load(Relaxed),
            total: slot.total.load(Relaxed),
        };
        let r = f(&mut v);
        slot.sealed.store(v.sealed, Relaxed);
        slot.pending.store(v.pending, Relaxed);
        slot.last.store(v.last, Relaxed);
        slot.total.store(v.total, Relaxed);
        r
    }
}

/// Seals partition `p`: from now on every growth point of `p` that allocates is noted (see
/// the [module docs](self)). Clears the counts of an earlier seal of `p`. Idempotent apart
/// from that; other partitions are unaffected. Usually called through `Ui::seal`, which also
/// checks the heap budget and seals its own partition. Never panics.
///
/// ```
/// use twine_core::{Partition, alloc_guard};
/// alloc_guard::seal(Partition::MAIN);
/// assert!(alloc_guard::is_sealed(Partition::MAIN));
/// alloc_guard::unseal(Partition::MAIN);
/// ```
pub fn seal(p: Partition) {
    imp::update(p, |s| {
        *s = State {
            sealed: true,
            ..State::NEW
        };
    });
}

/// Ends the seal of partition `p` (e.g. before a planned reconfiguration, or at the end of a
/// test). The counts stay readable ([`total`]). Never panics.
///
/// ```
/// use twine_core::{Partition, alloc_guard};
/// alloc_guard::seal(Partition::MAIN);
/// alloc_guard::unseal(Partition::MAIN);
/// assert!(!alloc_guard::is_sealed(Partition::MAIN));
/// ```
pub fn unseal(p: Partition) {
    imp::update(p, |s| s.sealed = false);
}

/// Whether partition `p` is sealed (one load).
///
/// ```
/// use twine_core::{Partition, alloc_guard};
/// assert!(!alloc_guard::is_sealed(Partition::MAIN));
/// ```
#[inline]
#[must_use]
pub fn is_sealed(p: Partition) -> bool {
    imp::sealed(p)
}

/// Notes that the growth point `site` of partition `p` allocates now — when `p` is sealed
/// (one load otherwise). For operations that always allocate (creating a node boxes its
/// value). Never panics.
///
/// ```
/// use twine_core::Partition;
/// use twine_core::alloc_guard::{self, AllocSite};
/// alloc_guard::note(Partition::MAIN, AllocSite::Widget); // not sealed: nothing
/// assert_eq!(alloc_guard::take(Partition::MAIN), None);
/// ```
#[inline]
pub fn note(p: Partition, site: AllocSite) {
    if is_sealed(p) {
        record(p, site);
    }
}

/// Notes that the growth point `site` of partition `p` allocated (its storage just grew) —
/// when `p` is sealed. For the cold branch of a growth point that already knows it grows.
/// Never panics.
///
/// ```
/// use twine_core::Partition;
/// use twine_core::alloc_guard::{self, AllocSite};
/// alloc_guard::seal(Partition::MAIN);
/// alloc_guard::grew(Partition::MAIN, AllocSite::ArenaSlots);
/// assert_eq!(alloc_guard::take(Partition::MAIN).map(|r| r.last), Some(AllocSite::ArenaSlots));
/// alloc_guard::unseal(Partition::MAIN);
/// ```
#[cold]
#[inline(never)]
pub fn grew(p: Partition, site: AllocSite) {
    if is_sealed(p) {
        record(p, site);
    }
}

/// Counts one allocation at `site` in partition `p` (sealed).
#[cold]
#[inline(never)]
fn record(p: Partition, site: AllocSite) {
    imp::update(p, |s| {
        s.pending = s.pending.saturating_add(1);
        s.total = s.total.saturating_add(1);
        s.last = site.code();
    });
    crate::warn!(target: "twine::alloc", "allocation after seal at {} ({})", site.name(), p);
}

/// Notes `site` of partition `p` when a container of length `len` and capacity `cap` is about
/// to grow by one (`len == cap`): the check a growth point makes before a push. Inline; the
/// noting is cold.
///
/// ```
/// use twine_core::Partition;
/// use twine_core::alloc_guard::{self, AllocSite};
/// let v: Vec<u8> = Vec::with_capacity(2);
/// alloc_guard::before_push(Partition::MAIN, v.len(), v.capacity(), AllocSite::EngineScratch);
/// ```
#[inline]
pub fn before_push(p: Partition, len: usize, cap: usize, site: AllocSite) {
    if len == cap {
        grew(p, site);
    }
}

/// `v.push(value)`, noting `site` of partition `p` when the push grows the vector (see
/// [`before_push`]).
///
/// ```
/// use twine_core::Partition;
/// use twine_core::alloc_guard::{self, AllocSite};
/// let mut v = Vec::new();
/// alloc_guard::push(Partition::MAIN, &mut v, 1u8, AllocSite::EngineScratch);
/// assert_eq!(v, [1]);
/// ```
#[inline]
pub fn push<T>(p: Partition, v: &mut Vec<T>, value: T, site: AllocSite) {
    before_push(p, v.len(), v.capacity(), site);
    v.push(value);
}

/// `q.push_back(value)`, noting `site` of partition `p` when the push grows the queue.
///
/// ```
/// use std::collections::VecDeque;
/// use twine_core::Partition;
/// use twine_core::alloc_guard::{self, AllocSite};
/// let mut q = VecDeque::new();
/// alloc_guard::push_back(Partition::MAIN, &mut q, 1u8, AllocSite::EffectQueue);
/// assert_eq!(q.front(), Some(&1));
/// ```
#[inline]
pub fn push_back<T>(p: Partition, q: &mut VecDeque<T>, value: T, site: AllocSite) {
    before_push(p, q.len(), q.capacity(), site);
    q.push_back(value);
}

/// Reserves room for `additional` more elements, noting `site` of partition `p` when that
/// allocates.
///
/// ```
/// use twine_core::Partition;
/// use twine_core::alloc_guard::{self, AllocSite};
/// let mut v: Vec<u8> = Vec::new();
/// alloc_guard::reserve(Partition::MAIN, &mut v, 4, AllocSite::EngineScratch);
/// assert!(v.capacity() >= 4);
/// ```
#[inline]
pub fn reserve<T>(p: Partition, v: &mut Vec<T>, additional: usize, site: AllocSite) {
    if v.capacity().saturating_sub(v.len()) < additional {
        grew(p, site);
        v.reserve(additional);
    }
}

/// The allocations noted in partition `p` since the last call (or since [`seal`]), clearing
/// them; `None` when there were none. Each `Ui` calls it for its partition at every update and
/// raises [`FaultKind::AllocAfterSeal`](crate::fault::FaultKind::AllocAfterSeal). Never panics.
///
/// ```
/// use twine_core::{Partition, alloc_guard};
/// assert_eq!(alloc_guard::take(Partition::MAIN), None);
/// ```
#[inline]
pub fn take(p: Partition) -> Option<AllocReport> {
    // Called at every update: one load when nothing was noted.
    if imp::pending(p) == 0 {
        return None;
    }
    take_noted(p)
}

/// The cold half of [`take`].
#[cold]
#[inline(never)]
fn take_noted(p: Partition) -> Option<AllocReport> {
    let (count, last) = imp::update(p, |s| (core::mem::take(&mut s.pending), s.last));
    if count == 0 {
        None
    } else {
        AllocSite::from_code(last).map(|last| AllocReport { count, last })
    }
}

/// Allocations noted in partition `p` since its last [`seal`] (not cleared by [`take`];
/// saturating).
///
/// ```
/// use twine_core::{Partition, alloc_guard};
/// alloc_guard::seal(Partition::MAIN);
/// assert_eq!(alloc_guard::total(Partition::MAIN), 0);
/// alloc_guard::unseal(Partition::MAIN);
/// ```
#[must_use]
pub fn total(p: Partition) -> u32 {
    imp::update(p, |s| s.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Partition = Partition::MAIN;

    #[test]
    fn seal_counts_growth_only_while_sealed() {
        let mut v: Vec<u8> = Vec::new();
        push(M, &mut v, 1, AllocSite::EngineScratch);
        assert_eq!(take(M), None, "not sealed");
        seal(M);
        v.reserve_exact(1);
        let cap = v.capacity();
        while v.len() < cap {
            push(M, &mut v, 0, AllocSite::EngineScratch);
        }
        assert_eq!(take(M), None, "within capacity");
        push(M, &mut v, 2, AllocSite::EngineScratch);
        note(M, AllocSite::Widget);
        assert_eq!(
            take(M),
            Some(AllocReport {
                count: 2,
                last: AllocSite::Widget
            })
        );
        assert_eq!(total(M), 2);
        let mut q = VecDeque::new();
        push_back(M, &mut q, 1u8, AllocSite::EffectQueue);
        let mut r: Vec<u8> = Vec::new();
        reserve(M, &mut r, 3, AllocSite::DeleteScratch);
        reserve(M, &mut r, 3, AllocSite::DeleteScratch); // fits: not noted
        assert_eq!(
            take(M).map(|r| (r.count, r.last)),
            Some((2, AllocSite::DeleteScratch))
        );
        unseal(M);
        note(M, AllocSite::Widget);
        assert_eq!(take(M), None);
        assert_eq!(total(M), 4);
        seal(M);
        assert_eq!(total(M), 0, "a new seal clears the counts");
        unseal(M);
        for code in 0..64 {
            if let Some(s) = AllocSite::from_code(code) {
                assert_eq!(s.code(), code);
                assert!(!s.name().is_empty());
            }
        }
    }

    #[cfg(feature = "partitions")]
    #[test]
    fn partitions_have_separate_seal_domains() {
        let [a, b, ..] = Partition::ALL;
        seal(a);
        assert!(is_sealed(a) && !is_sealed(b));
        note(b, AllocSite::Widget); // b is not sealed: not noted anywhere
        assert_eq!(take(a), None);
        seal(b);
        note(b, AllocSite::Widget);
        assert_eq!(take(a), None, "b's growth is never a's evidence");
        assert_eq!(total(a), 0);
        assert_eq!(take(b).map(|r| r.last), Some(AllocSite::Widget));
        unseal(b);
        assert!(is_sealed(a));
        unseal(a);
    }
}
