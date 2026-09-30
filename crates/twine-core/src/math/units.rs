//! Small unit newtypes: [`Fraction`] (0..=1 in 1/255) and [`AngularSpeed`] (degrees per
//! second).

/// A fraction from 0 to 1, stored in 1/255 steps (`255` = 1.0): gradient stop positions, LED
/// brightness and other "how far along" values.
///
/// The 8-bit representation is what the renderer uses directly (LVGL gradient stops are
/// `0..=255` too), so the newtype costs nothing. The field is private: build fractions with
/// [`Fraction::pct`], the constants, or [`Fraction::from_raw`]; read them with
/// [`Fraction::raw`].
///
/// ```
/// use twine_core::Fraction;
/// assert_eq!(Fraction::pct(50), Fraction::from_raw(128));
/// assert_eq!(Fraction::ONE.raw(), 255);
/// assert_eq!(Fraction::pct(250), Fraction::ONE); // clamped
/// assert_eq!(Fraction::ratio(1, 4), Fraction::from_raw(64));
/// ```
///
/// ```compile_fail
/// let _ = twine_core::Fraction(128); // private field: write Fraction::pct(50)
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Fraction(u8);

impl Fraction {
    /// 0.
    pub const ZERO: Fraction = Fraction(0);
    /// ½ (`128`, rounded up like [`Fraction::pct`]`(50)`).
    pub const HALF: Fraction = Fraction(128);
    /// 1.
    pub const ONE: Fraction = Fraction(255);

    /// The fraction whose 8-bit representation is `raw` (`255` = 1.0).
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u8) -> Fraction {
        Fraction(raw)
    }

    /// The 8-bit representation (`Fraction::ONE.raw() == 255`).
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// `p` percent (clamped to 100), rounded: `p · 255 / 100`.
    #[inline]
    #[must_use]
    pub const fn pct(p: u8) -> Fraction {
        let p = if p > 100 { 100 } else { p as u32 };
        Fraction(((p * 255 + 50) / 100) as u8)
    }

    /// `num / den`, clamped to `0..=1` and rounded. A zero `den` gives [`Fraction::ZERO`]
    /// (no division by zero).
    #[must_use]
    pub const fn ratio(num: u32, den: u32) -> Fraction {
        if den == 0 {
            return Fraction::ZERO;
        }
        if num >= den {
            return Fraction::ONE;
        }
        // DIV: `den != 0` checked above; `num < den` so the result is < 255.
        Fraction(((num as u64 * 255 + den as u64 / 2) / den as u64) as u8)
    }
}

/// An angular speed, in whole degrees per second (e.g. how fast dragging may turn an arc).
///
/// ```
/// use twine_core::AngularSpeed;
/// assert_eq!(AngularSpeed::deg_per_s(720).as_deg_per_s(), 720);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AngularSpeed(u16);

impl AngularSpeed {
    /// `d` degrees per second.
    #[inline]
    #[must_use]
    pub const fn deg_per_s(d: u16) -> AngularSpeed {
        AngularSpeed(d)
    }

    /// The speed in degrees per second.
    #[inline]
    #[must_use]
    pub const fn as_deg_per_s(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_constructors() {
        assert_eq!(Fraction::pct(0), Fraction::ZERO);
        assert_eq!(Fraction::pct(100), Fraction::ONE);
        assert_eq!(Fraction::pct(50), Fraction::HALF);
        assert_eq!(Fraction::pct(10).raw(), 26);
        assert_eq!(Fraction::ratio(3, 0), Fraction::ZERO);
        assert_eq!(Fraction::ratio(5, 3), Fraction::ONE);
        assert_eq!(Fraction::ratio(1, 2), Fraction::HALF);
        assert_eq!(Fraction::default(), Fraction::ZERO);
    }
}
