//! Value types of style properties and layout enums (LVGL equivalents noted on every type).

use twine_core::{Color, Duration, Opa};

use crate::design::{DesignValue, Element};

use twine_render::RADIUS_CIRCLE;
pub use twine_render::{BlendMode, BorderSide, Gradient};
pub use twine_text::{TextAlign, TextDecor};

/// Largest coordinate a style may hold (LVGL `LV_COORD_MAX` = 2²⁹ − 1): the default of
/// `MaxWidth`/`MaxHeight`. Kept well below `i32::MAX` so layout arithmetic (sums of sizes,
/// paddings and margins) cannot overflow.
pub const COORD_MAX: i32 = (1 << 29) - 1;

/// The reference DPI of density-independent lengths: one [`Length::Dp`] is one pixel on a
/// 160 DPI display (Android's `dp`, LVGL's `LV_DPX` base).
pub const REFERENCE_DPI: u16 = 160;

/// The DPI assumed for a display that does not report one (LVGL `LV_DPI_DEF`; also the
/// default of `DisplayInfo::dpi`).
pub const DEFAULT_DPI: u16 = 130;

/// `dp` density-independent pixels in pixels on a `dpi` display (LVGL `LV_DPX_CALC`):
/// `(dpi · dp + 80) / 160`, at least 1 for positive values, 0 for 0; negative values are
/// scaled symmetrically. One multiply and one divide by a constant; saturating.
///
/// ```
/// use twine_style::dpx;
/// assert_eq!(dpx(1, 130), 1);
/// assert_eq!(dpx(10, 160), 10);
/// assert_eq!(dpx(10, 320), 20);
/// assert_eq!(dpx(-5, 160), -5);
/// assert_eq!(dpx(0, 320), 0);
/// ```
#[inline]
#[must_use]
pub const fn dpx(dp: i32, dpi: u16) -> i32 {
    const HALF: i64 = REFERENCE_DPI as i64 / 2;
    let mag = (dp as i64).abs();
    if mag == 0 {
        return 0;
    }
    // DIV: constant divisor.
    let v = (dpi as i64 * mag + HALF) / REFERENCE_DPI as i64;
    let v = if v < 1 {
        1
    } else if v > i32::MAX as i64 {
        i32::MAX as i64
    } else {
        v
    };
    if dp < 0 { -(v as i32) } else { v as i32 }
}

/// A size, position or spacing that may depend on the display density, the parent or the
/// content (LVGL coordinates with `LV_PCT(x)` / `LV_SIZE_CONTENT` / `LV_DPX(x)`).
///
/// `Dp` lengths are converted to pixels with the DPI of the node's display when the engine
/// resolves the style (one multiply/divide per resolved value, see [`dpx`]); `Px` values pass
/// through unchanged. Spacing properties (padding, margin, gaps, border width) accept `Px` and
/// `Dp`; `Pct` and `Content` resolve to 0 there.
///
/// ```
/// use twine_style::Length;
///
/// assert_eq!(Length::px(40).resolve(200, 10), 40);
/// assert_eq!(Length::pct(25).resolve(200, 10), 50);
/// assert_eq!(Length::Content.resolve(200, 10), 10);
/// assert_eq!(Length::from(7), Length::Px(7));
/// // 8 dp are 8 px at 160 DPI and 16 px at 320 DPI.
/// assert_eq!(Length::dp(8).with_dpi(160), Length::Px(8));
/// assert_eq!(Length::dp(8).with_dpi(320), Length::Px(16));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Length {
    /// Pixels.
    Px(i32),
    /// Percent of the reference size (usually the parent's content area); values above 100
    /// are allowed.
    Pct(i16),
    /// Sized by the content (LVGL `LV_SIZE_CONTENT`).
    Content,
    /// Density-independent pixels: pixels on a [`REFERENCE_DPI`] (160 DPI) display, scaled
    /// to the display's DPI when the style is resolved (LVGL `LV_DPX`).
    Dp(i32),
}

impl Length {
    /// `v` pixels.
    #[must_use]
    pub const fn px(v: i32) -> Self {
        Length::Px(v)
    }

    /// `p` percent (LVGL `lv_pct`).
    #[must_use]
    pub const fn pct(p: i16) -> Self {
        Length::Pct(p)
    }

    /// `v` density-independent pixels (see [`Length::Dp`]).
    #[must_use]
    pub const fn dp(v: i32) -> Self {
        Length::Dp(v)
    }

    /// `v` pixels (`const` counterpart of `From<i32>`).
    #[must_use]
    pub const fn from_i32(v: i32) -> Self {
        Length::Px(v)
    }

    /// The length with a `Dp` value converted to pixels for a `dpi` display (see [`dpx`]);
    /// other lengths are unchanged.
    #[inline]
    #[must_use]
    pub const fn with_dpi(self, dpi: u16) -> Self {
        match self {
            Length::Dp(v) => Length::Px(dpx(v, dpi)),
            other => other,
        }
    }

    /// The length in pixels: `Pct` is relative to `parent` (truncated like LVGL
    /// `lv_pct_to_px`, saturating), `Content` is `content`. A `Dp` that was not converted
    /// with [`with_dpi`](Self::with_dpi) counts at the reference 160 DPI (1 dp = 1 px); style
    /// resolution converts `Dp` with the DPI of the source ([`StyleSource::dpi`](crate::StyleSource::dpi)).
    #[must_use]
    pub const fn resolve(self, parent: i32, content: i32) -> i32 {
        match self {
            Length::Px(v) | Length::Dp(v) => v,
            Length::Pct(p) => {
                let v = p as i64 * parent as i64 / 100;
                if v > i32::MAX as i64 {
                    i32::MAX
                } else if v < i32::MIN as i64 {
                    i32::MIN
                } else {
                    v as i32
                }
            }
            Length::Content => content,
        }
    }

    /// The pixel value of a `Px` length.
    #[must_use]
    pub const fn as_px(self) -> Option<i32> {
        match self {
            Length::Px(v) => Some(v),
            _ => None,
        }
    }
}

impl Default for Length {
    /// `Px(0)`.
    fn default() -> Self {
        Length::Px(0)
    }
}

impl From<i32> for Length {
    fn from(v: i32) -> Self {
        Length::Px(v)
    }
}

