//! Design elements: named, typed values (colors, spacings, radii, opacities, fonts, motion
//! timings, icons) that a style refers to by name and the **theme** supplies, resolved when the
//! style is resolved.
//!
//! A style that says `bg_color: design::SURFACE` instead of `bg_color: Color::WHITE` follows
//! the theme of its display: switching the theme's [`ThemeMode`] (light → dark) swaps the
//! theme's [`ElementTable`] and every node that uses a design element is re-resolved — no view
//! is rebuilt, no binding re-runs, nothing is allocated.
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
//! // A theme's values for one mode: a const table, in flash.
//! static LIGHT: ElementTable = ElementTable::new().color(design::SURFACE, Color::WHITE);
//! assert_eq!(LIGHT.get(design::SURFACE), Some(Color::WHITE));
//! assert_eq!(LIGHT.get(design::PRIMARY), None); // not defined by this table
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
//! | [`MotionElement`] | `&'static AnimSpec` | `anim` (the timing of a widget's own animations) |
//! | [`IconElement`] | `&'static ImageSource` | `bg_image` |
//!
//! A property of type `Color` stores a [`ColorValue`] (= [`DesignValue<Color>`]): a fixed
//! color or a color element. `Color` and `ColorElement` both convert into it, so the
//! `StyleBuf` builders, the `style!` macro and the `twine-view` modifiers take either (likewise
//! [`LengthValue`], [`RadiusValue`], [`OpacityValue`], [`FontValue`], [`MotionValue`],
//! [`IconValue`]).
//!
//! # The standard elements
//!
//! Every built-in theme (`Lvgl`, `Mono`, `Simple` of `twine-theme`) defines all of them
//! (icons excepted: they are optional), in every mode it supports. Use them in application
//! styles instead of literal values so the application follows the theme. The typed sets
//! ([`SystemColors`], [`Spacing`], [`Shape`], [`Elevation`], [`MotionSet`], [`IconSet`],
//! [`FontSet`], all of them in a [`DesignSet`]) name each one as a field, so a theme built from
//! them cannot leave one out.
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
//! | [`ACCENT_TINT`] | a light tint of the accent (tinted buttons, selected rows) |
//! | [`SEPARATOR`] | hairlines between list rows |
//! | [`GROUPED_BACKGROUND`] | the background behind grouped lists |
//! | [`FILL_1`], [`FILL_2`], [`FILL_3`] | fills of thin, medium and large shapes (tracks, segmented controls, gray buttons) |
//! | [`SPACE_XS`], [`SPACE_S`], [`SPACE_M`], [`SPACE_L`] | spacing scale (padding, gaps, margins), ascending |
//! | [`ELEVATION_1_WIDTH`] … [`ELEVATION_3_SPREAD`] | the shadow (width, y offset, spread) of three elevation levels |
//! | [`RADIUS_S`], [`RADIUS_M`], [`RADIUS_L`], [`RADIUS_FULL`] | corner radius scale, ascending; `RADIUS_FULL` is a circle / pill |
//! | [`FOCUS_RING_OPACITY`], [`SHADOW_OPACITY`], [`DISABLED_OPACITY`], [`SCRIM_OPACITY`] | the opacities of the focus ring, shadows, the disabled overlay and the backdrop |
//! | [`FONT_SMALL`], [`FONT_BODY`], [`FONT_LARGE`] | font scale |
//! | [`MOTION_FAST`], [`MOTION_STANDARD`], [`MOTION_EMPHASIZED`], [`MOTION_EXIT`] | motion timings by purpose |
//! | [`ICON_CHECK`], [`ICON_CHEVRON_RIGHT`], [`ICON_BACK`], [`ICON_CLOSE`], [`ICON_SEARCH`], [`ICON_PLUS`], [`ICON_MINUS`], [`ICON_CLEAR`] | icons (optional) |
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
//! # Application and theme-private elements
//!
//! [`Element::custom`] names an element of the application (a brand color, a gauge zone
//! color, …); the application gives it a value per mode in its theme's tables (the built-in
//! themes take it with `.element(..)` / `.element_in(mode, ..)`; a const table with
//! [`ElementTable::custom`]). The [`elements!`](crate::elements) macro declares a set of them
//! with a struct of values, so every mode gives every element a value:
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
//! [`Element::private`] names an element a **theme** defines for its own styles and components
//! (sizes it computes per display, component colors): a separate id space, so it never
//! collides with the application's elements.
//!
//! # Tables in flash
//!
//! A theme's per-mode tables are built in a `const`/`static` from the typed sets and `'static`
//! slices: in flash, no heap and no start-up code; switching the mode swaps a pointer. Values
//! that depend on the display (the LVGL theme's sizes scaled by its DPI and size class) are a
//! separate, small table the theme computes once when it is attached to a display
//! (`ThemeHook::metrics` in twine-engine); an element the mode table does not define is looked
//! up there.
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

mod sets;

use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use twine_anim::AnimSpec;
use twine_core::color::ContrastRatio;
use twine_core::{Color, Opa, Scale};
use twine_image::ImageSource;
use twine_text::Font;

pub use sets::{
    DesignSet, Elevation, ElevationLevel, FontSet, IconSet, MotionSet, Shape, Spacing, SystemColors,
};

use crate::value::{PropValue, StyleValue};
use crate::value_types::{Length, Radius};

/// The mode of a theme: which of its element tables is active. Switching the mode
/// (`Engine::set_theme_mode` in `twine-engine`, `ThemeHandle::set_mode` in `twine-view`)
/// swaps the table, so everything styled with [design elements](self) changes at once.
///
/// The built-in `Lvgl`, `Mono` and `Simple` themes support all four modes (an application
/// lists the ones its product shows). Every mode has documented
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

    /// A stable `snake_case` name for logs and audit reports (`"light"`, `"dark"`, `"night"`,
    /// `"high_contrast"`).
    ///
    /// ```
    /// use twine_style::ThemeMode;
    /// assert_eq!(ThemeMode::HighContrast.name(), "high_contrast");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
            ThemeMode::Night => "night",
            ThemeMode::HighContrast => "high_contrast",
        }
    }

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
        // The entry after `self`, wrapping to the first (also when `self` is not listed).
        let next = match modes.iter().position(|&m| m == self) {
            // SAFE-ARITH: `i < modes.len()`.
            Some(i) => modes.get(i.wrapping_add(1)),
            None => None,
        };
        next.or(modes.first()).copied().unwrap_or(self)
    }
}

/// The value kind of a design element (one standard array per kind in an [`ElementTable`]).
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
    /// A `&'static AnimSpec`: the timing of a motion ([`MotionElement`]).
    Motion,
    /// A `&'static ImageSource`: an icon ([`IconElement`]).
    Icon,
    /// A [`Scale`] factor ([`ScaleElement`]; e.g. the LVGL theme's DPI-dependent rotary
    /// sensitivity). No standard elements: application and private ones only.
    Scale,
}

impl ElementKind {
    /// Number of kinds.
    pub const COUNT: usize = 8;

