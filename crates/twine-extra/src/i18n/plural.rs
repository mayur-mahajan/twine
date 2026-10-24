//! CLDR plural rules: [`PluralCategory`], the operands of a number ([`PluralOperands`]) and one
//! function per rule (named after a language that uses it; the generated catalog picks the
//! function of each language, `twine_build::i18n::RULES` lists which languages use which).
//!
//! The rules are CLDR 44 cardinal rules evaluated on integer operands (no floating point):
//! decimals are described by their visible fraction digits, exactly as CLDR defines them
//! (`"1.0"` and `"1"` differ: in English `1 item` but `1.0 items`).
//!
//! ```
//! use twine_extra::i18n::plural::{self, PluralCategory, PluralOperands};
//!
//! let ru = |s: &str| plural::ru(PluralOperands::parse(s).unwrap());
//! assert_eq!(ru("1"), PluralCategory::One);
//! assert_eq!(ru("3"), PluralCategory::Few);
//! assert_eq!(ru("11"), PluralCategory::Many);
//! assert_eq!(ru("1.5"), PluralCategory::Other);
//! assert_eq!(plural::en(PluralOperands::integer(1)), PluralCategory::One);
//! ```

use core::fmt::{self, Display, Write};

/// A CLDR plural category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PluralCategory {
    /// `zero` (e.g. Arabic 0, Latvian 10).
    Zero,
    /// `one` (e.g. English 1, French 0 and 1, Russian 21).
    One,
    /// `two` (e.g. Arabic and Hebrew 2).
    Two,
    /// `few` (e.g. Russian 2–4, Polish 22).
    Few,
    /// `many` (e.g. Russian 5–20, French 1 000 000).
    Many,
    /// `other`: everything else (every language has it).
    Other,
}

/// The CLDR operands of a number: its absolute integer part `i`, the number of visible
/// fraction digits `v` and the visible fraction digits `f` as an integer (`"1.50"`: i = 1,
/// v = 2, f = 50). `n` (the absolute value) is `i` when `f` is 0; `t` is `f` without trailing
/// zeros.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PluralOperands {
    i: u64,
    v: u8,
    f: u64,
    t: u64,
    negative: bool,
}

/// A plural rule: the category of a number in one language.
pub type PluralRule = fn(PluralOperands) -> PluralCategory;

/// The most visible fraction digits [`PluralOperands::parse`] reads (they must fit a `u64`).
const MAX_FRACTION_DIGITS: u8 = 18;

impl PluralOperands {
    /// The operands of a non-negative integer.
    #[must_use]
    pub const fn integer(n: u64) -> PluralOperands {
        PluralOperands {
            i: n,
            v: 0,
            f: 0,
            t: 0,
            negative: false,
        }
    }

    /// The operands of a decimal number with absolute integer part `i` and `v` visible
    /// fraction digits `f` (`1.50`: `i` = 1, `v` = 2, `f` = 50); what a formatted
    /// [`Decimal`](super::number::Decimal) shows. `None` when `v` > 18 or `f` has more than `v`
    /// digits.
    ///
    /// ```
    /// use twine_extra::i18n::plural::PluralOperands;
    ///
    /// let o = PluralOperands::decimal(false, 1, 2, 50).unwrap();
    /// assert_eq!((o.i(), o.v(), o.f(), o.t()), (1, 2, 50, 5));
    /// ```
    #[must_use]
    pub fn decimal(negative: bool, i: u64, v: u8, f: u64) -> Option<PluralOperands> {
        if v > MAX_FRACTION_DIGITS || f >= 10u64.checked_pow(u32::from(v))? {
            return None;
        }
        let mut t = f;
        while t != 0 && t % 10 == 0 {
            t /= 10;
        }
        Some(PluralOperands { i, v, f, t, negative })
    }

