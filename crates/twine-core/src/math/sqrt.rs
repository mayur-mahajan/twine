//! Integer square roots.

use super::Fx;

/// Result of [`sqrt`]: integer part and an 8-bit binary fraction (`value ≈ i + f/256`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SqrtRes {
    /// Integer part.
    pub i: u16,
    /// Fraction in 1/256.
    pub f: u8,
}

/// Floor of the square root of `x` (exact digit-by-digit algorithm, no floating point).
///
/// ```
/// use twine_core::math::isqrt64;
/// assert_eq!(isqrt64(99), 9);
/// assert_eq!(isqrt64(u64::MAX), u32::MAX);
/// ```
#[must_use]
pub const fn isqrt64(x: u64) -> u32 {
    if x <= u32::MAX as u64 {
        // The same digits on native 32-bit integers (cheaper on 32-bit cores).
        return isqrt32(x as u32) as u32;
    }
    let mut op = x;
    let mut res: u64 = 0;
    // The largest power of four ≤ `x`: its top bit, rounded down to an even position.
    let top = match x.checked_ilog2() {
        Some(b) => b,
        None => 0,
    };
    // SHIFT: `x > 2^32` (above), so `ilog2` is `32..=63` and the amount `32..=62`.
    let mut one: u64 = 1 << (top & !1);
    while one != 0 {
        // SAFE-ARITH: with `one = 4^k` (`k ≤ 31`), `res = q · 2^(k+1)` where `q < 2^(32−k)` is
        // the root's bits found so far, so `res < 2^33` and `res + one < 2^63`; the
        // subtraction is guarded by the comparison.
        let next = res.wrapping_add(one);
        if op >= next {
            op = op.wrapping_sub(next);
            res = (res >> 1).wrapping_add(one);
        } else {
            res >>= 1;
        }
        one >>= 2;
    }
    res as u32
}

/// [`isqrt64`] of a 32-bit value, on 32-bit integers.
const fn isqrt32(x: u32) -> u16 {
    if x == 0 {
        return 0;
    }
    let mut op = x;
    let mut res: u32 = 0;
    let top = match x.checked_ilog2() {
        Some(b) => b,
        None => 0,
    };
    // SHIFT: `x ≠ 0`, so `ilog2` is `0..=31` and the amount `0..=30`.
    let mut one: u32 = 1 << (top & !1);
    while one != 0 {
        // SAFE-ARITH: as in `isqrt64` with `k ≤ 15`: `res < 2^17`, `res + one < 2^31`.
        let next = res.wrapping_add(one);
        if op >= next {
            op = op.wrapping_sub(next);
            res = (res >> 1).wrapping_add(one);
        } else {
            res >>= 1;
        }
        one >>= 2;
    }
    res as u16
}

/// Square root with an 8-bit fraction, like LVGL `lv_sqrt` (bitwise, exact:
/// `i·256 + f = ⌊√x · 256⌋`).
///
/// ```
/// use twine_core::math::{sqrt, SqrtRes};
/// assert_eq!(sqrt(2), SqrtRes { i: 1, f: 106 }); // 1.414 ≈ 1 + 106/256
/// ```
#[must_use]
pub const fn sqrt(x: u32) -> SqrtRes {
    let root = isqrt64((x as u64) << 16);
    SqrtRes {
        i: (root >> 8) as u16,
        f: (root & 0xFF) as u8,
    }
}

/// Square root of a fixed-point number (floor in 1/65536). Negative inputs return zero and
/// log a warning.
#[must_use]
pub fn sqrt_fx(x: Fx) -> Fx {
    if x.0 < 0 {
        crate::warn!(target: "twine::core", "sqrt_fx: negative input {}", x.0);
        return Fx::ZERO;
    }
    Fx(isqrt64((x.0 as u64) << 16) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_matches_lvgl_samples() {
        assert_eq!(sqrt(0), SqrtRes { i: 0, f: 0 });
        assert_eq!(sqrt(2), SqrtRes { i: 1, f: 106 });
        assert_eq!(sqrt(10_000), SqrtRes { i: 100, f: 0 });
        assert_eq!(sqrt(u32::MAX), SqrtRes { i: 65_535, f: 255 });
        assert_eq!(sqrt(1), SqrtRes { i: 1, f: 0 });
    }

    #[test]
    fn sqrt_fx_values() {
        assert_eq!(sqrt_fx(Fx::from_int(4)), Fx::from_int(2));
        assert_eq!(sqrt_fx(Fx::from_ratio(1, 4)), Fx::HALF);
        assert_eq!(sqrt_fx(Fx::from_int(-4)), Fx::ZERO);
        assert_eq!(sqrt_fx(Fx::MAX).to_int_floor(), 181);
    }

    /// The digit-by-digit loop as it was (finding the start by shifting): the reference.
    fn isqrt64_reference(x: u64) -> u32 {
        let mut op = x;
        let mut res: u64 = 0;
        let mut one: u64 = 1 << 62;
        while one > op {
            one >>= 2;
        }
        while one != 0 {
            let next = res + one;
            if op >= next {
                op -= next;
                res = (res >> 1) + one;
            } else {
                res >>= 1;
            }
            one >>= 2;
        }
        res as u32
    }

    #[test]
    fn isqrt64_matches_the_reference_at_every_boundary() {
        for x in 0..70_000u64 {
            assert_eq!(isqrt64(x), isqrt64_reference(x), "{x}");
        }
        for k in 0..64u32 {
            let p = 1u64 << k;
            for x in [p.wrapping_sub(1), p, p.wrapping_add(1)] {
                assert_eq!(isqrt64(x), isqrt64_reference(x), "{x}");
            }
            // Squares and their neighbours.
            let r = 1u64 << (k / 2);
            for x in [r * r - 1, r * r, r * r + 1] {
                assert_eq!(isqrt64(x), isqrt64_reference(x), "{x}");
            }
        }
        assert_eq!(isqrt64(u64::MAX), isqrt64_reference(u64::MAX));
        assert_eq!(isqrt64(u64::from(u32::MAX)), 65_535);
    }

    proptest::proptest! {
        #[test]
        fn isqrt64_matches_the_reference(x in proptest::num::u64::ANY) {
            proptest::prop_assert_eq!(isqrt64(x), isqrt64_reference(x));
        }
    }

    #[test]
    fn isqrt64_small() {
        for (x, r) in [
            (0u64, 0u32),
            (1, 1),
            (3, 1),
            (4, 2),
            (15, 3),
            (16, 4),
            (1 << 62, 1 << 31),
        ] {
            assert_eq!(isqrt64(x), r, "{x}");
        }
    }
}