/// A corner radius (the `radius` style property): pixels, density-independent pixels, or
/// fully round.
///
/// Stored in styles as a pixel [`Length`], with `Circle` in the renderer's compact encoding
/// (`twine_render::RADIUS_CIRCLE`, clamped to half the shorter side when drawn), so it costs
/// nothing over a plain integer.
///
/// ```
/// use twine_style::{PropId, Radius, StyleBuf, StyleValue};
///
/// let pill = StyleBuf::new().radius(Radius::Circle);
/// assert_eq!(pill.get(PropId::Radius).and_then(StyleValue::get::<Radius>), Some(Radius::Circle));
/// assert_eq!(Radius::from(6), Radius::Px(6));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Radius {
    /// Pixels. Drawn clamped to `0..=` half the shorter side of the node, so a negative value
    /// draws square corners and a large one a fully round shape. A value at or above the
    /// renderer's circle marker (`twine_render::RADIUS_CIRCLE`, 32 767) is stored like
    /// [`Circle`](Self::Circle) and reads back as `Circle` (it draws the same).
    ///
    /// ```
    /// use twine_style::{Length, Radius};
    /// assert_eq!(Radius::from_length(Radius::Px(40_000).to_length()), Some(Radius::Circle));
    /// assert_eq!(Radius::from_length(Radius::Px(-3).to_length()), Some(Radius::Px(-3)));
    /// ```
    Px(i32),
    /// Density-independent pixels (see [`Length::Dp`]).
    Dp(i32),
    /// Fully round: a circle for a square node, a pill otherwise (LVGL `LV_RADIUS_CIRCLE`).
    #[doc(alias = "RADIUS_CIRCLE")]
    #[doc(alias = "LV_RADIUS_CIRCLE")]
    Circle,
}

impl Radius {
    /// The radius as the pixel [`Length`] stored in styles (`Circle` → the renderer's
    /// circle marker).
    #[inline]
    #[must_use]
    pub const fn to_length(self) -> Length {
        match self {
            Radius::Px(v) => Length::Px(v),
            Radius::Dp(v) => Length::Dp(v),
            Radius::Circle => Length::Px(RADIUS_CIRCLE),
        }
    }

    /// The radius stored as `l` (`None` for `Pct`/`Content`, which a radius cannot be).
    #[inline]
    #[must_use]
    pub const fn from_length(l: Length) -> Option<Radius> {
        match l {
            Length::Px(v) if v >= RADIUS_CIRCLE => Some(Radius::Circle),
            Length::Px(v) => Some(Radius::Px(v)),
            Length::Dp(v) => Some(Radius::Dp(v)),
            Length::Pct(_) | Length::Content => None,
        }
    }
}

impl Default for Radius {
    /// `Px(0)`.
    fn default() -> Self {
        Radius::Px(0)
    }
}

impl From<i32> for Radius {
    fn from(v: i32) -> Self {
        Radius::Px(v)
    }
}

/// A duration stored in a style property (`anim_duration`): whole milliseconds in a `u32`
/// (LVGL's unit), so a [`StyleProp`](crate::StyleProp) stays 4-byte aligned and 12 bytes on
/// 32-bit targets, which a `u64` [`Duration`] payload would break. Builders, `style!` and the
/// view modifiers take a [`Duration`] (saturating at `u32::MAX` ms, about 49 days).
///
/// ```
/// use twine_core::Duration;
/// use twine_style::{DurationMs, PropId, StyleBuf, StyleValue};
///
/// let s = StyleBuf::new().anim_duration(Duration::ms(200));
/// assert_eq!(s.get(PropId::AnimDuration).and_then(StyleValue::get::<Duration>), Some(Duration::ms(200)));
/// assert_eq!(DurationMs::from(Duration::secs(1)).as_duration(), Duration::ms(1000));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DurationMs(u32);

impl DurationMs {
    /// `d` in whole milliseconds (truncated, saturating).
    #[inline]
    #[must_use]
    pub const fn from_duration(d: Duration) -> Self {
        DurationMs(d.as_millis_u32())
    }

    /// The duration.
    #[inline]
    #[must_use]
    pub const fn as_duration(self) -> Duration {
        Duration::ms(self.0 as u64)
    }

    /// Whole milliseconds.
    #[inline]
    #[must_use]
    pub const fn as_millis(self) -> u32 {
        self.0
    }
}

impl From<Duration> for DurationMs {
    fn from(d: Duration) -> Self {
        DurationMs::from_duration(d)
    }
}

impl From<DurationMs> for Duration {
    fn from(d: DurationMs) -> Self {
        d.as_duration()
    }
}

/// Converts `style!` values into [`DurationMs`] in `const` context: a [`Duration`] or a
/// `DurationMs`.
#[doc(hidden)]
pub struct __DurationArg<T>(pub T);

impl __DurationArg<Duration> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> DurationMs {
        DurationMs::from_duration(self.0)
    }
}

impl __DurationArg<DurationMs> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> DurationMs {
        self.0
    }
}

/// Converts `style!` values into a [`LengthValue`](crate::design::LengthValue) in `const`
/// context: integers become pixels; a `Length`, a `LengthElement` or a `LengthValue` is taken
/// as is (inherent methods on concrete instantiations; no trait needed).
#[doc(hidden)]
pub struct __LengthArg<T>(pub T);

/// Converts `style!` values into a [`RadiusValue`](crate::design::RadiusValue) in `const`
/// context: integers become pixels; a `Radius`, a `RadiusElement` or a `RadiusValue` is taken
/// as is.
#[doc(hidden)]
pub struct __RadiusArg<T>(pub T);

macro_rules! design_arg {
    ($arg:ident, $t:ident, $px:expr) => {
        impl $arg<i32> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                DesignValue::Fixed($px(self.0))
            }
        }

        impl $arg<$t> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                DesignValue::Fixed(self.0)
            }
        }

        impl $arg<Element<$t>> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                DesignValue::Element(self.0)
            }
        }

        impl $arg<DesignValue<$t>> {
            #[doc(hidden)]
            #[must_use]
            pub const fn get(self) -> DesignValue<$t> {
                self.0
            }
        }
    };
}

design_arg!(__LengthArg, Length, Length::Px);
design_arg!(__RadiusArg, Radius, Radius::Px);

