//! SVG number, length and list parsing into [`Fx`] without floating point.
//!
//! Numbers follow the SVG grammar (`[+-]? (digits [. digits] | . digits) ([eE] [+-]? digits)?`)
//! and are converted exactly (up to 19 significant digits) with integer arithmetic, rounded to
//! the nearest 1/65536 and saturated to the `Fx` range.

use twine_core::{Angle, Fx};

/// A cursor over SVG micro-syntax (numbers separated by whitespace and/or one comma).
#[derive(Clone, Debug)]
pub(crate) struct Cursor<'a> {
    b: &'a [u8],
    pub(crate) pos: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0C')
}

impl<'a> Cursor<'a> {
    /// Moves the position forward by `n` bytes that the caller has just read or found, so
    /// `pos + n ≤ len`.
    #[inline]
    pub(crate) fn bump(&mut self, n: usize) {
        // SAFE-ARITH: `pos + n ≤ len ≤ isize::MAX` (the caller's contract above).
        self.pos = self.pos.wrapping_add(n);
    }

    pub(crate) fn new(s: &'a str) -> Self {
        Self {
            b: s.as_bytes(),
            pos: 0,
        }
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    pub(crate) fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.b.len()
    }

    pub(crate) fn skip_ws(&mut self) {
        while self.peek().is_some_and(is_ws) {
            self.bump(1);
        }
    }

    /// Skips whitespace, at most one comma, whitespace.
    pub(crate) fn skip_sep(&mut self) {
        self.skip_ws();
        if self.peek() == Some(b',') {
            self.bump(1);
            self.skip_ws();
        }
    }

    /// Consumes `c` if it is next (after whitespace).
    pub(crate) fn eat(&mut self, c: u8) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.bump(1);
            true
        } else {
            false
        }
    }

    /// The remaining text.
    pub(crate) fn rest(&self) -> &'a [u8] {
        self.b.get(self.pos..).unwrap_or(&[])
    }

    /// Parses a number (leading whitespace allowed), without a trailing separator.
    pub(crate) fn number(&mut self) -> Option<Fx> {
        self.skip_ws();
        let (v, n) = parse_number(self.rest())?;
        self.bump(n);
        Some(v)
    }

    /// A number followed by an optional separator.
    pub(crate) fn number_sep(&mut self) -> Option<Fx> {
        let v = self.number()?;
        self.skip_sep();
        Some(v)
    }

    /// An arc flag (`0` or `1`, may be followed directly by the next number).
    pub(crate) fn flag(&mut self) -> Option<bool> {
        self.skip_ws();
        let v = match self.peek()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.bump(1);
        self.skip_sep();
        Some(v)
    }

    /// A unit directly after a number (letters or `%`, no whitespace before it).
    pub(crate) fn unit(&mut self) -> &'a [u8] {
        let s = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == b'%') {
            self.bump(1);
        }
        self.b.get(s..self.pos).unwrap_or_default()
    }

    /// An identifier-like word (letters, digits, `-`).
    pub(crate) fn word(&mut self) -> &'a [u8] {
        self.skip_ws();
        let s = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'%')
        {
            self.bump(1);
        }
        self.b.get(s..self.pos).unwrap_or_default()
    }
}

