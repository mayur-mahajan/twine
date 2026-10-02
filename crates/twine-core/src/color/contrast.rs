//! WCAG 2.x relative luminance and contrast ratio ([`Color::relative_luminance`],
//! [`Color::contrast_ratio`], [`ContrastRatio`]), in integer arithmetic.
//!
//! Themes use them to state and verify legibility requirements (minimum contrast between text
//! and its background, see `twine_style::design::ContrastPair`); applications can check their
//! own colors the same way. Nothing here runs while rendering.
//!
//! The sRGB transfer function is a 256-entry table of linear channel values in parts per
//! million (1 KiB of flash, linked only when used), so the results are deterministic on every
//! target and need no floating point.

use core::fmt;

use super::Color;

/// The linear value of each 8-bit sRGB channel value in parts per million (WCAG 2.x:
/// `c ≤ 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055)^2.4` with `c = v / 255`), rounded
/// (generated; verified against the formula by a unit test).
#[allow(clippy::unreadable_literal)]
static SRGB_TO_LINEAR_PPM: [u32; 256] = [
    0, 304, 607, 911, 1214, 1518, 1821, 2125, 2428, 2732, 3035, 3347, 3677, 4025, 4391, 4777, 5182, 5605,
    6049, 6512, 6995, 7499, 8023, 8568, 9134, 9721, 10330, 10960, 11612, 12286, 12983, 13702, 14444, 15209,
    15996, 16807, 17642, 18500, 19382, 20289, 21219, 22174, 23153, 24158, 25187, 26241, 27321, 28426, 29557,
    30713, 31896, 33105, 34340, 35601, 36889, 38204, 39546, 40915, 42311, 43735, 45186, 46665, 48172, 49707,
    51269, 52861, 54480, 56128, 57805, 59511, 61246, 63010, 64803, 66626, 68478, 70360, 72272, 74214, 76185,
    78187, 80220, 82283, 84376, 86500, 88656, 90842, 93059, 95307, 97587, 99899, 102242, 104616, 107023,
    109462, 111932, 114435, 116971, 119538, 122139, 124772, 127438, 130136, 132868, 135633, 138432, 141263,
    144128, 147027, 149960, 152926, 155926, 158961, 162029, 165132, 168269, 171441, 174647, 177888, 181164,
    184475, 187821, 191202, 194618, 198069, 201556, 205079, 208637, 212231, 215861, 219526, 223228, 226966,
    230740, 234551, 238398, 242281, 246201, 250158, 254152, 258183, 262251, 266356, 270498, 274677, 278894,
    283149, 287441, 291771, 296138, 300544, 304987, 309469, 313989, 318547, 323143, 327778, 332452, 337164,
    341914, 346704, 351533, 356400, 361307, 366253, 371238, 376262, 381326, 386429, 391572, 396755, 401978,
    407240, 412543, 417885, 423268, 428690, 434154, 439657, 445201, 450786, 456411, 462077, 467784, 473531,
    479320, 485150, 491021, 496933, 502886, 508881, 514918, 520996, 527115, 533276, 539479, 545724, 552011,
    558340, 564712, 571125, 577580, 584078, 590619, 597202, 603827, 610496, 617207, 623960, 630757, 637597,
    644480, 651406, 658375, 665387, 672443, 679542, 686685, 693872, 701102, 708376, 715694, 723055, 730461,
    737910, 745404, 752942, 760525, 768151, 775822, 783538, 791298, 799103, 806952, 814847, 822786, 830770,
    838799, 846873, 854993, 863157, 871367, 879622, 887923, 896269, 904661, 913099, 921582, 930111, 938686,
    947307, 955973, 964686, 973445, 982251, 991102, 1000000,
];

/// A contrast ratio between two colors as defined by WCAG 2.x: `(L1 + 0.05) / (L2 + 0.05)`
/// for the relative luminances `L1 ≥ L2`, from 1:1 (equal luminance) to 21:1 (black on
/// white). Stored in hundredths, rounded **down**, so a ratio that compares `>=` a
/// requirement really meets it.
///
/// ```
/// use twine_core::Color;
/// use twine_core::color::ContrastRatio;
///
/// let r = Color::BLACK.contrast_ratio(Color::WHITE);
/// assert_eq!(r, ContrastRatio::MAX);
/// assert_eq!(r.to_string(), "21.00:1");
/// assert!(Color::hex(0x757575).contrast_ratio(Color::WHITE) >= ContrastRatio::WCAG_AA);
/// assert!(Color::hex(0x9E9E9E).contrast_ratio(Color::WHITE) < ContrastRatio::WCAG_NON_TEXT);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ContrastRatio(u16);

impl ContrastRatio {
    /// 1:1, two colors of the same luminance.
    pub const MIN: ContrastRatio = ContrastRatio(100);
    /// 21:1, black on white.
    pub const MAX: ContrastRatio = ContrastRatio(2100);
    /// 3:1, WCAG 2.x's minimum for large text and for user-interface components and graphical
    /// objects (success criteria 1.4.3 and 1.4.11).
    pub const WCAG_NON_TEXT: ContrastRatio = ContrastRatio(300);
    /// 4.5:1, WCAG 2.x level AA for normal text (success criterion 1.4.3).
    pub const WCAG_AA: ContrastRatio = ContrastRatio(450);
    /// 7:1, WCAG 2.x level AAA for normal text (success criterion 1.4.6).
    pub const WCAG_AAA: ContrastRatio = ContrastRatio(700);

