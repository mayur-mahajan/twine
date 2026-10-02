//! Design elements: named, typed values (colors, spacings, radii, opacities, fonts) that a
//! style refers to by name and the **theme** supplies, resolved when the style is resolved.
//!
//! A style that says `bg_color: design::SURFACE` instead of `bg_color: Color::WHITE` follows
//! the theme of its display: switching the theme's [`ThemeMode`] (light → dark) swaps the
//! theme's [`ElementTable`] and every node that uses a design element is re-resolved — no view
//! is rebuilt, no binding re-runs, nothing is allocated after the tables were built once.
//!
//! ```
//! use twine_core::Color;
//! use twine_style::design::{self, ElementTable};
//! use twine_style::{PropId, StyleBuf, StyleValue};
//!
//! // A style that takes its colors from the theme.
//! let card = StyleBuf::new().bg(design::SURFACE).text_color(design::ON_SURFACE).padding(design::SPACE_M);
//! assert!(matches!(card.get(PropId::BgColor), Some(StyleValue::Element(_))));
//!
//! // A theme's values for one mode.
//! let light = ElementTable::new().with(design::SURFACE, Color::WHITE);
//! assert_eq!(light.get(design::SURFACE), Some(Color::WHITE));
//! assert_eq!(light.get(design::PRIMARY), None); // not defined by this table
//! ```
//!
//! # Kinds
//!
//! | Element type | Value | Accepted by |
//! |--------------|-------|-------------|
//! | [`ColorElement`] | [`Color`] | every color property (`bg_color`, `text_color`, `border_color`, …, the `bg`, `border`, `outline` shorthands) |
//! | [`LengthElement`] | [`Length`] (`Px` or `Dp`) | every length property (`width`, `x`, `padding_*`, `margin_*`, `row_gap`, `border_width`, …) |
//! | [`RadiusElement`] | [`Radius`] | `radius` |
//! | [`OpacityElement`] | [`Opa`] | every opacity property (`bg_opacity`, `opacity`, …) |
//! | [`FontElement`] | `&'static Font` | `font` |
//!
//! A property of type `Color` stores a [`ColorValue`] (= [`DesignValue<Color>`]): a fixed
//! color or a color element. `Color` and `ColorElement` both convert into it, so the
//! `StyleBuf` builders, the `style!` macro and the `twine-view` modifiers take either.
//!
//! # The standard elements
//!
//! Every built-in theme (`DefaultTheme`, `MonoTheme`, `SimpleTheme`) defines all of them, in
//! every mode it supports. Use them in application styles instead of literal colors so the
//! application follows the theme.
//!
//! | Element | Meaning |
//! |---------|---------|
//! | [`BACKGROUND`] | screen background |
//! | [`SURFACE`] | cards, containers, inputs, list items |
//! | [`ON_SURFACE`] | text and icons on `BACKGROUND` and `SURFACE` |
//! | [`ON_SURFACE_MUTED`] | secondary text: hints, captions, units |
//! | [`PLACEHOLDER`] | the placeholder text of empty inputs |
//! | [`NEUTRAL`] | mid-tone neutral: lines inside containers, pressed-row overlays |
//! | [`SURFACE_VARIANT`] | neutral buttons, slider and arc tracks |
//! | [`OUTLINE`] | borders and dividers |
//! | [`PRIMARY`] / [`ON_PRIMARY`] | accent (buttons, indicators, knobs) / text on it |
//! | [`SECONDARY`] / [`ON_SECONDARY`] | second accent (checked buttons, edit outlines) / text on it |
//! | [`DANGER`], [`WARNING`], [`OK`] | status colors (alarm, caution, normal) |
//! | [`DISABLED`] | the overlay of disabled widgets |
//! | [`FOCUS_RING`] | the outline of the focused widget |
//! | [`SCROLLBAR`] | scrollbars |
//! | [`SHADOW`] | shadows |
//! | [`SCRIM`] | the backdrop behind modal dialogs |
//! | [`SPACE_XS`], [`SPACE_S`], [`SPACE_M`], [`SPACE_L`] | spacing scale (padding, gaps, margins), ascending |
//! | [`RADIUS_S`], [`RADIUS_M`], [`RADIUS_L`], [`RADIUS_FULL`] | corner radius scale, ascending; `RADIUS_FULL` is a circle / pill |
//! | [`FOCUS_RING_OPACITY`], [`SHADOW_OPACITY`], [`DISABLED_OPACITY`], [`SCRIM_OPACITY`] | the opacities of the focus ring, shadows, the disabled overlay and the backdrop |
//! | [`FONT_SMALL`], [`FONT_BODY`], [`FONT_LARGE`] | font scale |
//!
//! # Theme modes and contrast requirements
//!
//! A theme gives the elements one value per [`ThemeMode`] it supports: [`ThemeMode::Light`],
//! [`ThemeMode::Dark`], [`ThemeMode::Night`] (dimmed, low luminance, reduced blue — cockpits
//! and vehicles at night) and [`ThemeMode::HighContrast`] (maximum legibility). Each mode
//! specifies minimum WCAG contrast ratios between standard foreground and background
//! elements ([`ContrastPair`], [`ThemeMode::min_contrast`]); [`ElementTable::check_contrast`]
//! verifies a table against them (with [`Color::contrast_ratio`], integer arithmetic). The
//! built-in themes meet them (with their default colors) in every mode they support, and an
//! application that overrides standard elements can check its table the same way.
//!
//! # Application-defined elements
//!
//! [`Element::custom`] names an element of the application (a brand color, a gauge zone
//! color, …); the application gives it a value per mode in the theme (the built-in themes take
//! it with `.element(..)` / `.element_in(mode, ..)`):
//!
//! ```
//! use twine_core::Color;
//! use twine_style::design::{ColorElement, ElementTable};
//!
//! /// The color of the "boost" zone of a gauge.
//! pub const BOOST: ColorElement = ColorElement::custom(0);
//! let dark = ElementTable::new().with(BOOST, Color::hex(0xFF8A65));
//! assert_eq!(dark.get(BOOST), Some(Color::hex(0xFF8A65)));
//! assert!(BOOST.is_custom());
//! ```
//!
//! # Resolution and fallback
//!
//! Design elements are resolved by [`resolve`](crate::resolve) at its single exit point,
//! through [`StyleSource::design_value`](crate::StyleSource::design_value) (the engine: the
//! element table of the node's display, a direct read with one display). A fixed value costs
//! nothing extra (one discriminant test, shared with the `Dp` conversion); an element costs one
//! out-of-line call and one bounds-checked table read. An element the table does not define
//! (no theme, a theme without element tables, or an application element the theme was not
//! given) resolves to the **property's default** (as if no style set it, e.g. a white
//! `bg_color`, a black `text_color`, the display's default font), and the first such lookup of
//! each kind logs a warning (`"twine::style"`). It never panics.