/// Parses a number at the start of `b`; returns the value and the bytes consumed.
pub(crate) fn parse_number(b: &[u8]) -> Option<(Fx, usize)> {
    // Positions (`i`, `j`) stay at most `b.len()`: each step reads an existing byte first.
    // Digit counters saturate (a number longer than 2^31 digits is not a number anyway).
    let mut i = 0usize;
    let neg = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let mut mant: u64 = 0;
    let mut exp10: i32 = 0;
    let mut digits = 0i32;
    let mut sig = 0u8;
    // Appends the digit `c` to the mantissa. SAFE-ARITH: called with at most 18 significant
    // digits in `mant` (`sig < 19`), so `mant · 10 + 9 < 10^19 < 2^64`; `c` is an ASCII digit.
    let push = |mant: u64, c: u8| {
        mant.wrapping_mul(10)
            .wrapping_add(u64::from(c.wrapping_sub(b'0')))
    };
    while let Some(&c) = b.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        if sig < 19 {
            if mant != 0 || c != b'0' {
                // SAFE-ARITH: `sig < 19`.
                sig = sig.wrapping_add(1);
            }
            mant = push(mant, c);
        } else {
            exp10 = exp10.saturating_add(1);
        }
        digits = digits.saturating_add(1);
        // SAFE-ARITH: `i < b.len()` (the byte exists).
        i = i.wrapping_add(1);
    }
    if b.get(i) == Some(&b'.') {
        // SAFE-ARITH: `i < b.len()` (the `.` exists), and likewise for `j` below.
        let mut j = i.wrapping_add(1);
        let mut frac_digits = 0i32;
        while let Some(&c) = b.get(j) {
            if !c.is_ascii_digit() {
                break;
            }
            if sig < 19 {
                if mant != 0 || c != b'0' {
                    // SAFE-ARITH: `sig < 19`.
                    sig = sig.wrapping_add(1);
                }
                mant = push(mant, c);
                exp10 = exp10.saturating_sub(1);
            }
            frac_digits = frac_digits.saturating_add(1);
            j = j.wrapping_add(1);
        }
        if digits.saturating_add(frac_digits) > 0 {
            digits = digits.saturating_add(frac_digits);
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    // Exponent only when followed by digits (so "1em" is 1 with unit "em").
    if matches!(b.get(i), Some(b'e' | b'E')) {
        // SAFE-ARITH: `i < b.len()` (the `e` exists), and likewise for `j` below.
        let mut j = i.wrapping_add(1);
        let eneg = match b.get(j) {
            Some(b'-') => {
                j = j.wrapping_add(1);
                true
            }
            Some(b'+') => {
                j = j.wrapping_add(1);
                false
            }
            _ => false,
        };
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            let mut e: i32 = 0;
            while let Some(&c) = b.get(j) {
                if !c.is_ascii_digit() {
                    break;
                }
                // SAFE-ARITH: `e ≤ 10_000` (the `min`), so `e · 10 + 9 < 2^17`.
                e = e
                    .wrapping_mul(10)
                    .wrapping_add(i32::from(c.wrapping_sub(b'0')))
                    .min(10_000);
                j = j.wrapping_add(1);
            }
            exp10 = exp10.saturating_add(if eneg { e.wrapping_neg() } else { e });
            i = j;
        }
    }
    let raw = to_raw(mant, exp10);
    // SAFE-ARITH: `|raw| ≤ 2^40` (`to_raw`).
    let v = if neg { raw.wrapping_neg() } else { raw };
    Some((
        Fx::from_raw(v.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32),
        i,
    ))
}

/// `mant · 10^exp10 · 65536`, rounded, saturated to ±2^40.
fn to_raw(mant: u64, exp10: i32) -> i128 {
    const CAP: i128 = 1 << 40;
    if mant == 0 {
        return 0;
    }
    let m = i128::from(mant) << 16;
    if exp10 >= 0 {
        if exp10 > 12 {
            return CAP;
        }
        // SAFE-ARITH: `m < 2^80` and `10^12 < 2^40`.
        m.wrapping_mul(10i128.pow(exp10 as u32)).min(CAP)
    } else {
        let k = exp10.unsigned_abs();
        if k > 38 {
            return 0;
        }
        // `d = 10^k ≥ 1` (`k ≤ 38`, so no overflow): the fallback is never taken.
        // SAFE-ARITH: `m < 2^80` and `d / 2 < 2^126`.
        let d = 10i128.pow(k);
        m.wrapping_add(d / 2).checked_div(d).unwrap_or(0).min(CAP)
    }
}

/// Parses a whole attribute as one number (surrounding whitespace allowed).
pub(crate) fn number(s: &str) -> Option<Fx> {
    let mut c = Cursor::new(s);
    let v = c.number()?;
    c.at_end().then_some(v)
}

/// A length: a number with an optional unit, or a percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Length {
    /// User units (px).
    Px(Fx),
    /// Percentage (value in percent).
    Pct(Fx),
}

