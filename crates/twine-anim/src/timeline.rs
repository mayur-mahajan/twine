//! [`Timeline`]: runs many [`Anim`]s, applies their values and reports the next deadline.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use twine_core::arena::next_generation;
use twine_core::{Duration, Generation, Id, Instant, Partition};

use crate::Anim;
use crate::anim::{AnimCallbacks, AnimTarget, NodeKey, Phase, PropKey};

/// Handle of an animation in a [`Timeline`]. Generational: once the animation ends or is
/// removed, its id is rejected even if the slot is reused. The handle is a [`twine_core::Id`]
/// (4 bytes; 8 with a 48-bit [`Generation`] in the safety profile, `Option<AnimId>` as small)
/// and never wraps: a slot that used up its generations is retired (see
/// [`Timeline::retired`]), so a stale id can never name a later animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AnimId(Id<Slot>);

// As small as an arena id in both profiles (4 / 8 bytes), `Option` included (R5.S04).
const _: () = {
    use core::mem::size_of;
    assert!(size_of::<AnimId>() == size_of::<Id<()>>());
    assert!(size_of::<Option<AnimId>>() == size_of::<Option<Id<()>>>());
};

impl AnimId {
    /// An id that never refers to an animation (returned when the timeline is full).
    pub const DANGLING: AnimId = AnimId(Id::INVALID);

    const fn new(index: u16, generation: Generation) -> Self {
        AnimId(Id::from_parts(index, generation))
    }

    const fn index(self) -> u16 {
        self.0.index()
    }

    const fn generation(self) -> Generation {
        self.0.generation()
    }
}

/// A boxed exec closure (see [`Exec::Fn`]).
pub type ExecFn = Box<dyn FnMut(&mut dyn Any, i32)>;

/// How an animation's value is applied.
pub enum Exec {
    /// Through [`TickSink::apply`] with the animation's [`AnimTarget`].
    Target,
    /// Through a closure receiving the sink's context ([`TickSink::ctx`]) and the value.
    Fn(ExecFn),
}

impl fmt::Debug for Exec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exec::Target => f.write_str("Target"),
            Exec::Fn(_) => f.write_str("Fn(..)"),
        }
    }
}

/// Receives the values produced by [`Timeline::tick`].
pub trait TickSink {
    /// Applies `value` to `target` (animations added with [`Timeline::add`]).
    fn apply(&mut self, target: AnimTarget, value: i32);
    /// The context passed to [`Exec::Fn`] closures and to animation callbacks ([`AnimCx::ctx`]).
    fn ctx(&mut self) -> &mut dyn Any;
}

/// Context of an animation callback (`on_start`, `on_repeat`, `on_complete`).
///
/// The callback is taken out of the timeline while it runs, so it can add, remove, pause or
/// restart animations (including its own) through [`AnimCx::timeline`]. Animations added from
/// a callback are first ticked by the next [`Timeline::tick`].
pub struct AnimCx<'a> {
    /// The animation the callback belongs to (already stale in `on_complete`).
    pub id: AnimId,
    /// The time passed to [`Timeline::tick`].
    pub now: Instant,
    /// Completed cycles of the animation.
    pub repeats_done: u16,
    timeline: &'a mut Timeline,
    ctx: &'a mut dyn Any,
}

impl AnimCx<'_> {
    /// The timeline running the animation.
    pub fn timeline(&mut self) -> &mut Timeline {
        self.timeline
    }

    /// The sink's context ([`TickSink::ctx`]); downcast it to the caller's type.
    pub fn ctx(&mut self) -> &mut dyn Any {
        self.ctx
    }
}

impl fmt::Debug for AnimCx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnimCx")
            .field("id", &self.id)
            .field("now", &self.now)
            .field("repeats_done", &self.repeats_done)
            .finish_non_exhaustive()
    }
}

struct Slot {
    /// Starts at 0; bumped when the slot is freed; a slot freed at `MAX_GENERATION` is
    /// retired.
    generation: Generation,
    anim: Option<Anim>,
    start_time: Instant,
    paused_at: Option<Instant>,
    started: bool,
    last_value: Option<i32>,
    last_repeats: u16,
    /// Tick counter value when added; a slot added during a tick is skipped by that tick.
    epoch: u32,
    exec: Exec,
    cbs: AnimCallbacks,
}

#[derive(Clone, Copy)]
enum Which {
    Start,
    Repeat,
}

