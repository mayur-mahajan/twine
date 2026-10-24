//! Text gamma: how a glyph's stored coverage becomes opacity, chosen per text run by polarity.
//!
//! Blending anti-aliased coverage linearly in the panel's (sRGB-like) color space makes
//! partially covered pixels look lighter than their coverage: dark text on a light background
//! keeps its weight, light text on a dark background reads thin. A [`TextGamma`] holds one
//! coverage → opacity table per [`Polarity`]; [`draw_text`](crate::draw_text) picks the
//! polarity once per run from the text and background colors ([`Polarity::of`]) and the glyph
//! decoder maps each stored value through the table **instead of** the fixed LVGL expansion
//! (4 bpp `v × 17`, 2 bpp `v × 85`). The per-pixel work is the same (one table lookup instead
//! of one multiplication); nothing is computed per run beyond a luminance comparison.
//!
//! Each table is `round(255 · (v / max)^(1 / γ))`: identity at both ends (0 stays transparent,
//! full coverage stays opaque), monotonic, and heavier in between for `γ > 1`. A bitmap font
//! carries its tables ([`BitmapFont::gamma`](crate::BitmapFont::gamma)), so the curve
//! strengths are tuned per font when the font is generated (`twine-build`'s `Font::gamma`);
//! the built-in fonts use [`DEFAULT`] (γ = 1.2 dark-on-light, 1.8 light-on-dark), and
//! [`LINEAR`] reproduces LVGL's mapping exactly.
//!
//! | Depth | Mapping |
//! |-------|---------|
//! | 1 bpp | `0, 255` (no levels in between) |
//! | 2 bpp | [`GammaRamp::table2`] |
//! | 4 bpp | [`GammaRamp::table4`] |
//! | 8 bpp | identity (8-bit coverage is used as stored; run-time TrueType glyphs likewise) |
//!
//! ```
//! use twine_core::Color;
//! use twine_text::{Polarity, gamma};
//!
//! let p = Polarity::of(Color::WHITE, Some(Color::hex(0x282B30)));
//! assert_eq!(p, Polarity::LightOnDark);
//! let ramp = gamma::DEFAULT.ramp(p);
//! assert_eq!((ramp.table4()[0], ramp.table4()[15]), (0, 255)); // identity at the ends
//! assert!(ramp.table4()[8] > 8 * 17); // heavier than LVGL's linear mapping
//! assert_eq!(gamma::LINEAR.ramp(p).table4()[8], 8 * 17);
//! ```

use twine_core::Color;

use crate::decode::Bpp;

/// Which way a text run's contrast goes: the input of the gamma choice (see the
/// [module docs](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Polarity {
    /// Text darker than its background (the usual light theme).
    #[default]
    DarkOnLight,
    /// Text lighter than its background (dark themes, text on filled buttons).
    LightOnDark,
}

impl Polarity {
    /// The polarity of `text` drawn over `background`: [`LightOnDark`](Self::LightOnDark) when
    /// the text is brighter ([`Color::luminance`]). Without a known background the text alone
    /// decides: light text (luminance ≥ 128) is taken to be drawn on a dark background.
    ///
    /// Never panics; O(1).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_text::Polarity;
    ///
    /// assert_eq!(Polarity::of(Color::hex(0x212121), Some(Color::WHITE)), Polarity::DarkOnLight);
    /// assert_eq!(Polarity::of(Color::hex(0x2196F3), Some(Color::hex(0x15171A))), Polarity::LightOnDark);
    /// assert_eq!(Polarity::of(Color::hex(0xBDBDBD), None), Polarity::LightOnDark);
    /// ```
    #[inline]
    #[must_use]
    pub const fn of(text: Color, background: Option<Color>) -> Polarity {
        let lt = text.luminance();
        let light = match background {
            Some(bg) => lt > bg.luminance(),
            None => lt >= 128,
        };
        if light {
            Polarity::LightOnDark
        } else {
            Polarity::DarkOnLight
        }
    }
}

/// Coverage → opacity tables of one curve, for 2 and 4 bpp glyphs (1 bpp maps to `0, 255`,
/// 8 bpp is used as stored).
///
/// Built from the 16-entry 4 bpp table; the 2 bpp table takes its entries at the same
/// coverage (`v / 3 = 5v / 15`), so both depths follow one curve.
///
/// ```
/// use twine_text::GammaRamp;
///
/// let r = GammaRamp::new([0, 30, 50, 68, 85, 100, 115, 129, 143, 157, 170, 184, 197, 210, 222, 255]);
/// assert_eq!(r.table2(), &[0, 100, 170, 255]);
/// assert_eq!(GammaRamp::LINEAR.table4()[1], 17);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GammaRamp {
    a2: [u8; 4],
    a4: [u8; 16],
}

