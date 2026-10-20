//! The allocation guard: detects heap allocation by Twine after start-up ("seal").
//!
//! A safety case for a fixed-memory product claims that after start-up the UI allocates
//! nothing: everything it needs was sized from [`Limits`](crate::Limits) and reserved while
//! the `Ui` was built. Sealing makes that claim checkable at run time:
//!
//! 1. The application builds its UI, then calls `Ui::seal()` (or [`seal`] directly).
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
//! # Scope of the seal
//!
//! The seal is process-wide without `std` (one product, one start-up), and per thread with
//! `std` (like the reactive runtime: tests and host tools run independent UIs on their
//! threads). The state uses plain atomic loads and stores (no compare-and-swap, so it works
//! on `thumbv6m`); growth points are reached from the UI context only, so the
//! read-modify-write of the counter is not contended.
//!
//! ```
//! use twine_core::alloc_guard::{self, AllocSite};
//!
//! alloc_guard::seal();
//! let mut v: Vec<u32> = Vec::with_capacity(1);
//! alloc_guard::push(&mut v, 1, AllocSite::EngineScratch); // within capacity: nothing
//! alloc_guard::push(&mut v, 2, AllocSite::EngineScratch); // grows: counted
//! let report = alloc_guard::take().unwrap();
//! assert_eq!((report.count, report.last), (1, AllocSite::EngineScratch));
//! assert_eq!(alloc_guard::take(), None); // taken
//! alloc_guard::unseal();
//! ```

use alloc::collections::VecDeque;
use alloc::vec::Vec;

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

#[cfg(any(feature = "std", test))]
mod imp {
    use core::cell::Cell;

    /// `(sealed, pending count, last site code, total since seal)`.
    #[derive(Clone, Copy)]
    pub(super) struct State {
        pub(super) sealed: bool,
        pub(super) pending: u32,
        pub(super) last: u32,
        pub(super) total: u32,
    }

    std::thread_local! {
        static STATE: Cell<State> = const {
            Cell::new(State { sealed: false, pending: 0, last: 0, total: 0 })
        };
    }

    #[inline]
    pub(super) fn sealed() -> bool {
        STATE.try_with(|s| s.get().sealed).unwrap_or(false)
    }

    #[inline]
    pub(super) fn pending() -> u32 {
        STATE.try_with(|s| s.get().pending).unwrap_or(0)
    }

    /// Runs `f` on the thread's state (during thread teardown, when the slot is gone, on a
    /// cleared copy that is then dropped).
    pub(super) fn update<R>(f: impl FnOnce(&mut State) -> R) -> R {
        let mut v = STATE.try_with(Cell::get).unwrap_or(State {
            sealed: false,
            pending: 0,
            last: 0,
            total: 0,
        });
        let r = f(&mut v);
        let _ = STATE.try_with(|s| s.set(v));
        r
    }
}

#[cfg(not(any(feature = "std", test)))]
mod imp {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

    static SEALED: AtomicBool = AtomicBool::new(false);
    static PENDING: AtomicU32 = AtomicU32::new(0);
    static LAST: AtomicU32 = AtomicU32::new(0);
    static TOTAL: AtomicU32 = AtomicU32::new(0);

    /// A copy of the state (loaded field by field; see the module docs on contention).
    #[derive(Clone, Copy)]
    pub(super) struct State {
        pub(super) sealed: bool,
        pub(super) pending: u32,
        pub(super) last: u32,
        pub(super) total: u32,
    }

    #[inline]
    pub(super) fn sealed() -> bool {
        SEALED.load(Relaxed)
    }

    #[inline]
    pub(super) fn pending() -> u32 {
        PENDING.load(Relaxed)
    }

    /// Runs `f` on the process-wide state.
    pub(super) fn update<R>(f: impl FnOnce(&mut State) -> R) -> R {
        let mut v = State {
            sealed: SEALED.load(Relaxed),
            pending: PENDING.load(Relaxed),
            last: LAST.load(Relaxed),
            total: TOTAL.load(Relaxed),
        };
        let r = f(&mut v);
        SEALED.store(v.sealed, Relaxed);
        PENDING.store(v.pending, Relaxed);
        LAST.store(v.last, Relaxed);
        TOTAL.store(v.total, Relaxed);
        r
    }
}