    /// The operands of a number written in plain decimal notation (`"3"`, `"-2"`, `"1.50"`;
    /// no grouping separators, no exponent): what CLDR rules are defined on. `None` for any
    /// other text, or when the number does not fit (more than 18 fraction digits or an integer
    /// part above `u64::MAX`).
    ///
    /// ```
    /// use twine_extra::i18n::plural::PluralOperands;
    ///
    /// let o = PluralOperands::parse("-1.50").unwrap();
    /// assert_eq!((o.i(), o.v(), o.f(), o.t()), (1, 2, 50, 5));
    /// assert!(PluralOperands::parse("1,000").is_none());
    /// assert!(PluralOperands::parse("").is_none());
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Option<PluralOperands> {
        let (negative, digits) = match text.as_bytes().first() {
            Some(b'-') => (true, text.get(1..)?),
            Some(b'+') => (false, text.get(1..)?),
            _ => (false, text),
        };
        let (int, frac) = match digits.split_once('.') {
            Some((i, f)) if !f.is_empty() => (i, f),
            Some(_) => return None,
            None => (digits, ""),
        };
        if int.is_empty() || frac.len() > usize::from(MAX_FRACTION_DIGITS) {
            return None;
        }
        let i = parse_digits(int)?;
        let f = if frac.is_empty() { 0 } else { parse_digits(frac)? };
        let mut t = f;
        while t != 0 && t % 10 == 0 {
            t /= 10;
        }
        Some(PluralOperands {
            i,
            // The length was checked against MAX_FRACTION_DIGITS above.
            v: frac.len() as u8,
            f,
            t,
            negative,
        })
    }

    /// The operands of `value` as it displays (formatted into a 40-byte stack buffer, no
    /// allocation): `None` when the text is not a plain decimal number (see
    /// [`parse`](Self::parse)). This is how `tr!` reads the argument a plural message
    /// chooses on, so the category always matches the digits shown.
    ///
    /// ```
    /// use twine_extra::i18n::plural::PluralOperands;
    ///
    /// assert_eq!(PluralOperands::of(&21u8), Some(PluralOperands::integer(21)));
    /// assert_eq!(PluralOperands::of(&"many"), None);
    /// ```
    #[must_use]
    pub fn of(value: &dyn Display) -> Option<PluralOperands> {
        let mut buf = StackText { buf: [0; 40], len: 0 };
        write!(buf, "{value}").ok()?;
        let text = core::str::from_utf8(buf.buf.get(..buf.len)?).ok()?;
        PluralOperands::parse(text)
    }

    /// The absolute integer part `i`.
    #[must_use]
    pub const fn i(&self) -> u64 {
        self.i
    }

    /// The number of visible fraction digits `v`.
    #[must_use]
    pub const fn v(&self) -> u8 {
        self.v
    }

    /// The visible fraction digits `f` (with trailing zeros).
    #[must_use]
    pub const fn f(&self) -> u64 {
        self.f
    }

    /// The visible fraction digits without trailing zeros `t`.
    #[must_use]
    pub const fn t(&self) -> u64 {
        self.t
    }

    /// Whether the number is negative (the rules use the absolute value).
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// `n` when it is an integer (`f` = 0), else `None`: CLDR conditions on `n` (`n = 1`,
    /// `n % 10 = 3..9`) only hold for integral values.
    #[must_use]
    pub const fn n_integer(&self) -> Option<u64> {
        if self.f == 0 { Some(self.i) } else { None }
    }

    /// Whether the value is exactly the non-negative integer `n` (what a `[0]` variant
    /// matches).
    #[must_use]
    pub const fn is_exactly(&self, n: u64) -> bool {
        !self.negative && self.f == 0 && self.i == n
    }
}

fn parse_digits(s: &str) -> Option<u64> {
    let mut v: u64 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
    }
    Some(v)
}

/// A fixed-size text buffer (formatting fails when it is full).
struct StackText {
    buf: [u8; 40],
    len: usize,
}

impl Write for StackText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(fmt::Error)?;
        let dst = self.buf.get_mut(self.len..end).ok_or(fmt::Error)?;
        dst.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

use PluralCategory::{Few, Many, One, Other, Two, Zero};

/// `n % m` when `n` is an integer.
const fn n_mod(o: &PluralOperands, m: u64) -> Option<u64> {
    match o.n_integer() {
        // DIV: `m` is a non-zero literal at every call site.
        Some(n) => Some(n % m),
        None => None,
    }
}

const fn in_range(x: Option<u64>, lo: u64, hi: u64) -> bool {
    matches!(x, Some(v) if v >= lo && v <= hi)
}

/// French-style `many`: `e = 0 and i != 0 and i % 1000000 = 0 and v = 0`.
const fn million(o: &PluralOperands) -> bool {
    o.v == 0 && o.i != 0 && o.i % 1_000_000 == 0
}

/// No plural distinction: always `other` (Chinese, Japanese, Korean, Thai, Vietnamese,
/// Indonesian, …).
#[must_use]
pub fn other(_: PluralOperands) -> PluralCategory {
    Other
}

/// `one: i = 1 and v = 0` (English, German, Dutch, Swedish, Finnish, Estonian, Urdu, …).
#[must_use]
pub fn en(o: PluralOperands) -> PluralCategory {
    if o.i == 1 && o.v == 0 { One } else { Other }
}

