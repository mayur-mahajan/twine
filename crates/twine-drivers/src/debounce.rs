//! [`Debouncer`]: time-based debouncing of a digital input.

use twine_core::{Duration, Instant};

/// Debounces one digital input: a new level is accepted once the raw input has kept it for
/// `debounce` (default 20 ms).
///
/// ```
/// use twine_core::{Duration, Instant};
/// use twine_drivers::debounce::Debouncer;
///
/// let mut d = Debouncer::new(Duration::ms(20));
/// assert!(!d.update(true, Instant::from_millis(0)));   // just changed
/// assert!(!d.update(true, Instant::from_millis(19)));
/// assert!(d.update(true, Instant::from_millis(20)));   // stable for 20 ms
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Debouncer {
    stable: bool,
    candidate: bool,
    since: Instant,
    debounce: Duration,
}

impl Debouncer {
    /// The default debounce time.
    pub const DEFAULT: Duration = Duration::ms(20);

    /// A debouncer that starts released (`false`).
    #[must_use]
    pub const fn new(debounce: Duration) -> Self {
        Self {
            stable: false,
            candidate: false,
            since: Instant::from_micros(0),
            debounce,
        }
    }

    /// Feeds the raw level sampled at `now`; returns the debounced level.
    pub fn update(&mut self, raw: bool, now: Instant) -> bool {
        if raw != self.candidate {
            self.candidate = raw;
            self.since = now;
        }
        if self.candidate != self.stable && now.saturating_duration_since(self.since) >= self.debounce {
            self.stable = self.candidate;
        }
        self.stable
    }

    /// The debounced level.
    #[must_use]
    pub const fn level(&self) -> bool {
        self.stable
    }
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::new(Self::DEFAULT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounces_are_ignored() {
        let mut d = Debouncer::default();
        for t in 0..10 {
            assert!(!d.update(t % 2 == 0, Instant::from_millis(t * 3)));
        }
        assert!(!d.update(true, Instant::from_millis(40)));
        assert!(d.update(true, Instant::from_millis(60)));
        assert!(d.level());
        assert!(d.update(false, Instant::from_millis(61)));
        assert!(!d.update(false, Instant::from_millis(81)));
    }
}
