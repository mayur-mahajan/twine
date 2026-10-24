//! Locale-aware number formatting without floating point or allocation: [`Decimal`] (an
//! integer or a fixed-point value with fraction-digit options) written with a language's
//! [`NumberFormat`] (decimal and grouping separators, grouping sizes — Indian `12,34,567`
//! included —, minus sign, percent pattern and native digits such as Arabic-Indic `٠١٢`).
//!
//! The generated catalog holds one `NumberFormat` per language (CLDR data, chosen at build
//! time; `Language::NUMBER_FORMATS`), and [`tr!`](crate::tr) formats integer and `Decimal`
//! arguments with the current language's. Nothing of it is linked unless a number is
//! formatted.
//!
//! ```
//! use twine_extra::i18n::number::{Decimal, NumberFormat};
//!
//! let de = NumberFormat { decimal: ",", group: ".", ..NumberFormat::ROOT };
//! let mut s = String::new();
//! Decimal::new(1_234_567, 2).write(&de, &mut s).unwrap();
//! assert_eq!(s, "12.345,67");
//! ```

use core::fmt::{self, Display, Write};

use super::plural::PluralOperands;

/// How one language writes numbers (CLDR symbols and patterns of its default numbering
/// system).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    /// Decimal separator (`"."`, `","`, `"٫"`).
    pub decimal: &'static str,
    /// Grouping separator (`","`, `"."`, `"\u{202F}"`, `"٬"`).
    pub group: &'static str,
    /// Digits in the group nearest the decimal separator (3); 0 = never group.
    pub primary_group: u8,
    /// Digits in the further groups (3, or 2 for the Indian `12,34,567`).
    pub secondary_group: u8,
    /// Grouping starts only when the integer part has at least `primary_group +
    /// min_grouping` digits (Spanish and Polish: 2, so `1234` but `12 345`).
    pub min_grouping: u8,
    /// Minus sign, with any bidi marks (`"-"`, `"−"`, `"\u{61C}-"`).
    pub minus: &'static str,
    /// Text before a percentage (`""`, Turkish `"%"`).
    pub percent_prefix: &'static str,
    /// Text after a percentage (`"%"`, German `"\u{A0}%"`).
    pub percent_suffix: &'static str,
    /// The digit zero of the numbering system (`'0'`, Arabic-Indic `'٠'`, Extended
    /// Arabic-Indic `'۰'`); digits 1–9 follow it in Unicode.
    pub zero: char,
}

impl NumberFormat {
    /// CLDR's root locale: `1,234.5`, `-1`, `12%`, ASCII digits.
    pub const ROOT: NumberFormat = NumberFormat {
        decimal: ".",
        group: ",",
        primary_group: 3,
        secondary_group: 3,
        min_grouping: 1,
        minus: "-",
        percent_prefix: "",
        percent_suffix: "%",
        zero: '0',
    };

    /// Writes digit `d` (0–9).
    fn digit(&self, d: u8, w: &mut dyn Write) -> fmt::Result {
        let c = char::from_u32(u32::from(self.zero) + u32::from(d)).unwrap_or('?');
        w.write_char(c)
    }
}

/// The largest scale (fraction digits) a [`Decimal`] carries.
pub const MAX_SCALE: u8 = 18;