/// Runs animations: an allocation-free [`tick`](Timeline::tick) applies the value of every
/// running animation and returns the next time anything will change, so the caller can sleep
/// until then (no polling).
///
/// Slots are reused (generational [`AnimId`]); after warm-up, adding and ticking animations does
/// not allocate (only boxed closures and callbacks do). Cost per tick is O(slots).
///
/// ```
/// use core::any::Any;
/// use twine_anim::{Anim, AnimTarget, TickSink, Timeline};
/// use twine_core::{Duration, Instant};
///
/// struct Sink(Vec<(AnimTarget, i32)>);
/// impl TickSink for Sink {
///     fn apply(&mut self, target: AnimTarget, value: i32) { self.0.push((target, value)); }
///     fn ctx(&mut self) -> &mut dyn Any { self }
/// }
///
/// let mut tl = Timeline::new();
/// let t0 = Instant::ZERO;
/// let target = AnimTarget::Node(7, 0); // node key 7, property code 0 (the owner's meaning)
/// tl.add(Anim::new(0, 100).duration(Duration::ms(100)).target(target), t0);
/// let mut sink = Sink(Vec::new());
/// assert_eq!(tl.tick(t0 + Duration::ms(50), &mut sink), Some(t0 + Duration::ms(50)));
/// assert_eq!(sink.0, [(target, 50)]);
/// assert_eq!(tl.tick(t0 + Duration::ms(100), &mut sink), None); // finished: nothing to wake for
/// assert_eq!(sink.0.last(), Some(&(target, 100)));
/// ```
pub struct Timeline {
    slots: Vec<Slot>,
    free: Vec<u16>,
    running: u16,
    /// Most animations running at once ([`set_limit`](Timeline::set_limit)).
    limit: u16,
    /// Slots that used up their generations and are never reused.
    retired: u16,
    epoch: u32,
    ticking: bool,
    added_while_ticking: bool,
    /// The partition whose seal domain the table's growth is noted in.
    part: Partition,
}

impl fmt::Debug for Timeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Timeline")
            .field("slots", &self.slots.len())
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}

impl Default for Timeline {
    /// [`Timeline::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl Timeline {
    /// An empty timeline (does not allocate).
    #[must_use]
    pub const fn new() -> Self {
        Timeline {
            slots: Vec::new(),
            free: Vec::new(),
            running: 0,
            limit: Self::MAX_RUNNING,
            retired: 0,
            epoch: 0,
            ticking: false,
            added_while_ticking: false,
            part: Partition::MAIN,
        }
    }

    /// The largest number of animations a timeline can run at once (65 534: slot indices are
    /// `u16`, one value is reserved).
    pub const MAX_RUNNING: u16 = u16::MAX - 1;

    /// Moves the table to partition `p`'s seal domain: growth of its storage after `p` was
    /// sealed is noted there ([`twine_core::alloc_guard`]). The engine calls it from
    /// `Engine::set_partition`; a table starts in [`Partition::MAIN`]. Never panics.
    ///
    /// ```
    /// use twine_anim::Timeline;
    /// use twine_core::Partition;
    /// let mut t = Timeline::new();
    /// t.set_partition(Partition::MAIN);
    /// assert_eq!(t.partition(), Partition::MAIN);
    /// ```
    pub fn set_partition(&mut self, p: Partition) {
        self.part = p;
    }

    /// The partition set with [`set_partition`](Self::set_partition).
    #[must_use]
    pub fn partition(&self) -> Partition {
        self.part
    }

    /// Sets how many animations may run at once (capped at [`MAX_RUNNING`](Self::MAX_RUNNING);
    /// at least 1). Starting one more returns [`AnimId::DANGLING`] (the caller reports it,
    /// e.g. the engine's `Capacity` fault) — already running animations are kept. Never
    /// panics.
    ///
    /// ```
    /// use twine_anim::{Anim, AnimId, Timeline};
    /// use twine_core::{Duration, Instant};
    ///
    /// let mut tl = Timeline::new();
    /// tl.set_limit(1);
    /// let a = || Anim::new(0, 10).duration(Duration::ms(100));
    /// assert_ne!(tl.add(a(), Instant::ZERO), AnimId::DANGLING);
    /// assert_eq!(tl.add(a(), Instant::ZERO), AnimId::DANGLING);
    /// ```
    pub fn set_limit(&mut self, limit: u16) {
        self.limit = limit.clamp(1, Self::MAX_RUNNING);
    }

    /// The limit set with [`set_limit`](Self::set_limit) (default
    /// [`MAX_RUNNING`](Self::MAX_RUNNING)).
    ///
    /// ```
    /// use twine_anim::Timeline;
    /// assert_eq!(Timeline::new().limit(), Timeline::MAX_RUNNING);
    /// ```
    #[must_use]
    pub fn limit(&self) -> u16 {
        self.limit
    }

    /// Reserves the storage of `n` animations (slots and free list), so starting and removing
    /// animations never allocates while at most `n` run at once. Allocates once; never panics.
    ///
    /// ```
    /// use twine_anim::Timeline;
    /// let mut tl = Timeline::new();
    /// tl.reserve(8);
    /// ```
    pub fn reserve(&mut self, n: u16) {
        let n = usize::from(n);
        self.slots.reserve_exact(n.saturating_sub(self.slots.len()));
        self.free.reserve_exact(n.saturating_sub(self.free.len()));
    }

