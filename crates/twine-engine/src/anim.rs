//! Animations and timers in the engine: [`Engine::anim_start`], [`Engine::timer_add`] and the
//! update-cycle steps [`Engine::run_timers`] / [`Engine::run_anims`].
//!
//! The engine owns a [`Timeline`] and a [`Timers`] table. Their callbacks cannot receive the
//! engine directly (the engine owns them), so ticking only *records* what has to happen in a
//! [`Deferred`] queue (reused, no allocation in steady state): values for node properties,
//! calls of exec closures and timer callbacks. Right after the tick the queue is drained with
//! full access to the engine, in order. Node properties are written through the same idempotent
//! setters as user code (P3), so invalidation stays precise.
//!
//! Time is wall-clock. Animations and timers started between two updates (outside
//! `run_timers` / `run_anims`) start at the time of the next update, so a UI that slept for a
//! long time does not skip the beginning of a new animation.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;

use twine_anim::{Anim, AnimCx, AnimId, AnimProp, AnimTarget, TickSink, Timeline, TimerId, Timers};
use twine_core::{Angle, Duration, Instant, Opa, Scale};
use twine_style::{Length, PropId, Selector, StyleProp, StyleValue};

use crate::{Engine, NodeId, Wake, fmt_node_id};

/// A shared exec closure of [`Engine::anim_start_fn`].
type ExecRc = Rc<RefCell<dyn FnMut(&mut Engine, i32)>>;
/// A shared timer callback of [`Engine::timer_add`].
type TimerRc = Rc<RefCell<dyn FnMut(&mut Engine, TimerId)>>;

/// Work recorded while the timeline or the timers run, executed afterwards with the engine.
pub(crate) enum Op {
    /// A value for a node property ([`AnimTarget::Node`]).
    Apply(AnimTarget, i32),
    /// A value for an exec closure.
    Exec(ExecRc, i32),
    /// A due timer.
    Timer(TimerRc, TimerId),
    /// A call queued with [`Deferred::defer`].
    Call(Box<dyn FnOnce(&mut Engine)>),
    /// A screen load animation event (display index, event).
    Screen(u8, crate::screen_anim::ScreenOp),
}

/// The context of the engine's animation and timer callbacks: a queue of work executed with
/// the engine right after the timeline or the timers have run.
///
/// The `ctx()` of the [`AnimCx`] passed to the `on_start` / `on_repeat` / `on_complete`
/// callbacks of animations started through the engine is a `Deferred`; use [`defer`] to run
/// code with the engine from there.
///
/// ```
/// use twine_anim::Anim;
/// use twine_core::{Duration, Instant};
/// use twine_engine::{Engine, EngineConfig, Obj, defer};
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let root = e.create_root(Box::new(Obj)).unwrap();
/// let a = Anim::new(0, 10).duration(Duration::ms(10)).on_complete(move |cx| {
///     defer(cx, move |engine| engine.delete(root).unwrap());
/// });
/// e.anim_start_fn(a, |_, _| {});
/// e.run_anims(Instant::ZERO);
/// e.run_anims(Instant::from_millis(20));
/// assert!(!e.tree().contains(root));
/// ```
#[derive(Default)]
pub struct Deferred {
    pub(crate) ops: Vec<Op>,
}

impl core::fmt::Debug for Deferred {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Deferred").field("ops", &self.ops.len()).finish()
    }
}

impl Deferred {
    /// Runs `f` with the engine once the current animation or timer step is done (in queue
    /// order, after the values computed before). Allocates the boxed closure.
    pub fn defer(&mut self, f: impl FnOnce(&mut Engine) + 'static) {
        self.ops.push(Op::Call(Box::new(f)));
    }
}

impl TickSink for Deferred {
    fn apply(&mut self, target: AnimTarget, value: i32) {
        self.ops.push(Op::Apply(target, value));
    }

    fn ctx(&mut self) -> &mut dyn Any {
        self
    }
}

/// Runs `f` with the engine after the animation step of the callback `cx` belongs to (see
/// [`Deferred`]). Returns `false` (and logs `warn!`) if `cx` does not come from an engine
/// animation.
pub fn defer(cx: &mut AnimCx<'_>, f: impl FnOnce(&mut Engine) + 'static) -> bool {
    if let Some(q) = cx.ctx().downcast_mut::<Deferred>() {
        q.defer(f);
        true
    } else {
        twine_core::warn!(target: "twine::anim", "defer: not an engine animation callback");
        false
    }
}