use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use twine_core::color::ContrastRatio;
use twine_core::{Color, Opa};
use twine_text::Font;

use crate::value::{PropValue, StyleValue};
use crate::value_types::{Length, Radius};

/// The mode of a theme: which of its element tables is active. Switching the mode
/// (`Engine::set_theme_mode` in `twine-engine`, `ThemeHandle::set_mode` in `twine-view`)
/// swaps the table, so everything styled with [design elements](self) changes at once.
///
/// The built-in `DefaultTheme` and `MonoTheme` support all four modes, `SimpleTheme`
/// [`Light`](Self::Light) and [`HighContrast`](Self::HighContrast). Every mode has documented
/// minimum contrast ratios between the standard foreground and background elements
/// ([`min_contrast`](Self::min_contrast), checked with [`ElementTable::check_contrast`]); the
/// built-in themes meet them, with their default colors, in every mode they support (tested;
/// an application that changes `PRIMARY` & co. checks its table the same way).
///
/// ```
/// use twine_style::ThemeMode;
/// use twine_style::design::ContrastPair;
///
/// assert_eq!(ThemeMode::default(), ThemeMode::Light);
/// assert_eq!(ThemeMode::ALL.len(), 4);
/// // High contrast: text at WCAG AAA level.
/// assert_eq!(
///     ThemeMode::HighContrast.min_contrast(ContrastPair::OnSurface).unwrap().to_string(),
///     "7.00:1"
/// );
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThemeMode {
    /// Light screens and cards, dark text: daylight, offices, indoor use.
    #[default]
    Light,
    /// Dark screens and cards, light text: dim rooms, less emitted light on emissive panels.
    Dark,
    /// Night operation — cockpits, vehicle dashboards, control rooms and bedside equipment
    /// at night: a dark, **dimmed, low-luminance** palette with **reduced blue** (warm amber
    /// and red tones), which preserves the operator's dark adaptation and limits glare and
    /// reflections (e.g. in a windshield or canopy). The built-in color themes keep every
    /// color at most 30 % relative luminance (white is 100 %) and its blue channel at most
    /// half its red channel; text still meets 4.5:1. Dimming the panel itself is the
    /// backlight's job (the application's); a monochrome theme can only invert (white on
    /// black, like [`Dark`](Self::Dark)).
    Night,
    /// Maximum legibility — bright sunlight, low vision, safety-relevant readouts: black and
    /// white with saturated accents, opaque focus rings, no shadows. Text meets 7:1 (WCAG
    /// AAA), the focus ring and disabled content 4.5:1 (see [`min_contrast`](Self::min_contrast)).
    HighContrast,
}

impl ThemeMode {
    /// Every mode, in the order a mode switch cycles through them.
    pub const ALL: [ThemeMode; 4] = [
        ThemeMode::Light,
        ThemeMode::Dark,
        ThemeMode::Night,
        ThemeMode::HighContrast,
    ];

    /// The minimum WCAG contrast ratio `pair` must reach in this mode (`None`: no requirement
    /// in this mode). The requirements, as hundredths-exact ratios:
    ///
    /// | Pair | `Light` | `Dark` | `Night` | `HighContrast` |
    /// |------|---------|--------|---------|----------------|
    /// | [`OnSurface`](ContrastPair::OnSurface), [`OnBackground`](ContrastPair::OnBackground), [`OnSurfaceVariant`](ContrastPair::OnSurfaceVariant), [`OnSurfaceMuted`](ContrastPair::OnSurfaceMuted) (text) | 4.5:1 | 4.5:1 | 4.5:1 | 7:1 |
    /// | [`OnPrimary`](ContrastPair::OnPrimary), [`OnSecondary`](ContrastPair::OnSecondary) (text on accents) | 3:1 | 3:1 | 4.5:1 | 7:1 |
    /// | [`FocusRing`](ContrastPair::FocusRing) | — | — | 3:1 | 4.5:1 |
    /// | [`DisabledContent`](ContrastPair::DisabledContent) | — | — | 3:1 | 4.5:1 |
    ///
    /// 4.5:1 and 7:1 are WCAG 2.x levels AA and AAA for normal text, 3:1 its minimum for
    /// large text and user-interface components. `Light` and `Dark` keep LVGL's look (white
    /// on its blue and red accents is 3.1:1 and 3.7:1, its focus ring is 50 % translucent), so
    /// their accent text is held to 3:1 and focus ring and disabled content are not
    /// specified; `Night` and `HighContrast` specify every pair.
    ///
    /// ```
    /// use twine_core::color::ContrastRatio;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::ContrastPair;
    ///
    /// assert_eq!(ThemeMode::Light.min_contrast(ContrastPair::OnSurface), Some(ContrastRatio::WCAG_AA));
    /// assert_eq!(ThemeMode::Light.min_contrast(ContrastPair::FocusRing), None);
    /// assert_eq!(ThemeMode::Night.min_contrast(ContrastPair::FocusRing), Some(ContrastRatio::WCAG_NON_TEXT));
    /// ```
    #[must_use]
    pub const fn min_contrast(self, pair: ContrastPair) -> Option<ContrastRatio> {
        use ContrastPair as P;
        use ThemeMode as M;
        let text = matches!(
            pair,
            P::OnSurface | P::OnBackground | P::OnSurfaceVariant | P::OnSurfaceMuted
        );
        let accent = matches!(pair, P::OnPrimary | P::OnSecondary);
        Some(match self {
            M::Light | M::Dark if text => ContrastRatio::WCAG_AA,
            M::Light | M::Dark if accent => ContrastRatio::WCAG_NON_TEXT,
            M::Light | M::Dark => return None,
            M::Night if text || accent => ContrastRatio::WCAG_AA,
            M::Night => ContrastRatio::WCAG_NON_TEXT,
            M::HighContrast if text || accent => ContrastRatio::WCAG_AAA,
            M::HighContrast => ContrastRatio::WCAG_AA,
        })
    }