    /// Starts `anim` at `now`, applying values through [`TickSink::apply`]. An animation with
    /// the same [`AnimTarget::Node`] target (node and property) is removed first (LVGL). With
    /// `early_apply`, the start value is applied by the next tick even during the delay.
    pub fn add(&mut self, anim: Anim, now: Instant) -> AnimId {
        self.insert(anim, Exec::Target, now)
    }

    /// Starts `anim` at `now`, applying values through `exec` (called with [`TickSink::ctx`]).
    pub fn add_fn(
        &mut self,
        anim: Anim,
        exec: impl FnMut(&mut dyn Any, i32) + 'static,
        now: Instant,
    ) -> AnimId {
        self.insert(anim, Exec::Fn(Box::new(exec)), now)
    }

    fn insert(&mut self, mut anim: Anim, exec: Exec, now: Instant) -> AnimId {
        if let AnimTarget::Node(key, prop) = anim.target {
            self.remove_target_prop(key, prop);
        }
        if self.running >= self.limit {
            twine_core::warn!(target: "twine::anim", "timeline full ({} animations), animation dropped", self.limit);
            return AnimId::DANGLING;
        }
        let cbs = core::mem::take(&mut anim.callbacks);
        let index = if let Some(i) = self.free.pop() {
            i
        } else if let Ok(i) = u16::try_from(self.slots.len())
            && i < u16::MAX
        {
            twine_core::alloc_guard::before_push(
                self.part,
                self.slots.len(),
                self.slots.capacity(),
                twine_core::alloc_guard::AllocSite::Animation,
            );
            self.slots.push(Slot {
                generation: 0,
                anim: None,
                start_time: now,
                paused_at: None,
                started: false,
                last_value: None,
                last_repeats: 0,
                epoch: 0,
                exec: Exec::Target,
                cbs: AnimCallbacks::default(),
            });
            i
        } else {
            twine_core::warn!(target: "twine::anim", "timeline full ({} animations), animation dropped", u16::MAX);
            return AnimId::DANGLING;
        };
        let Some(slot) = self.slots.get_mut(usize::from(index)) else {
            debug_assert!(false, "timeline: free slot {index} out of range");
            return AnimId::DANGLING;
        };
        let id = AnimId::new(index, slot.generation);
        twine_core::debug!(target: "twine::anim", "anim {:?} added: {} -> {}", id, anim.start, anim.end);
        *slot = Slot {
            generation: slot.generation,
            anim: Some(anim),
            start_time: now,
            paused_at: None,
            started: false,
            last_value: None,
            last_repeats: 0,
            epoch: self.epoch,
            exec,
            cbs,
        };
        // At most one per slot (`u16` slots).
        self.running = self.running.saturating_add(1);
        self.added_while_ticking |= self.ticking;
        id
    }

    fn slot(&self, id: AnimId) -> Option<&Slot> {
        self.slots
            .get(usize::from(id.index()))
            .filter(|s| s.generation == id.generation() && s.anim.is_some())
    }

    fn slot_mut(&mut self, id: AnimId) -> Option<&mut Slot> {
        self.slots
            .get_mut(usize::from(id.index()))
            .filter(|s| s.generation == id.generation() && s.anim.is_some())
    }

    /// Frees a live slot and returns its callbacks.
    fn free_slot(&mut self, index: u16) -> AnimCallbacks {
        let Some(slot) = self.slots.get_mut(usize::from(index)) else {
            debug_assert!(false, "timeline: freeing slot {index} out of range");
            return AnimCallbacks::default();
        };
        slot.anim = None;
        slot.exec = Exec::Target;
        let cbs = core::mem::take(&mut slot.cbs);
        // A slot whose generation would wrap is retired (never reused), so no stale id can
        // match a later animation (no ABA).
        if let Some(next) = next_generation(slot.generation) {
            slot.generation = next;
            twine_core::alloc_guard::push(
                self.part,
                &mut self.free,
                index,
                twine_core::alloc_guard::AllocSite::Animation,
            );
        } else {
            self.retired = self.retired.saturating_add(1);
            twine_core::debug!(target: "twine::anim", "animation slot {} retired", index);
        }
        self.running = self.running.saturating_sub(1);
        cbs
    }

    /// Removes an animation without calling `on_complete`. Returns `false` for a stale id.
    pub fn remove(&mut self, id: AnimId) -> bool {
        if self.slot(id).is_none() {
            return false;
        }
        twine_core::debug!(target: "twine::anim", "anim {:?} removed", id);
        drop(self.free_slot(id.index()));
        true
    }