/// What to do with an animation at the next animation step (time known).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// (Re)start from the beginning.
    Restart,
    /// Continue after a pause.
    Resume,
}

/// Warnings logged once per engine.
#[derive(Clone, Copy, Debug, Default)]
struct Warned {
    style_prop: bool,
}

/// The engine's animation and timer state.
#[derive(Default)]
pub(crate) struct AnimState {
    pub(crate) timeline: Timeline,
    pub(crate) timers: Timers,
    pub(crate) deferred: Deferred,
    /// The time of the running `run_timers` / `run_anims` (`None` between them).
    clock: Option<Instant>,
    /// The latest known time (last update).
    last_now: Instant,
    /// Operations waiting for the next animation step's time.
    pending: Vec<(AnimId, Pending)>,
    /// Timers created or reset between updates (period restarts at the next update).
    pending_timers: Vec<TimerId>,
    /// An animation was added after the last tick.
    added_since_tick: bool,
    /// The deadline returned by the last tick.
    deadline: Option<Instant>,
    /// Timers removed or paused while the due timers' callbacks run (their queued runs are
    /// skipped).
    timer_skip: Vec<TimerId>,
    draining_timers: bool,
    warned: Warned,
}

impl core::fmt::Debug for AnimState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimState")
            .field("timeline", &self.timeline)
            .field("timers", &self.timers)
            .finish_non_exhaustive()
    }
}

impl AnimState {
    /// The time to start something at: the running step's time, else the last known time.
    fn now(&self) -> Instant {
        self.clock.unwrap_or(self.last_now)
    }

    fn after_add(&mut self, id: AnimId) {
        self.added_since_tick = true;
        if self.clock.is_none() {
            self.pending.push((id, Pending::Restart));
        }
    }
}

/// Clamps to `0..=max`.
fn clamp_u(v: i32, max: i32) -> i32 {
    v.clamp(0, max)
}

impl Engine {
    /// Starts `anim` on property `prop` of `node` (its target is set to
    /// [`AnimTarget::Node`]); a running animation of the same node and property is replaced
    /// (LVGL). Values are written through the idempotent setters:
    ///
    /// | `AnimProp` | writes |
    /// |------------|--------|
    /// | `X`, `Y`, `Width`, `Height` | local `X` / `Y` / `Width` / `Height` in px (layout runs in the same update) |
    /// | `Opa` | local `Opa` (clamped to 0..=255) |
    /// | `TranslateX`, `TranslateY` | local `TranslateX` / `TranslateY` in px |
    /// | `ScaleX`, `ScaleY`, `Rotation` | local `TransformScaleX` / `TransformScaleY` / `TransformRotation` |
    /// | `StyleProp(id)` | the local integer-like style property `id` (ints, px lengths, opacities, angles, scales; use [`anim_start_fn`](Self::anim_start_fn) for colors) |
    /// | `ScrollX`, `ScrollY` | the scroll offset (content moved without bounds, `Scroll` events; see [`scroll_by`](Self::scroll_by)) |
    /// | `Value`, `Custom(id)` | [`Widget::anim_value`](crate::Widget::anim_value) / [`Widget::anim_custom`](crate::Widget::anim_custom) |
    ///
    /// The animation starts at the current update (or the next one when called between
    /// updates). Deleting the node stops it.
    ///
    /// ```
    /// use twine_anim::{Anim, AnimProp};
    /// use twine_core::{Duration, Instant};
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_style::{Part, PropId};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// let n = e.create(root, Box::new(Obj)).unwrap();
    /// e.anim_start(n, AnimProp::Opa, Anim::new(0, 200).duration(Duration::ms(100)));
    /// e.run_anims(Instant::ZERO);
    /// e.run_anims(Instant::from_millis(50));
    /// assert_eq!(e.style_opa(n, Part::Main, PropId::Opa).0, 100);
    /// ```
    pub fn anim_start(&mut self, node: NodeId, prop: AnimProp, anim: Anim) -> AnimId {
        if !self.tree.contains(node) {
            twine_core::warn!(target: "twine::anim", "anim_start: node {} not found", fmt_node_id(node));
            return AnimId::DANGLING;
        }
        let anim = anim.target(AnimTarget::Node(node.to_raw(), prop));
        let now = self.anim.now();
        let id = self.anim.timeline.add(anim, now);
        self.anim.after_add(id);
        id
    }

