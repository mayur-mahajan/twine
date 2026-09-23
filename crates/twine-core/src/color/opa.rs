//! Opacity ([`Opa`]).

use crate::math::udiv255;

/// Opacity, `0` (transparent) to `255` (fully covering).
///
/// ```
/// use twine_core::Opa;
/// assert_eq!(Opa::from_percent(50), Opa(128));
/// assert_eq!(Opa::COVER.mul(Opa(77)), Opa(77));
/// assert!(Opa(2).is_transparent() && Opa(253).is_cover());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Opa(pub u8);

#[allow(clippy::should_implement_trait)] // `mul` combines opacities; there is no `Mul` impl to confuse it with
impl Opa {
    /// Fully transparent.
    pub const TRANSP: Opa = Opa(0);
    /// Fully covering.
    pub const COVER: Opa = Opa(255);
    /// 0 %.
    pub const P0: Opa = Opa(0);
    /// 10 %.
    pub const P10: Opa = Opa(25);
    /// 20 %.
    pub const P20: Opa = Opa(51);
    /// 30 %.
    pub const P30: Opa = Opa(76);
    /// 40 %.
    pub const P40: Opa = Opa(102);
    /// 50 %.
    pub const P50: Opa = Opa(127);
    /// 60 %.
    pub const P60: Opa = Opa(153);
    /// 70 %.
    pub const P70: Opa = Opa(178);
    /// 80 %.
    pub const P80: Opa = Opa(204);
    /// 90 %.
    pub const P90: Opa = Opa(229);
    /// 100 %.
    pub const P100: Opa = Opa(255);
    /// Values `<= MIN` are treated as transparent (LVGL `LV_OPA_MIN`).
    pub const MIN: Opa = Opa(2);
    /// Values `>= MAX` are treated as covering (LVGL `LV_OPA_MAX`).
    pub const MAX: Opa = Opa(253);

    /// `p` percent (clamped to 100), rounded: `p · 255 / 100`.
    #[must_use]
    pub const fn from_percent(p: u8) -> Opa {
        let p = if p > 100 { 100 } else { p as u32 };
        Opa(((p * 255 + 50) / 100) as u8)
    }

    /// Combined opacity `udiv255(a · b)`; `COVER.mul(x) == x` exactly.
    #[must_use]
    pub const fn mul(self, other: Opa) -> Opa {
        Opa(udiv255(self.0 as u32 * other.0 as u32) as u8)
    }

    /// `<= MIN`: drawing can be skipped.
    #[must_use]
    pub const fn is_transparent(self) -> bool {
        self.0 <= Self::MIN.0
    }

    /// `>= MAX`: drawing can ignore what is below.
    #[must_use]
    pub const fn is_cover(self) -> bool {
        self.0 >= Self::MAX.0
    }

    /// `255 − self`.
    #[must_use]
    pub const fn inverse(self) -> Opa {
        Opa(255 - self.0)
    }
}

impl From<u8> for Opa {
    fn from(v: u8) -> Self {
        Opa(v)
    }
}

impl From<Opa> for u8 {
    fn from(o: Opa) -> Self {
        o.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opa_mul_cover_is_identity() {
        for x in 0..=255u8 {
            assert_eq!(Opa::COVER.mul(Opa(x)), Opa(x));
            assert_eq!(Opa(x).mul(Opa::COVER), Opa(x));
            assert_eq!(Opa(x).mul(Opa::TRANSP), Opa::TRANSP);
        }
        assert_eq!(Opa(128).mul(Opa(128)), Opa(64));
    }

    #[test]
    fn opa_from_percent_table() {
        let table = [
            (0, 0),
            (1, 3),
            (10, 26),
            (25, 64),
            (50, 128),
            (75, 191),
            (99, 252),
            (100, 255),
            (200, 255),
        ];
        for (p, v) in table {
            assert_eq!(Opa::from_percent(p), Opa(v), "{p}%");
        }
    }

    #[test]
    fn predicates() {
        assert!(Opa(0).is_transparent() && Opa(2).is_transparent() && !Opa(3).is_transparent());
        assert!(Opa(253).is_cover() && !Opa(252).is_cover());
        assert_eq!(Opa(3).inverse(), Opa(252));
        assert_eq!(u8::from(Opa::from(7)), 7);
    }
}