    /// The ratio `hundredths / 100` (`450` is 4.5:1), clamped to `1:1 ..= 21:1`.
    ///
    /// ```
    /// use twine_core::color::ContrastRatio;
    /// assert_eq!(ContrastRatio::from_hundredths(450), ContrastRatio::WCAG_AA);
    /// assert_eq!(ContrastRatio::from_hundredths(0), ContrastRatio::MIN);
    /// ```
    #[must_use]
    pub const fn from_hundredths(hundredths: u16) -> ContrastRatio {
        if hundredths < Self::MIN.0 {
            Self::MIN
        } else if hundredths > Self::MAX.0 {
            Self::MAX
        } else {
            ContrastRatio(hundredths)
        }
    }

    /// The ratio in hundredths (`450` for 4.5:1).
    #[must_use]
    pub const fn hundredths(self) -> u16 {
        self.0
    }
}

/// `"4.50:1"`.
impl fmt::Display for ContrastRatio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}:1", self.0 / 100, self.0 % 100)
    }
}

impl Color {
    /// The WCAG 2.x relative luminance of the color in parts per million: `0` for black,
    /// `1_000_000` for white (`0.2126·R + 0.7152·G + 0.0722·B` of the linearized sRGB
    /// channels). Integer arithmetic; not meant for rendering (see [`luminance`](Self::luminance)
    /// for a cheap perceived brightness).
    ///
    /// ```
    /// use twine_core::Color;
    /// assert_eq!(Color::BLACK.relative_luminance(), 0);
    /// assert_eq!(Color::WHITE.relative_luminance(), 1_000_000);
    /// assert_eq!(Color::hex(0x808080).relative_luminance(), 215_861);
    /// ```
    #[must_use]
    pub fn relative_luminance(self) -> u32 {
        let lin = |v: u8| u64::from(SRGB_TO_LINEAR_PPM[usize::from(v)]);
        let y = (2126 * lin(self.r) + 7152 * lin(self.g) + 722 * lin(self.b) + 5000) / 10_000;
        // At most 1_000_000 (the coefficients sum to 10_000).
        u32::try_from(y).unwrap_or(1_000_000)
    }

    /// The WCAG 2.x contrast ratio between this color and `other` (symmetric).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_core::color::ContrastRatio;
    ///
    /// let r = Color::hex(0x212121).contrast_ratio(Color::WHITE);
    /// assert_eq!(r.hundredths(), 1610); // 16.10:1
    /// assert_eq!(r, Color::WHITE.contrast_ratio(Color::hex(0x212121)));
    /// assert!(r >= ContrastRatio::WCAG_AAA);
    /// ```
    #[must_use]
    pub fn contrast_ratio(self, other: Color) -> ContrastRatio {
        let (a, b) = (self.relative_luminance(), other.relative_luminance());
        let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
        // (hi + 0.05) / (lo + 0.05) in hundredths, rounded down: at most 2100.
        let r = (hi + 50_000) * 100 / (lo + 50_000);
        ContrastRatio::from_hundredths(u16::try_from(r).unwrap_or(u16::MAX))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use alloc::string::ToString;

    /// The table and the ratios agree with the floating-point WCAG formulas.
    #[test]
    #[allow(
        clippy::float_arithmetic,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn matches_the_wcag_formulas() {
        fn lin(v: u8) -> f64 {
            let c = f64::from(v) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        for v in 0..=255u8 {
            let want = (lin(v) * 1e6).round() as u32;
            assert_eq!(SRGB_TO_LINEAR_PPM[usize::from(v)], want, "{v}");
        }
        let samples = [
            0x00_0000, 0xFF_FFFF, 0x21_96F3, 0xF4_4336, 0x75_7575, 0x28_2B30, 0xBC_8644, 0x10_0C08,
        ];
        for &a in &samples {
            for &b in &samples {
                let (ca, cb) = (Color::hex(a), Color::hex(b));
                let la = 0.2126 * lin(ca.r) + 0.7152 * lin(ca.g) + 0.0722 * lin(ca.b);
                let lb = 0.2126 * lin(cb.r) + 0.7152 * lin(cb.g) + 0.0722 * lin(cb.b);
                let want = (la.max(lb) + 0.05) / (la.min(lb) + 0.05);
                let got = f64::from(ca.contrast_ratio(cb).hundredths()) / 100.0;
                assert!(
                    got <= want + 1e-9 && want - got < 0.011,
                    "{a:06X}/{b:06X}: {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn display_and_clamping() {
        assert_eq!(ContrastRatio::WCAG_AA.to_string(), "4.50:1");
        assert_eq!(ContrastRatio::from_hundredths(5000), ContrastRatio::MAX);
        assert_eq!(Color::RED.contrast_ratio(Color::RED), ContrastRatio::MIN);
    }
}
