//! Small arithmetic helpers whose range arguments are proven once, here, instead of at every
//! pixel loop that uses them (the safety profile's `clippy::arithmetic_side_effects`,
//! `docs/safety/profile.md` § 4.1). Each compiles to the plain operator.

use twine_core::math::udiv255;

/// `a · b / 255` (rounded like [`udiv255`]) of two 8-bit values: coverage × opacity,
/// alpha × alpha.
#[inline(always)]
pub(crate) const fn mul8(a: u8, b: u8) -> u8 {
    // SAFE-ARITH: a product of two `u8`s is at most `255 · 255 < 2^16`.
    udiv255((a as u32).wrapping_mul(b as u32)) as u8
}

/// `255 − a` for an 8-bit value (the complementary weight of an alpha).
#[inline(always)]
pub(crate) const fn inv8(a: u8) -> u8 {
    // `255 − a` is `!a` for a `u8` (no arithmetic, no proof needed).
    !a
}

/// Number of pixels (or bytes) in `[a, b)`: `b − a`, or 0 when `b ≤ a`. Exact for every pair
/// of `i32`s (the distance is below 2^32).
#[inline(always)]
pub(crate) const fn span(a: i32, b: i32) -> usize {
    if b <= a { 0 } else { b.abs_diff(a) as usize }
}

/// `a + n` for a position `a` and a count `n` whose sum is known to be at most `limit`
/// (an `i32`), e.g. a chunk of a span that ends at `limit`. Exact in that case; saturates at
/// `limit` otherwise.
#[inline(always)]
pub(crate) const fn advance(a: i32, n: usize, limit: i32) -> i32 {
    if n >= span(a, limit) {
        limit
    } else {
        // SAFE-ARITH: `n < limit − a < 2^32`, so `a + n < limit` fits in `i32`: the `u32`
        // reinterpretation of `n` added with wrapping is the exact sum.
        a.wrapping_add(n as u32 as i32)
    }
}

/// Saturates an `i64` into the `i32` range.
#[inline(always)]
pub(crate) const fn sat32(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

/// Saturates an `i128` into the `i64` range.
#[inline(always)]
pub(crate) const fn sat64(v: i128) -> i64 {
    if v > i64::MAX as i128 {
        i64::MAX
    } else if v < i64::MIN as i128 {
        i64::MIN
    } else {
        v as i64
    }
}
