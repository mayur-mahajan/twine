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
    let mut op = x;
    let mut res: u64 = 0;
    let mut one: u64 = 1 << 62;
    while one > op {
        one >>= 2;
    }
    while one != 0 {
        if op >= res + one {
            op -= res + one;
            res = (res >> 1) + one;
        } else {
            res >>= 1;
        }
        one >>= 2;
    }
    res as u32
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