    /// A short name for logs (`"color"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ElementKind::Color => "color",
            ElementKind::Length => "length",
            ElementKind::Radius => "radius",
            ElementKind::Opacity => "opacity",
            ElementKind::Font => "font",
            ElementKind::Motion => "motion",
            ElementKind::Icon => "icon",
            ElementKind::Scale => "scale",
        }
    }
}

/// The id bit of application elements ([`Element::custom`]).
const CUSTOM_BIT: u16 = 0x4000;
/// The id bit of theme-private elements ([`Element::private`]).
const PRIVATE_BIT: u16 = 0x8000;

/// Where an element id lives in an [`ElementTable`].
#[derive(Copy, Clone)]
enum Region {
    /// A standard element: its index in the kind's standard array.
    Standard(usize),
    /// An application element ([`Element::custom`]): its index among them.
    Custom(usize),
    /// A theme-private element ([`Element::private`]): its index among them.
    Private(usize),
}

impl Region {
    #[inline]
    const fn of(id: u16) -> Self {
        // `as usize` widens a `u16`: lossless.
        let index = (id & Element::<Color>::MAX_INDEX) as usize;
        if id & PRIVATE_BIT != 0 {
            Region::Private(index)
        } else if id & CUSTOM_BIT != 0 {
            Region::Custom(index)
        } else {
            Region::Standard(id as usize)
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

    /// The element's raw id (see [`Element::id`]).
    #[inline]
    #[must_use]
    pub const fn id(self) -> u16 {
        self.id
    }
}

/// `"color element #3"`, `"custom color element #0"`, `"private length element #2"`.
impl fmt::Display for ElementRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match Region::of(self.id) {
            Region::Standard(i) => write!(f, "{} element #{}", self.kind.name(), i),
            Region::Custom(i) => write!(f, "custom {} element #{}", self.kind.name(), i),
            Region::Private(i) => write!(f, "private {} element #{}", self.kind.name(), i),
        }
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for twine_core::Color {}
    impl Sealed for crate::Length {}
    impl Sealed for crate::Radius {}
    impl Sealed for twine_core::Opa {}
    impl Sealed for &'static twine_text::Font {}
    impl Sealed for &'static twine_anim::AnimSpec {}
    impl Sealed for &'static twine_image::ImageSource {}
    impl Sealed for twine_core::Scale {}
}

/// A value type that design elements can have: [`Color`], [`Length`], [`Radius`], [`Opa`],
/// `&'static Font`, `&'static AnimSpec`, `&'static ImageSource` and [`Scale`] (sealed).
pub trait ElementType: PropValue + Copy + 'static + sealed::Sealed {
    /// The kind of the elements of this type.
    const KIND: ElementKind;
    /// Number of standard elements of this type (ids `0..STANDARD`).
    const STANDARD: u16;
    /// The standard values of this type in `t`.
    #[doc(hidden)]
    fn standard(t: &ElementTable) -> &[Option<Self>];
    /// The standard values of this type in `t`, mutable.
    #[doc(hidden)]
    fn standard_mut(t: &mut ElementTable) -> &mut [Option<Self>];
}

macro_rules! element_type {
    ($t:ty, $kind:ident, $field:ident, $standard:ident) => {
        impl ElementType for $t {
            const KIND: ElementKind = ElementKind::$kind;
            // `as u16`: the counts are small constants (checked by a test).
            const STANDARD: u16 = $standard as u16;
            #[inline]
            fn standard(t: &ElementTable) -> &[Option<Self>] {
                &t.$field
            }
            #[inline]
            fn standard_mut(t: &mut ElementTable) -> &mut [Option<Self>] {
                &mut t.$field
            }
        }
    };
}

/// Number of standard color elements.
const COLORS: usize = 26;
/// Number of standard length elements.
const LENGTHS: usize = 13;
/// Number of standard radius elements.
const RADII: usize = 4;
/// Number of standard opacity elements.
const OPACITIES: usize = 4;
/// Number of standard font elements.
const FONTS: usize = 3;
/// Number of standard motion elements.
const MOTIONS: usize = 4;
/// Number of standard icon elements.
const ICONS: usize = 8;
/// Number of standard scale elements (none: scale elements are application or private).
const SCALES: usize = 0;