impl GammaRamp {
    /// LVGL's linear mapping: 4 bpp `v × 17`, 2 bpp `v × 85`.
    pub const LINEAR: GammaRamp = GammaRamp::new([
        0, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255,
    ]);

    /// The ramp of the 4 bpp table `a4` (opacity of each coverage level `0..=15`); the
    /// 2 bpp table is `[a4[0], a4[5], a4[10], a4[15]]`.
    ///
    /// Any table is accepted; a sensible one starts at 0, ends at 255 and never decreases
    /// (the generator's tables do: `twine-build` tests it).
    #[must_use]
    pub const fn new(a4: [u8; 16]) -> GammaRamp {
        GammaRamp {
            a2: [a4[0], a4[5], a4[10], a4[15]],
            a4,
        }
    }

    /// The 4 bpp table: opacity of coverage level `v` (`0..=15`).
    #[must_use]
    pub const fn table4(&self) -> &[u8; 16] {
        &self.a4
    }

    /// The 2 bpp table: opacity of coverage level `v` (`0..=3`).
    #[must_use]
    pub const fn table2(&self) -> &[u8; 4] {
        &self.a2
    }

    /// Maps a `depth`-bit coverage value to opacity (only the value's low `depth` bits count).
    ///
    /// Never panics: the index is masked to the table.
    #[inline(always)]
    #[must_use]
    pub(crate) fn a8(&self, v: u8, depth: Bpp) -> u8 {
        match depth {
            Bpp::One => {
                if v & 1 == 0 {
                    0
                } else {
                    255
                }
            }
            // The masks keep the index inside the tables (the `0` is never taken).
            Bpp::Two => self.a2.get(usize::from(v & 3)).copied().unwrap_or(0),
            Bpp::Four => self.a4.get(usize::from(v & 15)).copied().unwrap_or(0),
            Bpp::Eight => v,
        }
    }
}

/// The coverage → opacity curves of a bitmap font, one per [`Polarity`] (see the
/// [module docs](self)).
///
/// ```
/// use twine_text::{GammaRamp, Polarity, TextGamma};
///
/// static MY_GAMMA: TextGamma = TextGamma {
///     dark_on_light: GammaRamp::LINEAR,
///     light_on_dark: GammaRamp::new([0, 40, 64, 84, 101, 117, 132, 146, 159, 172, 184, 196, 208, 220, 232, 255]),
/// };
/// assert_eq!(MY_GAMMA.ramp(Polarity::DarkOnLight), &GammaRamp::LINEAR);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextGamma {
    /// The curve for text darker than its background.
    pub dark_on_light: GammaRamp,
    /// The curve for text lighter than its background.
    pub light_on_dark: GammaRamp,
}

impl TextGamma {
    /// The curve of `polarity`.
    #[inline]
    #[must_use]
    pub const fn ramp(&self, polarity: Polarity) -> &GammaRamp {
        match polarity {
            Polarity::DarkOnLight => &self.dark_on_light,
            Polarity::LightOnDark => &self.light_on_dark,
        }
    }
}

/// LVGL's linear mapping for both polarities (fonts generated without text gamma).
pub static LINEAR: TextGamma = TextGamma {
    dark_on_light: GammaRamp::LINEAR,
    light_on_dark: GammaRamp::LINEAR,
};

/// The default curves of generated fonts: γ = 1.2 dark-on-light (already heavy with linear
/// blending: a mild lift) and γ = 1.8 light-on-dark (thin without it: a strong lift). The
/// built-in fonts use these.
pub static DEFAULT: TextGamma = TextGamma {
    // round(255 · (v / 15)^(1 / 1.2)); twine-build checks these against its generator.
    dark_on_light: GammaRamp::new([
        0, 27, 48, 67, 85, 102, 119, 135, 151, 167, 182, 197, 212, 226, 241, 255,
    ]),
    // round(255 · (v / 15)^(1 / 1.8)).
    light_on_dark: GammaRamp::new([
        0, 57, 83, 104, 122, 139, 153, 167, 180, 192, 204, 215, 225, 236, 245, 255,
    ]),
};
