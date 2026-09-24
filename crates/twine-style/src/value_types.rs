//! Value types of style properties and layout enums (LVGL equivalents noted on every type).

use twine_core::{Color, Opa};

pub use twine_render::{BlendMode, BorderSide, Gradient, RADIUS_CIRCLE};
pub use twine_text::{TextAlign, TextDecor};

/// Largest coordinate a style may hold (LVGL `LV_COORD_MAX` = 2²⁹ − 1): the default of
/// `MaxWidth`/`MaxHeight`. Kept well below `i32::MAX` so layout arithmetic (sums of sizes,
/// paddings and margins) cannot overflow.
pub const COORD_MAX: i32 = (1 << 29) - 1;

/// A size or position that may depend on the parent or the content (LVGL coordinates with
/// `LV_PCT(x)` / `LV_SIZE_CONTENT` encoding).
///
/// ```
/// use twine_style::Length;
///
/// assert_eq!(Length::px(40).resolve(200, 10), 40);
/// assert_eq!(Length::pct(25).resolve(200, 10), 50);
/// assert_eq!(Length::Content.resolve(200, 10), 10);
/// assert_eq!(Length::from(7), Length::Px(7));
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

    /// `v` pixels (`const` counterpart of `From<i32>`).
    #[must_use]
    pub const fn from_i32(v: i32) -> Self {
        Length::Px(v)
    }

    /// The length in pixels: `Pct` is relative to `parent` (truncated like LVGL
    /// `lv_pct_to_px`, saturating), `Content` is `content`.
    #[must_use]
    pub const fn resolve(self, parent: i32, content: i32) -> i32 {
        match self {
            Length::Px(v) => v,
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

/// Converts `style!` values into [`Length`] in `const` context: integers become pixels, a
/// `Length` stays unchanged (inherent methods on two instantiations; no trait needed).
#[doc(hidden)]
pub struct __LengthArg<T>(pub T);

impl __LengthArg<i32> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> Length {
        Length::Px(self.0)
    }
}

impl __LengthArg<Length> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> Length {
        self.0
    }
}

/// Converts `style!` values into [`Scale`](twine_core::Scale) in `const` context: integers are
/// raw scale values (256 = 1.0, saturating to `0..=u16::MAX`), a `Scale` stays unchanged.
#[doc(hidden)]
pub struct __ScaleArg<T>(pub T);

impl __ScaleArg<i32> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> twine_core::Scale {
        twine_core::Scale(if self.0 < 0 {
            0
        } else if self.0 > u16::MAX as i32 {
            u16::MAX
        } else {
            self.0 as u16
        })
    }
}

impl __ScaleArg<twine_core::Scale> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> twine_core::Scale {
        self.0
    }
}

/// Alignment of an object relative to its parent or to another object (LVGL `lv_align_t`, same
/// discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Align {
    /// `LV_ALIGN_DEFAULT`: top-left, or top-right with right-to-left base direction.
    #[default]
    Default = 0,
    /// `LV_ALIGN_TOP_LEFT`
    TopLeft = 1,
    /// `LV_ALIGN_TOP_MID`
    TopMid = 2,
    /// `LV_ALIGN_TOP_RIGHT`
    TopRight = 3,
    /// `LV_ALIGN_BOTTOM_LEFT`
    BottomLeft = 4,
    /// `LV_ALIGN_BOTTOM_MID`
    BottomMid = 5,
    /// `LV_ALIGN_BOTTOM_RIGHT`
    BottomRight = 6,
    /// `LV_ALIGN_LEFT_MID`
    LeftMid = 7,
    /// `LV_ALIGN_RIGHT_MID`
    RightMid = 8,
    /// `LV_ALIGN_CENTER`
    Center = 9,
    /// `LV_ALIGN_OUT_TOP_LEFT`
    OutTopLeft = 10,
    /// `LV_ALIGN_OUT_TOP_MID`
    OutTopMid = 11,
    /// `LV_ALIGN_OUT_TOP_RIGHT`
    OutTopRight = 12,
    /// `LV_ALIGN_OUT_BOTTOM_LEFT`
    OutBottomLeft = 13,
    /// `LV_ALIGN_OUT_BOTTOM_MID`
    OutBottomMid = 14,
    /// `LV_ALIGN_OUT_BOTTOM_RIGHT`
    OutBottomRight = 15,
    /// `LV_ALIGN_OUT_LEFT_TOP`
    OutLeftTop = 16,
    /// `LV_ALIGN_OUT_LEFT_MID`
    OutLeftMid = 17,
    /// `LV_ALIGN_OUT_LEFT_BOTTOM`
    OutLeftBottom = 18,
    /// `LV_ALIGN_OUT_RIGHT_TOP`
    OutRightTop = 19,
    /// `LV_ALIGN_OUT_RIGHT_MID`
    OutRightMid = 20,
    /// `LV_ALIGN_OUT_RIGHT_BOTTOM`
    OutRightBottom = 21,
}