    /// The mode after this one in `modes` (wrapping around): what a "switch mode" button or
    /// hotkey selects next among the modes a theme supports (`ThemeHook::modes`). The first
    /// of `modes` if this mode is not in it; this mode if `modes` is empty.
    ///
    /// ```
    /// use twine_style::ThemeMode;
    ///
    /// let supported = [ThemeMode::Light, ThemeMode::HighContrast];
    /// assert_eq!(ThemeMode::Light.next_in(&supported), ThemeMode::HighContrast);
    /// assert_eq!(ThemeMode::HighContrast.next_in(&supported), ThemeMode::Light);
    /// assert_eq!(ThemeMode::Night.next_in(&supported), ThemeMode::Light);
    /// assert_eq!(ThemeMode::Dark.next_in(&[]), ThemeMode::Dark);
    /// assert_eq!(ThemeMode::Dark.next_in(&ThemeMode::ALL), ThemeMode::Night);
    /// ```
    #[must_use]
    pub fn next_in(self, modes: &[ThemeMode]) -> ThemeMode {
        match modes.iter().position(|&m| m == self) {
            Some(i) => modes[(i + 1) % modes.len()],
            None => modes.first().copied().unwrap_or(self),
        }
    }
}

/// The value kind of a design element (one table per kind in an [`ElementTable`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum ElementKind {
    /// A [`Color`] ([`ColorElement`]).
    Color,
    /// A [`Length`] ([`LengthElement`]).
    Length,
    /// A [`Radius`] ([`RadiusElement`]).
    Radius,
    /// An [`Opa`] ([`OpacityElement`]).
    Opacity,
    /// A `&'static Font` ([`FontElement`]).
    Font,
}

impl ElementKind {
    /// Number of kinds.
    pub const COUNT: usize = 5;

    /// A short name for logs (`"color"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ElementKind::Color => "color",
            ElementKind::Length => "length",
            ElementKind::Radius => "radius",
            ElementKind::Opacity => "opacity",
            ElementKind::Font => "font",
        }
    }
}

/// A design element without its value type: what a style stores and the resolver looks up
/// ([`StyleValue::Element`],
/// [`StyleSource::design_value`](crate::StyleSource::design_value)). 4 bytes, `Copy`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ElementRef {
    kind: ElementKind,
    id: u16,
}

impl ElementRef {
    /// The element's kind.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> ElementKind {
        self.kind
    }

    /// The element's id within its kind (standard elements first, then the application's).
    #[inline]
    #[must_use]
    pub const fn id(self) -> u16 {
        self.id
    }
}

impl fmt::Display for ElementRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} element #{}", self.kind.name(), self.id)
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for twine_core::Color {}
    impl Sealed for crate::Length {}
    impl Sealed for crate::Radius {}
    impl Sealed for twine_core::Opa {}
    impl Sealed for &'static twine_text::Font {}
}

/// A value type that design elements can have: [`Color`], [`Length`], [`Radius`], [`Opa`] and
/// `&'static Font` (sealed).
pub trait ElementType: PropValue + Copy + 'static + sealed::Sealed {
    /// The kind of the elements of this type.
    const KIND: ElementKind;
    /// Number of standard elements of this type (ids `0..STANDARD`; application elements
    /// follow).
    const STANDARD: u16;
    /// The table of this type in `t`.
    #[doc(hidden)]
    fn slots(t: &ElementTable) -> &Vec<Option<Self>>;
    /// The table of this type in `t`, mutable.
    #[doc(hidden)]
    fn slots_mut(t: &mut ElementTable) -> &mut Vec<Option<Self>>;
}

macro_rules! element_type {
    ($t:ty, $kind:ident, $field:ident, $standard:expr) => {
        impl ElementType for $t {
            const KIND: ElementKind = ElementKind::$kind;
            const STANDARD: u16 = $standard;
            #[inline]
            fn slots(t: &ElementTable) -> &Vec<Option<Self>> {
                &t.$field
            }
            #[inline]
            fn slots_mut(t: &mut ElementTable) -> &mut Vec<Option<Self>> {
                &mut t.$field
            }
        }
    };
}

