//! [`Wake`]: when the engine needs to run again; [`StepBudget`]: how much one step may render.

use core::num::NonZeroU16;

use twine_core::{Duration, Instant};

/// When [`Engine::step`](crate::Engine::step) (or the view layer's `Ui::update`) must be called
/// again. The caller may sleep until then (P1: an idle UI returns [`Wake::Idle`] and needs no
/// CPU at all until an input interrupt or a new signal value arrives).
///
/// Ordering by urgency: `Now` < `At(earlier)` < `At(later)` < `Idle` = `IdleFor(_)`.
///
/// [`IdleFor`](Wake::IdleFor) is `Idle` with one more piece of information: no user input for
/// at least `EngineConfig::idle_timeout`. A loop sleeps on it exactly as on `Idle` (no
/// deadline), but may choose a deeper sleep mode, switch the display off, etc. It is only
/// returned when `EngineConfig::idle_timeout` is set; until the timeout has passed, the engine
/// asks to be woken once at the moment it passes (one wake-up per inactive period, no polling).
///
/// ```
/// use twine_core::{Duration, Instant};
/// use twine_engine::Wake;
/// let a = Wake::At(Instant::from_millis(5));
/// assert_eq!(a.min(Wake::Idle), a);
/// assert_eq!(a.min(Wake::At(Instant::from_millis(3))), Wake::At(Instant::from_millis(3)));
/// assert_eq!(Wake::Idle.min(Wake::Now), Wake::Now);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Wake {
    /// Nothing is pending: sleep until an external event.
    Idle,
    /// Call again at (or after) this instant.
    At(Instant),
    /// Call again as soon as possible (work is pending, e.g. a partially rendered frame).
    Now,
    /// Nothing is pending (like [`Idle`](Wake::Idle)), and there has been no user input for
    /// this long — at least `EngineConfig::idle_timeout`: a candidate for deep sleep.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_engine::Wake;
    /// let w = Wake::IdleFor(Duration::secs(30));
    /// assert!(w.is_idle());
    /// assert_eq!(w.min(Wake::Idle), w); // keeps the inactivity
    /// assert_eq!(w.inactive_for(), Some(Duration::secs(30)));
    /// ```
    IdleFor(Duration),
}

impl Wake {
    /// The more urgent of `self` and `other`.
    #[must_use]
    pub fn min(self, other: Wake) -> Wake {
        match (self, other) {
            (Wake::Now, _) | (_, Wake::Now) => Wake::Now,
            (Wake::At(a), Wake::At(b)) => Wake::At(a.min(b)),
            (Wake::At(a), Wake::Idle | Wake::IdleFor(_)) | (Wake::Idle | Wake::IdleFor(_), Wake::At(a)) => {
                Wake::At(a)
            }
            (Wake::IdleFor(a), Wake::IdleFor(b)) => Wake::IdleFor(a.min(b)),
            (Wake::IdleFor(a), Wake::Idle) | (Wake::Idle, Wake::IdleFor(a)) => Wake::IdleFor(a),
            (Wake::Idle, Wake::Idle) => Wake::Idle,
        }
    }

    /// Whether nothing is pending: [`Wake::Idle`] or [`Wake::IdleFor`].
    #[must_use]
    pub const fn is_idle(self) -> bool {
        matches!(self, Wake::Idle | Wake::IdleFor(_))
    }

    /// The inactivity [`Wake::IdleFor`] reports (`None` for the other variants).
    #[must_use]
    pub const fn inactive_for(self) -> Option<Duration> {
        match self {
            Wake::IdleFor(d) => Some(d),
            _ => None,
        }
    }
}