bitflags::bitflags! {
    /// Directions (LVGL `lv_dir_t`, same bit values), e.g. allowed scroll directions.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Dir: u8 {
        /// `LV_DIR_NONE`
        const NONE = 0x00;
        /// `LV_DIR_LEFT`
        const LEFT = 0x01;
        /// `LV_DIR_RIGHT`
        const RIGHT = 0x02;
        /// `LV_DIR_TOP`
        const TOP = 0x04;
        /// `LV_DIR_BOTTOM`
        const BOTTOM = 0x08;
        /// `LV_DIR_HOR` (left and right)
        const HOR = 0x03;
        /// `LV_DIR_VER` (top and bottom)
        const VER = 0x0C;
        /// `LV_DIR_ALL`
        const ALL = 0x0F;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Dir {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Dir({=u8:#x})", self.bits());
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

/// Flex main axis, wrapping and order (LVGL `lv_flex_flow_t`, same discriminants: bit 0 =
/// column, bit 2 = wrap, bit 3 = reverse).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FlexFlow {
    /// `LV_FLEX_FLOW_ROW`
    #[default]
    Row = 0x00,
    /// `LV_FLEX_FLOW_COLUMN`
    Column = 0x01,
    /// `LV_FLEX_FLOW_ROW_WRAP`
    RowWrap = 0x04,
    /// `LV_FLEX_FLOW_ROW_REVERSE`
    RowReverse = 0x08,
    /// `LV_FLEX_FLOW_ROW_WRAP_REVERSE`
    RowWrapReverse = 0x0C,
    /// `LV_FLEX_FLOW_COLUMN_WRAP`
    ColumnWrap = 0x05,
    /// `LV_FLEX_FLOW_COLUMN_REVERSE`
    ColumnReverse = 0x09,
    /// `LV_FLEX_FLOW_COLUMN_WRAP_REVERSE`
    ColumnWrapReverse = 0x0D,
}

impl FlexFlow {
    /// Main axis is vertical.
    #[must_use]
    pub const fn is_column(self) -> bool {
        self as u8 & 0x01 != 0
    }

    /// Items wrap into several tracks.
    #[must_use]
    pub const fn is_wrap(self) -> bool {
        self as u8 & 0x04 != 0
    }

    /// Items are placed in reverse order.
    #[must_use]
    pub const fn is_reverse(self) -> bool {
        self as u8 & 0x08 != 0
    }
}

/// Placement of flex items and tracks (LVGL `lv_flex_align_t`, same discriminants).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FlexAlign {
    /// `LV_FLEX_ALIGN_START`
    #[default]
    Start = 0,
    /// `LV_FLEX_ALIGN_END`
    End = 1,
    /// `LV_FLEX_ALIGN_CENTER`
    Center = 2,
    /// `LV_FLEX_ALIGN_SPACE_EVENLY`
    SpaceEvenly = 3,
    /// `LV_FLEX_ALIGN_SPACE_AROUND`
    SpaceAround = 4,
    /// `LV_FLEX_ALIGN_SPACE_BETWEEN`
    SpaceBetween = 5,
}

/// One grid column or row size (LVGL grid template values: pixels, `LV_GRID_CONTENT`,
/// `LV_GRID_FR(x)`; templates are slices, so no `LV_GRID_TEMPLATE_LAST` terminator).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GridTrack {
    /// Fixed size in pixels.
    Px(i32),
    /// As large as the largest item in the track (`LV_GRID_CONTENT`).
    Content,
    /// Share of the free space (`LV_GRID_FR(x)`).
    Fr(u8),
}

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
    Align::BottomRight, Align::LeftMid, Align::RightMid, Align::Center, Align::OutTopLeft, Align::OutTopMid,
    Align::OutTopRight, Align::OutBottomLeft, Align::OutBottomMid, Align::OutBottomRight, Align::OutLeftTop,
    Align::OutLeftMid, Align::OutLeftBottom, Align::OutRightTop, Align::OutRightMid, Align::OutRightBottom,
});
style_enum!(BaseDir { BaseDir::Ltr, BaseDir::Rtl, BaseDir::Auto, BaseDir::Neutral, BaseDir::Weak });
style_enum!(FlexFlow {
    FlexFlow::Row, FlexFlow::Column, FlexFlow::RowWrap, FlexFlow::RowReverse, FlexFlow::RowWrapReverse,
    FlexFlow::ColumnWrap, FlexFlow::ColumnReverse, FlexFlow::ColumnWrapReverse,
});
style_enum!(FlexAlign {
    FlexAlign::Start, FlexAlign::End, FlexAlign::Center, FlexAlign::SpaceEvenly, FlexAlign::SpaceAround,
    FlexAlign::SpaceBetween,
});
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