/// An integer or fixed-point number to format: `magnitude / 10^scale`, with a sign and
/// display options. `Copy`; integers convert with `From`.
///
/// Fraction digits: by default all `scale` digits are shown (`Decimal::new(100, 2)` is
/// `1.00`, as CLDR's visible-fraction plural operands expect). Once either option is set,
/// the number shows between [`min_fraction_digits`](Self::min_fraction_digits) (default 0;
/// pads with zeros) and [`max_fraction_digits`](Self::max_fraction_digits) (default the scale;
/// rounds half to even) fraction digits, without trailing zeros beyond the minimum.
///
/// ```
/// use twine_extra::i18n::number::{Decimal, NumberFormat};
///
/// let f = |d: Decimal| { let mut s = String::new(); d.write(&NumberFormat::ROOT, &mut s).unwrap(); s };
/// assert_eq!(f(Decimal::new(-12345, 2)), "-123.45");
/// assert_eq!(f(Decimal::new(12345, 2).max_fraction_digits(1)), "123.4");   // half to even
/// assert_eq!(f(Decimal::new(150, 2).max_fraction_digits(2).min_fraction_digits(0)), "1.5");
/// assert_eq!(f(Decimal::from(7).min_fraction_digits(2)), "7.00");
/// assert_eq!(f(Decimal::new(25, 2).percent()), "25%");
/// assert_eq!(f(Decimal::from(1234567u32).without_grouping()), "1234567");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decimal {
    magnitude: u64,
    negative: bool,
    scale: u8,
    min_fraction: u8,
    max_fraction: u8,
    percent: bool,
    grouping: bool,
    /// Whether `min_fraction` / `max_fraction` were set (else they follow the scale).
    explicit: bool,
}

impl Decimal {
    /// `mantissa / 10^scale` (`Decimal::new(12345, 2)` is 123.45); `scale` is clamped to
    /// [`MAX_SCALE`].
    #[must_use]
    pub const fn new(mantissa: i64, scale: u8) -> Decimal {
        let scale = if scale > MAX_SCALE { MAX_SCALE } else { scale };
        Decimal {
            magnitude: mantissa.unsigned_abs(),
            negative: mantissa < 0,
            scale,
            min_fraction: scale,
            max_fraction: scale,
            percent: false,
            grouping: true,
            explicit: false,
        }
    }

    /// A non-negative integer.
    #[must_use]
    pub const fn unsigned(n: u64) -> Decimal {
        Decimal {
            magnitude: n,
            negative: false,
            scale: 0,
            min_fraction: 0,
            max_fraction: 0,
            percent: false,
            grouping: true,
            explicit: false,
        }
    }

    /// Shows at least `n` fraction digits (padding with zeros; at most [`MAX_SCALE`]).
    #[must_use]
    pub const fn min_fraction_digits(mut self, n: u8) -> Decimal {
        self = self.make_explicit();
        self.min_fraction = if n > MAX_SCALE { MAX_SCALE } else { n };
        if self.max_fraction < self.min_fraction {
            self.max_fraction = self.min_fraction;
        }
        self
    }

    /// Shows at most `n` fraction digits (rounding half to even, then dropping trailing
    /// zeros down to the minimum).
    #[must_use]
    pub const fn max_fraction_digits(mut self, n: u8) -> Decimal {
        self = self.make_explicit();
        self.max_fraction = if n > MAX_SCALE { MAX_SCALE } else { n };
        if self.min_fraction > self.max_fraction {
            self.min_fraction = self.max_fraction;
        }
        self
    }

    /// Switches from "every digit of the scale" to explicit options: at least 0 and at most
    /// `scale` fraction digits until set.
    const fn make_explicit(mut self) -> Decimal {
        if !self.explicit {
            self.explicit = true;
            self.min_fraction = 0;
            self.max_fraction = self.scale;
        }
        self
    }

    /// Formats as a percentage of 1 (`0.25` → `25%`, with the language's pattern).
    #[must_use]
    pub const fn percent(mut self) -> Decimal {
        self.percent = true;
        self
    }

    /// Writes the integer part without grouping separators.
    #[must_use]
    pub const fn without_grouping(mut self) -> Decimal {
        self.grouping = false;
        self
    }

    /// The value as shown: `(negative, magnitude, scale)` after the percent shift, rounding
    /// and padding.
    fn shown(&self) -> (bool, u128, u8) {
        let mut mag = u128::from(self.magnitude);
        let mut scale = self.scale;
        if self.percent {
            if scale >= 2 {
                scale -= 2;
            } else {
                mag *= 10u128.pow(u32::from(2 - scale));
                scale = 0;
            }
        }
        // The options count digits as shown (of the percentage); without them, every digit.
        let (min, max) = if self.explicit {
            (self.min_fraction, self.max_fraction)
        } else {
            (scale, scale)
        };
        if scale > max {
            let div = 10u128.pow(u32::from(scale - max));
            // DIV: `div` is 10^k with k ≥ 1 (scale > max), never zero.
            let (q, r) = (mag / div, mag % div);
            let half = div / 2;
            mag = if r > half || (r == half && q % 2 == 1) {
                q + 1
            } else {
                q
            };
            scale = max;
        }
        if self.explicit {
            // Trailing zeros are shown only down to the minimum.
            while scale > min && mag % 10 == 0 {
                mag /= 10;
                scale -= 1;
            }
        }
        while scale < min {
            mag *= 10;
            scale += 1;
        }
        (self.negative && mag != 0, mag, scale)
    }

