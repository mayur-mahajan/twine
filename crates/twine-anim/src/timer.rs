//! [`Timers`]: periodic callbacks (LVGL `lv_timer`) that report their next deadline instead of
//! being polled.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use twine_core::{Duration, Instant};

/// Handle of a timer in [`Timers`]. Generational: stale ids are rejected after the timer is
/// removed, even if its slot is reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TimerId {
    index: u16,
    generation: u16,
}

impl TimerId {
    /// An id that never refers to a timer (returned when the table is full).
    pub const DANGLING: TimerId = TimerId {
        index: u16::MAX,
        generation: u16::MAX,
    };
}

/// Context of a timer callback.
///
/// The callback is taken out of [`Timers`] while it runs, so it can add, remove (including
/// itself), pause or reconfigure timers through [`TimerCx::timers`].
pub struct TimerCx<'a> {
    /// The timer being run.
    pub id: TimerId,
    /// The time passed to [`Timers::run`].
    pub now: Instant,
    timers: &'a mut Timers,
    ctx: &'a mut dyn Any,
}

impl TimerCx<'_> {
    /// The context passed to [`Timers::run`]; downcast it to the caller's type.
    pub fn ctx(&mut self) -> &mut dyn Any {
        self.ctx
    }

    /// The timer table (to add, remove or reconfigure timers).
    pub fn timers(&mut self) -> &mut Timers {
        self.timers
    }
}

impl fmt::Debug for TimerCx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimerCx")
            .field("id", &self.id)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