/// Seals Twine: from now on every growth point that allocates is noted (see the
/// [module docs](self)). Clears the counts of an earlier seal. Idempotent apart from that.
/// Usually called through `Ui::seal`, which also checks the heap budget. Never panics.
///
/// ```
/// use twine_core::alloc_guard;
/// alloc_guard::seal();
/// assert!(alloc_guard::is_sealed());
/// alloc_guard::unseal();
/// ```
pub fn seal() {
    imp::update(|s| {
        *s = imp::State {
            sealed: true,
            pending: 0,
            last: 0,
            total: 0,
        };
    });
}

/// Ends the seal (e.g. before a planned reconfiguration, or at the end of a test). The counts
/// stay readable ([`total`]). Never panics.
///
/// ```
/// use twine_core::alloc_guard;
/// alloc_guard::seal();
/// alloc_guard::unseal();
/// assert!(!alloc_guard::is_sealed());
/// ```
pub fn unseal() {
    imp::update(|s| s.sealed = false);
}

/// Whether Twine is sealed (one load).
///
/// ```
/// use twine_core::alloc_guard;
/// assert!(!alloc_guard::is_sealed());
/// ```
#[inline]
#[must_use]
pub fn is_sealed() -> bool {
    imp::sealed()
}

/// Notes that the growth point `site` allocates now — when sealed (one load otherwise). For
/// operations that always allocate (creating a node boxes its value). Never panics.
///
/// ```
/// use twine_core::alloc_guard::{self, AllocSite};
/// alloc_guard::note(AllocSite::Widget); // not sealed: nothing
/// assert_eq!(alloc_guard::take(), None);
/// ```
#[inline]
pub fn note(site: AllocSite) {
    if is_sealed() {
        record(site);
    }
}

/// Notes that the growth point `site` allocated (its storage just grew) — when sealed. For
/// the cold branch of a growth point that already knows it grows. Never panics.
///
/// ```
/// use twine_core::alloc_guard::{self, AllocSite};
/// alloc_guard::seal();
/// alloc_guard::grew(AllocSite::ArenaSlots);
/// assert_eq!(alloc_guard::take().map(|r| r.last), Some(AllocSite::ArenaSlots));
/// alloc_guard::unseal();
/// ```
#[cold]
#[inline(never)]
pub fn grew(site: AllocSite) {
    if is_sealed() {
        record(site);
    }
}

/// Counts one allocation at `site` (sealed).
#[cold]
#[inline(never)]
fn record(site: AllocSite) {
    imp::update(|s| {
        s.pending = s.pending.saturating_add(1);
        s.total = s.total.saturating_add(1);
        s.last = site.code();
    });
    crate::warn!(target: "twine::alloc", "allocation after seal at {}", site.name());
}

/// Notes `site` when a container of length `len` and capacity `cap` is about to grow by one
/// (`len == cap`): the check a growth point makes before a push. Inline; the noting is cold.
///
/// ```
/// use twine_core::alloc_guard::{self, AllocSite};
/// let v: Vec<u8> = Vec::with_capacity(2);
/// alloc_guard::before_push(v.len(), v.capacity(), AllocSite::EngineScratch);
/// ```
#[inline]
pub fn before_push(len: usize, cap: usize, site: AllocSite) {
    if len == cap {
        grew(site);
    }
}

/// `v.push(value)`, noting `site` when the push grows the vector (see [`before_push`]).
///
/// ```
/// use twine_core::alloc_guard::{self, AllocSite};
/// let mut v = Vec::new();
/// alloc_guard::push(&mut v, 1u8, AllocSite::EngineScratch);
/// assert_eq!(v, [1]);
/// ```
#[inline]
pub fn push<T>(v: &mut Vec<T>, value: T, site: AllocSite) {
    before_push(v.len(), v.capacity(), site);
    v.push(value);
}