    /// The CLDR plural operands of the number as [`write`](Self::write) shows it (`1.0` has
    /// `v` = 1 and is `other` in English, `1` is `one`). `None` only beyond `u64`.
    #[must_use]
    pub fn operands(&self) -> Option<PluralOperands> {
        let (negative, mag, scale) = self.shown();
        let p = 10u128.pow(u32::from(scale));
        // DIV: `p` is 10^scale ≥ 1.
        let i = u64::try_from(mag / p).ok()?;
        // DIV: `p` is 10^scale ≥ 1.
        let f = u64::try_from(mag % p).ok()?;
        PluralOperands::decimal(negative, i, scale, f)
    }

    /// Writes the number with `format`. Allocation-free.
    ///
    /// # Errors
    /// Only those of `w`.
    pub fn write(&self, format: &NumberFormat, w: &mut dyn Write) -> fmt::Result {
        let (negative, mag, scale) = self.shown();
        let p = 10u128.pow(u32::from(scale));
        // DIV: `p` is 10^scale ≥ 1.
        let (int, frac) = (mag / p, mag % p);
        if negative {
            w.write_str(format.minus)?;
        }
        if self.percent {
            w.write_str(format.percent_prefix)?;
        }
        // Integer digits, most significant first (u128 has at most 39 digits).
        let mut digits = [0u8; 40];
        let mut n = 0;
        let mut v = int;
        loop {
            if let Some(d) = digits.get_mut(n) {
                *d = (v % 10) as u8;
            }
            n += 1;
            v /= 10;
            if v == 0 || n >= digits.len() {
                break;
            }
        }
        let primary = usize::from(format.primary_group);
        let secondary = usize::from(format.secondary_group.max(1));
        let group = self.grouping && primary > 0 && n >= primary + usize::from(format.min_grouping.max(1));
        for i in (0..n).rev() {
            // `i` digits follow this one.
            let d = digits.get(i).copied().unwrap_or(0);
            format.digit(d, w)?;
            if group && i > 0 && (i == primary || (i > primary && (i - primary) % secondary == 0)) {
                w.write_str(format.group)?;
            }
        }
        if scale > 0 {
            w.write_str(format.decimal)?;
            let mut place = p / 10;
            while place > 0 {
                // DIV: the loop runs only while `place` > 0.
                format.digit(((frac / place) % 10) as u8, w)?;
                place /= 10;
            }
        }
        if self.percent {
            w.write_str(format.percent_suffix)?;
        }
        Ok(())
    }

    /// A `Display` of the number in `format` (for `text!`, `format!` or a `Write`).
    ///
    /// ```
    /// use twine_extra::i18n::number::{Decimal, NumberFormat};
    ///
    /// assert_eq!(Decimal::from(-5).with(&NumberFormat::ROOT).to_string(), "-5");
    /// ```
    #[must_use]
    pub const fn with(self, format: &NumberFormat) -> Localized<'_> {
        Localized { value: self, format }
    }
}

/// A [`Decimal`] with its [`NumberFormat`]: `Display` writes it localized.
#[derive(Clone, Copy, Debug)]
pub struct Localized<'a> {
    value: Decimal,
    format: &'a NumberFormat,
}

impl Display for Localized<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.write(self.format, f)
    }
}

/// Without a language: [`NumberFormat::ROOT`] (`1,234.5`).
impl Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(&NumberFormat::ROOT, f)
    }
}