/// How much rendering one step may do: at most [`max_chunks`](Self::max_chunks) chunks (a
/// partial buffer's rows, or one dirty area of a framebuffer display) over all displays,
/// and at most [`max_time`](Self::max_time) of the step's time. The hook for schedulers that
/// must bound the time a step takes (an RTOS time slice, a cooperative super-loop, a frame
/// budget): the rest of a frame continues in the next step, which the step requests by
/// returning [`Wake::Now`] — work is deferred, never dropped.
///
/// **Chunks** are a fixed slice of work (counting costs one comparison per chunk).
/// **Time** is measured with [`EngineConfig::hires_timer`](crate::EngineConfig::hires_timer)
/// from the start of the step (`Engine::begin_step`, i.e. including the view layer's
/// messages, timers, animations and bindings) and checked after each chunk: once the step
/// has used its time, no further chunk is rendered in it, and the step raises
/// [`FaultKind::FrameOverrun`](twine_core::fault::FaultKind::FrameOverrun) — as it does when
/// a step that defers nothing still ends past its time (a long layout, one slow chunk).
/// Every step renders at least one chunk, so a loop stepping on [`Wake::Now`] always
/// finishes the frame; a step can therefore overrun its time by at most one chunk's render
/// and flush (see `Engine::timing`). Without a `hires_timer` a time limit cannot be
/// measured and is ignored (`EngineConfig::validate` refuses a configured one).
///
/// A frame spread over several steps produces exactly the pixels of a frame rendered in one
/// step: the frame's areas are fixed when it starts and rendered in the same order, and an
/// area invalidated meanwhile is redrawn by the next frame (as with
/// [`EngineConfig::cooperative_flush`](crate::EngineConfig::cooperative_flush)). Displays
/// added with [`add_chunked_display`](crate::Engine::add_chunked_display) are rendered by
/// their caller and are not counted.
///
/// The budget of [`Engine::step`](crate::Engine::step) (and the view layer's `Ui::update`) is
/// [`EngineConfig::step_budget`](crate::EngineConfig::step_budget) (default
/// [`UNLIMITED`](Self::UNLIMITED)); [`Engine::step_budgeted`](crate::Engine::step_budgeted)
/// and `Ui::update_budgeted` take one per call. An unlimited budget costs one comparison per
/// chunk and nothing else.
///
/// ```
/// use twine_core::Duration;
/// use twine_engine::StepBudget;
/// assert_eq!(StepBudget::chunks(4).max_chunks(), Some(4));
/// assert_eq!(StepBudget::chunks(0).max_chunks(), Some(1)); // at least one chunk per step
/// assert_eq!(StepBudget::UNLIMITED.max_chunks(), None);
/// assert_eq!(StepBudget::default(), StepBudget::UNLIMITED);
/// let both = StepBudget::time(Duration::ms(8)).with_chunks(4);
/// assert_eq!((both.max_chunks(), both.max_time()), (Some(4), Some(Duration::ms(8))));
/// ```
#[doc(alias = "max_chunks_per_update")]
#[doc(alias = "frame_budget")]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StepBudget {
    /// `None`: no chunk limit.
    max_chunks: Option<NonZeroU16>,
    /// `None`: no time limit.
    max_time: Option<Duration>,
}

impl StepBudget {
    /// No limit: a step renders every frame that is due completely (unless it has to wait
    /// for a draw buffer, see `EngineConfig::cooperative_flush`).
    pub const UNLIMITED: StepBudget = StepBudget {
        max_chunks: None,
        max_time: None,
    };

    /// At most `n` chunks per step (`0` counts as `1`: every step makes progress, so a loop
    /// stepping on [`Wake::Now`] always finishes the frame). Never panics.
    ///
    /// ```
    /// use twine_engine::StepBudget;
    /// let budget = StepBudget::chunks(2); // e.g. what fits in one RTOS time slice
    /// assert_eq!(budget.max_chunks(), Some(2));
    /// ```
    #[must_use]
    pub const fn chunks(n: u16) -> StepBudget {
        StepBudget::UNLIMITED.with_chunks(n)
    }

    /// At most `t` of time per step, measured with the `hires_timer` (see the type's docs: a
    /// step that reaches it renders no further chunk and raises `FrameOverrun`). Never
    /// panics.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_engine::StepBudget;
    /// let budget = StepBudget::time(Duration::ms(10)); // e.g. a 10 ms frame budget
    /// assert_eq!(budget.max_time(), Some(Duration::ms(10)));
    /// assert_eq!(budget.max_chunks(), None);
    /// ```
    #[doc(alias = "frame_budget")]
    #[must_use]
    pub const fn time(t: Duration) -> StepBudget {
        StepBudget::UNLIMITED.with_time(t)
    }

    /// This budget with at most `n` chunks per step (`0` counts as `1`). Never panics.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_engine::StepBudget;
    /// let b = StepBudget::time(Duration::ms(5)).with_chunks(3);
    /// assert_eq!(b.max_chunks(), Some(3));
    /// ```
    #[must_use]
    pub const fn with_chunks(self, n: u16) -> StepBudget {
        StepBudget {
            max_chunks: NonZeroU16::new(if n == 0 { 1 } else { n }),
            ..self
        }
    }