impl StyleEnum for Dir {
    fn to_code(self) -> u8 {
        self.bits()
    }
    fn from_code(code: u8) -> Option<Self> {
        Dir::from_bits(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_resolve_px_pct_content() {
        const L: Length = __LengthArg(12).get();
        const P: Length = __LengthArg(Length::pct(5)).get();
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
        assert_eq!((L, P), (Length::Px(12), Length::Pct(5)));
    }

    #[test]
    fn align_variants_match_lvgl_discriminants() {
        // lv_align_t (LVGL v9.6.0 include/lvgl/core/lv_area.h), in declaration order from 0.
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
            Align::OutTopLeft,
            Align::OutTopMid,
            Align::OutTopRight,
            Align::OutBottomLeft,
            Align::OutBottomMid,
            Align::OutBottomRight,
            Align::OutLeftTop,
            Align::OutLeftMid,
            Align::OutLeftBottom,
            Align::OutRightTop,
            Align::OutRightMid,
            Align::OutRightBottom,
        ];
        for (i, a) in lvgl.into_iter().enumerate() {
            assert_eq!(a as usize, i, "{a:?}");
            assert_eq!(Align::from_code(i as u8), Some(a));
        }
        assert_eq!(Align::from_code(22), None);
        assert_eq!(BaseDir::Neutral as u8, 0x20);
        assert_eq!(FlexFlow::ColumnWrapReverse as u8, 0x0D);
        assert!(FlexFlow::ColumnWrapReverse.is_column() && FlexFlow::RowWrap.is_wrap());
        assert!(FlexFlow::RowReverse.is_reverse() && !FlexFlow::Row.is_reverse());
        assert_eq!(GridAlign::SpaceBetween as u8, 6);
        assert_eq!(GradDir::Conical as u8, 5);
    }

    #[test]
    fn dir_bitflags_values() {
        assert_eq!(Dir::NONE.bits(), 0);
        assert_eq!(Dir::LEFT.bits(), 1);
        assert_eq!(Dir::RIGHT.bits(), 2);
        assert_eq!(Dir::TOP.bits(), 4);
        assert_eq!(Dir::BOTTOM.bits(), 8);
        assert_eq!(Dir::HOR, Dir::LEFT | Dir::RIGHT);
        assert_eq!(Dir::VER, Dir::TOP | Dir::BOTTOM);
        assert_eq!(Dir::ALL, Dir::HOR | Dir::VER);
        assert_eq!(Dir::from_code(0x0F), Some(Dir::ALL));
        assert_eq!(Dir::from_code(0x10), None);
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
