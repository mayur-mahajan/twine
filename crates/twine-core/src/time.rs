//! Monotonic time in microseconds: [`Instant`] and [`Duration`].
//!
//! There is no clock here; the `Clock` trait lives in `twine-hal`.

use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};

/// A span of time in microseconds. Arithmetic saturates.
///
/// ```
/// use twine_core::Duration;
/// let d = Duration::ms(300);
/// assert_eq!(d.as_micros(), 300_000);
/// assert_eq!(Duration::fraction_1024(Duration::ms(150), d), 512);
/// assert_eq!(Duration::ms(1) - Duration::ms(2), Duration::ZERO);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Duration(u64);

#[allow(clippy::should_implement_trait)] // `mul`/`div` take integer factors, unlike the `Mul`/`Div` traits
impl Duration {
    /// Zero length.
    pub const ZERO: Duration = Duration(0);
    /// The longest representable duration.
    pub const MAX: Duration = Duration(u64::MAX);

    /// `v` microseconds.
    #[must_use]
    pub const fn us(v: u64) -> Duration {
        Duration(v)
    }

    /// `v` milliseconds (saturating).
    #[must_use]
    pub const fn ms(v: u64) -> Duration {
        Duration(v.saturating_mul(1_000))
    }

    /// `v` seconds (saturating).
    #[must_use]
    pub const fn secs(v: u64) -> Duration {
        Duration(v.saturating_mul(1_000_000))
    }

    /// Alias of [`Duration::us`].
    #[must_use]
    pub const fn from_micros(v: u64) -> Duration {
        Duration::us(v)
    }

    /// Alias of [`Duration::ms`].
    #[must_use]
    pub const fn from_millis(v: u64) -> Duration {
        Duration::ms(v)
    }

    /// Length in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Length in whole milliseconds (floor).
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000
    }

    /// Length in whole milliseconds, saturating at `u32::MAX`.
    #[must_use]
    pub const fn as_millis_u32(self) -> u32 {
        let ms = self.as_millis();
        if ms > u32::MAX as u64 { u32::MAX } else { ms as u32 }
    }

    /// Saturating addition.
    #[must_use]
    pub const fn saturating_add(self, o: Duration) -> Duration {
        Duration(self.0.saturating_add(o.0))
    }

    /// Saturating subtraction (floors at zero).
    #[must_use]
    pub const fn saturating_sub(self, o: Duration) -> Duration {
        Duration(self.0.saturating_sub(o.0))
    }

    /// Checked addition.
    #[must_use]
    pub const fn checked_add(self, o: Duration) -> Option<Duration> {
        match self.0.checked_add(o.0) {
            Some(v) => Some(Duration(v)),
            None => None,
        }
    }

    /// Checked subtraction.
    #[must_use]
    pub const fn checked_sub(self, o: Duration) -> Option<Duration> {
        match self.0.checked_sub(o.0) {
            Some(v) => Some(Duration(v)),
            None => None,
        }
    }

    /// Multiplies by `n` (saturating).
    #[must_use]
    pub const fn mul(self, n: u32) -> Duration {
        Duration(self.0.saturating_mul(n as u64))
    }

    /// Divides by `n`; dividing by zero returns [`Duration::MAX`] and logs a warning.
    #[must_use]
    pub fn div(self, n: u32) -> Duration {
        if n == 0 {
            crate::warn!(target: "twine::core", "Duration::div: division by zero");
            return Duration::MAX;
        }
        Duration(self.0 / u64::from(n))
    }

    /// Progress `elapsed / total` scaled to `0..=1024` (clamped). A zero `total` returns 1024.
    #[must_use]
    pub const fn fraction_1024(elapsed: Duration, total: Duration) -> u16 {
        if total.0 == 0 || elapsed.0 >= total.0 {
            return 1024;
        }
        ((elapsed.0 as u128 * 1024) / total.0 as u128) as u16
    }
}

impl Add for Duration {
    type Output = Duration;
    #[inline]
    fn add(self, o: Duration) -> Duration {
        self.saturating_add(o)
    }
}

impl Sub for Duration {
    type Output = Duration;
    #[inline]
    fn sub(self, o: Duration) -> Duration {
        self.saturating_sub(o)
    }
}

impl AddAssign for Duration {
    #[inline]
    fn add_assign(&mut self, o: Duration) {
        *self = *self + o;
    }
}

impl SubAssign for Duration {
    #[inline]
    fn sub_assign(&mut self, o: Duration) {
        *self = *self - o;
    }
}

impl fmt::Display for Duration {
    /// Formatted like [`Instant`]: `"{ms}.{µs:03} ms"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:03} ms", self.0 / 1_000, self.0 % 1_000)
    }
}

/// A point in time: microseconds since an arbitrary epoch (e.g. boot).
///
/// ```
/// use twine_core::{Duration, Instant};
/// let t0 = Instant::from_millis(10);
/// let t1 = t0 + Duration::us(1_500);
/// assert_eq!(t1.saturating_duration_since(t0), Duration::us(1_500));
/// assert_eq!(t0.checked_duration_since(t1), None);
/// assert_eq!(t1.to_string(), "11.500 ms");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Instant(u64);