type TimerCb = Box<dyn FnMut(&mut TimerCx<'_>)>;

struct Slot {
    generation: u16,
    live: bool,
    period: Duration,
    last_run: Instant,
    paused: bool,
    ready: bool,
    repeat: Option<u32>,
    auto_delete: bool,
    /// `run` counter value when added: a timer added while running is skipped by that run.
    epoch: u32,
    cb: Option<TimerCb>,
}

impl Slot {
    /// When the timer is due (`ready` timers: `now`).
    fn due_at(&self, now: Instant) -> Instant {
        if self.ready {
            now
        } else {
            (self.last_run + self.period).max(now)
        }
    }
}

/// A table of timers (LVGL `lv_timer`): each calls its callback every `period`, optionally a
/// limited number of times.
///
/// [`Timers::run`] runs the due timers and returns the earliest next deadline, so the caller can
/// sleep until then. A timer that is late by several periods runs **once** and then waits a full
/// period from the time it ran (LVGL). Timers run in creation order.
///
/// ```
/// use twine_anim::Timers;
/// use twine_core::{Duration, Instant};
///
/// let mut timers = Timers::new();
/// let t0 = Instant::ZERO;
/// let mut ctx = 0u32;
/// timers.add(Duration::ms(100), t0, |cx| *cx.ctx().downcast_mut::<u32>().unwrap() += 1);
/// assert_eq!(timers.run(t0 + Duration::ms(50), &mut ctx), Some(t0 + Duration::ms(100)));
/// assert_eq!(ctx, 0);
/// // 350 ms late: runs once, next deadline one period later.
/// assert_eq!(timers.run(t0 + Duration::ms(450), &mut ctx), Some(t0 + Duration::ms(550)));
/// assert_eq!(ctx, 1);
/// ```
#[derive(Default)]
pub struct Timers {
    slots: Vec<Slot>,
    free: Vec<u16>,
    /// Live slot indices in creation order.
    order: Vec<u16>,
    /// Slots removed during `run`, freed when it ends.
    pending_free: Vec<u16>,
    running: bool,
    epoch: u32,
}

impl fmt::Debug for Timers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Timers")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl Timers {
    /// An empty table (does not allocate).
    #[must_use]
    pub const fn new() -> Self {
        Timers {
            slots: Vec::new(),
            free: Vec::new(),
            order: Vec::new(),
            pending_free: Vec::new(),
            running: false,
            epoch: 0,
        }
    }

    /// Adds a timer calling `cb` every `period`, first at `now + period`. It repeats forever
    /// until removed (see [`Timers::set_repeat_count`]).
    pub fn add(
        &mut self,
        period: Duration,
        now: Instant,
        cb: impl FnMut(&mut TimerCx<'_>) + 'static,
    ) -> TimerId {
        let slot = Slot {
            generation: 0,
            live: true,
            period,
            last_run: now,
            paused: false,
            ready: false,
            repeat: None,
            auto_delete: true,
            epoch: self.epoch,
            cb: Some(Box::new(cb)),
        };
        let index = if let Some(i) = self.free.pop() {
            let s = &mut self.slots[usize::from(i)];
            *s = Slot {
                generation: s.generation,
                ..slot
            };
            i
        } else if self.slots.len() < usize::from(u16::MAX) {
            self.slots.push(slot);
            (self.slots.len() - 1) as u16
        } else {
            twine_core::warn!(target: "twine::anim", "timer table full, timer dropped");
            return TimerId::DANGLING;
        };
        self.order.push(index);
        let id = TimerId {
            index,
            generation: self.slots[usize::from(index)].generation,
        };
        twine_core::debug!(target: "twine::anim", "timer {:?} added, period {}", id, period);
        id
    }

    fn slot_mut(&mut self, id: TimerId) -> Option<&mut Slot> {
        self.slots
            .get_mut(usize::from(id.index))
            .filter(|s| s.live && s.generation == id.generation)
    }

    fn slot(&self, id: TimerId) -> Option<&Slot> {
        self.slots
            .get(usize::from(id.index))
            .filter(|s| s.live && s.generation == id.generation)
    }

    /// Removes a timer (safe from inside any timer callback, including its own). Returns `false`
    /// for a stale id.
    pub fn remove(&mut self, id: TimerId) -> bool {
        let running = self.running;
        let Some(s) = self.slot_mut(id) else { return false };
        s.live = false;
        s.cb = None;
        s.generation = s.generation.wrapping_add(1);
        if running {
            self.pending_free.push(id.index);
        } else {
            self.order.retain(|&i| i != id.index);
            self.free.push(id.index);
        }
        twine_core::debug!(target: "twine::anim", "timer {:?} removed", id);
        true
    }

    /// Stops a timer from running until [`Timers::resume`]. Returns `false` for a stale id.
    pub fn pause(&mut self, id: TimerId) -> bool {
        self.slot_mut(id).map(|s| s.paused = true).is_some()
    }

    /// Resumes a paused timer. As in LVGL its period keeps counting from its last run, so a
    /// timer paused for longer than its period runs at the next [`Timers::run`]. Returns `false`
    /// for a stale id.
    pub fn resume(&mut self, id: TimerId) -> bool {
        self.slot_mut(id).map(|s| s.paused = false).is_some()
    }

    /// Changes the period (counted from the last run). Returns `false` for a stale id.
    pub fn set_period(&mut self, id: TimerId, period: Duration) -> bool {
        self.slot_mut(id).map(|s| s.period = period).is_some()
    }

    /// Limits the number of remaining runs (`None` = forever). When it reaches zero the timer is
    /// removed, or paused if [`Timers::set_auto_delete`] was set to `false`. Returns `false` for
    /// a stale id.
    pub fn set_repeat_count(&mut self, id: TimerId, count: Option<u32>) -> bool {
        self.slot_mut(id).map(|s| s.repeat = count).is_some()
    }

    /// Whether a timer whose repeat count ran out is removed (`true`, default) or paused.
    /// Returns `false` for a stale id.
    pub fn set_auto_delete(&mut self, id: TimerId, auto_delete: bool) -> bool {
        self.slot_mut(id).map(|s| s.auto_delete = auto_delete).is_some()
    }

    /// Restarts the period from `now`. Returns `false` for a stale id.
    pub fn reset(&mut self, id: TimerId, now: Instant) -> bool {
        self.slot_mut(id)
            .map(|s| {
                s.last_run = now;
                s.ready = false;
            })
            .is_some()
    }

    /// Makes a timer run at the next [`Timers::run`] regardless of its period. Returns `false`
    /// for a stale id.
    pub fn ready(&mut self, id: TimerId) -> bool {
        self.slot_mut(id).map(|s| s.ready = true).is_some()
    }

    /// Whether `id` refers to a timer in the table (running or paused).
    #[must_use]
    pub fn contains(&self, id: TimerId) -> bool {
        self.slot(id).is_some()
    }

    /// Whether `id` is a paused timer.
    #[must_use]
    pub fn is_paused(&self, id: TimerId) -> bool {
        self.slot(id).is_some_and(|s| s.paused)
    }

    /// Number of timers (running or paused).
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.live).count()
    }

    /// Whether there are no timers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The earliest time a non-paused timer is due, never earlier than `now` (`now` for due and
    /// [`ready`](Timers::ready) timers); `None` when every timer is paused or there is none.
    #[must_use]
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.order
            .iter()
            .map(|&i| &self.slots[usize::from(i)])
            .filter(|s| s.live && !s.paused)
            .map(|s| s.due_at(now))
            .min()
    }

    /// Runs every due timer (in creation order) with `ctx` available to callbacks through
    /// [`TimerCx::ctx`], and returns [`Timers::next_deadline`]. Timers added by callbacks first
    /// run in a later call.
    pub fn run(&mut self, now: Instant, ctx: &mut dyn Any) -> Option<Instant> {
        self.epoch = self.epoch.wrapping_add(1);
        self.running = true;
        let mut pos = 0;
        while pos < self.order.len() {
            let index = self.order[pos];
            pos += 1;
            let slot = &mut self.slots[usize::from(index)];
            if !slot.live || slot.paused || slot.epoch == self.epoch {
                continue;
            }
            let id = TimerId {
                index,
                generation: slot.generation,
            };
            if slot.due_at(now) <= now {
                // LVGL: decrement the repeat count before running; a count already at zero does
                // not run the callback.
                let original = slot.repeat;
                if let Some(n) = &mut slot.repeat {
                    *n = n.saturating_sub(1);
                }
                slot.last_run = now;
                slot.ready = false;
                if original != Some(0) {
                    if let Some(mut cb) = slot.cb.take() {
                        twine_core::trace!(target: "twine::anim", "timer {:?} fired", id);
                        cb(&mut TimerCx {
                            id,
                            now,
                            timers: self,
                            ctx,
                        });
                        if let Some(s) = self.slot_mut(id) {
                            s.cb.get_or_insert(cb);
                        }
                    }
                }
            }
            // The callback may have removed or reconfigured the timer.
            let Some(slot) = self.slot_mut(id) else { continue };
            if slot.repeat == Some(0) {
                if slot.auto_delete {
                    self.remove(id);
                } else {
                    slot.paused = true;
                    twine_core::debug!(target: "twine::anim", "timer {:?} paused: repeat count over", id);
                }
            }
        }
        self.running = false;
        if !self.pending_free.is_empty() {
            let slots = &self.slots;
            self.order.retain(|&i| slots[usize::from(i)].live);
            self.free.append(&mut self.pending_free);
        }
        self.next_deadline(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec;
    use core::cell::RefCell;

    fn ms(v: u64) -> Instant {
        Instant::from_millis(v)
    }

    type Log = Rc<RefCell<Vec<(u16, u64)>>>;

    /// A callback logging `(tag, now in ms)`.
    fn rec(log: &Log, tag: u16) -> impl FnMut(&mut TimerCx<'_>) + 'static {
        let log = log.clone();
        move |cx| log.borrow_mut().push((tag, cx.now.as_millis()))
    }

    #[test]
    fn timer_fires_at_period() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        t.add(Duration::ms(100), ms(0), rec(&log, 1));
        assert_eq!(t.run(ms(99), &mut ()), Some(ms(100)));
        assert_eq!(t.run(ms(100), &mut ()), Some(ms(200)));
        assert_eq!(t.run(ms(150), &mut ()), Some(ms(200)));
        assert_eq!(t.run(ms(200), &mut ()), Some(ms(300)));
        assert_eq!(*log.borrow(), [(1, 100), (1, 200)]);
    }

    #[test]
    fn late_timer_runs_once() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        t.add(Duration::ms(10), ms(0), rec(&log, 1));
        assert_eq!(t.run(ms(1000), &mut ()), Some(ms(1010)));
        assert_eq!(*log.borrow(), [(1, 1000)]);
    }

    #[test]
    fn repeat_count_removes() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        let id = t.add(Duration::ms(10), ms(0), rec(&log, 1));
        t.set_repeat_count(id, Some(2));
        t.run(ms(10), &mut ());
        assert!(t.contains(id));
        assert_eq!(t.run(ms(20), &mut ()), None);
        assert!(!t.contains(id));
        assert!(t.is_empty());
        t.run(ms(30), &mut ());
        assert_eq!(*log.borrow(), [(1, 10), (1, 20)]);
        // Count 0: removed at the next run without running.
        let id = t.add(Duration::ms(10), ms(30), rec(&log, 2));
        t.set_repeat_count(id, Some(0));
        t.run(ms(31), &mut ());
        assert!(!t.contains(id));
        assert_eq!(log.borrow().len(), 2);
    }

    #[test]
    fn auto_delete_false_pauses() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        let id = t.add(Duration::ms(10), ms(0), rec(&log, 1));
        t.set_repeat_count(id, Some(1));
        t.set_auto_delete(id, false);
        assert_eq!(t.run(ms(10), &mut ()), None);
        assert!(t.contains(id) && t.is_paused(id));
        // Restart it with a new count.
        t.set_repeat_count(id, Some(1));
        t.resume(id);
        t.reset(id, ms(100));
        t.run(ms(110), &mut ());
        assert_eq!(*log.borrow(), [(1, 10), (1, 110)]);
        assert!(t.is_paused(id));
    }

    #[test]
    fn pause_resume() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        let id = t.add(Duration::ms(10), ms(0), rec(&log, 1));
        assert!(t.pause(id));
        assert_eq!(t.run(ms(50), &mut ()), None);
        assert!(log.borrow().is_empty());
        assert!(t.resume(id));
        // LVGL: the period counts from the last run, so it is overdue now.
        assert_eq!(t.next_deadline(ms(50)), Some(ms(50)));
        assert_eq!(t.run(ms(50), &mut ()), Some(ms(60)));
        assert_eq!(*log.borrow(), [(1, 50)]);
    }

    #[test]
    fn ready_runs_next() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        let id = t.add(Duration::secs(10), ms(0), rec(&log, 1));
        assert_eq!(t.next_deadline(ms(5)), Some(ms(10_000)));
        assert!(t.ready(id));
        assert_eq!(t.next_deadline(ms(5)), Some(ms(5)));
        assert_eq!(t.run(ms(5), &mut ()), Some(ms(10_005)));
        assert_eq!(*log.borrow(), [(1, 5)]);
    }

    #[test]
    fn callback_removes_itself_safely() {
        let log: Log = Rc::default();
        let mut t = Timers::new();
        let l2 = log.clone();
        let first = t.add(Duration::ms(10), ms(0), move |cx| {
            l2.borrow_mut().push((1, cx.now.as_millis()));
            let id = cx.id;
            assert!(cx.timers().remove(id));
            assert!(!cx.timers().remove(id));
            // Adding from a callback is allowed too; the new timer runs in a later call.
            let now = cx.now;
            cx.timers().add(Duration::ZERO, now, |_| {});
        });
        let second = t.add(Duration::ms(10), ms(0), rec(&log, 2));
        assert_eq!(
            t.run(ms(10), &mut ()),
            Some(ms(10)),
            "the new zero-period timer is due"
        );
        assert!(!t.contains(first) && t.contains(second));
        assert_eq!(*log.borrow(), [(1, 10), (2, 10)]);
        assert_eq!(t.len(), 2);
        // The freed slot is reused; the stale id stays invalid.
        let third = t.add(Duration::ms(1), ms(10), |_| {});
        assert_eq!(third.index, first.index);
        assert!(!t.contains(first));
    }

    #[test]
    fn callback_can_remove_other_timer() {
        let mut t = Timers::new();
        let log: Log = Rc::default();
        let victim = Rc::new(RefCell::new(None::<TimerId>));
        let v = victim.clone();
        t.add(Duration::ms(10), ms(0), move |cx| {
            let id = v.borrow().unwrap();
            cx.timers().remove(id);
        });
        *victim.borrow_mut() = Some(t.add(Duration::ms(10), ms(0), rec(&log, 2)));
        t.run(ms(10), &mut ());
        assert!(log.borrow().is_empty());
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn deadline_is_earliest() {
        let mut t = Timers::new();
        t.add(Duration::ms(300), ms(0), |_| {});
        let b = t.add(Duration::ms(70), ms(0), |_| {});
        t.add(Duration::ms(100), ms(0), |_| {});
        assert_eq!(t.run(ms(0), &mut ()), Some(ms(70)));
        assert_eq!(t.run(ms(70), &mut ()), Some(ms(100)));
        t.set_period(b, Duration::ms(10));
        assert_eq!(t.next_deadline(ms(71)), Some(ms(80)));
        // Never in the past.
        assert_eq!(t.next_deadline(ms(500)), Some(ms(500)));
    }

    #[test]
    fn no_deadline_when_all_paused() {
        let mut t = Timers::new();
        let ids = vec![
            t.add(Duration::ms(1), ms(0), |_| {}),
            t.add(Duration::ms(2), ms(0), |_| {}),
        ];
        for &id in &ids {
            t.pause(id);
        }
        assert_eq!(t.run(ms(10), &mut ()), None);
        assert_eq!(Timers::new().run(ms(0), &mut ()), None);
    }

    #[test]
    fn creation_order_and_ctx() {
        let mut t = Timers::new();
        let mut ctx: Vec<u16> = Vec::new();
        for tag in [3u16, 1, 2] {
            t.add(Duration::ms(5), ms(0), move |cx| {
                cx.ctx().downcast_mut::<Vec<u16>>().unwrap().push(tag);
            });
        }
        t.run(ms(5), &mut ctx);
        assert_eq!(ctx, [3, 1, 2]);
        assert!(!t.pause(TimerId::DANGLING) && !t.ready(TimerId::DANGLING));
    }
}
