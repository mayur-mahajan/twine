//! Bounded flush waits ([`EngineConfig::flush_timeout`](crate::EngineConfig::flush_timeout)).
//!
//! Every flush handed to a driver (a partial draw buffer in flight, a framebuffer swap) is
//! stamped when it starts. Whenever the engine finds it still unfinished — while spinning for a
//! free buffer, or at the start of a later step — it compares the stamp with the current time.
//! Past the timeout the flush counts as hung: the fault is raised once, the display is marked
//! failed and no frame starts until the driver hands the buffer back.
//!
//! The clock is the `hires_timer` when there is one (it advances inside a step, so a spinning
//! wait can be bounded), else the step's `now` (which only advances between steps, so a bounded
//! wait yields instead of spinning; see `acquire_slot`).

use twine_core::{Duration, Instant, Rect};

use super::Strategy;
use crate::{Engine, InvalidateReason};

/// A point in time on the clock that bounds flush waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitStamp {
    /// `hires_timer` microseconds.
    Hires(u64),
    /// The engine time (the running step's `now`).
    Step(Instant),
}

impl WaitStamp {
    /// Microseconds from `self` to `later`, `None` when they come from different clocks (the
    /// `hires_timer` was set or removed meanwhile).
    fn micros_until(self, later: WaitStamp) -> Option<u64> {
        match (self, later) {
            (WaitStamp::Hires(a), WaitStamp::Hires(b)) => Some(b.saturating_sub(a)),
            (WaitStamp::Step(a), WaitStamp::Step(b)) => Some(b.saturating_duration_since(a).as_micros()),
            _ => None,
        }
    }
}

/// A partial draw buffer the driver holds: what it carries and since when.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Flight {
    /// The logical chunk rendered into it (redrawn if the flush times out).
    pub(crate) area: Rect,
    /// When `begin_flush` was called.
    pub(crate) since: WaitStamp,
    /// The flush timed out (the fault was raised for it).
    pub(crate) timed_out: bool,
}

/// A framebuffer swap in progress.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingPresent {
    pub(crate) since: WaitStamp,
    pub(crate) timed_out: bool,
}

/// The outcome of a wait check.
enum Check {
    /// Nothing to wait for, or still within the timeout.
    Fine,
    /// Past the timeout now (first time): `(micros waited, areas to redraw)`.
    Expired(u64, [Option<Rect>; 2]),
    /// Already timed out earlier and still not finished.
    Stuck,
}

impl Engine {
    /// The current time on the clock that bounds flush waits.
    pub(crate) fn wait_stamp(&self) -> WaitStamp {
        match self.config.hires_timer {
            Some(f) => WaitStamp::Hires(f().as_micros()),
            None => WaitStamp::Step(self.anim_now()),
        }
    }

    /// Checks the oldest flush of display `d` (partial buffers or framebuffer swap) against
    /// [`flush_timeout`](crate::EngineConfig::flush_timeout). The first time it is exceeded,
    /// raises `FlushTimeout`, marks the display failed and marks the chunks in flight dirty
    /// again (unless `FlushPolicy::Ignore`). Returns whether the display's driver is hung: a
    /// flush timed out and has not finished since.
    pub(crate) fn check_flush_timeout(&mut self, d: usize) -> bool {
        let timeout = self.config.flush_timeout.map(Duration::as_micros);
        let now = self.wait_stamp();
        let check = match &mut self.displays[d].refresher.strategy {
            Strategy::Partial(p) => {
                let Some(front) = p.in_flight.front().map(|&s| usize::from(s)) else {
                    return false;
                };
                match p.flights[front].as_mut() {
                    None => Check::Fine,
                    Some(f) if f.timed_out => Check::Stuck,
                    Some(f) => match (timeout, f.since.micros_until(now)) {
                        (Some(t), Some(waited)) if waited >= t => {
                            let mut areas = [None; 2];
                            for (i, &s) in p.in_flight.iter().enumerate() {
                                if let Some(f) = p.flights[usize::from(s)].as_mut() {
                                    if !f.timed_out {
                                        f.timed_out = true;
                                        areas[i] = Some(f.area);
                                    }
                                }
                            }
                            Check::Expired(waited, areas)
                        }
                        (_, None) => {
                            // The clock changed: measure from now on the new one.
                            f.since = now;
                            Check::Fine
                        }
                        _ => Check::Fine,
                    },
                }
            }
            Strategy::Full(st) => match st.present.as_mut() {
                None => Check::Fine,
                Some(pp) if pp.timed_out => Check::Stuck,
                Some(pp) => match (timeout, pp.since.micros_until(now)) {
                    (Some(t), Some(waited)) if waited >= t => {
                        pp.timed_out = true;
                        Check::Expired(waited, [None; 2])
                    }
                    (_, None) => {
                        pp.since = now;
                        Check::Fine
                    }
                    _ => Check::Fine,
                },
            },
            Strategy::Direct(_) => Check::Fine,
        };
        match check {
            Check::Fine => false,
            Check::Stuck => true,
            Check::Expired(waited, areas) => {
                if self.flush_timed_out(d, Duration::from_micros(waited)) {
                    for a in areas.into_iter().flatten() {
                        self.add_dirty(d, a, InvalidateReason::FlushRetry);
                    }
                }
                true
            }
        }
    }

    /// Measures the flushes of display `d` still in progress from now on, as if they had just
    /// started (after [`recover_display`](Self::recover_display)).
    pub(crate) fn restart_flush_timeouts(&mut self, d: usize) {
        let now = self.wait_stamp();
        match &mut self.displays[d].refresher.strategy {
            Strategy::Partial(p) => {
                for f in p.flights.iter_mut().flatten() {
                    *f = Flight {
                        since: now,
                        timed_out: false,
                        ..*f
                    };
                }
            }
            Strategy::Full(st) => {
                if let Some(pp) = st.present.as_mut() {
                    *pp = PendingPresent {
                        since: now,
                        timed_out: false,
                    };
                }
            }
            Strategy::Direct(_) => {}
        }
    }

    /// Whether some display's driver still holds a buffer or a swap that the engine is waiting
    /// for (hung flushes excluded: waiting for them again at once is pointless).
    pub(crate) fn flush_awaited(&self) -> bool {
        self.displays.iter().any(|disp| match &disp.refresher.strategy {
            Strategy::Partial(p) => p
                .in_flight
                .front()
                .is_some_and(|&s| p.flights[usize::from(s)].is_none_or(|f| !f.timed_out)),
            Strategy::Full(st) => st.present.is_some_and(|pp| !pp.timed_out),
            Strategy::Direct(_) => false,
        })
    }
}