impl Instant {
    /// The epoch.
    pub const ZERO: Instant = Instant(0);

    /// `v` microseconds after the epoch.
    #[must_use]
    pub const fn from_micros(v: u64) -> Instant {
        Instant(v)
    }

    /// `v` milliseconds after the epoch (saturating).
    #[must_use]
    pub const fn from_millis(v: u64) -> Instant {
        Instant(v.saturating_mul(1_000))
    }

    /// Microseconds since the epoch.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Whole milliseconds since the epoch (floor).
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000
    }

    /// Time elapsed since `earlier`, or zero if `earlier` is later.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Instant) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }

    /// Time elapsed since `earlier`, or `None` if `earlier` is later.
    #[must_use]
    pub const fn checked_duration_since(self, earlier: Instant) -> Option<Duration> {
        match self.0.checked_sub(earlier.0) {
            Some(v) => Some(Duration(v)),
            None => None,
        }
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;
    #[inline]
    fn add(self, d: Duration) -> Instant {
        Instant(self.0.saturating_add(d.0))
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;
    /// Saturates at the epoch.
    #[inline]
    fn sub(self, d: Duration) -> Instant {
        Instant(self.0.saturating_sub(d.0))
    }
}

impl Sub for Instant {
    type Output = Duration;
    /// Same as [`Instant::saturating_duration_since`].
    #[inline]
    fn sub(self, earlier: Instant) -> Duration {
        self.saturating_duration_since(earlier)
    }
}

impl AddAssign<Duration> for Instant {
    #[inline]
    fn add_assign(&mut self, d: Duration) {
        *self = *self + d;
    }
}

impl SubAssign<Duration> for Instant {
    #[inline]
    fn sub_assign(&mut self, d: Duration) {
        *self = *self - d;
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:03} ms", self.0 / 1_000, self.0 % 1_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn duration_units() {
        assert_eq!(Duration::secs(2), Duration::ms(2_000));
        assert_eq!(Duration::ms(3), Duration::us(3_000));
        assert_eq!(Duration::from_millis(4), Duration::ms(4));
        assert_eq!(Duration::from_micros(4), Duration::us(4));
        assert_eq!(Duration::us(2_999).as_millis(), 2);
        assert_eq!(Duration::MAX.as_millis_u32(), u32::MAX);
        assert_eq!(Duration::ms(7).as_millis_u32(), 7);
        assert_eq!(Duration::secs(u64::MAX), Duration::MAX);
        assert_eq!(Duration::ms(5).mul(3), Duration::ms(15));
        assert_eq!(Duration::ms(15).div(3), Duration::ms(5));
        assert_eq!(Duration::ms(15).div(0), Duration::MAX);
        assert_eq!(Duration::MAX + Duration::us(1), Duration::MAX);
        assert_eq!(Duration::MAX.checked_add(Duration::us(1)), None);
        assert_eq!(Duration::ZERO.checked_sub(Duration::us(1)), None);
        assert_eq!(
            Duration::us(3).checked_sub(Duration::us(1)),
            Some(Duration::us(2))
        );
        assert_eq!(
            Duration::us(3).checked_add(Duration::us(1)),
            Some(Duration::us(4))
        );
        assert_eq!(Duration::us(3).min(Duration::us(1)), Duration::us(1));
        assert_eq!(Duration::us(3).max(Duration::us(1)), Duration::us(3));
        let mut d = Duration::us(1);
        d += Duration::us(2);
        d -= Duration::us(5);
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn fraction_1024_bounds() {
        let total = Duration::ms(300);
        assert_eq!(Duration::fraction_1024(Duration::ZERO, total), 0);
        assert_eq!(Duration::fraction_1024(total, total), 1024);
        assert_eq!(Duration::fraction_1024(Duration::secs(5), total), 1024);
        assert_eq!(Duration::fraction_1024(Duration::ms(1), Duration::ZERO), 1024);
        assert_eq!(Duration::fraction_1024(Duration::ms(75), total), 256);
        assert_eq!(
            Duration::fraction_1024(Duration::MAX.saturating_sub(Duration::us(1)), Duration::MAX),
            1023
        );
    }

    #[test]
    fn instant_sub_saturates() {
        let t = Instant::from_millis(1);
        assert_eq!(t - Duration::ms(5), Instant::ZERO);
        assert_eq!(Instant::ZERO - t, Duration::ZERO);
        assert_eq!(t - Instant::ZERO, Duration::ms(1));
        assert_eq!(
            Instant::from_micros(u64::MAX) + Duration::us(1),
            Instant::from_micros(u64::MAX)
        );
        let mut u = t;
        u += Duration::us(10);
        u -= Duration::us(3);
        assert_eq!(u.as_micros(), 1_007);
        assert_eq!(u.as_millis(), 1);
        assert!(u > t);
    }

    #[test]
    fn display_formats() {
        assert_eq!(format!("{}", Instant::from_micros(1_234_567)), "1234.567 ms");
        assert_eq!(format!("{}", Instant::from_micros(5)), "0.005 ms");
        assert_eq!(format!("{}", Duration::us(16_667)), "16.667 ms");
    }
}
