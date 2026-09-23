//! The time source: [`Clock`].

use twine_core::Instant;

/// A monotonic clock with microsecond resolution.
///
/// Implemented by the platform (embassy-time, a hardware timer, `std::time` in the simulator)
/// and by `twine-testing`'s `MockClock`. `now()` must never go backwards.
///
/// ```
/// use core::cell::Cell;
/// use twine_core::{Duration, Instant};
/// use twine_hal::Clock;
///
/// struct Fixed(Cell<Instant>);
/// impl Clock for Fixed {
///     fn now(&self) -> Instant { self.0.get() }
/// }
///
/// fn elapsed(c: &impl Clock, since: Instant) -> Duration {
///     c.now().saturating_duration_since(since)
/// }
///
/// let c = Fixed(Cell::new(Instant::from_millis(5)));
/// assert_eq!(elapsed(&&c, Instant::from_millis(2)), Duration::ms(3));
/// ```
pub trait Clock {
    /// The current time.
    fn now(&self) -> Instant;
}

impl<C: Clock + ?Sized> Clock for &C {
    #[inline]
    fn now(&self) -> Instant {
        (**self).now()
    }
}