/// Alignment of an object inside its parent's content area (the `align` style property; LVGL
/// `lv_align_t` without the `OUT_*` values, same discriminants).
///
/// Placing an object *next to* another one is [`Anchor`] (`align_to`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[doc(alias = "lv_align_t")]
pub enum Align {
    /// Top-left, or top-right with a right-to-left base direction.
    #[default]
    #[doc(alias = "LV_ALIGN_DEFAULT")]
    Default = 0,
    /// Top-left corner.
    #[doc(alias = "LV_ALIGN_TOP_LEFT")]
    TopLeft = 1,
    /// Top edge, centered horizontally.
    #[doc(alias = "LV_ALIGN_TOP_MID")]
    TopMid = 2,
    /// Top-right corner.
    #[doc(alias = "LV_ALIGN_TOP_RIGHT")]
    TopRight = 3,
    /// Bottom-left corner.
    #[doc(alias = "LV_ALIGN_BOTTOM_LEFT")]
    BottomLeft = 4,
    /// Bottom edge, centered horizontally.
    #[doc(alias = "LV_ALIGN_BOTTOM_MID")]
    BottomMid = 5,
    /// Bottom-right corner.
    #[doc(alias = "LV_ALIGN_BOTTOM_RIGHT")]
    BottomRight = 6,
    /// Left edge, centered vertically.
    #[doc(alias = "LV_ALIGN_LEFT_MID")]
    LeftMid = 7,
    /// Right edge, centered vertically.
    #[doc(alias = "LV_ALIGN_RIGHT_MID")]
    RightMid = 8,
    /// Centered.
    #[doc(alias = "LV_ALIGN_CENTER")]
    Center = 9,
}

/// Where an object is placed relative to another one (`align_to`, LVGL `lv_obj_align_to`):
/// inside the base's content area ([`Anchor::Inside`], any [`Align`]) or next to its outer
/// rectangle (the other variants, LVGL `LV_ALIGN_OUT_*`).
///
/// `Above*`/`Below*` put the object above/below the base with the named horizontal edge (or
/// center) lined up; `Left*`/`Right*` put it left/right of the base with the named vertical
/// edge (or middle) lined up. An [`Align`] converts to `Anchor::Inside`.
///
/// ```
/// use twine_style::{Align, Anchor};
///
/// assert_eq!(Anchor::from(Align::Center), Anchor::Inside(Align::Center));
/// assert!(Anchor::BelowLeft.is_outside());
/// assert_eq!(core::mem::size_of::<Anchor>(), 1);
/// ```
///
/// A child's own alignment is always inside its parent: an outside anchor is not an [`Align`].
///
/// ```compile_fail
/// use twine_style::{Anchor, Style, style};
/// static S: Style = style! { align: Anchor::BelowLeft };
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Anchor {
    /// Inside the base's content area, aligned like a child.
    Inside(Align),
    /// Above the base, left edges lined up.
    #[doc(alias = "OutTopLeft")]
    #[doc(alias = "LV_ALIGN_OUT_TOP_LEFT")]
    AboveLeft,
    /// Above the base, centered horizontally.
    #[doc(alias = "OutTopMid")]
    #[doc(alias = "LV_ALIGN_OUT_TOP_MID")]
    AboveMid,
    /// Above the base, right edges lined up.
    #[doc(alias = "OutTopRight")]
    #[doc(alias = "LV_ALIGN_OUT_TOP_RIGHT")]
    AboveRight,
    /// Below the base, left edges lined up.
    #[doc(alias = "OutBottomLeft")]
    #[doc(alias = "LV_ALIGN_OUT_BOTTOM_LEFT")]
    BelowLeft,
    /// Below the base, centered horizontally.
    #[doc(alias = "OutBottomMid")]
    #[doc(alias = "LV_ALIGN_OUT_BOTTOM_MID")]
    BelowMid,
    /// Below the base, right edges lined up.
    #[doc(alias = "OutBottomRight")]
    #[doc(alias = "LV_ALIGN_OUT_BOTTOM_RIGHT")]
    BelowRight,
    /// Left of the base, top edges lined up.
    #[doc(alias = "OutLeftTop")]
    #[doc(alias = "LV_ALIGN_OUT_LEFT_TOP")]
    LeftTop,
    /// Left of the base, centered vertically.
    #[doc(alias = "OutLeftMid")]
    #[doc(alias = "LV_ALIGN_OUT_LEFT_MID")]
    LeftMid,
    /// Left of the base, bottom edges lined up.
    #[doc(alias = "OutLeftBottom")]
    #[doc(alias = "LV_ALIGN_OUT_LEFT_BOTTOM")]
    LeftBottom,
    /// Right of the base, top edges lined up.
    #[doc(alias = "OutRightTop")]
    #[doc(alias = "LV_ALIGN_OUT_RIGHT_TOP")]
    RightTop,
    /// Right of the base, centered vertically.
    #[doc(alias = "OutRightMid")]
    #[doc(alias = "LV_ALIGN_OUT_RIGHT_MID")]
    RightMid,
    /// Right of the base, bottom edges lined up.
    #[doc(alias = "OutRightBottom")]
    #[doc(alias = "LV_ALIGN_OUT_RIGHT_BOTTOM")]
    RightBottom,
}

impl Anchor {
    /// Whether the object is placed outside the base (not [`Anchor::Inside`]).
    #[inline]
    #[must_use]
    pub const fn is_outside(self) -> bool {
        !matches!(self, Anchor::Inside(_))
    }
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor::Inside(Align::Default)
    }
}

impl From<Align> for Anchor {
    #[inline]
    fn from(a: Align) -> Self {
        Anchor::Inside(a)
    }
}

/// A scroll axis: which directions a node scrolls in (`scroll_view`, `scroll_dir`).
///
/// Converts to [`Sides`] (`Horizontal` = left and right, …), the engine's per-direction set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Axis {
    /// Left and right (`LV_DIR_HOR`).
    #[doc(alias = "LV_DIR_HOR")]
    Horizontal,
    /// Up and down (`LV_DIR_VER`).
    #[doc(alias = "LV_DIR_VER")]
    Vertical,
    /// Every direction (`LV_DIR_ALL`, the default of scrollable nodes).
    #[default]
    #[doc(alias = "LV_DIR_ALL")]
    Both,
}

impl Axis {
    /// The directions of this axis as a [`Sides`] set.
    #[inline]
    #[must_use]
    pub const fn sides(self) -> Sides {
        match self {
            Axis::Horizontal => Sides::HORIZONTAL,
            Axis::Vertical => Sides::VERTICAL,
            Axis::Both => Sides::ALL,
        }
    }
}

