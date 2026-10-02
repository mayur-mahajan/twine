//! [`EmbassyPlatform`] (feature `platform-embassy`).

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use twine_core::Instant;

use super::AsyncPlatform;
use crate::Clock;

/// [`AsyncPlatform`] over [`embassy-time`](https://docs.rs/embassy-time) (feature
/// `platform-embassy`): the clock is `embassy_time::Instant::now()` in microseconds (its
/// resolution is the embassy tick rate), and the timer is an `embassy_time::Timer`, so it
/// works on every embassy HAL (embassy-rp, embassy-stm32, esp-hal's `esp-rtos`, …). The
/// application links an embassy time driver, as for any embassy program.
///
/// A zero-sized `Copy` type: every copy reads the same clock. `twine-embassy` builds the
/// `AsyncUi` with it (`UiBuilderExt::with_embassy_platform`); the run loop waits with
/// embassy's own `select` over the timer, the UI waker and the input interrupt.
///
/// ```no_run
/// use twine_core::Duration;
/// use twine_hal::{AsyncPlatform, Clock, EmbassyPlatform};
///
/// async fn tick() {
///     let mut p = EmbassyPlatform;
///     let deadline = p.now() + Duration::ms(16);
///     p.wait_until(deadline).await; // an `embassy_time::Timer::at(deadline)`
/// }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EmbassyPlatform;

impl EmbassyPlatform {
    /// `t` (µs since boot) as an `embassy-time` instant (at the embassy tick rate's
    /// resolution), e.g. for an `embassy_time::Timer::at` of a deadline the UI returned.
    /// Never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::EmbassyPlatform;
    ///
    /// let at = EmbassyPlatform::to_embassy(Instant::from_millis(16));
    /// assert_eq!(at.as_millis(), 16);
    /// ```
    #[must_use]
    pub fn to_embassy(t: Instant) -> embassy_time::Instant {
        embassy_time::Instant::from_micros(t.as_micros())
    }
}

impl Clock for EmbassyPlatform {
    #[inline]
    fn now(&self) -> Instant {
        Instant::from_micros(embassy_time::Instant::now().as_micros())
    }
}

impl AsyncPlatform for EmbassyPlatform {
    fn poll_wait_until(&mut self, deadline: Instant, cx: &mut Context<'_>) -> Poll<()> {
        let at = Self::to_embassy(deadline);
        if embassy_time::Instant::now() >= at {
            return Poll::Ready(());
        }
        // The first poll of a fresh timer only schedules the wake-up at `at` (`Pending`), so
        // no state has to be kept between polls.
        let mut timer = embassy_time::Timer::at(at);
        Pin::new(&mut timer).poll(cx)
    }
}