/// `q.push_back(value)`, noting `site` when the push grows the queue.
///
/// ```
/// use std::collections::VecDeque;
/// use twine_core::alloc_guard::{self, AllocSite};
/// let mut q = VecDeque::new();
/// alloc_guard::push_back(&mut q, 1u8, AllocSite::EffectQueue);
/// assert_eq!(q.front(), Some(&1));
/// ```
#[inline]
pub fn push_back<T>(q: &mut VecDeque<T>, value: T, site: AllocSite) {
    before_push(q.len(), q.capacity(), site);
    q.push_back(value);
}

/// Reserves room for `additional` more elements, noting `site` when that allocates.
///
/// ```
/// use twine_core::alloc_guard::{self, AllocSite};
/// let mut v: Vec<u8> = Vec::new();
/// alloc_guard::reserve(&mut v, 4, AllocSite::EngineScratch);
/// assert!(v.capacity() >= 4);
/// ```
#[inline]
pub fn reserve<T>(v: &mut Vec<T>, additional: usize, site: AllocSite) {
    if v.capacity().saturating_sub(v.len()) < additional {
        grew(site);
        v.reserve(additional);
    }
}

/// The allocations noted since the last call (or since [`seal`]), clearing them; `None` when
/// there were none. The `Ui` calls it at each update and raises
/// [`FaultKind::AllocAfterSeal`](crate::fault::FaultKind::AllocAfterSeal). Never panics.
///
/// ```
/// use twine_core::alloc_guard;
/// assert_eq!(alloc_guard::take(), None);
/// ```
#[inline]
pub fn take() -> Option<AllocReport> {
    // Called at every update: one load when nothing was noted.
    if imp::pending() == 0 {
        return None;
    }
    take_noted()
}

/// The cold half of [`take`].
#[cold]
#[inline(never)]
fn take_noted() -> Option<AllocReport> {
    let (count, last) = imp::update(|s| (core::mem::take(&mut s.pending), s.last));
    if count == 0 {
        None
    } else {
        AllocSite::from_code(last).map(|last| AllocReport { count, last })
    }
}

/// Allocations noted since the last [`seal`] (not cleared by [`take`]; saturating).
///
/// ```
/// use twine_core::alloc_guard;
/// alloc_guard::seal();
/// assert_eq!(alloc_guard::total(), 0);
/// alloc_guard::unseal();
/// ```
#[must_use]
pub fn total() -> u32 {
    imp::update(|s| s.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_counts_growth_only_while_sealed() {
        let mut v: Vec<u8> = Vec::new();
        push(&mut v, 1, AllocSite::EngineScratch);
        assert_eq!(take(), None, "not sealed");
        seal();
        v.reserve_exact(1);
        let cap = v.capacity();
        while v.len() < cap {
            push(&mut v, 0, AllocSite::EngineScratch);
        }
        assert_eq!(take(), None, "within capacity");
        push(&mut v, 2, AllocSite::EngineScratch);
        note(AllocSite::Widget);
        assert_eq!(
            take(),
            Some(AllocReport {
                count: 2,
                last: AllocSite::Widget
            })
        );
        assert_eq!(total(), 2);
        let mut q = VecDeque::new();
        push_back(&mut q, 1u8, AllocSite::EffectQueue);
        let mut r: Vec<u8> = Vec::new();
        reserve(&mut r, 3, AllocSite::DeleteScratch);
        reserve(&mut r, 3, AllocSite::DeleteScratch); // fits: not noted
        assert_eq!(
            take().map(|r| (r.count, r.last)),
            Some((2, AllocSite::DeleteScratch))
        );
        unseal();
        note(AllocSite::Widget);
        assert_eq!(take(), None);
        assert_eq!(total(), 4);
        seal();
        assert_eq!(total(), 0, "a new seal clears the counts");
        unseal();
        for code in 0..64 {
            if let Some(s) = AllocSite::from_code(code) {
                assert_eq!(s.code(), code);
                assert!(!s.name().is_empty());
            }
        }
    }
}