/// `one: n = 1` (Turkish, Greek, Hungarian, Bulgarian, Norwegian, Albanian, …).
#[must_use]
pub fn tr(o: PluralOperands) -> PluralCategory {
    if o.n_integer() == Some(1) { One } else { Other }
}

/// `one: i = 0,1`; `many`: millions (French, Brazilian Portuguese).
#[must_use]
pub fn fr(o: PluralOperands) -> PluralCategory {
    if o.i <= 1 {
        One
    } else if million(&o) {
        Many
    } else {
        Other
    }
}

/// `one: n = 1`; `many`: millions (Spanish).
#[must_use]
pub fn es(o: PluralOperands) -> PluralCategory {
    if o.n_integer() == Some(1) {
        One
    } else if million(&o) {
        Many
    } else {
        Other
    }
}

/// `one: i = 1 and v = 0`; `many`: millions (Italian, Catalan, European Portuguese).
#[must_use]
pub fn it(o: PluralOperands) -> PluralCategory {
    if o.i == 1 && o.v == 0 {
        One
    } else if million(&o) {
        Many
    } else {
        Other
    }
}

/// Russian and Ukrainian: `one` 1, 21, 31…; `few` 2–4, 22–24…; `many` 0, 5–20, 25–30…;
/// `other` decimals.
#[must_use]
pub fn ru(o: PluralOperands) -> PluralCategory {
    if o.v != 0 {
        return Other;
    }
    let (m10, m100) = (o.i % 10, o.i % 100);
    if m10 == 1 && m100 != 11 {
        One
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        Few
    } else {
        Many
    }
}

/// Belarusian: like Russian but on `n` (`1.0` is `one`), decimals with a fraction `other`.
#[must_use]
pub fn be(o: PluralOperands) -> PluralCategory {
    let (m10, m100) = (n_mod(&o, 10), n_mod(&o, 100));
    if m10 == Some(1) && m100 != Some(11) {
        One
    } else if in_range(m10, 2, 4) && !in_range(m100, 12, 14) {
        Few
    } else if m10 == Some(0) || in_range(m10, 5, 9) || in_range(m100, 11, 14) {
        Many
    } else {
        Other
    }
}

/// Polish: `one` 1; `few` 2–4, 22–24…; `many` 0, 5–21, 25–31…; `other` decimals.
#[must_use]
pub fn pl(o: PluralOperands) -> PluralCategory {
    if o.v != 0 {
        return Other;
    }
    let (m10, m100) = (o.i % 10, o.i % 100);
    if o.i == 1 {
        One
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        Few
    } else {
        Many
    }
}

/// Czech and Slovak: `one` 1; `few` 2–4; `many` decimals; `other` 0, 5….
#[must_use]
pub fn cs(o: PluralOperands) -> PluralCategory {
    if o.v != 0 {
        Many
    } else if o.i == 1 {
        One
    } else if (2..=4).contains(&o.i) {
        Few
    } else {
        Other
    }
}

/// Arabic: `zero` 0; `one` 1; `two` 2; `few` 3–10, 103–110…; `many` 11–99, 111–199…;
/// `other` 100–102, decimals.
#[must_use]
pub fn ar(o: PluralOperands) -> PluralCategory {
    let m100 = n_mod(&o, 100);
    match o.n_integer() {
        Some(0) => Zero,
        Some(1) => One,
        Some(2) => Two,
        _ if in_range(m100, 3, 10) => Few,
        _ if in_range(m100, 11, 99) => Many,
        _ => Other,
    }
}

/// Hebrew: `one` 1 (and 0.x); `two` 2; `other` everything else.
#[must_use]
pub fn he(o: PluralOperands) -> PluralCategory {
    if (o.i == 1 && o.v == 0) || (o.i == 0 && o.v != 0) {
        One
    } else if o.i == 2 && o.v == 0 {
        Two
    } else {
        Other
    }
}

/// `one: i = 0 or n = 1` (Hindi, Persian, Bengali, Gujarati, Kannada, Amharic, Zulu, …).
#[must_use]
pub fn hi(o: PluralOperands) -> PluralCategory {
    if o.i == 0 || o.n_integer() == Some(1) {
        One
    } else {
        Other
    }
}

/// Danish: `one: n = 1 or t != 0 and i = 0,1`.
#[must_use]
pub fn da(o: PluralOperands) -> PluralCategory {
    if o.n_integer() == Some(1) || (o.t != 0 && o.i <= 1) {
        One
    } else {
        Other
    }
}