element_type!(Color, Color, colors, COLORS);
element_type!(Length, Length, lengths, LENGTHS);
element_type!(Radius, Radius, radii, RADII);
element_type!(Opa, Opacity, opacities, OPACITIES);
element_type!(&'static Font, Font, fonts, FONTS);
element_type!(&'static AnimSpec, Motion, motions, MOTIONS);
element_type!(&'static ImageSource, Icon, icons, ICONS);
element_type!(Scale, Scale, scales, SCALES);

/// A design element whose value is a `T`: a typed name the theme gives a value (see the
/// [module documentation](self)). `Copy`, 2 bytes.
///
/// Three id spaces that never collide:
///
/// | Elements | Made with | Who gives them values |
/// |----------|-----------|-----------------------|
/// | standard | the constants of [`design`](self) ([`SURFACE`], [`SPACE_M`], [`MOTION_FAST`], …) | every theme (a typed set: [`DesignSet`]) |
/// | application | [`custom`](Self::custom), or the [`elements!`](crate::elements) macro | the application, in its theme's tables ([`ElementTable::custom`], [`ElementTable::set`]) |
/// | theme-private | [`private`](Self::private) | the theme that defines them (its component elements; [`ElementTable::private`]) |
pub struct Element<T> {
    id: u16,
    _t: PhantomData<fn() -> T>,
}

/// A design element of color type ([`SURFACE`], [`PRIMARY`], …).
pub type ColorElement = Element<Color>;
/// A design element of length type ([`SPACE_M`], [`ELEVATION_1_WIDTH`], …).
pub type LengthElement = Element<Length>;
/// A design element of radius type ([`RADIUS_M`], …).
pub type RadiusElement = Element<Radius>;
/// A design element of opacity type ([`SHADOW_OPACITY`], …).
pub type OpacityElement = Element<Opa>;
/// A design element of font type ([`FONT_BODY`], …).
pub type FontElement = Element<&'static Font>;
/// A design element of motion type: the timing (duration, delay, easing) of an animation or a
/// transition ([`MOTION_FAST`], …).
pub type MotionElement = Element<&'static AnimSpec>;
/// A design element of icon type: an image the theme supplies ([`ICON_CHECK`], …).
pub type IconElement = Element<&'static ImageSource>;
/// A design element of scale type ([`Scale`]): application or theme-private only (no standard
/// scale elements), e.g. a theme's rotary sensitivity scaled by the display's DPI.
pub type ScaleElement = Element<Scale>;

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
        write!(f, "{}Element({:#06x})", T::KIND.name(), self.id)
    }
}

impl<T> Element<T> {
    /// The largest index of an application or theme-private element (16 383): larger numbers
    /// given to [`custom`](Self::custom) and [`private`](Self::private) saturate to it.
    pub const MAX_INDEX: u16 = 0x3FFF;

    /// Standard element `id`.
    const fn standard(id: u16) -> Self {
        Self { id, _t: PhantomData }
    }

    /// The element with id `id` (as stored in an [`ElementRef`]).
    #[inline]
    const fn from_id(id: u16) -> Self {
        Self { id, _t: PhantomData }
    }

    /// Application element number `n`: an element of the application (a brand color, a gauge
    /// zone color, …), which it gives a value in each mode of its theme; until then it
    /// resolves to the property's default (see the [module documentation](self)).
    ///
    /// **Numbering is shared by all kinds:** `ColorElement::custom(0)` and
    /// `LengthElement::custom(0)` name the same table slot, so number the application's
    /// elements 0, 1, 2, … across kinds — the [`elements!`](crate::elements) macro does it.
    /// A table stores one slot per number up to the highest it defines. A huge `n` saturates
    /// at [`MAX_INDEX`](Self::MAX_INDEX). Application elements never collide with standard or
    /// theme-private ones. `const`; never panics.
    ///
    /// ```
    /// use twine_style::design::{self, ColorElement, LengthElement};
    ///
    /// const BRAND: ColorElement = ColorElement::custom(0);
    /// const GAUGE_WIDTH: LengthElement = LengthElement::custom(1);
    /// assert_ne!(BRAND, design::BACKGROUND);
    /// assert!(BRAND.is_custom() && GAUGE_WIDTH.is_custom());
    /// assert_eq!(BRAND.index(), 0);
    /// ```
    #[must_use]
    pub const fn custom(n: u16) -> Self {
        Self::from_id(CUSTOM_BIT | min_index(n))
    }

    /// Theme-private element number `n`: an element a theme defines for its own styles and
    /// components (e.g. sizes it computes per display), outside the standard set. Applications
    /// use [`custom`](Self::custom) instead; private and application elements never collide,
    /// so a theme's private elements cannot overwrite an application's values (or the other
    /// way round). Numbering is shared by all kinds, like [`custom`](Self::custom)'s; a huge
    /// `n` saturates at [`MAX_INDEX`](Self::MAX_INDEX). `const`; never panics.
    ///
    /// ```
    /// use twine_style::design::{ColorElement, LengthElement};
    ///
    /// const KNOB_INSET: LengthElement = LengthElement::private(0);
    /// assert!(KNOB_INSET.is_private() && !KNOB_INSET.is_custom());
    /// assert_ne!(KNOB_INSET.id(), LengthElement::custom(0).id());
    /// assert_ne!(ColorElement::private(3).id(), ColorElement::custom(3).id());
    /// ```
    #[must_use]
    pub const fn private(n: u16) -> Self {
        Self::from_id(PRIVATE_BIT | min_index(n))
    }

    /// Whether this is an application element ([`custom`](Self::custom)).
    #[must_use]
    pub const fn is_custom(self) -> bool {
        self.id & (CUSTOM_BIT | PRIVATE_BIT) == CUSTOM_BIT
    }

    /// Whether this is a theme-private element ([`private`](Self::private)).
    #[must_use]
    pub const fn is_private(self) -> bool {
        self.id & PRIVATE_BIT != 0
    }

    /// The element's number within its id space: the standard element's index, or the `n` of
    /// [`custom`](Self::custom) / [`private`](Self::private).
    #[must_use]
    pub const fn index(self) -> u16 {
        if self.id & (CUSTOM_BIT | PRIVATE_BIT) == 0 {
            self.id
        } else {
            self.id & Self::MAX_INDEX
        }
    }

    /// The raw id: the standard element's index (below 16 384), or the index with bit 14 set
    /// (application elements) or bit 15 set (theme-private elements). Stable within a build;
    /// for logs and tables, not for persisting.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.id
    }
}

impl<T: ElementType> Element<T> {
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

/// `n` saturated at [`Element::MAX_INDEX`].
const fn min_index(n: u16) -> u16 {
    if n > Element::<Color>::MAX_INDEX {
        Element::<Color>::MAX_INDEX
    } else {
        n
    }
}

/// The value of a style property that a theme can supply: a fixed value or a
/// [design element](self). Every color, length, radius, opacity and font property, the
/// `anim` property and the `bg_image` property store one (as [`ColorValue`], [`LengthValue`],
/// [`RadiusValue`], [`OpacityValue`], [`FontValue`], [`MotionValue`], [`IconValue`]), so it
/// can be set to either:
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
/// let _s = StyleBuf::new().bg_color(Color::RED).border_color(design::OUTLINE).anim(design::MOTION_FAST);
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
/// The `anim` property's value: a `&'static AnimSpec` or a [`MotionElement`].
pub type MotionValue = DesignValue<&'static AnimSpec>;
/// The `bg_image` property's value: a `&'static ImageSource` or an [`IconElement`].
pub type IconValue = DesignValue<&'static ImageSource>;
/// A [`Scale`] or a scale element (`rotary_sensitivity`).
pub type ScaleValue = DesignValue<Scale>;

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

// ---- Element tables --------------------------------------------------------------------------

/// The values of application or theme-private elements in an [`ElementTable`]: `'static`
/// values (const tables: [`ElementTable::custom`] / [`ElementTable::private`]) with the values
/// set at run time ([`ElementTable::set`]) on top. One slot per index; a slot holds the value
/// as a [`StyleValue`] of its kind.
#[derive(Clone, Debug)]
struct Slots {
    /// `'static` values.
    fixed: &'static [Option<StyleValue>],
    /// Values set at run time; a defined slot wins over `fixed`'s.
    set: Vec<Option<StyleValue>>,
}

impl Slots {
    /// No values (`const`; no allocation).
    const EMPTY: Slots = Slots {
        fixed: &[],
        set: Vec::new(),
    };

    #[inline]
    fn get(&self, i: usize) -> Option<StyleValue> {
        match self.set.get(i).copied().flatten() {
            Some(v) => Some(v),
            None => self.fixed.get(i).copied().flatten(),
        }
    }

    fn set(&mut self, i: usize, v: StyleValue) {
        if self.set.len() <= i {
            // SAFE-ARITH: `i` is at most `Element::MAX_INDEX`.
            self.set.resize(i.wrapping_add(1), None);
        }
        // SAFE-INDEX: the vector holds at least `i + 1` slots (resized above); the fallback
        // is never taken.
        if let Some(slot) = self.set.get_mut(i) {
            *slot = Some(v);
        }
    }

    /// The number of slots that may hold a value.
    fn len(&self) -> usize {
        self.set.len().max(self.fixed.len())
    }