    /// Starts `anim` with a closure receiving the engine and each new value (LVGL `exec_cb`).
    /// The closure runs right after the animation step, like every engine animation write.
    ///
    /// ```
    /// use twine_anim::Anim;
    /// use twine_core::{Color, Duration, Instant};
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_style::{Part, PropId, Selector, StyleProp};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// e.anim_start_fn(Anim::new(0, 255).duration(Duration::ms(10)), move |e, v| {
    ///     e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(Color::new(v as u8, 0, 0)));
    /// });
    /// e.run_anims(Instant::ZERO);
    /// e.run_anims(Instant::from_millis(10));
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::new(255, 0, 0));
    /// ```
    pub fn anim_start_fn(&mut self, anim: Anim, exec: impl FnMut(&mut Engine, i32) + 'static) -> AnimId {
        let f: ExecRc = Rc::new(RefCell::new(exec));
        let now = self.anim.now();
        let id = self.anim.timeline.add_fn(
            anim,
            move |ctx, v| {
                if let Some(q) = ctx.downcast_mut::<Deferred>() {
                    q.ops.push(Op::Exec(f.clone(), v));
                }
            },
            now,
        );
        self.anim.after_add(id);
        id
    }

    /// Adds an animation whose exec closure pushes engine work (internal: transitions, screen
    /// loads).
    pub(crate) fn anim_start_internal(
        &mut self,
        anim: Anim,
        exec: impl FnMut(&mut dyn Any, i32) + 'static,
    ) -> AnimId {
        let now = self.anim.now();
        let id = self.anim.timeline.add_fn(anim, exec, now);
        self.anim.after_add(id);
        id
    }

    /// Adds an animation whose values arrive as [`Op::Apply`] of its own target (internal:
    /// style transitions use [`AnimTarget::Custom`] with their serial). Allocates nothing once
    /// the timeline has a free slot.
    pub(crate) fn anim_start_target(&mut self, anim: Anim) -> AnimId {
        let now = self.anim.now();
        let id = self.anim.timeline.add(anim, now);
        self.anim.after_add(id);
        id
    }

    /// Stops an animation (no callbacks). Returns `false` for a finished or unknown id.
    pub fn anim_stop(&mut self, id: AnimId) -> bool {
        self.anim.pending.retain(|(p, _)| *p != id);
        self.anim.timeline.remove(id)
    }

    /// Pauses an animation at its current value. Returns `false` for a finished or unknown id.
    pub fn anim_pause(&mut self, id: AnimId) -> bool {
        let now = self.anim.now();
        self.anim
            .pending
            .retain(|(p, k)| *p != id || *k != Pending::Resume);
        self.anim.timeline.pause(id, now)
    }

    /// Continues a paused animation where it stopped. Returns `false` for a finished or
    /// unknown id.
    pub fn anim_resume(&mut self, id: AnimId) -> bool {
        if !self.anim.timeline.is_paused(id) {
            return self.anim.timeline.get(id).is_some();
        }
        if let Some(now) = self.anim.clock {
            self.anim.timeline.resume(id, now)
        } else {
            self.anim.pending.push((id, Pending::Resume));
            self.anim.added_since_tick = true;
            true
        }
    }

    /// Restarts an animation from its beginning (delay included). Returns `false` for a
    /// finished or unknown id.
    pub fn anim_restart(&mut self, id: AnimId) -> bool {
        let now = self.anim.now();
        if !self.anim.timeline.restart(id, now) {
            return false;
        }
        self.anim.after_add(id);
        true
    }

    /// Whether `id` is a live animation (running or paused).
    #[must_use]
    pub fn anim_exists(&self, id: AnimId) -> bool {
        self.anim.timeline.get(id).is_some()
    }

    /// Whether `id` is a paused animation.
    #[must_use]
    pub fn anim_is_paused(&self, id: AnimId) -> bool {
        self.anim.timeline.is_paused(id)
    }

    /// The animations running on properties of `node` (started with
    /// [`anim_start`](Self::anim_start)).
    pub fn anims_of(&self, node: NodeId) -> impl Iterator<Item = AnimId> + '_ {
        let key = node.to_raw();
        self.anim
            .timeline
            .iter()
            .filter(move |(_, a)| matches!(a.target, AnimTarget::Node(k, _) if k == key))
            .map(|(id, _)| id)
    }

    /// Number of live animations (running or paused, including style transitions and screen
    /// load animations).
    #[must_use]
    pub fn anim_count(&self) -> usize {
        self.anim.timeline.running_count()
    }

    /// Adds a timer calling `cb(engine, id)` every `period` (LVGL `lv_timer_create`), first one
    /// period after the current update (or after the next update when called between
    /// updates). A timer late by several periods runs once. `step` wakes up exactly when the
    /// next timer is due.
    ///
    /// ```
    /// use core::cell::Cell;
    /// use std::rc::Rc;
    /// use twine_core::{Duration, Instant};
    /// use twine_engine::{Engine, EngineConfig};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let runs = Rc::new(Cell::new(0));
    /// let r = runs.clone();
    /// e.timer_add(Duration::ms(100), move |_, _| r.set(r.get() + 1));
    /// e.run_timers(Instant::ZERO); // starts counting
    /// e.run_timers(Instant::from_millis(100));
    /// assert_eq!(runs.get(), 1);
    /// ```
    pub fn timer_add(&mut self, period: Duration, cb: impl FnMut(&mut Engine, TimerId) + 'static) -> TimerId {
        let f: TimerRc = Rc::new(RefCell::new(cb));
        let now = self.anim.now();
        let id = self.anim.timers.add(period, now, move |cx| {
            let id = cx.id;
            if let Some(q) = cx.ctx().downcast_mut::<Deferred>() {
                q.ops.push(Op::Timer(f.clone(), id));
            }
        });
        if self.anim.clock.is_none() {
            self.anim.pending_timers.push(id);
        }
        id
    }

    fn timer_skip(&mut self, id: TimerId) {
        if self.anim.draining_timers && !self.anim.timer_skip.contains(&id) {
            self.anim.timer_skip.push(id);
        }
    }

    /// Number of timers (running or paused).
    #[must_use]
    pub fn timer_count(&self) -> usize {
        self.anim.timers.len()
    }

    /// Removes a timer (also from inside its own callback). Returns `false` for an unknown id.
    pub fn timer_remove(&mut self, id: TimerId) -> bool {
        self.timer_skip(id);
        self.anim.pending_timers.retain(|t| *t != id);
        self.anim.timers.remove(id)
    }

    /// Pauses a timer. Returns `false` for an unknown id.
    pub fn timer_pause(&mut self, id: TimerId) -> bool {
        self.timer_skip(id);
        self.anim.timers.pause(id)
    }

    /// Resumes a paused timer; its period keeps counting from its last run (LVGL). Returns
    /// `false` for an unknown id.
    pub fn timer_resume(&mut self, id: TimerId) -> bool {
        self.anim.timers.resume(id)
    }

    /// Changes a timer's period. Returns `false` for an unknown id.
    pub fn timer_set_period(&mut self, id: TimerId, period: Duration) -> bool {
        self.anim.timers.set_period(id, period)
    }

    /// Runs a timer at the next update regardless of its period. Returns `false` for an
    /// unknown id.
    pub fn timer_ready(&mut self, id: TimerId) -> bool {
        self.anim.timers.ready(id)
    }

    /// Restarts a timer's period from now (from the next update when called between updates).
    /// Returns `false` for an unknown id.
    pub fn timer_reset(&mut self, id: TimerId) -> bool {
        let now = self.anim.now();
        if !self.anim.timers.reset(id, now) {
            return false;
        }
        if self.anim.clock.is_none() && !self.anim.pending_timers.contains(&id) {
            self.anim.pending_timers.push(id);
        }
        true
    }

    /// Limits the number of remaining runs of a timer (`None` = forever); at zero it is
    /// removed. Returns `false` for an unknown id.
    pub fn timer_set_repeat_count(&mut self, id: TimerId, count: Option<u32>) -> bool {
        self.anim.timers.set_repeat_count(id, count)
    }

    /// Whether `id` is a timer of the engine (running or paused).
    #[must_use]
    pub fn timer_exists(&self, id: TimerId) -> bool {
        self.anim.timers.contains(id)
    }

    /// Whether `id` is a paused timer ([`timer_pause`](Self::timer_pause); `false` for running
    /// and unknown timers).
    #[must_use]
    pub fn timer_is_paused(&self, id: TimerId) -> bool {
        self.anim.timers.is_paused(id)
    }

    /// Runs the due timers at `now` (update cycle step 4; called by [`step`](Self::step)).
    pub fn run_timers(&mut self, now: Instant) {
        self.anim.last_now = now;
        self.anim.clock = Some(now);
        let pending = core::mem::take(&mut self.anim.pending_timers);
        for &id in &pending {
            self.anim.timers.reset(id, now);
        }
        self.anim.pending_timers = pending;
        self.anim.pending_timers.clear();
        let AnimState { timers, deferred, .. } = &mut self.anim;
        timers.run(now, deferred);
        self.anim.draining_timers = true;
        self.drain_ops();
        self.anim.draining_timers = false;
        self.anim.timer_skip.clear();
        self.anim.clock = None;
    }

    /// Advances every animation to `now` and applies the new values (update cycle step 5;
    /// called by [`step`](Self::step)). Animations started by the applied values or callbacks
    /// get their first values in the same call.
    pub fn run_anims(&mut self, now: Instant) {
        self.anim.last_now = now;
        self.anim.clock = Some(now);
        let pending = core::mem::take(&mut self.anim.pending);
        for &(id, what) in &pending {
            match what {
                Pending::Restart => self.anim.timeline.restart(id, now),
                Pending::Resume => self.anim.timeline.resume(id, now),
            };
        }
        self.anim.pending = pending;
        self.anim.pending.clear();
        for _ in 0..4 {
            self.anim.added_since_tick = false;
            let AnimState {
                timeline, deferred, ..
            } = &mut self.anim;
            self.anim.deadline = timeline.tick(now, deferred);
            self.drain_ops();
            if !self.anim.added_since_tick {
                break;
            }
        }
        self.anim.clock = None;
    }

    /// Records `now` as the latest known time (start of an update).
    pub(crate) fn set_anim_now(&mut self, now: Instant) {
        self.anim.last_now = now;
    }

    /// When animations and timers need the next update: `Now` for animations started since
    /// the last animation step, the next frame (at least `refr_period` after `now`) while an
    /// animation plays, the end of a delay, the next due timer; `Idle` when nothing runs.
    pub(crate) fn anim_wake(&self, now: Instant) -> Wake {
        if self.anim.added_since_tick || !self.anim.pending.is_empty() || !self.anim.pending_timers.is_empty()
        {
            return Wake::Now;
        }
        let anims = self.anim.deadline.map_or(Wake::Idle, |d| {
            let last = self
                .displays
                .iter()
                .filter_map(|d| d.refresher.last_refresh)
                .max();
            let frame = last
                .map(|l| l + self.config.refr_period)
                .filter(|t| *t > now)
                .unwrap_or(now + self.config.refr_period);
            Wake::At(d.max(frame))
        });
        let timers = self.anim.timers.next_deadline(now).map_or(Wake::Idle, Wake::At);
        anims.min(timers)
    }

    /// Executes the queued work of the last timer run or animation tick.
    fn drain_ops(&mut self) {
        loop {
            let mut ops = core::mem::take(&mut self.anim.deferred.ops);
            if ops.is_empty() {
                self.anim.deferred.ops = ops;
                return;
            }
            for op in ops.drain(..) {
                self.run_op(op);
            }
            // Keep the (empty) allocation; work queued meanwhile runs in the next round.
            core::mem::swap(&mut ops, &mut self.anim.deferred.ops);
            if ops.is_empty() {
                return;
            }
            self.anim.deferred.ops.append(&mut ops);
        }
    }

    fn run_op(&mut self, op: Op) {
        match op {
            Op::Apply(AnimTarget::Node(key, prop), v) => {
                let id = NodeId::from_raw(key);
                if self.tree.contains(id) {
                    self.anim_apply(id, prop, v);
                }
            }
            // The engine's own timeline uses custom targets only for style transitions.
            Op::Apply(AnimTarget::Custom(serial), v) => self.transition_value(serial, v),
            Op::Exec(f, v) => match f.try_borrow_mut() {
                Ok(mut f) => f(self, v),
                Err(_) => {
                    twine_core::warn!(target: "twine::anim", "re-entrant animation exec closure skipped");
                }
            },
            Op::Timer(f, id) => {
                if self.anim.timer_skip.contains(&id) {
                    return;
                }
                twine_core::trace!(target: "twine::anim", "timer {:?} callback", id);
                match f.try_borrow_mut() {
                    Ok(mut f) => f(self, id),
                    Err(_) => {
                        twine_core::warn!(target: "twine::anim", "re-entrant timer callback skipped");
                    }
                }
            }
            Op::Call(f) => f(self),
            Op::Screen(d, s) => self.screen_anim_op(usize::from(d), s),
        }
    }

    /// Writes an animated value to a node property (see [`anim_start`](Self::anim_start)).
    fn anim_apply(&mut self, id: NodeId, prop: AnimProp, v: i32) {
        let px = |v| Length::Px(v);
        let p = match prop {
            AnimProp::X => StyleProp::X(px(v)),
            AnimProp::Y => StyleProp::Y(px(v)),
            AnimProp::Width => StyleProp::Width(px(v)),
            AnimProp::Height => StyleProp::Height(px(v)),
            AnimProp::Opa => StyleProp::Opa(Opa(clamp_u(v, 255) as u8)),
            AnimProp::TranslateX => StyleProp::TranslateX(px(v)),
            AnimProp::TranslateY => StyleProp::TranslateY(px(v)),
            AnimProp::ScaleX => StyleProp::TransformScaleX(Scale(clamp_u(v, i32::from(u16::MAX)) as u16)),
            AnimProp::ScaleY => StyleProp::TransformScaleY(Scale(clamp_u(v, i32::from(u16::MAX)) as u16)),
            AnimProp::Rotation => StyleProp::TransformRotation(Angle(v)),
            AnimProp::StyleProp(raw) => {
                let Some(p) = PropId::from_u8(raw).and_then(|pid| int_prop(pid, v)) else {
                    if !self.anim.warned.style_prop {
                        self.anim.warned.style_prop = true;
                        twine_core::warn!(
                            target: "twine::anim",
                            "AnimProp::StyleProp({}) is not an integer-like property (animate it with anim_start_fn)",
                            raw
                        );
                    }
                    return;
                };
                p
            }
            AnimProp::ScrollX => {
                self.anim_scroll_to(id, crate::scroll::Axis::X, v);
                return;
            }
            AnimProp::ScrollY => {
                self.anim_scroll_to(id, crate::scroll::Axis::Y, v);
                return;
            }
            AnimProp::Value => {
                self.with_widget(id, |w, cx| w.anim_value(cx, v));
                return;
            }
            AnimProp::Custom(c) => {
                self.with_widget(id, |w, cx| w.anim_custom(cx, c, v));
                return;
            }
        };
        self.set_local_prop(id, Selector::MAIN, p);
    }

    /// Calls `f` with `id`'s widget taken out of its node.
    fn with_widget(&mut self, id: NodeId, f: impl FnOnce(&mut dyn crate::Widget, &mut crate::WidgetCx<'_>)) {
        let Some(n) = self.tree.node_mut(id) else {
            return;
        };
        let mut w: Box<dyn crate::Widget> = core::mem::replace(&mut n.widget, Box::new(crate::obj::Detached));
        f(&mut *w, &mut crate::WidgetCx::new(self, id));
        self.restore_widget(id, w);
    }

    /// Stops every animation of the deleted nodes `ids` (node properties and transitions).
    pub(crate) fn anims_forget_nodes(&mut self, ids: &[NodeId]) {
        for &id in ids {
            let n = self.anim.timeline.remove_target(id.to_raw());
            if n > 0 {
                twine_core::debug!(target: "twine::anim", "{} animations of deleted {} removed", n, fmt_node_id(id));
            }
        }
        self.transitions_forget_nodes(ids);
    }
}

/// `prop` set to the integer `v` in its own unit (`None` for non-integer properties).
fn int_prop(prop: PropId, v: i32) -> Option<StyleProp> {
    let value = match prop.meta().default {
        StyleValue::Int(_) => StyleValue::Int(v),
        StyleValue::Length(_) => StyleValue::Length(Length::Px(v)),
        StyleValue::Opa(_) => StyleValue::Opa(Opa(clamp_u(v, 255) as u8)),
        StyleValue::Angle(_) => StyleValue::Angle(Angle(v)),
        StyleValue::Scale(_) => StyleValue::Scale(Scale(clamp_u(v, i32::from(u16::MAX)) as u16)),
        _ => return None,
    };
    StyleProp::from_value(prop, value)
}
