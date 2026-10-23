//! 128-bit division, one out-of-line copy per image.
//!
//! On 32-bit targets (Cortex-M, RISC-V `imc`) the runtime library has no 128-bit division, so
//! LLVM expands every `i128` `/` and `%` inline: a bit-serial loop of several hundred bytes of
//! code at each use (two for a quotient plus a remainder). Twine's renderer forms products of
//! `i32` coordinates in `i128` to stay exact for any input, which made those expansions a large
//! share of its code. Dividing through [`div_rem_i128`] instead keeps a single copy, and its
//! 64-bit fast path (taken whenever both operands fit an `i64`, i.e. for every on-screen
//! coordinate) is a hardware or library 64-bit division instead of a 128-step loop.

/// Quotient and remainder of `a / b`, truncated toward zero: `(a / b, a % b)` exactly as
/// [`i128::checked_div`] and [`i128::checked_rem`] give them. `None` when `b == 0` or for
/// `i128::MIN / −1` (the quotient does not fit), like those methods.
///
/// One out-of-line function shared by every 128-bit division in Twine: 32-bit targets have no
/// 128-bit division in their runtime library, so LLVM expands each `i128` `/` or `%` inline (a
/// bit-serial loop of several hundred bytes); call this instead in code that runs on such
/// targets. When both operands fit an `i64` it divides in 64 bits (same result, and much faster
/// there); otherwise it performs one 128-bit division and derives the remainder by a multiply.
/// Never panics; allocates nothing.
///
/// ```
/// use twine_core::math::div_rem_i128;
/// assert_eq!(div_rem_i128(-7, 2), Some((-3, -1)));
/// assert_eq!(div_rem_i128(1 << 100, 3), Some(((1 << 100) / 3, (1 << 100) % 3)));
/// assert_eq!(div_rem_i128(5, 0), None);
/// assert_eq!(div_rem_i128(i128::MIN, -1), None);
/// ```
#[inline(never)]
#[must_use]
pub fn div_rem_i128(a: i128, b: i128) -> Option<(i128, i128)> {
    if let (Ok(a64), Ok(b64)) = (i64::try_from(a), i64::try_from(b))
        && let Some(q) = a64.checked_div(b64)
    {
        // SAFE-ARITH: `q · b64` is `a64` minus the remainder (`|r| < |b64|`), so it fits.
        return Some((i128::from(q), i128::from(a64.wrapping_sub(q.wrapping_mul(b64)))));
    }
    // `b == 0` or an operand beyond `i64` (or `i64::MIN / −1`): one 128-bit division.
    let q = a.checked_div(b)?;
    // SAFE-ARITH: `q · b` is `a` minus the remainder (`|r| < |b|`), so it fits.
    Some((q, a.wrapping_sub(q.wrapping_mul(b))))
}

/// Euclidean quotient and remainder of `a / b`: `(a.div_euclid(b), a.rem_euclid(b))` exactly
/// as [`i128::checked_div_euclid`] and [`i128::checked_rem_euclid`] give them (the remainder is
/// in `0..|b|`). `None` when `b == 0` or for `i128::MIN / −1`.
///
/// A thin wrapper over the shared [`div_rem_i128`] (one division). Never panics.
///
/// ```
/// use twine_core::math::div_rem_euclid_i128;
/// assert_eq!(div_rem_euclid_i128(-7, 2), Some((-4, 1)));
/// assert_eq!(div_rem_euclid_i128(-7, -2), Some((4, 1)));
/// assert_eq!(div_rem_euclid_i128(7, 0), None);
/// ```
#[inline]
#[must_use]
pub fn div_rem_euclid_i128(a: i128, b: i128) -> Option<(i128, i128)> {
    let (q, r) = div_rem_i128(a, b)?;
    Some(if r >= 0 {
        (q, r)
    } else if b > 0 {
        // SAFE-ARITH: `r < 0` means `a < 0` and `a ≠ q · b`, so `q > i128::MIN`; `−b < r < 0`.
        (q.wrapping_sub(1), r.wrapping_add(b))
    } else {
        // SAFE-ARITH: `r < 0` with `b < 0` means `q ≥ 0`, below `i128::MAX`; `b < r < 0`.
        (q.wrapping_add(1), r.wrapping_sub(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: [i128; 14] = [
        0,
        1,
        -1,
        2,
        -2,
        7,
        -7,
        65_536,
        i64::MAX as i128,
        i64::MIN as i128,
        (i64::MAX as i128) + 1,
        1 << 100,
        -(1 << 100) + 3,
        i128::MAX,
    ];

    #[test]
    fn matches_the_std_operations() {
        for &a in SAMPLES.iter().chain(&[i128::MIN]) {
            for &b in &SAMPLES {
                let std = a.checked_div(b).zip(a.checked_rem(b));
                assert_eq!(div_rem_i128(a, b), std, "{a} / {b}");
                let euclid = a.checked_div_euclid(b).zip(a.checked_rem_euclid(b));
                assert_eq!(div_rem_euclid_i128(a, b), euclid, "{a} /e {b}");
            }
        }
        assert_eq!(div_rem_i128(i128::MIN, -1), None);
        assert_eq!(
            div_rem_i128(i64::MIN.into(), -1),
            Some((-i128::from(i64::MIN), 0))
        );
    }
}
