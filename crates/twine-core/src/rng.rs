//! Deterministic pseudo-random numbers ([`XorShift32`]).

use crate::color::Opa;

/// Marsaglia xorshift32 generator: tiny, fast and deterministic on every platform.
///
/// **Not cryptographically secure.** Used by the monkey tester and tests.
///
/// ```
/// use twine_core::XorShift32;
/// let mut a = XorShift32::new(42);
/// let mut b = XorShift32::new(42);
/// assert_eq!(a.next_u32(), b.next_u32());
/// let v = a.range(-5, 5);
/// assert!((-5..5).contains(&v));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct XorShift32(u32);

impl XorShift32 {
    /// Seed used when `0` is given (the all-zero state would be stuck).
    pub const DEFAULT_SEED: u32 = 0x9E37_79B9;

    /// Creates a generator; a zero seed is replaced by [`Self::DEFAULT_SEED`].
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        Self(if seed == 0 { Self::DEFAULT_SEED } else { seed })
    }

    /// The next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A value in the half-open range `lo..hi`; `lo >= hi` returns `lo`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if lo >= hi {
            return lo;
        }
        let span = (i64::from(hi) - i64::from(lo)) as u64;
        let v = u64::from(self.next_u32()) % span;
        (i64::from(lo) + v as i64) as i32
    }

    /// A random boolean.
    pub fn next_bool(&mut self) -> bool {
        self.next_u32() >> 31 != 0
    }

    /// A random opacity.
    pub fn next_opa(&mut self) -> Opa {
        Opa((self.next_u32() >> 24) as u8)
    }
}

impl Default for XorShift32 {
    /// A generator seeded with [`XorShift32::DEFAULT_SEED`].
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_sequence_is_stable() {
        let mut r = XorShift32::new(1);
        let got: [u32; 5] = core::array::from_fn(|_| r.next_u32());
        assert_eq!(
            got,
            [270_369, 67_634_689, 2_647_435_461, 307_599_695, 2_398_689_233]
        );
        assert_eq!(XorShift32::new(0), XorShift32::new(XorShift32::DEFAULT_SEED));
        assert_eq!(XorShift32::default(), XorShift32::new(0));
    }

    #[test]
    fn range_bounds() {
        let mut r = XorShift32::new(7);
        let mut seen_lo = false;
        let mut seen_hi = false;
        for _ in 0..10_000 {
            let v = r.range(-3, 4);
            assert!((-3..4).contains(&v));
            seen_lo |= v == -3;
            seen_hi |= v == 3;
        }
        assert!(seen_lo && seen_hi);
        assert_eq!(r.range(5, 5), 5);
        assert_eq!(r.range(9, 2), 9);
        for _ in 0..1000 {
            let v = r.range(i32::MIN, i32::MAX);
            assert!(v < i32::MAX);
        }
        let bools = (0..1000).filter(|_| r.next_bool()).count();
        assert!((300..700).contains(&bools));
        let _ = r.next_opa();
    }
}