element_type!(Color, Color, colors, 20);
element_type!(Length, Length, lengths, 4);
element_type!(Radius, Radius, radii, 4);
element_type!(Opa, Opacity, opacities, 4);
element_type!(&'static Font, Font, fonts, 3);

/// A design element whose value is a `T`: a typed name the theme gives a value (see the
/// [module documentation](self)). `Copy`, 2 bytes.
///
/// The standard elements are the constants of [`design`](self) ([`SURFACE`], [`SPACE_M`], …);
/// application elements are made with [`custom`](Self::custom).
pub struct Element<T> {
    id: u16,
    _t: PhantomData<fn() -> T>,
}

/// A design element of color type ([`SURFACE`], [`PRIMARY`], …).
pub type ColorElement = Element<Color>;
/// A design element of length type ([`SPACE_M`], …).
pub type LengthElement = Element<Length>;
/// A design element of radius type ([`RADIUS_M`], …).
pub type RadiusElement = Element<Radius>;
/// A design element of opacity type ([`SHADOW_OPACITY`], …).
pub type OpacityElement = Element<Opa>;
/// A design element of font type ([`FONT_BODY`], …).
pub type FontElement = Element<&'static Font>;

impl<T> Clone for Element<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Element<T> {}

impl<T> PartialEq for Element<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for Element<T> {}

impl<T> core::hash::Hash for Element<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: ElementType> fmt::Debug for Element<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Element({})", T::KIND.name(), self.id)
    }
}

impl<T: ElementType> Element<T> {
    /// Standard element `id`.
    const fn standard(id: u16) -> Self {
        Self { id, _t: PhantomData }
    }

    /// The element with id `id` (as stored in an [`ElementRef`]).
    #[inline]
    const fn from_id(id: u16) -> Self {
        Self { id, _t: PhantomData }
    }

    /// Application element number `n` of this type (`n` counts from 0 per type; the ids
    /// follow the standard elements). Give it a value in each mode of the theme; until then it
    /// resolves to the property's default (see the [module documentation](self)).
    ///
    /// The id is `T::STANDARD + n`, **saturating** at `u16::MAX`: a huge `n` (near
    /// `u16::MAX`) does not wrap into a standard element, but all such `n` map to the same
    /// last id. Number custom elements densely from 0 (an [`ElementTable`] allocates one slot
    /// per id up to the highest one it defines). `const`; never panics.
    ///
    /// ```
    /// use twine_style::design::{self, ColorElement, LengthElement};
    ///
    /// const BRAND: ColorElement = ColorElement::custom(0);
    /// const GAUGE_WIDTH: LengthElement = LengthElement::custom(0);
    /// assert_ne!(BRAND, design::BACKGROUND);
    /// assert!(BRAND.is_custom() && GAUGE_WIDTH.is_custom());
    /// ```
    #[must_use]
    pub const fn custom(n: u16) -> Self {
        Self::from_id(T::STANDARD.saturating_add(n))
    }

    /// Whether this is an application element ([`custom`](Self::custom)).
    #[must_use]
    pub const fn is_custom(self) -> bool {
        self.id >= T::STANDARD
    }

    /// The id within the kind (standard elements `0..T::STANDARD`, then the application's).
    #[must_use]
    pub const fn id(self) -> u16 {
        self.id
    }

    /// The element without its value type, as styles store it.
    #[inline]
    #[must_use]
    pub const fn erase(self) -> ElementRef {
        ElementRef {
            kind: T::KIND,
            id: self.id,
        }
    }
}

/// The value of a style property that a theme can supply: a fixed value or a
/// [design element](self). Every color, length, radius, opacity and font property stores one
/// (as [`ColorValue`], [`LengthValue`], [`RadiusValue`], [`OpacityValue`], [`FontValue`]), so
/// it can be set to either:
///
/// ```
/// use twine_core::Color;
/// use twine_style::design::{self, ColorValue, LengthValue};
/// use twine_style::{Length, StyleBuf};
///
/// let fixed: ColorValue = Color::RED.into();
/// let themed: ColorValue = design::PRIMARY.into();
/// assert_eq!(fixed, ColorValue::Fixed(Color::RED));
/// assert_eq!(themed, ColorValue::Element(design::PRIMARY));
/// assert_eq!(LengthValue::from(8), LengthValue::Fixed(Length::Px(8))); // integers are pixels
/// let _s = StyleBuf::new().bg_color(Color::RED).border_color(design::OUTLINE);
/// ```
///
/// Size: the fixed value's size or 4 bytes, whichever is larger (a `Length` niche holds the
/// element), so a [`StyleProp`](crate::StyleProp) stays 12 bytes on 32-bit targets.
#[derive(Clone, Copy)]
pub enum DesignValue<T> {
    /// A fixed value.
    Fixed(T),
    /// The value the theme gives the element.
    Element(Element<T>),
}

impl<T: ElementType + fmt::Debug> fmt::Debug for DesignValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DesignValue::Fixed(v) => f.debug_tuple("Fixed").field(v).finish(),
            DesignValue::Element(e) => f.debug_tuple("Element").field(e).finish(),
        }
    }
}

impl<T: PartialEq> PartialEq for DesignValue<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Fixed(a), Self::Fixed(b)) => a == b,
            (Self::Element(a), Self::Element(b)) => a == b,
            _ => false,
        }
    }
}

impl<T: Eq> Eq for DesignValue<T> {}

/// A color property's value: a [`Color`] or a [`ColorElement`].
pub type ColorValue = DesignValue<Color>;
/// A length property's value: a [`Length`] (integers are pixels) or a [`LengthElement`].
pub type LengthValue = DesignValue<Length>;
/// The `radius` property's value: a [`Radius`] (integers are pixels) or a [`RadiusElement`].
pub type RadiusValue = DesignValue<Radius>;
/// An opacity property's value: an [`Opa`] or an [`OpacityElement`].
pub type OpacityValue = DesignValue<Opa>;
/// The `font` property's value: a `&'static Font` or a [`FontElement`].
pub type FontValue = DesignValue<&'static Font>;

impl<T> From<T> for DesignValue<T> {
    #[inline]
    fn from(v: T) -> Self {
        DesignValue::Fixed(v)
    }
}

impl<T> From<Element<T>> for DesignValue<T> {
    #[inline]
    fn from(e: Element<T>) -> Self {
        DesignValue::Element(e)
    }
}

/// Integers are pixels (`Length::Px`).
impl From<i32> for LengthValue {
    #[inline]
    fn from(v: i32) -> Self {
        DesignValue::Fixed(Length::Px(v))
    }
}

