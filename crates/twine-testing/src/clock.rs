//! [`MockClock`]: a manually advanced [`Clock`].

use std::cell::Cell;
use std::rc::Rc;

use twine_core::{Duration, Instant};
use twine_hal::Clock;

/// A clock that only moves when the test says so.
///
/// Cheap to clone: all clones share the same time, so a test keeps one handle while the engine
/// owns another.
///
/// ```
/// use twine_core::{Duration, Instant};
/// use twine_hal::Clock;
/// use twine_testing::MockClock;
///
/// let clock = MockClock::new();
/// let engine_side = clock.clone();
/// clock.advance(Duration::ms(16));
/// assert_eq!(engine_side.now(), Instant::from_millis(16));
/// ```
#[derive(Clone, Debug, Default)]
pub struct MockClock(Rc<Cell<Instant>>);

impl MockClock {
    /// A clock at `Instant` 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock at `start`.
    #[must_use]
    pub fn starting_at(start: Instant) -> Self {
        Self(Rc::new(Cell::new(start)))
    }

    /// Moves the time forward by `d` (saturating).
    pub fn advance(&self, d: Duration) {
        self.0.set(self.0.get() + d);
    }

    /// Sets the time. Moving backwards is allowed (for tests of robustness) but logged.
    pub fn set(&self, t: Instant) {
        if t < self.0.get() {
            log::warn!(target: "twine::core", "MockClock moved backwards: {} -> {}", self.0.get(), t);
        }
        self.0.set(t);
    }
}

impl Clock for MockClock {
    fn now(&self) -> Instant {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_time() {
        let a = MockClock::starting_at(Instant::from_millis(10));
        let b = a.clone();
        a.advance(Duration::ms(5));
        assert_eq!(b.now(), Instant::from_millis(15));
        b.set(Instant::from_millis(100));
        assert_eq!(a.now(), Instant::from_millis(100));
    }
}