/// Icelandic: `one: t = 0 and i % 10 = 1 and i % 100 != 11 or t % 10 = 1 and t % 100 != 11`.
#[must_use]
pub fn is(o: PluralOperands) -> PluralCategory {
    if (o.t == 0 && o.i % 10 == 1 && o.i % 100 != 11) || (o.t % 10 == 1 && o.t % 100 != 11) {
        One
    } else {
        Other
    }
}

/// Lithuanian: `one` 1, 21…; `few` 2–9, 22–29…; `many` decimals; `other` 0, 10–20….
#[must_use]
pub fn lt(o: PluralOperands) -> PluralCategory {
    let (m10, m100) = (n_mod(&o, 10), n_mod(&o, 100));
    if o.f != 0 {
        Many
    } else if m10 == Some(1) && !in_range(m100, 11, 19) {
        One
    } else if in_range(m10, 2, 9) && !in_range(m100, 11, 19) {
        Few
    } else {
        Other
    }
}

/// Latvian: `zero` 0, 10–20, 30…; `one` 1, 21, 31…; `other` the rest.
#[must_use]
pub fn lv(o: PluralOperands) -> PluralCategory {
    let (m10, m100) = (n_mod(&o, 10), n_mod(&o, 100));
    let (f10, f100) = (o.f % 10, o.f % 100);
    if m10 == Some(0) || in_range(m100, 11, 19) || (o.v == 2 && (11..=19).contains(&f100)) {
        Zero
    } else if (m10 == Some(1) && m100 != Some(11)) || (f10 == 1 && (o.v != 2 || f100 != 11)) {
        One
    } else {
        Other
    }
}

/// Romanian: `one` 1; `few` 0, 2–19, 101–119…, decimals; `other` 20–100….
#[must_use]
pub fn ro(o: PluralOperands) -> PluralCategory {
    let n = o.n_integer();
    if o.i == 1 && o.v == 0 {
        One
    } else if o.v != 0 || n == Some(0) || (n != Some(1) && in_range(n_mod(&o, 100), 1, 19)) {
        Few
    } else {
        Other
    }
}

/// Slovenian: `one` 1, 101…; `two` 2, 102…; `few` 3–4, 103–104…, decimals; `other` the rest.
#[must_use]
pub fn sl(o: PluralOperands) -> PluralCategory {
    let m100 = o.i % 100;
    if o.v != 0 {
        Few
    } else if m100 == 1 {
        One
    } else if m100 == 2 {
        Two
    } else if (3..=4).contains(&m100) {
        Few
    } else {
        Other
    }
}

/// Croatian, Serbian, Bosnian: `one` 1, 21…; `few` 2–4, 22–24…; `other` the rest (decimals by
/// their fraction digits).
#[must_use]
pub fn hr(o: PluralOperands) -> PluralCategory {
    let (i10, i100, f10, f100) = (o.i % 10, o.i % 100, o.f % 10, o.f % 100);
    if (o.v == 0 && i10 == 1 && i100 != 11) || (f10 == 1 && f100 != 11) {
        One
    } else if (o.v == 0 && (2..=4).contains(&i10) && !(12..=14).contains(&i100))
        || ((2..=4).contains(&f10) && !(12..=14).contains(&f100))
    {
        Few
    } else {
        Other
    }
}

/// Macedonian: `one: v = 0 and i % 10 = 1 and i % 100 != 11 or f % 10 = 1 and f % 100 != 11`.
#[must_use]
pub fn mk(o: PluralOperands) -> PluralCategory {
    if (o.v == 0 && o.i % 10 == 1 && o.i % 100 != 11) || (o.f % 10 == 1 && o.f % 100 != 11) {
        One
    } else {
        Other
    }
}

/// Filipino: `one` unless the last digit is 4, 6 or 9 (1, 2, 3 always `one`).
#[must_use]
pub fn fil(o: PluralOperands) -> PluralCategory {
    let bad = |d: u64| matches!(d, 4 | 6 | 9);
    let one = if o.v == 0 {
        (1..=3).contains(&o.i) || !bad(o.i % 10)
    } else {
        !bad(o.f % 10)
    };
    if one { One } else { Other }
}

/// Irish: `one` 1; `two` 2; `few` 3–6; `many` 7–10; `other` the rest.
#[must_use]
pub fn ga(o: PluralOperands) -> PluralCategory {
    match o.n_integer() {
        Some(1) => One,
        Some(2) => Two,
        Some(3..=6) => Few,
        Some(7..=10) => Many,
        _ => Other,
    }
}