/// Integers are pixels (`Radius::Px`).
impl From<i32> for RadiusValue {
    #[inline]
    fn from(v: i32) -> Self {
        DesignValue::Fixed(Radius::Px(v))
    }
}

impl<T> DesignValue<T> {
    /// The fixed value (`None` for an element).
    #[inline]
    #[must_use]
    pub fn fixed(self) -> Option<T> {
        match self {
            DesignValue::Fixed(v) => Some(v),
            DesignValue::Element(_) => None,
        }
    }
}

/// A design value is stored as its fixed value's [`StyleValue`], or as
/// [`StyleValue::Element`] until it is resolved.
impl<T: ElementType> PropValue for DesignValue<T> {
    #[inline]
    fn to_value(&self) -> StyleValue {
        match *self {
            DesignValue::Fixed(v) => v.into_value(),
            DesignValue::Element(e) => StyleValue::Element(e.erase()),
        }
    }

    #[inline]
    fn from_value(v: StyleValue) -> Option<Self> {
        match v {
            StyleValue::Element(r) if r.kind == T::KIND => Some(DesignValue::Element(Element::from_id(r.id))),
            StyleValue::Element(_) => None,
            v => T::from_value(v).map(DesignValue::Fixed),
        }
    }
}

/// The values a theme gives the design elements in one [`ThemeMode`]: one table per element
/// kind, indexed by element id. Built once (allocates while it is filled); reading is a
/// bounds-checked index.
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::design::{self, ElementTable};
/// use twine_style::{Length, StyleValue};
///
/// let t = ElementTable::new()
///     .with(design::PRIMARY, Color::hex(0x2196F3))
///     .with(design::SPACE_M, Length::dp(16))
///     .with(design::SHADOW_OPACITY, Opa::P50);
/// assert_eq!(t.get(design::SPACE_M), Some(Length::dp(16)));
/// assert_eq!(t.value(design::PRIMARY.erase()), Some(StyleValue::Color(Color::hex(0x2196F3))));
/// assert_eq!(t.missing_standard(), Some(design::BACKGROUND.erase()));
/// ```
#[derive(Clone, Debug, Default)]
pub struct ElementTable {
    colors: Vec<Option<Color>>,
    lengths: Vec<Option<Length>>,
    radii: Vec<Option<Radius>>,
    opacities: Vec<Option<Opa>>,
    fonts: Vec<Option<&'static Font>>,
}

impl ElementTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the value of `element` and returns the table (builder style).
    #[must_use]
    pub fn with<T: ElementType>(mut self, element: Element<T>, value: T) -> Self {
        self.set(element, value);
        self
    }

    /// Sets the value of `element` (grows the table to its id: an allocation while the table
    /// is built, never while it is read).
    ///
    /// Memory: the table of `element`'s kind grows to `element`'s id + 1 slots (one `Option`
    /// per id below it, defined or not), so number application elements densely from
    /// [`custom(0)`](Element::custom) up: `custom(60_000)` alone allocates about 60 000 slots.
    /// Never panics (an allocation failure aborts like any `Vec` growth).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::design::{self, ColorElement, ElementTable};
    ///
    /// const BRAND: ColorElement = ColorElement::custom(0);
    /// let mut t = ElementTable::new();
    /// t.set(design::PRIMARY, Color::BLUE);
    /// t.set(BRAND, Color::hex(0x00FF_8800));
    /// assert_eq!(t.get(BRAND), Some(Color::hex(0x00FF_8800)));
    /// ```
    pub fn set<T: ElementType>(&mut self, element: Element<T>, value: T) {
        let slots = T::slots_mut(self);
        let i = usize::from(element.id);
        if slots.len() <= i {
            slots.resize(i + 1, None);
        }
        slots[i] = Some(value);
    }

    /// The value of `element` (`None` if the table does not define it).
    #[inline]
    #[must_use]
    pub fn get<T: ElementType>(&self, element: Element<T>) -> Option<T> {
        T::slots(self).get(usize::from(element.id)).copied().flatten()
    }

    /// The value of `element` as a [`StyleValue`] (what the resolver substitutes); `None` if
    /// the table does not define it.
    #[must_use]
    pub fn value(&self, element: ElementRef) -> Option<StyleValue> {
        let i = usize::from(element.id);
        match element.kind {
            ElementKind::Color => self.colors.get(i).copied().flatten().map(StyleValue::Color),
            ElementKind::Length => self.lengths.get(i).copied().flatten().map(StyleValue::Length),
            ElementKind::Radius => self.radii.get(i).copied().flatten().map(PropValue::into_value),
            ElementKind::Opacity => self.opacities.get(i).copied().flatten().map(StyleValue::Opa),
            ElementKind::Font => self.fonts.get(i).copied().flatten().map(StyleValue::Font),
        }
    }

    /// Copies every value `other` defines into this table (replacing this table's values of
    /// those elements): layers application elements or overrides over a theme's table.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::design::{self, ElementTable};
    ///
    /// let mut theme = ElementTable::new().with(design::PRIMARY, Color::BLUE).with(design::OK, Color::GREEN);
    /// theme.overlay(&ElementTable::new().with(design::PRIMARY, Color::hex(0x6200EE)));
    /// assert_eq!(theme.get(design::PRIMARY), Some(Color::hex(0x6200EE)));
    /// assert_eq!(theme.get(design::OK), Some(Color::GREEN));
    /// ```
    pub fn overlay(&mut self, other: &ElementTable) {
        fn copy<T: Copy>(dst: &mut Vec<Option<T>>, src: &[Option<T>]) {
            if dst.len() < src.len() {
                dst.resize(src.len(), None);
            }
            for (d, s) in dst.iter_mut().zip(src) {
                if s.is_some() {
                    *d = *s;
                }
            }
        }
        copy(&mut self.colors, &other.colors);
        copy(&mut self.lengths, &other.lengths);
        copy(&mut self.radii, &other.radii);
        copy(&mut self.opacities, &other.opacities);
        copy(&mut self.fonts, &other.fonts);
    }

    /// The first standard element the table does not define (`None` if it defines all of
    /// them): a check for theme authors.
    #[must_use]
    pub fn missing_standard(&self) -> Option<ElementRef> {
        fn first<T: ElementType>(t: &ElementTable) -> Option<ElementRef> {
            (0..T::STANDARD)
                .map(Element::<T>::from_id)
                .find(|e| t.get(*e).is_none())
                .map(Element::erase)
        }
        first::<Color>(self)
            .or_else(|| first::<Length>(self))
            .or_else(|| first::<Radius>(self))
            .or_else(|| first::<Opa>(self))
            .or_else(|| first::<&'static Font>(self))
    }
}