/// One side or direction: where a dropdown list opens, a swipe gesture's direction, a tab
/// bar's position (LVGL `lv_dir_t` with a single bit set, same values).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[doc(alias = "Dir")]
#[doc(alias = "lv_dir_t")]
pub enum Side {
    /// Left (`LV_DIR_LEFT`).
    #[doc(alias = "LV_DIR_LEFT")]
    Left = 0x01,
    /// Right (`LV_DIR_RIGHT`).
    #[doc(alias = "LV_DIR_RIGHT")]
    Right = 0x02,
    /// Top, upwards (`LV_DIR_TOP`).
    #[doc(alias = "LV_DIR_TOP")]
    Top = 0x04,
    /// Bottom, downwards (`LV_DIR_BOTTOM`).
    #[doc(alias = "LV_DIR_BOTTOM")]
    Bottom = 0x08,
}

impl Side {
    /// Whether the side is `Top` or `Bottom`.
    #[inline]
    #[must_use]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Side::Top | Side::Bottom)
    }

    /// The opposite side.
    #[inline]
    #[must_use]
    pub const fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

bitflags::bitflags! {
    /// A set of sides/directions, where several are meaningful: the directions a node may be
    /// scrolled in (the engine's `scroll_dir`), the swipe directions a tile allows (LVGL
    /// `lv_dir_t`, same bit values).
    ///
    /// ```
    /// use twine_style::{Axis, Side, Sides};
    ///
    /// assert_eq!(Sides::from(Axis::Horizontal), Sides::LEFT | Sides::RIGHT);
    /// assert_eq!(Sides::from(Side::Top), Sides::TOP);
    /// assert!(Sides::ALL.contains(Sides::VERTICAL));
    /// ```
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    #[doc(alias = "Dir")]
    #[doc(alias = "lv_dir_t")]
    pub struct Sides: u8 {
        /// `LV_DIR_LEFT`
        const LEFT = 0x01;
        /// `LV_DIR_RIGHT`
        const RIGHT = 0x02;
        /// `LV_DIR_TOP`
        const TOP = 0x04;
        /// `LV_DIR_BOTTOM`
        const BOTTOM = 0x08;
        /// Left and right (`LV_DIR_HOR`).
        const HORIZONTAL = 0x03;
        /// Top and bottom (`LV_DIR_VER`).
        const VERTICAL = 0x0C;
        /// Every side (`LV_DIR_ALL`).
        const ALL = 0x0F;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Sides {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Sides({=u8:#x})", self.bits());
    }
}

impl Sides {
    /// The set holding only `side`.
    #[inline]
    #[must_use]
    pub const fn side(side: Side) -> Sides {
        Sides::from_bits_retain(side as u8)
    }

    /// Whether `side` is in the set.
    #[inline]
    #[must_use]
    pub const fn has(self, side: Side) -> bool {
        self.bits() & side as u8 != 0
    }
}

impl From<Side> for Sides {
    #[inline]
    fn from(s: Side) -> Self {
        Sides::side(s)
    }
}

impl From<Axis> for Sides {
    #[inline]
    fn from(a: Axis) -> Self {
        a.sides()
    }
}

/// Base text direction (LVGL `lv_base_dir_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BaseDir {
    /// `LV_BASE_DIR_LTR` (the style default, as in LVGL).
    #[default]
    Ltr = 0x00,
    /// `LV_BASE_DIR_RTL`
    Rtl = 0x01,
    /// `LV_BASE_DIR_AUTO`: detected from the text.
    Auto = 0x02,
    /// `LV_BASE_DIR_NEUTRAL`
    Neutral = 0x20,
    /// `LV_BASE_DIR_WEAK`
    Weak = 0x21,
}

/// The main axis of a flex container: items are placed in a row or in a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FlexDirection {
    /// Left to right (right to left with a right-to-left base direction).
    #[default]
    Row,
    /// Top to bottom.
    Column,
}

/// Flex main axis, wrapping and order (the `flex_flow` style property): built from a
/// [`FlexDirection`] with [`wrap`](Self::wrap) and [`reverse`](Self::reverse).
///
/// Stored in LVGL's `lv_flex_flow_t` encoding (bit 0 = column, bit 2 = wrap, bit 3 = reverse),
/// so every builder is a `const fn` bit operation and the layout reads the same byte as before.
///
/// ```
/// use twine_style::{FlexDirection, FlexFlow};
///
/// const F: FlexFlow = FlexFlow::new(FlexDirection::Column).wrap(true);
/// assert!(F.is_column() && F.is_wrap() && !F.is_reverse());
/// assert_eq!(FlexFlow::ROW.reverse(true).direction(), FlexDirection::Row);
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[doc(alias = "lv_flex_flow_t")]
pub struct FlexFlow(u8);

/// Written as the builder expression, e.g. `COLUMN.wrap(true)`.
impl core::fmt::Debug for FlexFlow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(if self.is_column() { "COLUMN" } else { "ROW" })?;
        if self.is_wrap() {
            f.write_str(".wrap(true)")?;
        }
        if self.is_reverse() {
            f.write_str(".reverse(true)")?;
        }
        Ok(())
    }
}

impl FlexFlow {
    const COLUMN_BIT: u8 = 0x01;
    const WRAP_BIT: u8 = 0x04;
    const REVERSE_BIT: u8 = 0x08;

    /// A row, no wrapping, in child order (`LV_FLEX_FLOW_ROW`, the default).
    #[doc(alias = "LV_FLEX_FLOW_ROW")]
    pub const ROW: FlexFlow = FlexFlow(0);
    /// A column, no wrapping, in child order (`LV_FLEX_FLOW_COLUMN`).
    #[doc(alias = "LV_FLEX_FLOW_COLUMN")]
    pub const COLUMN: FlexFlow = FlexFlow(Self::COLUMN_BIT);

    /// A flow along `direction`, no wrapping, in child order.
    #[inline]
    #[must_use]
    pub const fn new(direction: FlexDirection) -> Self {
        match direction {
            FlexDirection::Row => Self::ROW,
            FlexDirection::Column => Self::COLUMN,
        }
    }

    const fn with(self, bit: u8, on: bool) -> Self {
        FlexFlow(if on { self.0 | bit } else { self.0 & !bit })
    }

    /// Items wrap into several tracks when they do not fit (`LV_FLEX_FLOW_*_WRAP`).
    #[inline]
    #[must_use]
    pub const fn wrap(self, on: bool) -> Self {
        self.with(Self::WRAP_BIT, on)
    }

    /// Items are placed from the last child to the first (`LV_FLEX_FLOW_*_REVERSE`).
    #[inline]
    #[must_use]
    pub const fn reverse(self, on: bool) -> Self {
        self.with(Self::REVERSE_BIT, on)
    }

