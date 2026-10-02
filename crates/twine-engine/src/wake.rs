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
/// partial buffer's rows, or one dirty area of a framebuffer display) over all displays. The
/// hook for schedulers that must bound the time a step takes (an RTOS time slice, a
/// cooperative super-loop): the rest of a frame continues in the next step, which the step
/// requests by returning [`Wake::Now`].
///
/// A frame spread over several steps produces exactly the pixels of a frame rendered in one
/// step: the frame's areas are fixed when it starts and rendered in the same order, and an
/// area invalidated meanwhile is redrawn by the next frame (as with
/// [`EngineConfig::cooperative_flush`](crate::EngineConfig::cooperative_flush)). Displays
/// added with [`add_chunked_display`](crate::Engine::add_chunked_display) are rendered by
/// their caller and are not counted.
///
/// Used by [`Engine::step_budgeted`](crate::Engine::step_budgeted) and the view layer's
/// `Ui::update_budgeted`. The default is [`UNLIMITED`](Self::UNLIMITED) (what
/// [`Engine::step`](crate::Engine::step) uses). Checking the budget costs one comparison per
/// chunk.
///
/// ```
/// use twine_engine::StepBudget;
/// assert_eq!(StepBudget::chunks(4).max_chunks(), Some(4));
/// assert_eq!(StepBudget::chunks(0).max_chunks(), Some(1)); // at least one chunk per step
/// assert_eq!(StepBudget::UNLIMITED.max_chunks(), None);
/// assert_eq!(StepBudget::default(), StepBudget::UNLIMITED);
/// ```
#[doc(alias = "max_chunks_per_update")]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StepBudget {
    /// `None`: no limit.
    max_chunks: Option<NonZeroU16>,
}

impl StepBudget {
    /// No limit: a step renders every frame that is due completely (unless it has to wait
    /// for a draw buffer, see `EngineConfig::cooperative_flush`).
    pub const UNLIMITED: StepBudget = StepBudget { max_chunks: None };

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
        StepBudget {
            max_chunks: NonZeroU16::new(if n == 0 { 1 } else { n }),
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

    /// The chunks a step may render (`u32::MAX` when unlimited: never reached).
    #[inline]
    pub(crate) const fn chunk_allowance(self) -> u32 {
        match self.max_chunks {
            Some(n) => n.get() as u32,
            None => u32::MAX,
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