impl Length {
    /// Resolves the length: percentages relative to `base`.
    pub(crate) fn resolve(self, base: Fx) -> Fx {
        match self {
            Length::Px(v) => v,
            // SAFE-ARITH: a product of two `i32`s fits in `i64`.
            Length::Pct(p) => {
                Fx::from_raw((i64::from(p.raw()).wrapping_mul(i64::from(base.raw())) / (100 << 16)) as i32)
            }
        }
    }
}

/// Parses a length (`12`, `12px`, `1.5em`, `50%`, `2mm`, …).
pub(crate) fn length(s: &str) -> Option<Length> {
    let mut c = Cursor::new(s);
    let v = c.number()?;
    let unit = c.unit();
    if !c.at_end() {
        return None;
    }
    let scale = |num: i64, den: i64| {
        // SAFE-ARITH: `num ≤ 9600 < 2^14` times an `i32`. `den` is one of the literal divisors 3,
        // 1 and 254 passed below: the fallback is never taken.
        let v = i64::from(v.raw()).wrapping_mul(num).checked_div(den).unwrap_or(0);
        Fx::from_raw(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    };
    Some(match unit {
        b"" | b"px" => Length::Px(v),
        b"%" => Length::Pct(v),
        b"pt" => Length::Px(scale(4, 3)),
        b"pc" | b"em" => Length::Px(scale(16, 1)),
        b"in" => Length::Px(scale(96, 1)),
        b"cm" => Length::Px(scale(9600, 254)),
        b"mm" => Length::Px(scale(960, 254)),
        b"ex" => Length::Px(scale(8, 1)),
        _ => return None,
    })
}

/// Degrees (fixed point) to an [`Angle`] (0.1°), rounded.
pub(crate) fn deg_to_angle(d: Fx) -> Angle {
    // SAFE-ARITH: an `i32` times 10 is below 2^35.
    let v = i64::from(d.raw()).wrapping_mul(10);
    let r = if v >= 0 {
        v.wrapping_add(32_768) >> 16
    } else {
        (v.wrapping_neg().wrapping_add(32_768) >> 16).wrapping_neg()
    };
    Angle::deci_deg(r as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Option<(i32, usize)> {
        parse_number(s.as_bytes()).map(|(v, i)| (v.raw(), i))
    }

    #[test]
    fn numbers() {
        assert_eq!(n("12"), Some((12 << 16, 2)));
        assert_eq!(n("-0.5"), Some((-32_768, 4)));
        assert_eq!(n(".25x"), Some((16_384, 3)));
        assert_eq!(n("+1e2"), Some((100 << 16, 4)));
        assert_eq!(n("25e-1"), Some((163_840, 5)));
        assert_eq!(n("1em"), Some((1 << 16, 1)));
        assert_eq!(n("1.5.5"), Some((98_304, 3)));
        assert_eq!(n("1e999999"), Some((i32::MAX, 8)));
        assert_eq!(n("-1e999999"), Some((i32::MIN, 9)));
        assert_eq!(n("1e-999"), Some((0, 6)));
        assert_eq!(n("0.000000000000000000000001234"), Some((0, 29)));
        assert_eq!(n("123456789012345678901234567890"), Some((i32::MAX, 30)));
        assert_eq!(n("."), None);
        assert_eq!(n("-"), None);
        assert_eq!(n("e5"), None);
        assert_eq!(n("0.1"), Some((6554, 3)));
    }

    #[test]
    fn lengths() {
        assert_eq!(length("10px"), Some(Length::Px(Fx::from_int(10))));
        assert_eq!(length(" 50% "), Some(Length::Pct(Fx::from_int(50))));
        assert_eq!(length("3pt"), Some(Length::Px(Fx::from_int(4))));
        assert_eq!(length("1in"), Some(Length::Px(Fx::from_int(96))));
        assert_eq!(length("10 px"), None);
        assert_eq!(length("10q"), None);
        assert_eq!(
            Length::Pct(Fx::from_int(50)).resolve(Fx::from_int(24)),
            Fx::from_int(12)
        );
        assert_eq!(deg_to_angle(Fx::from_ratio(-45, 2)), Angle::deci_deg(-225));
    }
}