    /// This budget with at most `t` of time per step. Never panics.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_engine::StepBudget;
    /// let b = StepBudget::chunks(3).with_time(Duration::ms(5));
    /// assert_eq!(b.max_time(), Some(Duration::ms(5)));
    /// ```
    #[must_use]
    pub const fn with_time(self, t: Duration) -> StepBudget {
        StepBudget {
            max_time: Some(t),
            ..self
        }
    }

    /// The chunk limit (`None`: unlimited). Never panics.
    ///
    /// ```
    /// use twine_engine::StepBudget;
    /// assert_eq!(StepBudget::chunks(3).max_chunks(), Some(3));
    /// assert_eq!(StepBudget::UNLIMITED.max_chunks(), None);
    /// ```
    #[must_use]
    pub const fn max_chunks(self) -> Option<u16> {
        match self.max_chunks {
            Some(n) => Some(n.get()),
            None => None,
        }
    }

    /// The time limit (`None`: unlimited). Never panics.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_engine::StepBudget;
    /// assert_eq!(StepBudget::time(Duration::us(500)).max_time(), Some(Duration::us(500)));
    /// assert_eq!(StepBudget::UNLIMITED.max_time(), None);
    /// ```
    #[must_use]
    pub const fn max_time(self) -> Option<Duration> {
        self.max_time
    }

    /// The chunks a step may render (`u32::MAX` when unlimited: never reached).
    #[inline]
    pub(crate) const fn chunk_allowance(self) -> u32 {
        match self.max_chunks {
            Some(n) => n.get() as u32,
            None => u32::MAX,
        }
    }
}

/// What is left of a step's [`StepBudget`] while the step renders.
pub(crate) struct Allowance {
    /// Chunks the step may still render (`0`: none; the rest of the frame waits).
    pub(crate) chunks: u32,
    /// The high-resolution time (µs) at which the step's time is used up (`None`: no time
    /// limit, or no timer to measure it).
    pub(crate) deadline_us: Option<u64>,
    /// The display whose frame the time limit cut short (`Some` once the time is up).
    pub(crate) cut: Option<usize>,
}

impl Allowance {
    /// No limit (what [`Engine::refresh`](crate::Engine::refresh) renders with).
    pub(crate) const fn unlimited() -> Allowance {
        Allowance {
            chunks: u32::MAX,
            deadline_us: None,
            cut: None,
        }
    }

    /// The allowance of `budget` for a step that started at `started_us` (the
    /// high-resolution time; `None` without a timer).
    pub(crate) fn new(budget: StepBudget, started_us: Option<u64>) -> Allowance {
        Allowance {
            chunks: budget.chunk_allowance(),
            deadline_us: match (budget.max_time, started_us) {
                (Some(t), Some(s)) => Some(s.saturating_add(t.as_micros())),
                _ => None,
            },
            cut: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_budget_allowance() {
        assert_eq!(StepBudget::UNLIMITED.chunk_allowance(), u32::MAX);
        assert_eq!(StepBudget::chunks(0).chunk_allowance(), 1);
        assert_eq!(StepBudget::chunks(7).chunk_allowance(), 7);
        assert_eq!(StepBudget::chunks(u16::MAX).max_chunks(), Some(u16::MAX));
    }

    #[test]
    fn wake_min_ordering() {
        let t1 = Wake::At(Instant::from_millis(1));
        let t2 = Wake::At(Instant::from_millis(2));
        // Sorted by urgency.
        let order = [Wake::Now, t1, t2, Wake::Idle];
        for (i, a) in order.iter().enumerate() {
            for (j, b) in order.iter().enumerate() {
                let expect = order[i.min(j)];
                assert_eq!(a.min(*b), expect, "{a:?}.min({b:?})");
            }
        }
        assert!(Wake::Idle.is_idle());
        assert!(!t1.is_idle());
        let f = Wake::IdleFor(Duration::secs(5));
        assert_eq!(f.min(t1), t1);
        assert_eq!(Wake::Now.min(f), Wake::Now);
        assert_eq!(
            f.min(Wake::IdleFor(Duration::secs(2))),
            Wake::IdleFor(Duration::secs(2))
        );
        assert_eq!(Wake::Idle.inactive_for(), None);
    }
}