    /// The same wrapping and order along `direction`.
    #[inline]
    #[must_use]
    pub const fn with_direction(self, direction: FlexDirection) -> Self {
        self.with(Self::COLUMN_BIT, matches!(direction, FlexDirection::Column))
    }

    /// The main axis.
    #[inline]
    #[must_use]
    pub const fn direction(self) -> FlexDirection {
        if self.is_column() {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        }
    }

    /// Main axis is vertical.
    #[inline]
    #[must_use]
    pub const fn is_column(self) -> bool {
        self.0 & Self::COLUMN_BIT != 0
    }

    /// Items wrap into several tracks.
    #[inline]
    #[must_use]
    pub const fn is_wrap(self) -> bool {
        self.0 & Self::WRAP_BIT != 0
    }

    /// Items are placed in reverse order.
    #[inline]
    #[must_use]
    pub const fn is_reverse(self) -> bool {
        self.0 & Self::REVERSE_BIT != 0
    }

    /// The LVGL `lv_flex_flow_t` value.
    #[inline]
    #[must_use]
    pub const fn to_lvgl(self) -> u8 {
        self.0
    }
}

impl From<FlexDirection> for FlexFlow {
    #[inline]
    fn from(d: FlexDirection) -> Self {
        FlexFlow::new(d)
    }
}

/// Placement of flex items along the main axis (`flex_main_align`, `justify`) and of the tracks
/// of a wrapping flex container (`flex_track_align`, `align_content`) — LVGL
/// `lv_flex_align_t`, same discriminants.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[doc(alias = "FlexAlign")]
#[doc(alias = "lv_flex_align_t")]
pub enum MainAlign {
    /// At the start.
    #[default]
    #[doc(alias = "LV_FLEX_ALIGN_START")]
    Start = 0,
    /// At the end.
    #[doc(alias = "LV_FLEX_ALIGN_END")]
    End = 1,
    /// Centered.
    #[doc(alias = "LV_FLEX_ALIGN_CENTER")]
    Center = 2,
    /// Equal space before, between and after the items.
    #[doc(alias = "LV_FLEX_ALIGN_SPACE_EVENLY")]
    SpaceEvenly = 3,
    /// Equal space around every item (half of it at the edges).
    #[doc(alias = "LV_FLEX_ALIGN_SPACE_AROUND")]
    SpaceAround = 4,
    /// Equal space between the items, none at the edges.
    #[doc(alias = "LV_FLEX_ALIGN_SPACE_BETWEEN")]
    SpaceBetween = 5,
}

/// Placement of flex items across the main axis, inside their track (`flex_cross_align`,
/// `align_items`) — LVGL's cross placement, which has no distributed modes. Same codes as the
/// matching [`MainAlign`] values.
///
/// ```compile_fail
/// use twine_style::CrossAlign;
/// let _ = CrossAlign::SpaceBetween; // distributing space is a main-axis/track placement
/// ```
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CrossAlign {
    /// At the start of the track.
    #[default]
    #[doc(alias = "LV_FLEX_ALIGN_START")]
    Start = 0,
    /// At the end of the track.
    #[doc(alias = "LV_FLEX_ALIGN_END")]
    End = 1,
    /// Centered in the track.
    #[doc(alias = "LV_FLEX_ALIGN_CENTER")]
    Center = 2,
}

impl From<CrossAlign> for MainAlign {
    #[inline]
    fn from(a: CrossAlign) -> Self {
        a.to_main()
    }
}

impl CrossAlign {
    /// The same placement as a [`MainAlign`] (e.g. for the tracks).
    #[inline]
    #[must_use]
    pub const fn to_main(self) -> MainAlign {
        match self {
            CrossAlign::Start => MainAlign::Start,
            CrossAlign::End => MainAlign::End,
            CrossAlign::Center => MainAlign::Center,
        }
    }
}

/// One grid column or row size (LVGL grid template values: pixels, `LV_GRID_CONTENT`,
/// `LV_GRID_FR(x)`; templates are slices or vectors, so no `LV_GRID_TEMPLATE_LAST`
/// terminator). Lists are written with [`grid_tracks!`](crate::grid_tracks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GridTrack {
    /// Fixed size in pixels.
    Px(i32),
    /// As large as the largest item in the track (`LV_GRID_CONTENT`).
    #[doc(alias = "LV_GRID_CONTENT")]
    Content,
    /// Share of the free space (`LV_GRID_FR(x)`).
    #[doc(alias = "LV_GRID_FR")]
    Fr(u8),
}

/// A list of grid tracks, `Vec<GridTrack>`: `fr(k)` is [`GridTrack::Fr`], `px(n)`
/// [`GridTrack::Px`], `content` [`GridTrack::Content`], and any other expression is used as a
/// `GridTrack`.
///
/// ```
/// use twine_style::{GridTrack, grid_tracks};
///
/// let w = 80;
/// let t = grid_tracks![fr(1), px(w), content, GridTrack::Fr(2)];
/// assert_eq!(t, vec![GridTrack::Fr(1), GridTrack::Px(80), GridTrack::Content, GridTrack::Fr(2)]);
/// assert!(grid_tracks![].is_empty());
/// ```
#[macro_export]
macro_rules! grid_tracks {
    () => { $crate::__private::Vec::<$crate::GridTrack>::new() };
    ($($t:tt)+) => { $crate::__grid_tracks!([] $($t)+) };
}