    fn remove_where(&mut self, mut f: impl FnMut(AnimTarget) -> bool) -> usize {
        let mut n = 0usize;
        // Slot indices fit `u16` (`add` refuses more).
        for i in 0..=u16::MAX {
            let Some(slot) = self.slots.get(usize::from(i)) else {
                break;
            };
            if slot.anim.as_ref().is_some_and(|a| f(a.target)) {
                drop(self.free_slot(i));
                n = n.saturating_add(1);
            }
        }
        n
    }

    /// Removes every animation of node `key` (any property); returns how many were removed.
    pub fn remove_target(&mut self, key: NodeKey) -> usize {
        self.remove_where(|t| matches!(t, AnimTarget::Node(k, _) if k == key))
    }

    /// Removes the animations of property code `prop` of node `key`; returns how many were
    /// removed.
    pub fn remove_target_prop(&mut self, key: NodeKey, prop: PropKey) -> usize {
        self.remove_where(|t| t == AnimTarget::Node(key, prop))
    }

    /// The animation behind `id`, if it is still running (or paused).
    #[must_use]
    pub fn get(&self, id: AnimId) -> Option<&Anim> {
        self.slot(id).and_then(|s| s.anim.as_ref())
    }

    /// The timing of a live animation, to change it in place (e.g. to apply a motion
    /// preference before a [`restart`](Self::restart)); takes effect from the next tick as if
    /// the animation had always had it. `None` for a stale id. Never panics; O(1).
    ///
    /// The timeline knows no motion preference: the spec written here is used as is. The
    /// engine applies [`Motion::apply`](crate::Motion::apply) when it **starts** an animation
    /// (and, when the preference becomes stricter, only ends running animations that would not
    /// end by themselves), so a spec changed through `spec_mut` is not re-adjusted to the
    /// current preference — apply it yourself (`*spec = motion.apply(*spec)`) if it must
    /// honour it.
    ///
    /// ```
    /// use twine_anim::{Anim, Motion, Timeline};
    /// use twine_core::{Duration, Instant};
    ///
    /// let mut tl = Timeline::new();
    /// let id = tl.add(Anim::new(0, 100).duration(Duration::ms(400)), Instant::ZERO);
    /// if let Some(spec) = tl.spec_mut(id) {
    ///     *spec = Motion::Reduced.apply(*spec); // not done for you
    /// }
    /// assert_eq!(tl.get(id).unwrap().spec.duration, Motion::REDUCED_CAP);
    /// ```
    pub fn spec_mut(&mut self, id: AnimId) -> Option<&mut crate::AnimSpec> {
        self.slot_mut(id)
            .and_then(|s| s.anim.as_mut())
            .map(|a| &mut a.spec)
    }