// ---- Contrast requirements ----------------------------------------------------------------

/// A foreground / background pair of standard elements whose legibility a [`ThemeMode`]
/// specifies ([`ThemeMode::min_contrast`]). [`colors`](Self::colors) gives the two colors as
/// they are displayed: translucent elements (the focus ring, the disabled overlay) are blended
/// with their opacity element first.
///
/// ```
/// use twine_core::Color;
/// use twine_style::design::{self, ContrastPair, ElementTable};
///
/// let t = ElementTable::new().with(design::ON_SURFACE, Color::hex(0x212121)).with(design::SURFACE, Color::WHITE);
/// assert_eq!(ContrastPair::OnSurface.colors(&t), Some((Color::hex(0x212121), Color::WHITE)));
/// assert_eq!(ContrastPair::OnSurface.ratio(&t).unwrap().to_string(), "16.10:1");
/// assert_eq!(ContrastPair::OnPrimary.ratio(&t), None); // not defined
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContrastPair {
    /// [`ON_SURFACE`] on [`SURFACE`]: body text on cards and containers.
    OnSurface,
    /// [`ON_SURFACE`] on [`BACKGROUND`]: text on the screen.
    OnBackground,
    /// [`ON_SURFACE`] on [`SURFACE_VARIANT`]: text on neutral buttons.
    OnSurfaceVariant,
    /// [`ON_SURFACE_MUTED`] on [`SURFACE`]: secondary text (hints, captions, units).
    OnSurfaceMuted,
    /// [`ON_PRIMARY`] on [`PRIMARY`]: text on accent buttons.
    OnPrimary,
    /// [`ON_SECONDARY`] on [`SECONDARY`]: text on checked buttons.
    OnSecondary,
    /// [`FOCUS_RING`] at [`FOCUS_RING_OPACITY`] over [`SURFACE`], against [`SURFACE`]: the
    /// focused widget must be identifiable.
    FocusRing,
    /// [`ON_SURFACE`] recolored with [`DISABLED`] at [`DISABLED_OPACITY`] (how the themes draw
    /// disabled widgets), against [`SURFACE`]: disabled content stays readable.
    DisabledContent,
}

impl ContrastPair {
    /// Every pair.
    pub const ALL: [ContrastPair; 8] = [
        ContrastPair::OnSurface,
        ContrastPair::OnBackground,
        ContrastPair::OnSurfaceVariant,
        ContrastPair::OnSurfaceMuted,
        ContrastPair::OnPrimary,
        ContrastPair::OnSecondary,
        ContrastPair::FocusRing,
        ContrastPair::DisabledContent,
    ];

    /// A short name for messages (`"on-surface/surface"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ContrastPair::OnSurface => "on-surface/surface",
            ContrastPair::OnBackground => "on-surface/background",
            ContrastPair::OnSurfaceVariant => "on-surface/surface-variant",
            ContrastPair::OnSurfaceMuted => "on-surface-muted/surface",
            ContrastPair::OnPrimary => "on-primary/primary",
            ContrastPair::OnSecondary => "on-secondary/secondary",
            ContrastPair::FocusRing => "focus-ring/surface",
            ContrastPair::DisabledContent => "disabled-content/surface",
        }
    }

    /// The foreground and background colors of the pair as `t` displays them (blended with
    /// their opacity elements, see the variants); `None` if `t` lacks one of the elements.
    /// Never panics.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::design::{self, ContrastPair, ElementTable};
    ///
    /// let t = ElementTable::new().with(design::SURFACE, Color::WHITE).with(design::ON_SURFACE, Color::BLACK);
    /// assert_eq!(ContrastPair::OnSurface.colors(&t), Some((Color::BLACK, Color::WHITE)));
    /// assert_eq!(ContrastPair::OnPrimary.colors(&t), None); // no PRIMARY / ON_PRIMARY
    /// ```
    #[must_use]
    pub fn colors(self, t: &ElementTable) -> Option<(Color, Color)> {
        let surface = t.get(SURFACE)?;
        Some(match self {
            ContrastPair::OnSurface => (t.get(ON_SURFACE)?, surface),
            ContrastPair::OnBackground => (t.get(ON_SURFACE)?, t.get(BACKGROUND)?),
            ContrastPair::OnSurfaceVariant => (t.get(ON_SURFACE)?, t.get(SURFACE_VARIANT)?),
            ContrastPair::OnSurfaceMuted => (t.get(ON_SURFACE_MUTED)?, surface),
            ContrastPair::OnPrimary => (t.get(ON_PRIMARY)?, t.get(PRIMARY)?),
            ContrastPair::OnSecondary => (t.get(ON_SECONDARY)?, t.get(SECONDARY)?),
            ContrastPair::FocusRing => (
                Color::mix(t.get(FOCUS_RING)?, surface, t.get(FOCUS_RING_OPACITY)?),
                surface,
            ),
            ContrastPair::DisabledContent => (
                Color::mix(t.get(DISABLED)?, t.get(ON_SURFACE)?, t.get(DISABLED_OPACITY)?),
                surface,
            ),
        })
    }

    /// The WCAG contrast ratio of the pair in `t` (`None` if `t` lacks one of the elements).
    /// Never panics.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_core::color::ContrastRatio;
    /// use twine_style::design::{self, ContrastPair, ElementTable};
    ///
    /// let t = ElementTable::new().with(design::SURFACE, Color::WHITE).with(design::ON_SURFACE, Color::BLACK);
    /// let r = ContrastPair::OnSurface.ratio(&t).unwrap(); // about 21:1
    /// assert!(r >= ContrastRatio::WCAG_AAA);
    /// ```
    #[must_use]
    pub fn ratio(self, t: &ElementTable) -> Option<ContrastRatio> {
        self.colors(t).map(|(fg, bg)| fg.contrast_ratio(bg))
    }
}