/// The muncher of [`grid_tracks!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __grid_tracks {
    ([$($acc:expr),*]) => {
        $crate::__private::Vec::<$crate::GridTrack>::from([$($acc),*])
    };
    ([$($acc:expr),*] content $(, $($rest:tt)*)?) => {
        $crate::__grid_tracks!([$($acc,)* $crate::GridTrack::Content] $($($rest)*)?)
    };
    ([$($acc:expr),*] fr ($v:expr) $(, $($rest:tt)*)?) => {
        $crate::__grid_tracks!([$($acc,)* $crate::GridTrack::Fr($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] px ($v:expr) $(, $($rest:tt)*)?) => {
        $crate::__grid_tracks!([$($acc,)* $crate::GridTrack::Px($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] $v:expr $(, $($rest:tt)*)?) => {
        $crate::__grid_tracks!([$($acc,)* { let t: $crate::GridTrack = $v; t }] $($($rest)*)?)
    };
}

/// A grid item's column or row range (`grid_col`, `grid_row`): the first track and the number
/// of tracks it spans. An integer is one track; a range `a..b` spans `b - a` tracks from `a`,
/// `a..=b` includes `b`.
///
/// Out-of-template values are clamped by the layout with a warning (like LVGL).
///
/// ```
/// use twine_style::GridSpan;
///
/// assert_eq!(GridSpan::from(2), GridSpan::new(2, 1));
/// assert_eq!(GridSpan::from(0..2), GridSpan { start: 0, span: 2 });
/// assert_eq!(GridSpan::from(1..=3), GridSpan::new(1, 3));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct GridSpan {
    /// The first track (0-based; `grid_cell_column`/`grid_cell_row`).
    pub start: i32,
    /// The number of tracks (`grid_cell_column_span`/`grid_cell_row_span`).
    pub span: i32,
}

impl GridSpan {
    /// `span` tracks from `start`.
    #[inline]
    #[must_use]
    pub const fn new(start: i32, span: i32) -> Self {
        Self { start, span }
    }

    /// The one track `index`.
    #[inline]
    #[must_use]
    pub const fn cell(index: i32) -> Self {
        Self {
            start: index,
            span: 1,
        }
    }

    /// The tracks `r.start..r.end` (usable in `const` items and `style!`).
    #[inline]
    #[must_use]
    pub const fn range(r: core::ops::Range<i32>) -> Self {
        Self {
            start: r.start,
            span: r.end.saturating_sub(r.start),
        }
    }
}

impl Default for GridSpan {
    fn default() -> Self {
        Self::cell(0)
    }
}

macro_rules! grid_span_from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for GridSpan {
            #[inline]
            fn from(i: $t) -> Self {
                GridSpan::cell(i32::try_from(i).unwrap_or(i32::MAX))
            }
        }

        impl From<core::ops::Range<$t>> for GridSpan {
            #[inline]
            fn from(r: core::ops::Range<$t>) -> Self {
                let start = i32::try_from(r.start).unwrap_or(i32::MAX);
                let end = i32::try_from(r.end).unwrap_or(i32::MAX);
                GridSpan::new(start, end.saturating_sub(start))
            }
        }

        impl From<core::ops::RangeInclusive<$t>> for GridSpan {
            #[inline]
            fn from(r: core::ops::RangeInclusive<$t>) -> Self {
                let start = i32::try_from(*r.start()).unwrap_or(i32::MAX);
                let end = i32::try_from(*r.end()).unwrap_or(i32::MAX);
                GridSpan::new(start, end.saturating_sub(start).saturating_add(1))
            }
        }
    )*};
}

grid_span_from_int!(i32, u8, u16, u32, usize);

/// Alignment of grid tracks and cells (LVGL `lv_grid_align_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GridAlign {
    /// `LV_GRID_ALIGN_START`
    #[default]
    Start = 0,
    /// `LV_GRID_ALIGN_CENTER`
    Center = 1,
    /// `LV_GRID_ALIGN_END`
    End = 2,
    /// `LV_GRID_ALIGN_STRETCH`
    Stretch = 3,
    /// `LV_GRID_ALIGN_SPACE_EVENLY`
    SpaceEvenly = 4,
    /// `LV_GRID_ALIGN_SPACE_AROUND`
    SpaceAround = 5,
    /// `LV_GRID_ALIGN_SPACE_BETWEEN`
    SpaceBetween = 6,
}

/// Layout engine that places the children (LVGL `lv_layout_t`: `LV_LAYOUT_NONE/FLEX/GRID`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LayoutKind {
    /// Children are positioned by their own `X`/`Y`/`Align`.
    #[default]
    None = 0,
    /// Flexbox.
    Flex = 1,
    /// Grid.
    Grid = 2,
}

/// Background gradient direction (LVGL `lv_grad_dir_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GradDir {
    /// `LV_GRAD_DIR_NONE`: no gradient (`BgGradColor` ignored).
    #[default]
    None = 0,
    /// `LV_GRAD_DIR_VER`: top to bottom.
    Ver = 1,
    /// `LV_GRAD_DIR_HOR`: left to right.
    Hor = 2,
    /// `LV_GRAD_DIR_LINEAR` (needs `BgGrad`).
    Linear = 3,
    /// `LV_GRAD_DIR_RADIAL` (needs `BgGrad`).
    Radial = 4,
    /// `LV_GRAD_DIR_CONICAL` (needs `BgGrad`).
    Conical = 5,
}

/// When scrollbars are shown (LVGL `lv_scrollbar_mode_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ScrollbarMode {
    /// `LV_SCROLLBAR_MODE_OFF`: never.
    Off = 0,
    /// `LV_SCROLLBAR_MODE_ON`: always.
    On = 1,
    /// `LV_SCROLLBAR_MODE_ACTIVE`: while scrolling.
    Active = 2,
    /// `LV_SCROLLBAR_MODE_AUTO`: when the content is larger than the object (LVGL's default).
    #[default]
    Auto = 3,
}

/// Where snappable children align when scrolling stops (LVGL `lv_scroll_snap_t`, same
/// discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ScrollSnap {
    /// `LV_SCROLL_SNAP_NONE`
    #[default]
    None = 0,
    /// `LV_SCROLL_SNAP_START`
    Start = 1,
    /// `LV_SCROLL_SNAP_END`
    End = 2,
    /// `LV_SCROLL_SNAP_CENTER`
    Center = 3,
}

/// Speed/precision trade-off of blurs (LVGL `lv_blur_quality_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BlurQuality {
    /// `LV_BLUR_QUALITY_AUTO`
    #[default]
    Auto = 0,
    /// `LV_BLUR_QUALITY_SPEED`
    Speed = 1,
    /// `LV_BLUR_QUALITY_PRECISION`
    Precision = 2,
}

/// Removes the empty space above/below text based on font metrics, like CSS `text-box-trim`
/// (LVGL `lv_text_leading_trim_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TextLeadingTrim {
    /// `LV_TEXT_LEADING_TRIM_NONE`
    #[default]
    None = 0,
    /// `LV_TEXT_LEADING_TRIM_CAPITAL_BASELINE`
    CapitalBaseline = 1,
    /// `LV_TEXT_LEADING_TRIM_LOWER_BASELINE`
    LowerBaseline = 2,
    /// `LV_TEXT_LEADING_TRIM_CAPITAL`
    Capital = 3,
    /// `LV_TEXT_LEADING_TRIM_LOWER`
    Lower = 4,
}