macro_rules! from_signed {
    ($($t:ty),*) => {$(
        impl From<$t> for Decimal {
            #[inline]
            fn from(n: $t) -> Decimal {
                Decimal::new(i64::from(n), 0)
            }
        }
    )*};
}
macro_rules! from_unsigned {
    ($($t:ty),*) => {$(
        impl From<$t> for Decimal {
            #[inline]
            fn from(n: $t) -> Decimal {
                Decimal::unsigned(u64::from(n))
            }
        }
    )*};
}
from_signed!(i8, i16, i32, i64);
from_unsigned!(u8, u16, u32, u64);

impl From<isize> for Decimal {
    #[inline]
    fn from(n: isize) -> Decimal {
        // `isize` is at most 64 bits on every supported target.
        Decimal::new(n as i64, 0)
    }
}

impl From<usize> for Decimal {
    #[inline]
    fn from(n: usize) -> Decimal {
        Decimal::unsigned(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;
    use std::string::String;

    fn fmt(d: Decimal, f: &NumberFormat) -> String {
        let mut s = String::new();
        d.write(f, &mut s).unwrap();
        s
    }

    #[test]
    fn grouping_patterns() {
        let r = NumberFormat::ROOT;
        assert_eq!(fmt(Decimal::from(0), &r), "0");
        assert_eq!(fmt(Decimal::from(999), &r), "999");
        assert_eq!(fmt(Decimal::from(1000), &r), "1,000");
        assert_eq!(fmt(Decimal::from(-1_234_567), &r), "-1,234,567");
        assert_eq!(fmt(Decimal::from(u64::MAX), &r), "18,446,744,073,709,551,615");
        let india = NumberFormat {
            secondary_group: 2,
            ..r
        };
        assert_eq!(fmt(Decimal::from(1_234_567), &india), "12,34,567");
        assert_eq!(fmt(Decimal::from(123_456_789), &india), "12,34,56,789");
        let es = NumberFormat { min_grouping: 2, ..r };
        assert_eq!(fmt(Decimal::from(1234), &es), "1234");
        assert_eq!(fmt(Decimal::from(12345), &es), "12,345");
        let none = NumberFormat {
            primary_group: 0,
            ..r
        };
        assert_eq!(fmt(Decimal::from(1_234_567), &none), "1234567");
    }

    #[test]
    fn fractions_rounding_and_percent() {
        let r = NumberFormat::ROOT;
        assert_eq!(fmt(Decimal::new(5, 3), &r), "0.005");
        assert_eq!(fmt(Decimal::new(-4, 1).max_fraction_digits(0), &r), "0", "no -0");
        assert_eq!(fmt(Decimal::new(125, 2).max_fraction_digits(1), &r), "1.2");
        assert_eq!(fmt(Decimal::new(135, 2).max_fraction_digits(1), &r), "1.4");
        assert_eq!(fmt(Decimal::new(1999, 3).max_fraction_digits(2), &r), "2");
        assert_eq!(
            fmt(
                Decimal::new(1999, 3)
                    .max_fraction_digits(2)
                    .min_fraction_digits(2),
                &r
            ),
            "2.00"
        );
        assert_eq!(fmt(Decimal::new(1234, 4).percent(), &r), "12.34%");
        assert_eq!(fmt(Decimal::from(2).percent(), &r), "200%");
        let arab = NumberFormat {
            decimal: "\u{66B}",
            group: "\u{66C}",
            zero: '\u{660}',
            ..r
        };
        assert_eq!(fmt(Decimal::new(123_456, 2), &arab), "١٬٢٣٤٫٥٦");
    }

    #[test]
    fn operands_follow_the_shown_digits() {
        let o = Decimal::new(10, 1).operands().unwrap();
        assert_eq!((o.i(), o.v(), o.f()), (1, 1, 0));
        let o = Decimal::new(10, 1)
            .max_fraction_digits(1)
            .min_fraction_digits(0)
            .operands()
            .unwrap();
        assert_eq!((o.i(), o.v()), (1, 0));
        let o = Decimal::new(-15, 1).operands().unwrap();
        assert!(o.is_negative());
    }
}
