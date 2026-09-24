//! [`Wake`]: when the engine needs to run again.

use twine_core::Instant;

/// When [`Engine::step`](crate::Engine::step) (or the view layer's `Ui::update`) must be called
/// again. The caller may sleep until then (P1: an idle UI returns [`Wake::Idle`] and needs no
/// CPU at all until an input interrupt or a new signal value arrives).
///
/// Ordering by urgency: `Now` < `At(earlier)` < `At(later)` < `Idle`.
///
/// ```
/// use twine_core::Instant;
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
}

impl Wake {
    /// The more urgent of `self` and `other`.
    #[must_use]
    pub fn min(self, other: Wake) -> Wake {
        match (self, other) {
            (Wake::Now, _) | (_, Wake::Now) => Wake::Now,
            (Wake::At(a), Wake::At(b)) => Wake::At(a.min(b)),
            (Wake::At(a), Wake::Idle) | (Wake::Idle, Wake::At(a)) => Wake::At(a),
            (Wake::Idle, Wake::Idle) => Wake::Idle,
        }
    }

    /// Whether this is [`Wake::Idle`].
    #[must_use]
    pub const fn is_idle(self) -> bool {
        matches!(self, Wake::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