/// Image pixels with a color in `low..=high` (per channel) become transparent (LVGL
/// `lv_image_colorkey_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ImageColorkey {
    /// Lower bound (inclusive).
    pub low: Color,
    /// Upper bound (inclusive).
    pub high: Color,
}

/// A color filter applied to the colors a part draws (LVGL `lv_color_filter_dsc_t`): the
/// function receives the color and the `ColorFilterOpa` intensity.
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::ColorFilter;
///
/// assert_eq!((ColorFilter::SHADE.filter)(Color::WHITE, Opa::COVER), Color::BLACK);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ColorFilter {
    /// Maps a color with an intensity to the filtered color.
    pub filter: fn(Color, Opa) -> Color,
}

impl ColorFilter {
    /// Darkens by the intensity (LVGL `lv_color_filter_shade`).
    pub const SHADE: ColorFilter = ColorFilter {
        filter: Color::darken,
    };
}

/// Conversion of enum-like style values to and from the compact `StyleValue::Enum(u8)` form.
///
/// The code is the Rust discriminant for fieldless enums and the bits for flag sets; it is an
/// internal encoding (LVGL numbering is kept where the enum mirrors an LVGL enum).
pub trait StyleEnum: Copy + Sized {
    /// The compact code.
    fn to_code(self) -> u8;
    /// The value of a code, `None` for codes no value maps to.
    fn from_code(code: u8) -> Option<Self>;
}

macro_rules! style_enum {
    ($t:ty { $($v:path),* $(,)? }) => {
        impl StyleEnum for $t {
            fn to_code(self) -> u8 {
                self as u8
            }
            fn from_code(code: u8) -> Option<Self> {
                [$($v),*].into_iter().find(|v| *v as u8 == code)
            }
        }
    };
}

style_enum!(Align {
    Align::Default, Align::TopLeft, Align::TopMid, Align::TopRight, Align::BottomLeft, Align::BottomMid,
    Align::BottomRight, Align::LeftMid, Align::RightMid, Align::Center,
});
style_enum!(BaseDir { BaseDir::Ltr, BaseDir::Rtl, BaseDir::Auto, BaseDir::Neutral, BaseDir::Weak });
style_enum!(MainAlign {
    MainAlign::Start, MainAlign::End, MainAlign::Center, MainAlign::SpaceEvenly, MainAlign::SpaceAround,
    MainAlign::SpaceBetween,
});
style_enum!(CrossAlign { CrossAlign::Start, CrossAlign::End, CrossAlign::Center });

/// The code is the LVGL `lv_flex_flow_t` value; codes with other bits are invalid.
impl StyleEnum for FlexFlow {
    fn to_code(self) -> u8 {
        self.0
    }
    fn from_code(code: u8) -> Option<Self> {
        (code & !(Self::COLUMN_BIT | Self::WRAP_BIT | Self::REVERSE_BIT) == 0).then_some(FlexFlow(code))
    }
}

style_enum!(GridAlign {
    GridAlign::Start, GridAlign::Center, GridAlign::End, GridAlign::Stretch, GridAlign::SpaceEvenly,
    GridAlign::SpaceAround, GridAlign::SpaceBetween,
});
style_enum!(LayoutKind { LayoutKind::None, LayoutKind::Flex, LayoutKind::Grid });
style_enum!(GradDir { GradDir::None, GradDir::Ver, GradDir::Hor, GradDir::Linear, GradDir::Radial, GradDir::Conical });
style_enum!(ScrollbarMode { ScrollbarMode::Off, ScrollbarMode::On, ScrollbarMode::Active, ScrollbarMode::Auto });
style_enum!(ScrollSnap { ScrollSnap::None, ScrollSnap::Start, ScrollSnap::End, ScrollSnap::Center });
style_enum!(BlurQuality { BlurQuality::Auto, BlurQuality::Speed, BlurQuality::Precision });
style_enum!(TextLeadingTrim {
    TextLeadingTrim::None, TextLeadingTrim::CapitalBaseline, TextLeadingTrim::LowerBaseline,
    TextLeadingTrim::Capital, TextLeadingTrim::Lower,
});
style_enum!(TextAlign { TextAlign::Left, TextAlign::Center, TextAlign::Right, TextAlign::Auto });
style_enum!(BlendMode {
    BlendMode::Normal, BlendMode::Additive, BlendMode::Subtractive, BlendMode::Multiply, BlendMode::Difference,
});

impl StyleEnum for BorderSide {
    fn to_code(self) -> u8 {
        self.0
    }
    fn from_code(code: u8) -> Option<Self> {
        (code & !(BorderSide::FULL.0 | BorderSide::INTERNAL.0) == 0).then_some(BorderSide(code))
    }
}