    fn overlay(&mut self, other: &Slots) {
        for i in 0..other.len() {
            if let Some(v) = other.get(i) {
                self.set(i, v);
            }
        }
    }
}

/// The values a theme gives the design elements in one [`ThemeMode`]: the standard elements
/// of every kind (fixed-size arrays, one slot per standard element), the application's
/// elements ([`Element::custom`]) and the theme's private ones ([`Element::private`]).
///
/// A table is either **const** — built in a `const`/`static` from typed sets
/// ([`DesignSet::to_table`], [`colors`](Self::colors), …) and `'static` slices
/// ([`custom`](Self::custom), [`private`](Self::private)); it lives in flash and costs no heap
/// and no start-up code — or built at run time with [`with`](Self::with) / [`set`](Self::set)
/// (allocates for application and private elements while it is filled). Reading is a
/// bounds-checked index either way.
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::design::{self, ElementTable};
/// use twine_style::{Length, StyleValue};
///
/// // Const: in flash, no heap.
/// static BRAND: ElementTable = ElementTable::new().color(design::PRIMARY, Color::hex(0x2196F3));
/// assert_eq!(BRAND.get(design::PRIMARY), Some(Color::hex(0x2196F3)));
///
/// // Run time.
/// let t = ElementTable::new()
///     .with(design::PRIMARY, Color::hex(0x2196F3))
///     .with(design::SPACE_M, Length::dp(16))
///     .with(design::SHADOW_OPACITY, Opa::P50);
/// assert_eq!(t.get(design::SPACE_M), Some(Length::dp(16)));
/// assert_eq!(t.value(design::PRIMARY.erase()), Some(StyleValue::Color(Color::hex(0x2196F3))));
/// assert_eq!(t.missing_standard(), Some(design::BACKGROUND.erase()));
/// ```
#[derive(Clone, Debug)]
pub struct ElementTable {
    colors: [Option<Color>; COLORS],
    lengths: [Option<Length>; LENGTHS],
    radii: [Option<Radius>; RADII],
    opacities: [Option<Opa>; OPACITIES],
    fonts: [Option<&'static Font>; FONTS],
    motions: [Option<&'static AnimSpec>; MOTIONS],
    icons: [Option<&'static ImageSource>; ICONS],
    scales: [Option<Scale>; SCALES],
    custom: Slots,
    private: Slots,
}

impl Default for ElementTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Generates a const builder of one standard element of a kind.
macro_rules! const_setter {
    ($(#[$doc:meta])* $name:ident, $t:ty, $field:ident) => {
        $(#[$doc])*
        ///
        /// For **standard** elements, in `const` context too (the table stays const). An
        /// application or private element is not stored here (the table is returned unchanged):
        /// give those with [`custom`](Self::custom) / [`private`](Self::private) (const) or
        /// [`set`](Self::set). Never panics.
        #[must_use]
        pub const fn $name(mut self, element: Element<$t>, value: $t) -> Self {
            // `as usize` widens a `u16`: lossless. A non-standard id is past the array: no
            // split, nothing stored (`get_mut` is not `const`).
            if let Some((_, tail)) = self.$field.split_at_mut_checked(element.id as usize) {
                if let Some(slot) = tail.first_mut() {
                    *slot = Some(value);
                }
            }
            self
        }
    };
}

impl ElementTable {
    /// An empty table (`const`; no allocation).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            colors: [None; COLORS],
            lengths: [None; LENGTHS],
            radii: [None; RADII],
            opacities: [None; OPACITIES],
            fonts: [None; FONTS],
            motions: [None; MOTIONS],
            icons: [None; ICONS],
            scales: [None; SCALES],
            custom: Slots::EMPTY,
            private: Slots::EMPTY,
        }
    }

    const_setter!(
        /// Sets the standard color element `element` (builder style).
        color, Color, colors
    );
    const_setter!(
        /// Sets the standard length element `element` (builder style).
        length, Length, lengths
    );
    const_setter!(
        /// Sets the standard radius element `element` (builder style).
        radius, Radius, radii
    );
    const_setter!(
        /// Sets the standard opacity element `element` (builder style).
        opacity, Opa, opacities
    );
    const_setter!(
        /// Sets the standard font element `element` (builder style).
        font, &'static Font, fonts
    );
    const_setter!(
        /// Sets the standard motion element `element` (builder style).
        motion, &'static AnimSpec, motions
    );
    const_setter!(
        /// Sets the standard icon element `element` (builder style).
        icon, &'static ImageSource, icons
    );

    /// Uses `values` for the application's elements ([`Element::custom`]): slot `n` is the
    /// value of `custom(n)`, as made by the [`elements!`](crate::elements) macro's `slots`
    /// (`const`; replaces the table's `'static` application values; values given with
    /// [`set`](Self::set) stay on top of them).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::design::{ColorElement, ElementTable};
    /// use twine_style::StyleValue;
    ///
    /// const BRAND: ColorElement = ColorElement::custom(0);
    /// static T: ElementTable = ElementTable::new().custom(&[Some(StyleValue::Color(Color::RED))]);
    /// assert_eq!(T.get(BRAND), Some(Color::RED));
    /// ```
    #[must_use]
    pub const fn custom(mut self, values: &'static [Option<StyleValue>]) -> Self {
        self.custom.fixed = values;
        self
    }

    /// Uses `values` for the theme's private elements ([`Element::private`]): slot `n` is the
    /// value of `private(n)` (`const`; replaces the table's `'static` private values; values
    /// given with [`set`](Self::set) stay on top of them).
    ///
    /// ```
    /// use twine_style::design::{ElementTable, LengthElement};
    /// use twine_style::{Length, StyleValue};
    ///
    /// const KNOB_INSET: LengthElement = LengthElement::private(0);
    /// static T: ElementTable = ElementTable::new().private(&[Some(StyleValue::Length(Length::Px(2)))]);
    /// assert_eq!(T.get(KNOB_INSET), Some(Length::Px(2)));
    /// assert_eq!(T.get(LengthElement::custom(0)), None); // another id space
    /// ```
    #[must_use]
    pub const fn private(mut self, values: &'static [Option<StyleValue>]) -> Self {
        self.private.fixed = values;
        self
    }

    /// Sets the value of `element` and returns the table (builder style; run time, see
    /// [`set`](Self::set)).
    #[must_use]
    pub fn with<T: ElementType>(mut self, element: Element<T>, value: T) -> Self {
        self.set(element, value);
        self
    }

    /// Sets the value of `element`.
    ///
    /// A standard element is written in place. An application or private element grows the
    /// table's run-time slots to its index (one slot per index below it, defined or not —
    /// number them densely from 0; they override the table's `'static` values): an allocation
    /// while the table is built, never while it is read. Never panics (an
    /// allocation failure aborts like any `Vec` growth).
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
    // Out of line: a theme sets dozens of elements in one function; one copy per element type
    // instead of the code inlined at every call (flash; build time only).
    #[inline(never)]
    pub fn set<T: ElementType>(&mut self, element: Element<T>, value: T) {
        match Region::of(element.id) {
            Region::Standard(i) => {
                // A standard id below `STANDARD` (the constants of this module); a forged
                // larger one is ignored.
                if let Some(slot) = T::standard_mut(self).get_mut(i) {
                    *slot = Some(value);
                }
            }
            Region::Custom(i) => self.custom.set(i, value.into_value()),
            Region::Private(i) => self.private.set(i, value.into_value()),
        }
    }

    /// The value of `element` (`None` if the table does not define it).
    #[inline]
    #[must_use]
    pub fn get<T: ElementType>(&self, element: Element<T>) -> Option<T> {
        match Region::of(element.id) {
            Region::Standard(i) => T::standard(self).get(i).copied().flatten(),
            Region::Custom(i) => self.custom.get(i).and_then(T::from_value),
            Region::Private(i) => self.private.get(i).and_then(T::from_value),
        }
    }

    /// The value of `element` as a [`StyleValue`] (what the resolver substitutes); `None` if
    /// the table does not define it (an application or private slot holding a value of
    /// another kind counts as not defined).
    #[must_use]
    pub fn value(&self, element: ElementRef) -> Option<StyleValue> {
        match Region::of(element.id) {
            Region::Standard(i) => self.standard_value(element.kind, i),
            Region::Custom(i) => self.custom.get(i).filter(|v| holds(element.kind, *v)),
            Region::Private(i) => self.private.get(i).filter(|v| holds(element.kind, *v)),
        }
    }

    #[inline]
    fn standard_value(&self, kind: ElementKind, i: usize) -> Option<StyleValue> {
        match kind {
            ElementKind::Color => self.colors.get(i).copied().flatten().map(StyleValue::Color),
            ElementKind::Length => self.lengths.get(i).copied().flatten().map(StyleValue::Length),
            ElementKind::Radius => self.radii.get(i).copied().flatten().map(PropValue::into_value),
            ElementKind::Opacity => self.opacities.get(i).copied().flatten().map(StyleValue::Opa),
            ElementKind::Font => self.fonts.get(i).copied().flatten().map(StyleValue::Font),
            ElementKind::Motion => self.motions.get(i).copied().flatten().map(StyleValue::AnimSpec),
            ElementKind::Icon => self.icons.get(i).copied().flatten().map(StyleValue::Image),
            ElementKind::Scale => self.scales.get(i).copied().flatten().map(StyleValue::Scale),
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
        fn copy<T: Copy>(dst: &mut [Option<T>], src: &[Option<T>]) {
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
        copy(&mut self.motions, &other.motions);
        copy(&mut self.icons, &other.icons);
        copy(&mut self.scales, &other.scales);
        self.custom.overlay(&other.custom);
        self.private.overlay(&other.private);
    }

    /// The first required standard element the table does not define (`None` if it defines
    /// all of them): a check for theme authors. Icons are optional (an [`IconSet`] maps only
    /// the icons an application uses) and are not required.
    #[must_use]
    pub fn missing_standard(&self) -> Option<ElementRef> {
        fn first<T: ElementType>(t: &ElementTable) -> Option<ElementRef> {
            T::standard(t)
                .iter()
                .position(Option::is_none)
                // `as u16`: an index below a small `STANDARD`.
                .map(|i| Element::<T>::from_id(i as u16).erase())
        }
        first::<Color>(self)
            .or_else(|| first::<Length>(self))
            .or_else(|| first::<Radius>(self))
            .or_else(|| first::<Opa>(self))
            .or_else(|| first::<&'static Font>(self))
            .or_else(|| first::<&'static AnimSpec>(self))
    }
}

/// Where each kind's standard elements start in a [`FlatElements`] and how many there are,
/// in [`ElementKind`] order: one read for a standard element's range.
const STANDARD_RANGE: [(usize, usize); ElementKind::COUNT] = {
    const L: usize = COLORS;
    const R: usize = L + LENGTHS;
    const O: usize = R + RADII;
    const F: usize = O + OPACITIES;
    const M: usize = F + FONTS;
    const I: usize = M + MOTIONS;
    const S: usize = I + ICONS;
    [
        (0, COLORS),
        (L, LENGTHS),
        (R, RADII),
        (O, OPACITIES),
        (F, FONTS),
        (M, MOTIONS),
        (I, ICONS),
        (S, SCALES),
    ]
};

/// Every standard element of every kind.
const STANDARD_TOTAL: usize = COLORS + LENGTHS + RADII + OPACITIES + FONTS + MOTIONS + ICONS + SCALES;

/// The element values a display resolves with, finished for it (`Dp` lengths in pixels for its
/// DPI) and flattened into **one indexed array**: every standard element, then the
/// application's, then the theme-private ones — a table `top`
/// (the theme's table of the current mode) merged over a table `base` (the display's
/// metrics). What the resolver reads for every design element of a node (the engine keeps
/// one per themed display): one bounds-checked read instead of a search through two tables.
///
/// [`fill`](Self::fill) rebuilds it in place: after [`reserve`](Self::reserve) for every pair
/// of tables it will be filled from, a fill (a mode switch) allocates nothing.
///
/// ```
/// use twine_core::Color;
/// use twine_style::design::{self, ElementTable, FlatElements, LengthElement};
/// use twine_style::Length;
///
/// const GAP: LengthElement = LengthElement::private(0);
/// let metrics = ElementTable::new().with(GAP, Length::Px(6)).with(design::SPACE_M, Length::Px(9));
/// static DARK: ElementTable = ElementTable::new()
///     .color(design::SURFACE, Color::BLACK)
///     .length(design::SPACE_M, Length::Px(4));
/// let mut flat = FlatElements::new();
/// flat.reserve(Some(&metrics), &DARK);
/// flat.fill(Some(&metrics), &DARK, 160);
/// assert_eq!(flat.get(design::SURFACE), Some(Color::BLACK));
/// assert_eq!(flat.get(design::SPACE_M), Some(Length::Px(4))); // the mode's table wins
/// assert_eq!(flat.get(GAP), Some(Length::Px(6)));
/// assert_eq!(flat.get(design::DANGER), None);
/// ```
#[derive(Clone, Debug, Default)]
pub struct FlatElements {
    values: Vec<Option<StyleValue>>,
    /// The number of application slots (the private slots follow them).
    custom: usize,
}

impl FlatElements {
    /// Empty: every element undefined (no allocation).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: Vec::new(),
            custom: 0,
        }
    }

    /// Makes room for [`fill`](Self::fill)`(base, top)`, so that it allocates nothing. Call it
    /// for every `top` the array will be filled from (every mode a display lists). Never
    /// panics.
    pub fn reserve(&mut self, base: Option<&ElementTable>, top: &ElementTable) {
        let (bc, bp) = base.map_or((0, 0), |b| (b.custom.len(), b.private.len()));
        let n = STANDARD_TOTAL
            .saturating_add(bc.max(top.custom.len()))
            .saturating_add(bp.max(top.private.len()));
        if let Some(more) = n.checked_sub(self.values.len()) {
            self.values.reserve(more);
        }
    }

    /// Rebuilds the array in place: every element `top` defines with `top`'s value, every
    /// other one `base` defines with `base`'s, each **finished for the display**: a
    /// [`Length::Dp`] value is stored in pixels for `dpi`, so a read returns the value the
    /// resolver uses as is. Allocates only when [`reserve`](Self::reserve) was not called
    /// for this pair; never panics.
    pub fn fill(&mut self, base: Option<&ElementTable>, top: &ElementTable, dpi: u16) {
        fn std<T: Copy>(
            out: &mut Vec<Option<StyleValue>>,
            base: Option<&[Option<T>]>,
            top: &[Option<T>],
            into: impl Fn(T) -> StyleValue,
        ) {
            for (i, t) in top.iter().enumerate() {
                let b = base.and_then(|b| b.get(i)).copied().flatten();
                out.push(t.or(b).map(&into));
            }
        }
        fn slots(out: &mut Vec<Option<StyleValue>>, base: Option<&Slots>, top: &Slots) -> usize {
            let n = base.map_or(0, Slots::len).max(top.len());
            for i in 0..n {
                out.push(top.get(i).or_else(|| base.and_then(|b| b.get(i))));
            }
            n
        }
        let v = &mut self.values;
        v.clear();
        std(v, base.map(|b| &b.colors[..]), &top.colors, StyleValue::Color);
        std(v, base.map(|b| &b.lengths[..]), &top.lengths, StyleValue::Length);
        std(v, base.map(|b| &b.radii[..]), &top.radii, PropValue::into_value);
        std(v, base.map(|b| &b.opacities[..]), &top.opacities, StyleValue::Opa);
        std(v, base.map(|b| &b.fonts[..]), &top.fonts, StyleValue::Font);
        std(
            v,
            base.map(|b| &b.motions[..]),
            &top.motions,
            StyleValue::AnimSpec,
        );
        std(v, base.map(|b| &b.icons[..]), &top.icons, StyleValue::Image);
        std(v, base.map(|b| &b.scales[..]), &top.scales, StyleValue::Scale);
        self.custom = slots(v, base.map(|b| &b.custom), &top.custom);
        slots(v, base.map(|b| &b.private), &top.private);
        for x in v.iter_mut().flatten() {
            *x = x.with_dpi(dpi);
        }
    }

    /// The value of `element` as a [`StyleValue`] (`None` if undefined; an application or
    /// private slot holding a value of another kind counts as undefined). One bounds-checked
    /// read; never panics.
    #[inline]
    #[must_use]
    pub fn value(&self, element: ElementRef) -> Option<StyleValue> {
        let index = match Region::of(element.id) {
            Region::Standard(i) => {
                // `(offset, count)` of the kind's standard elements; a standard id of another
                // kind's range is not this kind's element.
                let (offset, count) = STANDARD_RANGE
                    .get(element.kind as usize)
                    .copied()
                    .unwrap_or((0, 0));
                if i >= count {
                    return None;
                }
                return self.values.get(offset.wrapping_add(i)).copied().flatten();
            }
            Region::Custom(i) if i < self.custom => STANDARD_TOTAL.wrapping_add(i),
            Region::Custom(_) => return None,
            Region::Private(i) => STANDARD_TOTAL.saturating_add(self.custom).saturating_add(i),
        };
        self.values
            .get(index)
            .copied()
            .flatten()
            .filter(|v| holds(element.kind, *v))
    }

    /// The value of `element` (`None` if undefined). Never panics.
    #[inline]
    #[must_use]
    pub fn get<T: ElementType>(&self, element: Element<T>) -> Option<T> {
        self.value(element.erase()).and_then(T::from_value)
    }

    /// Bytes of heap the array holds (its capacity).
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.values
            .capacity()
            .saturating_mul(core::mem::size_of::<Option<StyleValue>>())
    }
}

/// Whether `v` is a value of `kind` (lengths and radii share their representation).
#[inline]
fn holds(kind: ElementKind, v: StyleValue) -> bool {
    matches!(
        (kind, v),
        (ElementKind::Color, StyleValue::Color(_))
            | (ElementKind::Length | ElementKind::Radius, StyleValue::Length(_))
            | (ElementKind::Opacity, StyleValue::Opa(_))
            | (ElementKind::Font, StyleValue::Font(_))
            | (ElementKind::Motion, StyleValue::AnimSpec(_))
            | (ElementKind::Icon, StyleValue::Image(_))
            | (ElementKind::Scale, StyleValue::Scale(_))
    )
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
/// A light tint of the accent: the background of tinted buttons and selected rows, readable
/// with [`PRIMARY`] text on it.
pub const ACCENT_TINT: ColorElement = Element::standard(20);
/// Hairlines between the rows of a list or the items of a group (lighter than [`OUTLINE`]
/// where a theme distinguishes them).
pub const SEPARATOR: ColorElement = Element::standard(21);
/// The background behind grouped lists and settings sections (the groups themselves are
/// [`SURFACE`]).
pub const GROUPED_BACKGROUND: ColorElement = Element::standard(22);
/// Fill for thin and small shapes: switch and slider tracks, progress backgrounds (the
/// lightest of the three fills).
pub const FILL_1: ColorElement = Element::standard(23);
/// Fill for medium shapes: segmented-control and search-field backgrounds.
pub const FILL_2: ColorElement = Element::standard(24);
/// Fill for large shapes: gray buttons, input backgrounds (the strongest of the three fills).
pub const FILL_3: ColorElement = Element::standard(25);

/// Spacing scale, extra small (tight paddings).
pub const SPACE_XS: LengthElement = Element::standard(0);
/// Spacing scale, small (gaps between items).
pub const SPACE_S: LengthElement = Element::standard(1);
/// Spacing scale, medium (the default padding of cards and buttons).
pub const SPACE_M: LengthElement = Element::standard(2);
/// Spacing scale, large (sections).
pub const SPACE_L: LengthElement = Element::standard(3);
/// Elevation 1 (raised controls such as buttons): the shadow's width (blur).
pub const ELEVATION_1_WIDTH: LengthElement = Element::standard(4);
/// Elevation 1: the shadow's downward offset.
pub const ELEVATION_1_OFFSET_Y: LengthElement = Element::standard(5);
/// Elevation 1: the shadow's spread.
pub const ELEVATION_1_SPREAD: LengthElement = Element::standard(6);
/// Elevation 2 (cards and menus above the content): the shadow's width (blur).
pub const ELEVATION_2_WIDTH: LengthElement = Element::standard(7);
/// Elevation 2: the shadow's downward offset.
pub const ELEVATION_2_OFFSET_Y: LengthElement = Element::standard(8);
/// Elevation 2: the shadow's spread.
pub const ELEVATION_2_SPREAD: LengthElement = Element::standard(9);
/// Elevation 3 (dialogs, sheets, popovers): the shadow's width (blur).
pub const ELEVATION_3_WIDTH: LengthElement = Element::standard(10);
/// Elevation 3: the shadow's downward offset.
pub const ELEVATION_3_OFFSET_Y: LengthElement = Element::standard(11);
/// Elevation 3: the shadow's spread.
pub const ELEVATION_3_SPREAD: LengthElement = Element::standard(12);

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

/// Motion for small, quick feedback: press highlights, toggles, focus changes.
pub const MOTION_FAST: MotionElement = Element::standard(0);
/// Motion for ordinary changes of state and position (the default).
pub const MOTION_STANDARD: MotionElement = Element::standard(1);
/// Motion for large or important changes: elements entering, expanding.
pub const MOTION_EMPHASIZED: MotionElement = Element::standard(2);
/// Motion for elements leaving or collapsing (usually shorter than entering).
pub const MOTION_EXIT: MotionElement = Element::standard(3);

/// Icon: a check mark (checked checkboxes, selected rows).
pub const ICON_CHECK: IconElement = Element::standard(0);
/// Icon: a chevron pointing right (disclosure of list rows).
pub const ICON_CHEVRON_RIGHT: IconElement = Element::standard(1);
/// Icon: back (navigation bars).
pub const ICON_BACK: IconElement = Element::standard(2);
/// Icon: close (dialogs, sheets).
pub const ICON_CLOSE: IconElement = Element::standard(3);
/// Icon: search (search fields).
pub const ICON_SEARCH: IconElement = Element::standard(4);
/// Icon: plus (steppers, add buttons).
pub const ICON_PLUS: IconElement = Element::standard(5);
/// Icon: minus (steppers, remove buttons).
pub const ICON_MINUS: IconElement = Element::standard(6);
/// Icon: clear (the clear button of text fields).
pub const ICON_CLEAR: IconElement = Element::standard(7);

// ---- `style!` conversions ------------------------------------------------------------------

/// Converts `style!` values of design-value properties (colors, opacities, fonts, `anim`,
/// `bg_image`) in `const` context: a fixed value, an element or a design value; an `i32`
/// passes unchanged (the integer fields of the `shadow` and `grid_*` shorthands).
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

element_arg!(
    Color,
    Opa,
    &'static Font,
    &'static AnimSpec,
    &'static ImageSource,
    Scale
);

impl __ElementArg<i32> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// Converts an element value to its [`StyleValue`] in `const` context (the slots made by
/// [`elements!`](crate::elements)).
#[doc(hidden)]
pub struct __ElementValue<T>(pub T);

macro_rules! element_value {
    ($($t:ty => $variant:ident),*) => {$(
        impl __ElementValue<$t> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> StyleValue {
                StyleValue::$variant(self.0)
            }
        }
    )*};
}

element_value!(
    Color => Color,
    Length => Length,
    Opa => Opa,
    &'static Font => Font,
    &'static AnimSpec => AnimSpec,
    &'static ImageSource => Image,
    Scale => Scale
);

impl __ElementValue<Radius> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> StyleValue {
        StyleValue::Length(self.0.to_length())
    }
}

/// Defines a set of **application design elements**: a struct whose fields are the values
/// (one instance per mode, typically a `const` or `static`) and, for each field, an
/// associated constant naming the element ([`Element::custom`], numbered in field order), to
/// use in styles like a standard element.
///
/// Field types: `Color`, `Length`, `Radius`, `Opa`, `&'static Font`, `&'static AnimSpec` or
/// `&'static ImageSource` (any [`ElementType`]). A missing field is a compile error, so every
/// mode gives every element a value. The struct also gets `ELEMENT_COUNT` (the number of
/// fields) and `const fn slots(&self)`, the values in the form
/// [`ElementTable::custom`] takes: wrap the call in `&const { … }` to keep the table const.
/// One application set per theme (the elements are `custom(0)`, `custom(1)`, …: a second
/// set would reuse the numbers).
///
/// ```
/// use twine_core::Color;
/// use twine_style::design::{self, ElementTable};
/// use twine_style::{Length, StyleBuf};
///
/// twine_style::elements! {
///     /// Alarm colors per priority, and the size of vital-sign readouts.
///     pub struct Alarm {
///         /// High-priority alarms.
///         HIGH: Color,
///         /// Medium-priority alarms.
///         MEDIUM: Color,
///         /// Padding of a vital-sign tile.
///         TILE_PAD: Length,
///     }
/// }
///
/// const DAY: Alarm = Alarm { HIGH: Color::hex(0xD32F2F), MEDIUM: Color::hex(0xF9A825), TILE_PAD: Length::Px(8) };
/// static DAY_TABLE: ElementTable =
///     ElementTable::new().color(design::SURFACE, Color::WHITE).custom(&const { DAY.slots() });
///
/// let tile = StyleBuf::new().bg_color(Alarm::HIGH).padding(Alarm::TILE_PAD);
/// # let _ = tile;
/// assert_eq!(DAY_TABLE.get(Alarm::HIGH), Some(Color::hex(0xD32F2F)));
/// assert_eq!(DAY_TABLE.get(Alarm::TILE_PAD), Some(Length::Px(8)));
/// assert_eq!(Alarm::ELEMENT_COUNT, 3);
/// ```
#[macro_export]
macro_rules! elements {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[allow(non_snake_case)]
        #[derive(Clone, Copy)]
        $vis struct $name {
            $( $(#[$fmeta])* pub $field: $ty, )+
        }

        #[allow(non_snake_case)]
        impl $name {
            $crate::__elements_consts!(0u16; $( $(#[$fmeta])* $field : $ty, )+);

            /// The number of elements of the set.
            pub const ELEMENT_COUNT: usize = [$( stringify!($field) ),+].len();

            /// The values as the slots of the application elements of an element table
            /// (`ElementTable::custom`), in element order.
            #[must_use]
            pub const fn slots(&self) -> [::core::option::Option<$crate::StyleValue>; Self::ELEMENT_COUNT] {
                [ $( ::core::option::Option::Some($crate::design::__ElementValue(self.$field).get()), )+ ]
            }
        }
    };
}

/// The element constants of [`elements!`], numbered from `$n`.
#[doc(hidden)]
#[macro_export]
macro_rules! __elements_consts {
    ($n:expr; ) => {};
    ($n:expr; $(#[$fmeta:meta])* $field:ident : $ty:ty, $($rest:tt)*) => {
        $(#[$fmeta])*
        pub const $field: $crate::design::Element<$ty> = $crate::design::Element::custom($n);
        $crate::__elements_consts!($n + 1; $($rest)*);
    };
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
        assert!(size_of::<MotionValue>() <= 2 * size_of::<usize>());
        assert!(size_of::<IconValue>() <= 2 * size_of::<usize>());
    }

    #[test]
    fn standard_counts_match_the_constants() {
        assert_eq!(FILL_3.id() + 1, <Color as ElementType>::STANDARD);
        assert_eq!(ELEVATION_3_SPREAD.id() + 1, <Length as ElementType>::STANDARD);
        assert_eq!(RADIUS_FULL.id() + 1, <Radius as ElementType>::STANDARD);
        assert_eq!(SCRIM_OPACITY.id() + 1, <Opa as ElementType>::STANDARD);
        assert_eq!(FONT_LARGE.id() + 1, <&'static Font as ElementType>::STANDARD);
        assert_eq!(MOTION_EXIT.id() + 1, <&'static AnimSpec as ElementType>::STANDARD);
        assert_eq!(
            ICON_CLEAR.id() + 1,
            <&'static ImageSource as ElementType>::STANDARD
        );
    }

    #[test]
    fn private_and_custom_ids_never_collide() {
        for n in [0, 1, 7, 100, Element::<Color>::MAX_INDEX, u16::MAX] {
            let c = ColorElement::custom(n);
            let p = ColorElement::private(n);
            assert_ne!(c.id(), p.id());
            assert!(c.is_custom() && !c.is_private());
            assert!(p.is_private() && !p.is_custom());
            assert!(c.id() >= <Color as ElementType>::STANDARD && p.id() >= <Color as ElementType>::STANDARD);
            assert_eq!(c.index(), n.min(Element::<Color>::MAX_INDEX)); // saturates
            assert_eq!(p.index(), c.index());
        }
        assert!(!PLACEHOLDER.is_custom() && !PLACEHOLDER.is_private());
        // A table keeps them apart.
        let t = ElementTable::new()
            .with(ColorElement::custom(2), Color::RED)
            .with(ColorElement::private(2), Color::BLUE);
        assert_eq!(t.get(ColorElement::custom(2)), Some(Color::RED));
        assert_eq!(t.get(ColorElement::private(2)), Some(Color::BLUE));
        assert_eq!(t.get(ColorElement::custom(1)), None);
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
        let m = MotionValue::Element(MOTION_FAST);
        assert_eq!(MotionValue::from_value(m.to_value()), Some(m));
        assert_eq!(
            MotionValue::from_value(StyleValue::Element(ICON_CHECK.erase())),
            None
        );
    }

    static SPEC: AnimSpec = AnimSpec::new(twine_core::Duration::ms(120));
    static ICON: ImageSource = ImageSource::Symbol("x");

    /// Every kind resolves from a run-time table, standard and custom.
    #[test]
    fn table_lookup_of_every_kind() {
        let font = &twine_text::EMPTY_FONT;
        let mut t = ElementTable::new();
        assert_eq!(t.value(SURFACE.erase()), None);
        t.set(SURFACE, Color::WHITE);
        t.set(SPACE_M, Length::Px(6));
        t.set(RADIUS_FULL, Radius::Circle);
        t.set(SHADOW_OPACITY, Opa::P50);
        t.set(FONT_BODY, font);
        t.set(MOTION_FAST, &SPEC);
        t.set(ICON_CHECK, &ICON);
        t.set(ColorElement::custom(3), Color::RED);
        assert_eq!(t.get(SURFACE), Some(Color::WHITE));
        assert_eq!(t.get(BACKGROUND), None);
        assert_eq!(t.value(SPACE_M.erase()), Some(StyleValue::Length(Length::Px(6))));
        assert_eq!(
            t.value(RADIUS_FULL.erase()).and_then(StyleValue::get::<Radius>),
            Some(Radius::Circle)
        );
        assert_eq!(t.value(SHADOW_OPACITY.erase()), Some(StyleValue::Opa(Opa::P50)));
        assert_eq!(t.value(FONT_BODY.erase()), Some(StyleValue::Font(font)));
        assert_eq!(t.value(MOTION_FAST.erase()), Some(StyleValue::AnimSpec(&SPEC)));
        assert_eq!(t.value(ICON_CHECK.erase()), Some(StyleValue::Image(&ICON)));
        assert_eq!(t.get(ColorElement::custom(3)), Some(Color::RED));
        assert_eq!(t.get(ColorElement::custom(2)), None);
        // A custom slot read as another kind is not defined.
        assert_eq!(t.value(OpacityElement::custom(3).erase()), None);
        assert_eq!(t.missing_standard(), Some(BACKGROUND.erase()));
    }

    static CUSTOM: [Option<StyleValue>; 2] = [None, Some(StyleValue::Color(Color::GREEN))];
    static CONST_TABLE: ElementTable = ElementTable::new()
        .color(SURFACE, Color::WHITE)
        .length(SPACE_M, Length::Px(4))
        .motion(MOTION_STANDARD, &SPEC)
        .icon(ICON_BACK, &ICON)
        .color(ColorElement::custom(0), Color::BLACK) // not standard: ignored
        .custom(&CUSTOM);

    #[test]
    fn const_table_in_a_static() {
        assert_eq!(CONST_TABLE.get(SURFACE), Some(Color::WHITE));
        assert_eq!(CONST_TABLE.get(SPACE_M), Some(Length::Px(4)));
        assert!(
            CONST_TABLE
                .get(MOTION_STANDARD)
                .is_some_and(|s| core::ptr::eq(s, &raw const SPEC))
        );
        assert!(
            CONST_TABLE
                .get(ICON_BACK)
                .is_some_and(|s| core::ptr::eq(s, &raw const ICON))
        );
        assert_eq!(CONST_TABLE.get(ColorElement::custom(0)), None);
        assert_eq!(CONST_TABLE.get(ColorElement::custom(1)), Some(Color::GREEN));
        // Overriding a `'static` table copies it into an owned one; the static is unchanged.
        let mut t = CONST_TABLE.clone();
        t.set(ColorElement::custom(0), Color::RED);
        assert_eq!(t.get(ColorElement::custom(0)), Some(Color::RED));
        assert_eq!(t.get(ColorElement::custom(1)), Some(Color::GREEN));
        assert_eq!(CONST_TABLE.get(ColorElement::custom(0)), None);
    }

    #[test]
    fn overlay_layers_every_region() {
        let mut base = CONST_TABLE.clone();
        base.overlay(
            &ElementTable::new()
                .with(SURFACE, Color::BLACK)
                .with(ColorElement::custom(4), Color::BLUE)
                .with(LengthElement::private(0), Length::Px(9)),
        );
        assert_eq!(base.get(SURFACE), Some(Color::BLACK));
        assert_eq!(base.get(SPACE_M), Some(Length::Px(4)));
        assert_eq!(base.get(ColorElement::custom(1)), Some(Color::GREEN));
        assert_eq!(base.get(ColorElement::custom(4)), Some(Color::BLUE));
        assert_eq!(base.get(LengthElement::private(0)), Some(Length::Px(9)));
    }

    #[test]
    fn scale_elements_are_application_or_private() {
        const ROTARY: ScaleElement = ScaleElement::private(0);
        static T: ElementTable =
            ElementTable::new().private(&[Some(StyleValue::Scale(Scale::from_raw_256(512)))]);
        assert_eq!(T.get(ROTARY), Some(Scale::from_raw_256(512)));
        assert_eq!(
            T.value(ROTARY.erase()),
            Some(StyleValue::Scale(Scale::from_raw_256(512)))
        );
        // A slot of another kind is not a scale.
        assert_eq!(T.get(LengthElement::private(0)), None);
        let mut t = ElementTable::new();
        t.set(ScaleElement::custom(1), Scale::pct(50));
        assert_eq!(t.get(ScaleElement::custom(1)), Some(Scale::pct(50)));
        assert_eq!(t.missing_standard(), Some(BACKGROUND.erase()));
    }

    #[test]
    fn element_display_names_the_id_space() {
        extern crate std;
        use std::string::ToString;
        assert_eq!(SURFACE.erase().to_string(), "color element #1");
        assert_eq!(
            ColorElement::custom(2).erase().to_string(),
            "custom color element #2"
        );
        assert_eq!(
            LengthElement::private(5).erase().to_string(),
            "private length element #5"
        );
    }
}