/// Welsh: `zero` 0; `one` 1; `two` 2; `few` 3; `many` 6; `other` the rest.
#[must_use]
pub fn cy(o: PluralOperands) -> PluralCategory {
    match o.n_integer() {
        Some(0) => Zero,
        Some(1) => One,
        Some(2) => Two,
        Some(3) => Few,
        Some(6) => Many,
        _ => Other,
    }
}

/// Every rule by name (the names `twine_build::i18n::RULES` uses), for tests and tools.
pub const RULES: &[(&str, PluralRule)] = &[
    ("other", other),
    ("en", en),
    ("tr", tr),
    ("fr", fr),
    ("es", es),
    ("it", it),
    ("ru", ru),
    ("be", be),
    ("pl", pl),
    ("cs", cs),
    ("ar", ar),
    ("he", he),
    ("hi", hi),
    ("da", da),
    ("is", is),
    ("lt", lt),
    ("lv", lv),
    ("ro", ro),
    ("sl", sl),
    ("hr", hr),
    ("mk", mk),
    ("fil", fil),
    ("ga", ga),
    ("cy", cy),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(rule: PluralRule, s: &str) -> PluralCategory {
        rule(PluralOperands::parse(s).unwrap())
    }

    #[test]
    fn cldr_samples() {
        assert_eq!(cat(en, "1"), One);
        assert_eq!(cat(en, "1.0"), Other);
        assert_eq!(cat(en, "-1"), One);
        assert_eq!(cat(tr, "1.0"), One);
        assert_eq!(cat(fr, "0"), One);
        assert_eq!(cat(fr, "1.5"), One);
        assert_eq!(cat(fr, "1000000"), Many);
        assert_eq!(cat(fr, "2"), Other);
        assert_eq!(cat(ru, "21"), One);
        assert_eq!(cat(ru, "22"), Few);
        assert_eq!(cat(ru, "12"), Many);
        assert_eq!(cat(ru, "0"), Many);
        assert_eq!(cat(pl, "1"), One);
        assert_eq!(cat(pl, "21"), Many);
        assert_eq!(cat(pl, "22"), Few);
        assert_eq!(cat(cs, "3"), Few);
        assert_eq!(cat(cs, "1.5"), Many);
        assert_eq!(cat(cs, "5"), Other);
        assert_eq!(cat(ar, "0"), Zero);
        assert_eq!(cat(ar, "2"), Two);
        assert_eq!(cat(ar, "103"), Few);
        assert_eq!(cat(ar, "111"), Many);
        assert_eq!(cat(ar, "100"), Other);
        assert_eq!(cat(he, "2"), Two);
        assert_eq!(cat(he, "0.5"), One);
        assert_eq!(cat(hi, "0"), One);
        assert_eq!(cat(da, "0.1"), One);
        assert_eq!(cat(is, "21"), One);
        assert_eq!(cat(lt, "11"), Other);
        assert_eq!(cat(lt, "0.5"), Many);
        assert_eq!(cat(lv, "10"), Zero);
        assert_eq!(cat(lv, "21"), One);
        assert_eq!(cat(ro, "0"), Few);
        assert_eq!(cat(ro, "101"), Few);
        assert_eq!(cat(ro, "20"), Other);
        assert_eq!(cat(sl, "102"), Two);
        assert_eq!(cat(hr, "0.1"), One);
        assert_eq!(cat(mk, "11"), Other);
        assert_eq!(cat(fil, "4"), Other);
        assert_eq!(cat(fil, "13"), One);
        assert_eq!(cat(ga, "7"), Many);
        assert_eq!(cat(cy, "6"), Many);
        assert_eq!(cat(be, "1.0"), One);
        assert_eq!(cat(be, "1.5"), Other);
    }

    #[test]
    fn operands() {
        assert_eq!(
            PluralOperands::parse("1.50").map(|o| (o.i, o.v, o.f, o.t)),
            Some((1, 2, 50, 5))
        );
        assert_eq!(PluralOperands::parse("18446744073709551616"), None, "overflow");
        assert_eq!(PluralOperands::parse("1."), None);
        assert_eq!(PluralOperands::parse(".5"), None);
        assert_eq!(PluralOperands::parse("-"), None);
        assert_eq!(PluralOperands::parse("1e3"), None);
        assert!(
            PluralOperands::of(&"x".repeat(100)).is_none(),
            "longer than the buffer"
        );
        assert!(PluralOperands::parse("-0").unwrap().is_negative());
        assert!(!PluralOperands::parse("-0").unwrap().is_exactly(0));
        assert!(PluralOperands::parse("3.0").unwrap().is_exactly(3));
    }
}