/// A pair that does not reach its mode's minimum contrast
/// ([`ElementTable::check_contrast`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ContrastViolation {
    /// The pair.
    pub pair: ContrastPair,
    /// Its ratio in the table (`None`: the table lacks one of its elements).
    pub ratio: Option<ContrastRatio>,
    /// The mode's minimum.
    pub min: ContrastRatio,
}

/// `"on-primary/primary: 2.68:1 < 3.00:1"`.
impl fmt::Display for ContrastViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ratio {
            Some(r) => write!(f, "{}: {} < {}", self.pair.name(), r, self.min),
            None => write!(f, "{}: element missing (needs {})", self.pair.name(), self.min),
        }
    }
}

impl ElementTable {
    /// Checks every [`ContrastPair`] that `mode` specifies ([`ThemeMode::min_contrast`]) and
    /// returns the first that falls short: a check for theme authors and for applications that
    /// override standard elements (run it in a test; it is not meant for rendering).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::{self, ContrastPair, ElementTable};
    ///
    /// let mut t = ElementTable::new()
    ///     .with(design::BACKGROUND, Color::WHITE)
    ///     .with(design::SURFACE, Color::WHITE)
    ///     .with(design::SURFACE_VARIANT, Color::hex(0xE0E0E0))
    ///     .with(design::ON_SURFACE, Color::BLACK)
    ///     .with(design::ON_SURFACE_MUTED, Color::hex(0x616161))
    ///     .with(design::PRIMARY, Color::hex(0x1565C0))
    ///     .with(design::ON_PRIMARY, Color::WHITE)
    ///     .with(design::SECONDARY, Color::BLACK)
    ///     .with(design::ON_SECONDARY, Color::WHITE);
    /// assert_eq!(t.check_contrast(ThemeMode::Light), Ok(()));
    /// // A brand color too light for white text.
    /// t.set(design::PRIMARY, Color::hex(0x90CAF9));
    /// let v = t.check_contrast(ThemeMode::Light).unwrap_err();
    /// assert_eq!(v.pair, ContrastPair::OnPrimary);
    /// assert_eq!(v.to_string(), "on-primary/primary: 1.74:1 < 3.00:1");
    /// ```
    ///
    /// # Errors
    ///
    /// The first pair (in [`ContrastPair::ALL`] order) whose ratio is below the minimum, or
    /// whose elements the table lacks.
    pub fn check_contrast(&self, mode: ThemeMode) -> Result<(), ContrastViolation> {
        for pair in ContrastPair::ALL {
            let Some(min) = mode.min_contrast(pair) else {
                continue;
            };
            let ratio = pair.ratio(self);
            if ratio.is_none_or(|r| r < min) {
                return Err(ContrastViolation { pair, ratio, min });
            }
        }
        Ok(())
    }
}

// ---- Standard elements ---------------------------------------------------------------------

/// Screen background.
pub const BACKGROUND: ColorElement = Element::standard(0);
/// Cards, containers, inputs and list items.
pub const SURFACE: ColorElement = Element::standard(1);
/// Text and icons on [`BACKGROUND`] and [`SURFACE`].
pub const ON_SURFACE: ColorElement = Element::standard(2);
/// Secondary text on [`BACKGROUND`] and [`SURFACE`]: hints, captions, units.
pub const ON_SURFACE_MUTED: ColorElement = Element::standard(3);
/// Neutral buttons, slider and arc tracks.
pub const SURFACE_VARIANT: ColorElement = Element::standard(4);
/// Borders and dividers.
pub const OUTLINE: ColorElement = Element::standard(5);
/// The accent color: buttons, indicators, knobs, selections.
pub const PRIMARY: ColorElement = Element::standard(6);
/// Text and icons on [`PRIMARY`].
pub const ON_PRIMARY: ColorElement = Element::standard(7);
/// The second accent color: checked buttons, edit outlines.
pub const SECONDARY: ColorElement = Element::standard(8);
/// Text and icons on [`SECONDARY`].
pub const ON_SECONDARY: ColorElement = Element::standard(9);
/// Status color of an alarm or a destructive action.
pub const DANGER: ColorElement = Element::standard(10);
/// Status color of a caution.
pub const WARNING: ColorElement = Element::standard(11);
/// Status color of a normal / good state.
pub const OK: ColorElement = Element::standard(12);
/// The overlay (recolor) of disabled widgets.
pub const DISABLED: ColorElement = Element::standard(13);
/// The outline of the focused widget.
pub const FOCUS_RING: ColorElement = Element::standard(14);
/// Scrollbars.
pub const SCROLLBAR: ColorElement = Element::standard(15);
/// Shadows.
pub const SHADOW: ColorElement = Element::standard(16);
/// The backdrop behind modal dialogs.
pub const SCRIM: ColorElement = Element::standard(17);
/// The placeholder text of empty inputs (fainter than [`ON_SURFACE_MUTED`]).
pub const PLACEHOLDER: ColorElement = Element::standard(18);
/// A mid-tone neutral: lines and ticks drawn inside containers (chart and scale lines) and
/// the translucent overlay of pressed list-like rows (menu items). Darker than
/// [`OUTLINE`] in light modes (LVGL's `lv_palette_main(GREY)`).
pub const NEUTRAL: ColorElement = Element::standard(19);