    /// Every live animation (running or paused) with its id, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (AnimId, &Anim)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.anim.as_ref().map(|a| (AnimId::new(i as u16, s.generation), a)))
    }

    /// Whether `id` is live and not paused.
    #[must_use]
    pub fn is_running(&self, id: AnimId) -> bool {
        self.slot(id).is_some_and(|s| s.paused_at.is_none())
    }

    /// Whether `id` is live and paused.
    #[must_use]
    pub fn is_paused(&self, id: AnimId) -> bool {
        self.slot(id).is_some_and(|s| s.paused_at.is_some())
    }

    /// Freezes an animation at `now`. Returns `false` for a stale id.
    pub fn pause(&mut self, id: AnimId, now: Instant) -> bool {
        let Some(s) = self.slot_mut(id) else { return false };
        if s.paused_at.is_none() {
            s.paused_at = Some(now);
        }
        true
    }

    /// Continues a paused animation where it stopped (its start time moves forward by the paused
    /// time). Returns `false` for a stale id.
    pub fn resume(&mut self, id: AnimId, now: Instant) -> bool {
        let Some(s) = self.slot_mut(id) else { return false };
        if let Some(p) = s.paused_at.take() {
            s.start_time += now.saturating_duration_since(p);
        }
        true
    }

    /// Restarts an animation from the beginning (its delay included) at `now`; a paused
    /// animation stays paused at its beginning. Returns `false` for a stale id.
    pub fn restart(&mut self, id: AnimId, now: Instant) -> bool {
        let Some(s) = self.slot_mut(id) else { return false };
        s.start_time = now;
        s.started = false;
        s.last_repeats = 0;
        if s.paused_at.is_some() {
            s.paused_at = Some(now);
        }
        true
    }

    /// Number of live animations (running or paused).
    #[must_use]
    pub fn running_count(&self) -> usize {
        usize::from(self.running)
    }

    /// Number of **retired** slots: slots whose animations used up every [`Generation`]
    /// (65 535 by default, ≈ 2.8 × 10¹⁴ in the safety profile) and that are never reused, so
    /// a stale [`AnimId`] can never name a later animation. Each costs one slot of memory; 0
    /// in practice. Never panics; O(1).
    ///
    /// ```
    /// use twine_anim::{Anim, Timeline};
    /// use twine_core::{Duration, Instant};
    ///
    /// let mut tl = Timeline::new();
    /// let id = tl.add(Anim::new(0, 10).duration(Duration::ms(10)), Instant::ZERO);
    /// tl.remove(id);
    /// assert_eq!(tl.retired(), 0);
    /// ```
    #[must_use]
    pub fn retired(&self) -> usize {
        usize::from(self.retired)
    }

    /// Removes every animation (no callbacks are called).
    pub fn clear(&mut self) {
        self.remove_where(|_| true);
    }

    /// Ends every live animation for which `f` returns `true` at the next
    /// [`tick`](Self::tick): its timing becomes [`Motion::None`](crate::Motion::None)'s (no
    /// durations, no delays, one cycle; essential or not) and it restarts at `now`, so that tick
    /// applies its final value and runs its callbacks (`on_start` if it had not started yet,
    /// `on_complete`). A paused animation is resumed to end. Returns how many were ended. Used
    /// when the motion preference becomes stricter; allocation-free, never panics.
    ///
    /// ```
    /// use core::any::Any;
    /// use twine_anim::{Anim, AnimTarget, TickSink, Timeline};
    /// use twine_core::{Duration, Instant};
    ///
    /// struct Last(i32);
    /// impl TickSink for Last {
    ///     fn apply(&mut self, _: AnimTarget, v: i32) { self.0 = v; }
    ///     fn ctx(&mut self) -> &mut dyn Any { self }
    /// }
    /// let mut tl = Timeline::new();
    /// tl.add(Anim::new(0, 100).duration(Duration::secs(10)), Instant::ZERO);
    /// assert_eq!(tl.finish_where(Instant::ZERO, |a| !a.spec.essential), 1);
    /// let mut sink = Last(-1);
    /// assert_eq!(tl.tick(Instant::from_millis(1), &mut sink), None); // done
    /// assert_eq!(sink.0, 100);
    /// ```
    pub fn finish_where(&mut self, now: Instant, mut f: impl FnMut(&Anim) -> bool) -> usize {
        let mut n = 0usize;
        for s in &mut self.slots {
            let Some(anim) = s.anim.as_mut() else { continue };
            if !f(anim) {
                continue;
            }
            let essential = anim.spec.essential;
            anim.spec.essential = false;
            anim.spec = crate::Motion::None.apply(anim.spec);
            anim.spec.essential = essential;
            s.start_time = now;
            s.paused_at = None;
            n = n.saturating_add(1);
        }
        n
    }

    /// Calls callback `which` of slot `index`, then puts it back if the animation still exists.
    fn fire(&mut self, which: Which, id: AnimId, now: Instant, repeats_done: u16, sink: &mut dyn TickSink) {
        let Some(slot) = self.slots.get_mut(usize::from(id.index())) else {
            return;
        };
        let taken = match which {
            Which::Start => slot.cbs.on_start.take(),
            Which::Repeat => slot.cbs.on_repeat.take(),
        };
        let Some(mut cb) = taken else { return };
        cb(&mut AnimCx {
            id,
            now,
            repeats_done,
            timeline: self,
            ctx: sink.ctx(),
        });
        if let Some(slot) = self.slot_mut(id) {
            match which {
                Which::Start => slot.cbs.on_start.get_or_insert(cb),
                Which::Repeat => slot.cbs.on_repeat.get_or_insert(cb),
            };
        }
    }

    /// Advances every running animation to `now`: applies changed values (the start value during
    /// the delay only with `early_apply`), calls `on_start` when an animation leaves its delay,
    /// `on_repeat` when a new cycle starts and `on_complete` after the final value, and frees
    /// finished animations.
    ///
    /// Values are applied only when they differ from the last applied value. Returns the next
    /// deadline: `None` when no animation is running (all paused or none), `Some(now)` while any
    /// animation plays (the caller clamps it to its refresh period), else the earliest end of a
    /// delay (initial, playback or repeat delay).
    ///
    /// Does not allocate unless a callback does.
    pub fn tick(&mut self, now: Instant, sink: &mut dyn TickSink) -> Option<Instant> {
        self.epoch = self.epoch.wrapping_add(1);
        self.ticking = true;
        self.added_while_ticking = false;
        let mut deadline: Option<Instant> = None;
        let mut wake = |t: Instant| deadline = Some(deadline.map_or(t, |d| d.min(t)));
        // `slots` may grow while callbacks run; animations added during this tick are skipped
        // (and considered for the deadline below). Slot indices fit `u16` (`add` refuses more).
        for index in 0..=u16::MAX {
            let Some(slot) = self.slots.get_mut(usize::from(index)) else {
                break;
            };
            let Some(anim) = slot.anim.as_ref() else { continue };
            if slot.paused_at.is_some() {
                continue;
            }
            if slot.epoch == self.epoch {
                continue;
            }
            let s = anim.sample(now.saturating_duration_since(slot.start_time));
            let id = AnimId::new(index, slot.generation);

            if !slot.started && s.phase != Phase::Delay {
                slot.started = true;
                self.fire(Which::Start, id, now, s.repeats_done, sink);
                if self.slot(id).is_none() {
                    continue;
                }
            }

            let Some(slot) = self.slots.get_mut(usize::from(index)) else {
                continue;
            };
            let Some(anim) = slot.anim.as_ref() else { continue };
            if (s.phase != Phase::Delay || anim.early_apply) && slot.last_value != Some(s.value) {
                slot.last_value = Some(s.value);
                twine_core::trace!(target: "twine::anim", "anim {:?} value={}", id, s.value);
                match &mut slot.exec {
                    Exec::Target => sink.apply(anim.target, s.value),
                    Exec::Fn(f) => f(sink.ctx(), s.value),
                }
            }

            if s.repeats_done > slot.last_repeats {
                slot.last_repeats = s.repeats_done;
                self.fire(Which::Repeat, id, now, s.repeats_done, sink);
                if self.slot(id).is_none() {
                    continue;
                }
            }

            if s.finished {
                twine_core::debug!(target: "twine::anim", "anim {:?} complete", id);
                let mut cbs = self.free_slot(index);
                if let Some(cb) = cbs.on_complete.as_mut() {
                    cb(&mut AnimCx {
                        id,
                        now,
                        repeats_done: s.repeats_done,
                        timeline: self,
                        ctx: sink.ctx(),
                    });
                }
                continue;
            }

            if s.phase.is_active() {
                wake(now);
            } else if s.phase_left > Duration::ZERO {
                wake(now + s.phase_left);
            }
        }
        self.ticking = false;
        if self.added_while_ticking {
            // Added by callbacks: they need their first tick now, or at the end of their delay
            // when nothing is applied before.
            for slot in &self.slots {
                let Some(anim) = slot.anim.as_ref() else { continue };
                if slot.epoch != self.epoch || slot.paused_at.is_some() {
                    continue;
                }
                let s = anim.sample(now.saturating_duration_since(slot.start_time));
                if s.phase == Phase::Delay && !anim.early_apply {
                    wake(now + s.phase_left);
                } else {
                    wake(now);
                }
            }
        }
        deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Easing, Repeat};
    use twine_core::arena::MAX_GENERATION;

    /// Property codes of the tests (any numbers: the timeline only compares them).
    const X: PropKey = 0;
    const Y: PropKey = 1;
    const OPA: PropKey = 4;
    use alloc::rc::Rc;
    use alloc::string::String;
    use core::cell::RefCell;

    #[derive(Default)]
    struct Sink {
        applied: Vec<(AnimTarget, i32)>,
        log: Vec<String>,
    }

    impl TickSink for Sink {
        fn apply(&mut self, target: AnimTarget, value: i32) {
            self.applied.push((target, value));
        }
        fn ctx(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn ms(v: u64) -> Instant {
        Instant::from_millis(v)
    }

    fn node(k: NodeKey) -> AnimTarget {
        AnimTarget::Node(k, X)
    }

    /// A slot freed at the last generation retires instead of wrapping: no stale id can name
    /// a later animation, and the next animation gets a new slot. Runs in both generation
    /// widths (the generation is set directly).
    #[test]
    fn last_generation_retires_the_slot() {
        let mut tl = Timeline::new();
        let first = tl.add(lin(0, 1, 10), ms(0));
        assert!(tl.remove(first));
        tl.slots[0].generation = MAX_GENERATION;
        let last = tl.add(lin(0, 1, 10), ms(0));
        assert_eq!((last.index(), last.generation()), (0, MAX_GENERATION));
        assert_ne!(last, AnimId::DANGLING);
        assert!(tl.remove(last));
        assert_eq!(tl.retired(), 1);
        let next = tl.add(lin(0, 1, 10), ms(0));
        assert_eq!(next.index(), 1, "retired slot 0 must not be reused");
        for stale in [first, last] {
            assert!(tl.get(stale).is_none());
            assert!(!tl.remove(stale));
        }
        assert_eq!(tl.running_count(), 1);
    }

    /// The same through a finished animation (freed by `tick`).
    #[test]
    fn finished_animation_at_the_last_generation_retires() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        let first = tl.add(lin(0, 1, 10), ms(0));
        assert!(tl.remove(first));
        tl.slots[0].generation = MAX_GENERATION;
        let last = tl.add(lin(0, 1, 10), ms(0));
        assert_eq!(tl.tick(ms(10), &mut sink), None);
        assert!(tl.get(last).is_none());
        assert_eq!((tl.retired(), tl.running_count()), (1, 0));
        assert_eq!(tl.add(lin(0, 1, 10), ms(10)).index(), 1);
    }

    fn lin(start: i32, end: i32, dur_ms: u64) -> Anim {
        Anim::new(start, end)
            .duration(Duration::ms(dur_ms))
            .easing(Easing::Linear)
    }

    #[test]
    fn tick_applies_values_to_sink() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        tl.add(lin(0, 100, 100).target(node(1)), ms(0));
        tl.add(lin(10, 20, 100).target(AnimTarget::Node(1, OPA)), ms(0));
        assert_eq!(tl.tick(ms(0), &mut sink), Some(ms(0)));
        assert_eq!(tl.tick(ms(50), &mut sink), Some(ms(50)));
        assert_eq!(
            sink.applied,
            [
                (node(1), 0),
                (AnimTarget::Node(1, OPA), 10),
                (node(1), 50),
                (AnimTarget::Node(1, OPA), 15)
            ]
        );
        assert_eq!(tl.tick(ms(1000), &mut sink), None);
        assert_eq!(
            sink.applied[4..],
            [(node(1), 100), (AnimTarget::Node(1, OPA), 20)]
        );
        assert_eq!(tl.running_count(), 0);
    }

    #[test]
    fn same_value_not_reapplied() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        tl.add(lin(0, 2, 1000).target(node(1)), ms(0));
        for t in 0..=10 {
            tl.tick(ms(t * 100), &mut sink);
        }
        let values: Vec<i32> = sink.applied.iter().map(|&(_, v)| v).collect();
        assert_eq!(values, [0, 1, 2]);
    }

    #[test]
    fn replacing_same_target_prop_removes_old() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        let a = tl.add(lin(0, 100, 100).target(node(1)), ms(0));
        let b = tl.add(lin(0, 100, 100).target(AnimTarget::Node(1, Y)), ms(0));
        let c = tl.add(lin(500, 600, 100).target(node(1)), ms(0));
        assert!(tl.get(a).is_none());
        assert!(tl.get(b).is_some() && tl.get(c).is_some());
        assert_eq!(tl.running_count(), 2);
        // Custom targets are never replaced.
        tl.add(lin(0, 1, 100), ms(0));
        tl.add(lin(0, 1, 100), ms(0));
        assert_eq!(tl.running_count(), 4);
        tl.tick(ms(50), &mut sink);
        assert!(sink.applied.contains(&(node(1), 550)));
        assert_eq!(tl.remove_target(1), 2);
        assert_eq!(tl.running_count(), 2);
    }

    #[test]
    fn stale_id_rejected_after_reuse() {
        let mut tl = Timeline::new();
        let a = tl.add(lin(0, 1, 10), ms(0));
        assert!(tl.remove(a));
        let b = tl.add(lin(0, 1, 10), ms(0));
        assert_eq!(a.index(), b.index());
        assert_ne!(a, b);
        assert!(!tl.remove(a));
        assert!(!tl.pause(a, ms(0)) && !tl.resume(a, ms(0)) && !tl.restart(a, ms(0)));
        assert!(tl.get(a).is_none() && !tl.is_running(a));
        assert!(tl.is_running(b));
        assert!(!tl.remove(AnimId::DANGLING));
    }

    type Log = Rc<RefCell<Vec<String>>>;

    fn logger(log: &Log, what: &'static str) -> impl FnMut(&mut AnimCx<'_>) + 'static {
        let log = log.clone();
        move |cx| log.borrow_mut().push(alloc::format!("{what}{}", cx.repeats_done))
    }

    #[test]
    fn callbacks_order_start_repeat_complete() {
        let log: Log = Rc::default();
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        let a = lin(0, 100, 100)
            .delay(Duration::ms(10))
            .repeat(Repeat::Times(3))
            .on_start(logger(&log, "start"))
            .on_repeat(logger(&log, "repeat"))
            .on_complete(logger(&log, "complete"));
        tl.add(a, ms(0));
        for t in (0..=400).step_by(5) {
            tl.tick(ms(t), &mut sink);
        }
        assert_eq!(*log.borrow(), ["start0", "repeat1", "repeat2", "complete2"]);
        assert_eq!(tl.running_count(), 0);
    }

    #[test]
    fn callback_can_add_new_anim() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        tl.add(
            lin(0, 10, 10).target(node(1)).on_complete(|cx| {
                let now = cx.now;
                cx.timeline().add(lin(100, 200, 100).target(node(2)), now);
            }),
            ms(0),
        );
        // The first anim completes; the new one is not ticked yet but wants a frame now.
        assert_eq!(tl.tick(ms(10), &mut sink), Some(ms(10)));
        assert_eq!(sink.applied, [(node(1), 10)]);
        assert_eq!(tl.running_count(), 1);
        tl.tick(ms(60), &mut sink);
        assert_eq!(sink.applied[1..], [(node(2), 150)]);
    }

    #[test]
    fn callback_can_remove_other_anim() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        let victim = Rc::new(RefCell::new(None::<AnimId>));
        let v2 = victim.clone();
        tl.add(
            lin(0, 10, 100).target(node(1)).on_start(move |cx| {
                let id = v2.borrow().unwrap();
                assert!(cx.timeline().remove(id));
            }),
            ms(0),
        );
        *victim.borrow_mut() = Some(tl.add(lin(0, 10, 100).target(node(2)), ms(0)));
        // A callback removing its own animation is safe as well.
        tl.add(
            lin(0, 10, 100).target(node(3)).on_start(|cx| {
                let id = cx.id;
                assert!(cx.timeline().remove(id));
            }),
            ms(0),
        );
        tl.tick(ms(50), &mut sink);
        assert_eq!(sink.applied, [(node(1), 5)]);
        assert_eq!(tl.running_count(), 1);
    }

    #[test]
    fn exec_fn_gets_context() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        tl.add_fn(
            lin(0, 100, 100),
            |ctx, v| {
                ctx.downcast_mut::<Sink>()
                    .unwrap()
                    .log
                    .push(alloc::format!("v={v}"));
            },
            ms(0),
        );
        tl.tick(ms(25), &mut sink);
        assert_eq!(sink.log, ["v=25"]);
        assert!(sink.applied.is_empty());
    }

    #[test]
    fn pause_resume_shifts_time() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        let a = tl.add(lin(0, 100, 100).target(node(1)), ms(0));
        tl.tick(ms(20), &mut sink);
        assert!(tl.pause(a, ms(20)));
        assert!(tl.is_paused(a) && !tl.is_running(a));
        assert_eq!(tl.tick(ms(500), &mut sink), None, "paused anims need no wake-up");
        assert!(tl.resume(a, ms(1020)));
        assert_eq!(tl.tick(ms(1050), &mut sink), Some(ms(1050)));
        // Linear at 20 % of 100 ms: progress 204/1024 → 19 (LVGL rounding).
        assert_eq!(sink.applied, [(node(1), 19), (node(1), 50)]);
    }

    #[test]
    fn restart_resets() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        let log: Log = Rc::default();
        let a = tl.add(
            lin(0, 100, 100).target(node(1)).on_start(logger(&log, "start")),
            ms(0),
        );
        tl.tick(ms(80), &mut sink);
        assert!(tl.restart(a, ms(100)));
        tl.tick(ms(110), &mut sink);
        assert_eq!(sink.applied, [(node(1), 79), (node(1), 9)]);
        assert_eq!(*log.borrow(), ["start0", "start0"]);
    }

    #[test]
    fn deadline_none_when_idle() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        assert_eq!(tl.tick(ms(0), &mut sink), None);
        let a = tl.add(lin(0, 1, 10), ms(0));
        tl.remove(a);
        assert_eq!(tl.tick(ms(1), &mut sink), None);
        tl.add(lin(0, 1, 10), ms(0));
        tl.clear();
        assert_eq!(tl.running_count(), 0);
        assert_eq!(tl.tick(ms(2), &mut sink), None);
    }

    #[test]
    fn deadline_delay_end_when_only_delayed() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        tl.add(
            lin(0, 100, 100)
                .target(node(1))
                .delay(Duration::ms(300))
                .early_apply(false),
            ms(0),
        );
        tl.add(lin(0, 100, 100).target(node(2)).delay(Duration::ms(200)), ms(0));
        // Early apply: the start value is applied, then nothing changes until the delay ends.
        assert_eq!(tl.tick(ms(10), &mut sink), Some(ms(200)));
        assert_eq!(sink.applied, [(node(2), 0)]);
        assert_eq!(tl.tick(ms(250), &mut sink), Some(ms(250)));
        // Only the delayed one is left.
        assert_eq!(tl.tick(ms(300 - 1), &mut sink), Some(ms(299)));
        tl.tick(ms(301), &mut sink);
        tl.remove_target(2);
        assert_eq!(tl.tick(ms(400), &mut sink), None);
        // Playback delay: wake at its end.
        tl.add(
            lin(0, 10, 10)
                .target(node(3))
                .playback(Duration::ms(10))
                .playback_delay(Duration::ms(90)),
            ms(1000),
        );
        tl.tick(ms(1000), &mut sink);
        assert_eq!(tl.tick(ms(1020), &mut sink), Some(ms(1100)));
    }

    #[test]
    fn many_anims_reuse_slots() {
        let mut tl = Timeline::new();
        let mut sink = Sink::default();
        for round in 0..3u64 {
            let ids: Vec<AnimId> = (0..10)
                .map(|k| tl.add(lin(0, 10, 10).target(node(k)), ms(round * 100)))
                .collect();
            assert!(ids.iter().all(|&id| tl.is_running(id)));
            tl.tick(ms(round * 100 + 10), &mut sink);
        }
        assert_eq!(tl.slots.len(), 10);
        assert_eq!(tl.iter().count(), 0);
    }
}