impl StyleEnum for TextDecor {
    fn to_code(self) -> u8 {
        self.bits()
    }
    fn from_code(code: u8) -> Option<Self> {
        TextDecor::from_bits(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_resolve_px_pct_content() {
        const L: crate::design::LengthValue = __LengthArg(12).get();
        const P: crate::design::LengthValue = __LengthArg(Length::pct(5)).get();
        assert_eq!(Length::Px(-5).resolve(100, 7), -5);
        assert_eq!(Length::pct(50).resolve(201, 7), 100); // truncated like lv_pct_to_px
        assert_eq!(Length::pct(-50).resolve(201, 7), -100);
        assert_eq!(Length::pct(150).resolve(100, 7), 150);
        assert_eq!(Length::pct(i16::MAX).resolve(i32::MAX, 0), i32::MAX); // saturates
        assert_eq!(Length::Content.resolve(100, 7), 7);
        assert_eq!(Length::from_i32(3), Length::Px(3));
        assert_eq!(Length::default(), Length::Px(0));
        assert_eq!(Length::Px(4).as_px(), Some(4));
        assert_eq!(Length::Content.as_px(), None);
        assert_eq!((L, P), (Length::Px(12).into(), Length::Pct(5).into()));
    }

    #[test]
    fn align_variants_match_lvgl_discriminants() {
        // lv_align_t (LVGL v9.6.0 include/lvgl/core/lv_area.h), in declaration order from 0;
        // the OUT_* values (10..=21) are `Anchor`s.
        let lvgl = [
            Align::Default,
            Align::TopLeft,
            Align::TopMid,
            Align::TopRight,
            Align::BottomLeft,
            Align::BottomMid,
            Align::BottomRight,
            Align::LeftMid,
            Align::RightMid,
            Align::Center,
        ];
        for (i, a) in lvgl.into_iter().enumerate() {
            assert_eq!(a as usize, i, "{a:?}");
            assert_eq!(Align::from_code(i as u8), Some(a));
        }
        assert_eq!(Align::from_code(10), None);
        assert_eq!(BaseDir::Neutral as u8, 0x20);
        assert_eq!(GridAlign::SpaceBetween as u8, 6);
        assert_eq!(GradDir::Conical as u8, 5);
        assert_eq!(MainAlign::SpaceBetween as u8, 5);
        assert_eq!(core::mem::size_of::<Anchor>(), 1);
        assert_eq!(Anchor::default(), Anchor::Inside(Align::Default));
        assert!(!Anchor::Inside(Align::Center).is_outside() && Anchor::RightBottom.is_outside());
    }

    /// The eight LVGL `lv_flex_flow_t` values and the builder expression that replaces each.
    #[test]
    fn flex_flow_builder_matches_the_eight_lvgl_flows() {
        use FlexDirection::{Column, Row};
        let table: [(u8, FlexFlow); 8] = [
            (0x00, FlexFlow::new(Row)),                             // LV_FLEX_FLOW_ROW
            (0x01, FlexFlow::new(Column)),                          // LV_FLEX_FLOW_COLUMN
            (0x04, FlexFlow::new(Row).wrap(true)),                  // LV_FLEX_FLOW_ROW_WRAP
            (0x08, FlexFlow::new(Row).reverse(true)),               // LV_FLEX_FLOW_ROW_REVERSE
            (0x0C, FlexFlow::new(Row).wrap(true).reverse(true)),    // LV_FLEX_FLOW_ROW_WRAP_REVERSE
            (0x05, FlexFlow::new(Column).wrap(true)),               // LV_FLEX_FLOW_COLUMN_WRAP
            (0x09, FlexFlow::new(Column).reverse(true)),            // LV_FLEX_FLOW_COLUMN_REVERSE
            (0x0D, FlexFlow::new(Column).reverse(true).wrap(true)), // LV_FLEX_FLOW_COLUMN_WRAP_REVERSE
        ];
        for (code, flow) in table {
            assert_eq!(flow.to_lvgl(), code, "{flow:?}");
            assert_eq!(flow.to_code(), code);
            assert_eq!(FlexFlow::from_code(code), Some(flow));
            assert_eq!(flow.is_column(), code & 1 != 0);
            assert_eq!(flow.is_wrap(), code & 4 != 0);
            assert_eq!(flow.is_reverse(), code & 8 != 0);
            assert_eq!(flow.wrap(false).reverse(false), FlexFlow::new(flow.direction()));
            assert_eq!(flow.with_direction(Row).direction(), Row);
        }
        assert_eq!(FlexFlow::default(), FlexFlow::ROW);
        assert_eq!(FlexFlow::COLUMN, FlexFlow::from(Column));
        assert_eq!(FlexFlow::from_code(0x02), None);
        assert_eq!(FlexFlow::from_code(0x10), None);
    }

    #[test]
    fn cross_align_codes_are_the_main_align_codes() {
        for (c, m) in [
            (CrossAlign::Start, MainAlign::Start),
            (CrossAlign::End, MainAlign::End),
            (CrossAlign::Center, MainAlign::Center),
        ] {
            assert_eq!(c.to_code(), m.to_code());
            assert_eq!(MainAlign::from(c), m);
        }
        // The distributed modes have no cross-axis meaning (they acted as `Start`).
        assert_eq!(CrossAlign::from_code(MainAlign::SpaceBetween.to_code()), None);
    }

    #[test]
    fn sides_values_match_lvgl_dir() {
        assert_eq!(Sides::empty().bits(), 0);
        assert_eq!(Sides::LEFT.bits(), 1);
        assert_eq!(Sides::RIGHT.bits(), 2);
        assert_eq!(Sides::TOP.bits(), 4);
        assert_eq!(Sides::BOTTOM.bits(), 8);
        assert_eq!(Sides::HORIZONTAL, Sides::LEFT | Sides::RIGHT);
        assert_eq!(Sides::VERTICAL, Sides::TOP | Sides::BOTTOM);
        assert_eq!(Sides::ALL, Sides::HORIZONTAL | Sides::VERTICAL);
        for s in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
            assert_eq!(Sides::from(s).bits(), s as u8);
            assert!(Sides::ALL.has(s) && !Sides::from(s.opposite()).has(s));
            assert_eq!(s.is_vertical(), Sides::VERTICAL.has(s));
        }
        assert_eq!(Sides::from(Axis::Horizontal), Sides::HORIZONTAL);
        assert_eq!(Sides::from(Axis::Vertical), Sides::VERTICAL);
        assert_eq!(Sides::from(Axis::Both), Sides::ALL);
    }

    #[test]
    fn grid_span_conversions() {
        assert_eq!(GridSpan::from(3u8), GridSpan::new(3, 1));
        assert_eq!(GridSpan::from(1usize..4), GridSpan::new(1, 3));
        assert_eq!(GridSpan::from(2u16..=2), GridSpan::new(2, 1));
        assert_eq!(GridSpan::range(0..2), GridSpan::from(0..2));
        let (a, b) = (3, 1);
        assert_eq!(GridSpan::from(a..b), GridSpan::new(3, -2)); // clamped by the layout
        assert_eq!(GridSpan::default(), GridSpan::cell(0));
    }

    #[test]
    fn enum_codes_roundtrip() {
        fn rt<T: StyleEnum + PartialEq + core::fmt::Debug>(v: T) {
            assert_eq!(T::from_code(v.to_code()), Some(v));
        }
        rt(TextAlign::Auto);
        rt(BlendMode::Difference);
        rt(BorderSide::LEFT | BorderSide::INTERNAL);
        rt(TextDecor::UNDERLINE | TextDecor::STRIKETHROUGH);
        rt(ScrollbarMode::Active);
        rt(ScrollSnap::Center);
        rt(BlurQuality::Precision);
        rt(TextLeadingTrim::Lower);
        rt(LayoutKind::Grid);
        assert_eq!(BorderSide::from_code(0x20), None);
        assert_eq!(TextAlign::from_code(9), None);
    }

    #[test]
    fn shade_filter_darkens() {
        assert_eq!(
            (ColorFilter::SHADE.filter)(Color::WHITE, Opa::TRANSP),
            Color::WHITE
        );
        assert_eq!(
            (ColorFilter::SHADE.filter)(Color::hex(0x0080_8080), Opa::COVER),
            Color::BLACK
        );
    }
}