/// Spacing scale, extra small (tight paddings).
pub const SPACE_XS: LengthElement = Element::standard(0);
/// Spacing scale, small (gaps between items).
pub const SPACE_S: LengthElement = Element::standard(1);
/// Spacing scale, medium (the default padding of cards and buttons).
pub const SPACE_M: LengthElement = Element::standard(2);
/// Spacing scale, large (sections).
pub const SPACE_L: LengthElement = Element::standard(3);

/// Corner radius scale, small (checkbox markers, chips).
pub const RADIUS_S: RadiusElement = Element::standard(0);
/// Corner radius scale, medium (cards).
pub const RADIUS_M: RadiusElement = Element::standard(1);
/// Corner radius scale, large (dialogs, large buttons).
pub const RADIUS_L: RadiusElement = Element::standard(2);
/// Fully round: a circle for a square node, a pill otherwise.
pub const RADIUS_FULL: RadiusElement = Element::standard(3);

/// Opacity of the [`FOCUS_RING`].
pub const FOCUS_RING_OPACITY: OpacityElement = Element::standard(0);
/// Opacity of [`SHADOW`]s (transparent where a mode draws no shadows).
pub const SHADOW_OPACITY: OpacityElement = Element::standard(1);
/// Opacity of the [`DISABLED`] overlay.
pub const DISABLED_OPACITY: OpacityElement = Element::standard(2);
/// Opacity of the [`SCRIM`].
pub const SCRIM_OPACITY: OpacityElement = Element::standard(3);

/// Font scale, small (captions, markers).
pub const FONT_SMALL: FontElement = Element::standard(0);
/// Font scale, body text (the display's default font).
pub const FONT_BODY: FontElement = Element::standard(1);
/// Font scale, large (titles).
pub const FONT_LARGE: FontElement = Element::standard(2);

// ---- `style!` conversions ------------------------------------------------------------------

/// Converts `style!` values of design-value properties (colors, opacities, fonts) in `const`
/// context: a fixed value, an element or a design value; an `i32` passes unchanged (the
/// integer fields of the `shadow` and `grid_*` shorthands).
#[doc(hidden)]
pub struct __ElementArg<T>(pub T);

macro_rules! element_arg {
    ($($t:ty),*) => {$(
        impl __ElementArg<$t> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                DesignValue::Fixed(self.0)
            }
        }
        impl __ElementArg<Element<$t>> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                DesignValue::Element(self.0)
            }
        }
        impl __ElementArg<DesignValue<$t>> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                self.0
            }
        }
    )*};
}

element_arg!(Color, Opa, &'static Font);

impl __ElementArg<i32> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_keep_style_prop_small() {
        use core::mem::size_of;
        assert_eq!(size_of::<ElementRef>(), 4);
        assert_eq!(size_of::<ColorValue>(), 4);
        assert_eq!(size_of::<OpacityValue>(), 4);
        assert_eq!(size_of::<LengthValue>(), size_of::<Length>());
        assert_eq!(size_of::<RadiusValue>(), size_of::<Radius>());
        assert!(size_of::<FontValue>() <= 2 * size_of::<usize>());
    }

    #[test]
    fn custom_elements_follow_the_standard_ones() {
        assert_eq!(ColorElement::custom(0).id(), <Color as ElementType>::STANDARD);
        assert!(!PLACEHOLDER.is_custom());
        assert!(ColorElement::custom(0).is_custom());
        assert_eq!(ColorElement::custom(u16::MAX).id(), u16::MAX); // saturates
        assert_eq!(NEUTRAL.id() + 1, <Color as ElementType>::STANDARD);
        assert_eq!(SPACE_L.id() + 1, <Length as ElementType>::STANDARD);
        assert_eq!(RADIUS_FULL.id() + 1, <Radius as ElementType>::STANDARD);
        assert_eq!(SCRIM_OPACITY.id() + 1, <Opa as ElementType>::STANDARD);
        assert_eq!(FONT_LARGE.id() + 1, <&'static Font as ElementType>::STANDARD);
    }

    #[test]
    fn values_round_trip_through_style_values() {
        let v = ColorValue::Element(PRIMARY);
        assert_eq!(v.to_value(), StyleValue::Element(PRIMARY.erase()));
        assert_eq!(ColorValue::from_value(v.to_value()), Some(v));
        // An element of another kind does not fit.
        assert_eq!(ColorValue::from_value(StyleValue::Element(SPACE_M.erase())), None);
        assert_eq!(
            ColorValue::from_value(StyleValue::Color(Color::RED)),
            Some(ColorValue::Fixed(Color::RED))
        );
        assert_eq!(
            RadiusValue::Fixed(Radius::Circle).to_value().get::<Radius>(),
            Some(Radius::Circle)
        );
    }

    #[test]
    fn table_lookup_and_completeness() {
        let mut t = ElementTable::new();
        assert_eq!(t.value(SURFACE.erase()), None);
        t.set(SURFACE, Color::WHITE);
        t.set(RADIUS_FULL, Radius::Circle);
        t.set(ColorElement::custom(3), Color::RED);
        assert_eq!(t.get(SURFACE), Some(Color::WHITE));
        assert_eq!(t.get(BACKGROUND), None);
        assert_eq!(t.get(ColorElement::custom(3)), Some(Color::RED));
        assert_eq!(t.get(ColorElement::custom(2)), None);
        assert_eq!(
            t.value(RADIUS_FULL.erase()).and_then(StyleValue::get::<Radius>),
            Some(Radius::Circle)
        );
        assert_eq!(t.missing_standard(), Some(BACKGROUND.erase()));
    }
}
