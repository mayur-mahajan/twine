//! The property catalogue: [`PropId`], [`StyleProp`], [`PropMeta`] and [`PROP_META`].
//!
//! Everything here is generated from one table (`define_props!` below) so the id enum, the
//! typed property enum, the metadata, the `StyleBuf` builder methods and the `style!` name
//! mapping cannot drift apart.
//!
//! The table mirrors LVGL v9.6.0 (`include/lvgl/core/lv_style.h` for the list,
//! `src/misc/lv_style.c` for the flags table `lv_style_builtin_prop_flag_lookup_table` and the
//! defaults of `lv_style_prop_get_default`). Where LVGL's generated doc comments disagree with
//! that code, the code wins.

use twine_anim::AnimTemplate;
use twine_core::{Angle, Color, Opa, Scale};
use twine_image::ImageSource;
use twine_text::Font;

use crate::StyleBuf;
use crate::transition::TransitionDsc;
use crate::value::{PropValue, StyleValue};
use crate::value_types::{
    Align, BaseDir, BlendMode, BlurQuality, BorderSide, COORD_MAX, ColorFilter, FlexAlign, FlexFlow, GradDir,
    Gradient, GridAlign, GridTrack, ImageColorkey, LayoutKind, Length, TextAlign, TextDecor, TextLeadingTrim,
};

bitflags::bitflags! {
    /// What a property change affects (LVGL `LV_STYLE_PROP_FLAG_*`, same bit values).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct PropFlags: u8 {
        /// Inherited from the parent when not set (`LV_STYLE_PROP_FLAG_INHERITABLE`).
        const INHERITABLE = 1 << 0;
        /// Changes the extra draw area around the object (`LV_STYLE_PROP_FLAG_EXT_DRAW_UPDATE`).
        const EXT_DRAW = 1 << 1;
        /// Needs a layout update (`LV_STYLE_PROP_FLAG_LAYOUT_UPDATE`).
        const LAYOUT = 1 << 2;
        /// Needs a layout update of the parent (`LV_STYLE_PROP_FLAG_PARENT_LAYOUT_UPDATE`).
        const PARENT_LAYOUT = 1 << 3;
        /// Affects layer handling (`LV_STYLE_PROP_FLAG_LAYER_UPDATE`).
        const LAYER = 1 << 4;
        /// Affects the object's transformation (`LV_STYLE_PROP_FLAG_TRANSFORM`).
        const TRANSFORM = 1 << 5;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for PropFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "PropFlags({=u8:#x})", self.bits());
    }
}

/// Static metadata of one property (see [`PROP_META`]).
#[derive(Clone, Copy, Debug)]
pub struct PropMeta {
    /// Property name (`"BgColor"`).
    pub name: &'static str,
    /// Name in `style!` and of the `StyleBuf` builder method (`"bg_color"`).
    pub snake_name: &'static str,
    /// Rust payload type of the [`StyleProp`] variant (`"Color"`).
    pub type_name: &'static str,
    /// Inherited from the parent's `Main` part when not set.
    pub inherited: bool,
    /// A change needs a layout update.
    pub layout: bool,
    /// A change may change the extra draw area (shadow, outline, transforms…).
    pub ext_draw: bool,
    /// Every LVGL flag of the property.
    pub flags: PropFlags,
    /// Value when no style sets the property (LVGL `lv_style_prop_get_default`). `TextFont`
    /// holds `twine_text::EMPTY_FONT`; resolution substitutes the caller's `StyleDefaults::font`.
    pub default: StyleValue,
    /// Lookup group, `id >> 4` (see `Style::has_group`).
    pub group: u8,
}

impl PropMeta {
    const fn new(
        name: &'static str,
        snake_name: &'static str,
        type_name: &'static str,
        flags: PropFlags,
        default: StyleValue,
        id: u8,
    ) -> Self {
        Self {
            name,
            snake_name,
            type_name,
            inherited: flags.contains(PropFlags::INHERITABLE),
            layout: flags.contains(PropFlags::LAYOUT),
            ext_draw: flags.contains(PropFlags::EXT_DRAW),
            flags,
            default,
            group: id >> 4,
        }
    }
}

/// Converts `style!` values of `Length`/`Scale` properties (integers or typed values) in
/// `const` context.
#[doc(hidden)]
#[macro_export]
macro_rules! __style_wrap {
    (len, $v:expr) => {
        $crate::__LengthArg($v).get()
    };
    (scale, $v:expr) => {
        $crate::__ScaleArg($v).get()
    };
    (val, $v:expr) => {
        $v
    };
}

macro_rules! define_props {
    (
        ($d:tt)
        $(
            $(#[doc = $doc:literal])*
            $name:ident ( $snake:ident ) : $ty:ty , $kind:ident , [ $($flag:ident)* ] , $default:expr ;
        )*
    ) => {
        /// Declaration order (first property = 1), used to number [`PropId`].
        #[allow(dead_code, clippy::enum_variant_names)]
        #[repr(u8)]
        enum Order { Invalid, $($name),* }

        /// Identifier of a style property: the fieldless discriminant of [`StyleProp`]
        /// (`1..=PROP_COUNT`; LVGL `lv_style_prop_t`, with Twine's own numbering).
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub enum PropId {
            $( $(#[doc = $doc])* $name = Order::$name as u8, )*
        }

        /// Number of style properties.
        pub const PROP_COUNT: usize = [$(PropId::$name),*].len();

        impl PropId {
            /// Every property, in id order.
            pub const ALL: [PropId; PROP_COUNT] = [$(PropId::$name),*];

            /// The property with id `v` (`None` for 0 and ids past [`PROP_COUNT`]).
            #[must_use]
            pub const fn from_u8(v: u8) -> Option<PropId> {
                if v == 0 || v as usize > PROP_COUNT {
                    None
                } else {
                    Some(Self::ALL[v as usize - 1])
                }
            }
        }

        /// One style property with its value: one variant per [`PropId`], with the natural
        /// payload type. `Copy`; lives in `static` [`Style`](crate::Style)s or in
        /// [`StyleBuf`]s.
        #[derive(Clone, Copy, Debug)]
        pub enum StyleProp {
            $( $(#[doc = $doc])* $name($ty), )*
        }

        impl StyleProp {
            /// The property's id.
            #[inline]
            #[must_use]
            pub const fn id(&self) -> PropId {
                match self {
                    $( StyleProp::$name(_) => PropId::$name, )*
                }
            }

            /// The value as a [`StyleValue`].
            #[inline]
            #[must_use]
            pub fn value(&self) -> StyleValue {
                match *self {
                    $( StyleProp::$name(v) => PropValue::into_value(v), )*
                }
            }

            /// Builds the property `id` from a value of the matching variant (`None` if the
            /// variant or enum code does not fit the property).
            #[must_use]
            pub fn from_value(id: PropId, v: StyleValue) -> Option<StyleProp> {
                match id {
                    $( PropId::$name => <$ty as PropValue>::from_value(v).map(StyleProp::$name), )*
                }
            }
        }

        /// Metadata of every property, indexed by `id as usize - 1` (use [`PropId::meta`]).
        pub static PROP_META: [PropMeta; PROP_COUNT] = [
            $(
                PropMeta::new(
                    stringify!($name),
                    stringify!($snake),
                    stringify!($ty),
                    PropFlags::empty()$(.union(PropFlags::$flag))*,
                    $default,
                    PropId::$name as u8,
                ),
            )*
        ];

        /// Builder methods, one per property (named like the `style!` keys).
        impl StyleBuf {
            $(
                #[doc = concat!("Sets `", stringify!($name), "` and returns the buffer (builder style).")]
                #[must_use]
                pub fn $snake(mut self, v: impl Into<$ty>) -> Self {
                    self.set(StyleProp::$name(v.into()));
                    self
                }
            )*
        }

        /// Maps a `style!` key to its `StyleProp` (generated from the property table).
        #[doc(hidden)]
        #[macro_export]
        macro_rules! __style_prop {
            $(
                ($snake, $d v:expr) => { $crate::StyleProp::$name($crate::__style_wrap!($kind, $d v)) };
            )*
            ($d other:ident, $d v:expr) => {
                compile_error!(concat!("unknown style property: ", stringify!($d other)))
            };
        }

        #[cfg(test)]
        pub(crate) mod generated_tests {
            use super::*;

            /// `(id, snake name)` of every row, for tests.
            pub(crate) const ROWS: &[(PropId, &str)] = &[$((PropId::$name, stringify!($snake))),*];
        }
    };
}

const fn e(code: u8) -> StyleValue {
    StyleValue::Enum(code)
}

const INT0: StyleValue = StyleValue::Int(0);
const PX0: StyleValue = StyleValue::Length(Length::Px(0));
const BLACK: StyleValue = StyleValue::Color(Color::BLACK);
const COVER: StyleValue = StyleValue::Opa(Opa::COVER);
const TRANSP: StyleValue = StyleValue::Opa(Opa::TRANSP);
const NONE: StyleValue = StyleValue::None;
const FALSE: StyleValue = StyleValue::Bool(false);

// Columns: name(style! key): payload type, style! conversion (`len`: integers become
// `Length::Px`, `scale`: integers become `Scale`, `val`: as is), [LVGL flags], default.
define_props! {
    ($)
    // ---- Size and position -------------------------------------------------------------
    /// Width (`LV_STYLE_WIDTH`). Default `Content` (LVGL: widget class dependent).
    Width(width): Length, len, [LAYOUT], StyleValue::Length(Length::Content);
    /// Minimal width (`LV_STYLE_MIN_WIDTH`); percent of the parent's content width.
    MinWidth(min_width): Length, len, [LAYOUT], PX0;
    /// Maximal width (`LV_STYLE_MAX_WIDTH`).
    MaxWidth(max_width): Length, len, [LAYOUT], StyleValue::Length(Length::Px(COORD_MAX));
    /// Height (`LV_STYLE_HEIGHT`). Default `Content` (LVGL: widget class dependent).
    Height(height): Length, len, [LAYOUT], StyleValue::Length(Length::Content);
    /// Minimal height (`LV_STYLE_MIN_HEIGHT`).
    MinHeight(min_height): Length, len, [LAYOUT], PX0;
    /// Maximal height (`LV_STYLE_MAX_HEIGHT`).
    MaxHeight(max_height): Length, len, [LAYOUT], StyleValue::Length(Length::Px(COORD_MAX));
    /// X position relative to the `Align` reference (`LV_STYLE_X`).
    X(x): Length, len, [LAYOUT], PX0;
    /// Y position relative to the `Align` reference (`LV_STYLE_Y`).
    Y(y): Length, len, [LAYOUT], PX0;
    /// Alignment in the parent (`LV_STYLE_ALIGN`).
    Align(align): Align, val, [LAYOUT], e(Align::Default as u8);
    /// Draws the object wider on both sides; percent of its width (`LV_STYLE_TRANSFORM_WIDTH`).
    TransformWidth(transform_width): Length, len, [EXT_DRAW TRANSFORM], PX0;
    /// Draws the object taller on both sides (`LV_STYLE_TRANSFORM_HEIGHT`).
    TransformHeight(transform_height): Length, len, [EXT_DRAW TRANSFORM], PX0;
    /// Moves the object after layout; percent of its width (`LV_STYLE_TRANSLATE_X`).
    TranslateX(translate_x): Length, len, [LAYOUT PARENT_LAYOUT], PX0;
    /// Moves the object after layout; percent of its height (`LV_STYLE_TRANSLATE_Y`).
    TranslateY(translate_y): Length, len, [LAYOUT PARENT_LAYOUT], PX0;
    /// Moves radial items (e.g. scale labels) outward (`LV_STYLE_TRANSLATE_RADIAL`).
    TranslateRadial(translate_radial): i32, val, [], INT0;
    /// Horizontal zoom, 256 = 1.0 (`LV_STYLE_TRANSFORM_SCALE_X`).
    TransformScaleX(transform_scale_x): Scale, scale, [EXT_DRAW LAYER TRANSFORM], StyleValue::Scale(Scale::ONE);
    /// Vertical zoom, 256 = 1.0 (`LV_STYLE_TRANSFORM_SCALE_Y`).
    TransformScaleY(transform_scale_y): Scale, scale, [EXT_DRAW LAYER TRANSFORM], StyleValue::Scale(Scale::ONE);
    /// Rotation in 0.1° (`LV_STYLE_TRANSFORM_ROTATION`).
    TransformRotation(transform_rotation): Angle, val, [EXT_DRAW LAYER TRANSFORM], StyleValue::Angle(Angle(0));
    /// Pivot of rotation/zoom from the left edge; percent of the width (`LV_STYLE_TRANSFORM_PIVOT_X`).
    TransformPivotX(transform_pivot_x): Length, len, [], PX0;
    /// Pivot of rotation/zoom from the top edge (`LV_STYLE_TRANSFORM_PIVOT_Y`).
    TransformPivotY(transform_pivot_y): Length, len, [], PX0;
    /// Horizontal skew in 0.1° (`LV_STYLE_TRANSFORM_SKEW_X`).
    TransformSkewX(transform_skew_x): Angle, val, [EXT_DRAW LAYER TRANSFORM], StyleValue::Angle(Angle(0));
    /// Vertical skew in 0.1° (`LV_STYLE_TRANSFORM_SKEW_Y`).
    TransformSkewY(transform_skew_y): Angle, val, [EXT_DRAW LAYER TRANSFORM], StyleValue::Angle(Angle(0));
    // ---- Padding and margin -------------------------------------------------------------
    /// Top padding (`LV_STYLE_PAD_TOP`).
    PadTop(pad_top): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Bottom padding (`LV_STYLE_PAD_BOTTOM`).
    PadBottom(pad_bottom): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Left padding (`LV_STYLE_PAD_LEFT`).
    PadLeft(pad_left): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Right padding (`LV_STYLE_PAD_RIGHT`).
    PadRight(pad_right): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Gap between rows (`LV_STYLE_PAD_ROW`).
    PadRow(pad_row): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Gap between columns (`LV_STYLE_PAD_COLUMN`).
    PadColumn(pad_column): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Radial padding of radial items (`LV_STYLE_PAD_RADIAL`).
    PadRadial(pad_radial): i32, val, [], INT0;
    /// Top margin, used by flex/grid placement (`LV_STYLE_MARGIN_TOP`).
    MarginTop(margin_top): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Bottom margin (`LV_STYLE_MARGIN_BOTTOM`).
    MarginBottom(margin_bottom): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Left margin (`LV_STYLE_MARGIN_LEFT`).
    MarginLeft(margin_left): i32, val, [EXT_DRAW LAYOUT], INT0;
    /// Right margin (`LV_STYLE_MARGIN_RIGHT`).
    MarginRight(margin_right): i32, val, [EXT_DRAW LAYOUT], INT0;
    // ---- Background -------------------------------------------------------------------
    /// Background color (`LV_STYLE_BG_COLOR`).
    BgColor(bg_color): Color, val, [], StyleValue::Color(Color::WHITE);
    /// Background opacity (`LV_STYLE_BG_OPA`).
    BgOpa(bg_opa): Opa, val, [], TRANSP;
    /// Gradient end color for `Ver`/`Hor` gradients (`LV_STYLE_BG_GRAD_COLOR`).
    BgGradColor(bg_grad_color): Color, val, [], BLACK;
    /// Simple gradient direction (`LV_STYLE_BG_GRAD_DIR`).
    BgGradDir(bg_grad_dir): GradDir, val, [], e(GradDir::None as u8);
    /// Where the gradient starts, 0..=255 (`LV_STYLE_BG_MAIN_STOP`).
    BgMainStop(bg_main_stop): i32, val, [], INT0;
    /// Where the gradient ends, 0..=255 (`LV_STYLE_BG_GRAD_STOP`).
    BgGradStop(bg_grad_stop): i32, val, [], StyleValue::Int(255);
    /// Opacity of the gradient's start color (`LV_STYLE_BG_MAIN_OPA`).
    BgMainOpa(bg_main_opa): Opa, val, [], COVER;
    /// Opacity of the gradient's end color (`LV_STYLE_BG_GRAD_OPA`).
    BgGradOpa(bg_grad_opa): Opa, val, [], COVER;
    /// Full gradient descriptor; overrides the simple gradient (`LV_STYLE_BG_GRAD`).
    BgGrad(bg_grad): &'static Gradient, val, [], NONE;
    /// Background image or symbol (`LV_STYLE_BG_IMAGE_SRC`).
    BgImageSrc(bg_image_src): &'static ImageSource, val, [EXT_DRAW], NONE;
    /// Background image opacity (`LV_STYLE_BG_IMAGE_OPA`).
    BgImageOpa(bg_image_opa): Opa, val, [], COVER;
    /// Background image recolor (`LV_STYLE_BG_IMAGE_RECOLOR`).
    BgImageRecolor(bg_image_recolor): Color, val, [], BLACK;
    /// Background image recolor intensity (`LV_STYLE_BG_IMAGE_RECOLOR_OPA`).
    BgImageRecolorOpa(bg_image_recolor_opa): Opa, val, [], TRANSP;
    /// Tile the background image (`LV_STYLE_BG_IMAGE_TILED`).
    BgImageTiled(bg_image_tiled): bool, val, [], FALSE;
    // ---- Border -----------------------------------------------------------------------
    /// Border color (`LV_STYLE_BORDER_COLOR`).
    BorderColor(border_color): Color, val, [], BLACK;
    /// Border opacity (`LV_STYLE_BORDER_OPA`).
    BorderOpa(border_opa): Opa, val, [], COVER;
    /// Border width (`LV_STYLE_BORDER_WIDTH`).
    BorderWidth(border_width): i32, val, [LAYOUT], INT0;
    /// Which sides get a border (`LV_STYLE_BORDER_SIDE`).
    BorderSide(border_side): BorderSide, val, [], e(BorderSide::FULL.0);
    /// Draw the border after the children (`LV_STYLE_BORDER_POST`).
    BorderPost(border_post): bool, val, [], FALSE;
    // ---- Outline ----------------------------------------------------------------------
    /// Outline width (`LV_STYLE_OUTLINE_WIDTH`).
    OutlineWidth(outline_width): i32, val, [EXT_DRAW], INT0;
    /// Outline color (`LV_STYLE_OUTLINE_COLOR`).
    OutlineColor(outline_color): Color, val, [], BLACK;
    /// Outline opacity (`LV_STYLE_OUTLINE_OPA`).
    OutlineOpa(outline_opa): Opa, val, [EXT_DRAW], COVER;
    /// Gap between the object and the outline (`LV_STYLE_OUTLINE_PAD`).
    OutlinePad(outline_pad): i32, val, [EXT_DRAW], INT0;
    // ---- Shadow -----------------------------------------------------------------------
    /// Shadow blur width (`LV_STYLE_SHADOW_WIDTH`).
    ShadowWidth(shadow_width): i32, val, [EXT_DRAW], INT0;
    /// Shadow horizontal offset (`LV_STYLE_SHADOW_OFFSET_X`).
    ShadowOffsetX(shadow_offset_x): i32, val, [EXT_DRAW], INT0;
    /// Shadow vertical offset (`LV_STYLE_SHADOW_OFFSET_Y`).
    ShadowOffsetY(shadow_offset_y): i32, val, [EXT_DRAW], INT0;
    /// Shadow spread (`LV_STYLE_SHADOW_SPREAD`).
    ShadowSpread(shadow_spread): i32, val, [EXT_DRAW], INT0;
    /// Shadow color (`LV_STYLE_SHADOW_COLOR`).
    ShadowColor(shadow_color): Color, val, [], BLACK;
    /// Shadow opacity (`LV_STYLE_SHADOW_OPA`).
    ShadowOpa(shadow_opa): Opa, val, [EXT_DRAW], COVER;
    // ---- Drop shadow (shadow of the drawn content) ------------------------------------
    /// Drop shadow blur radius (`LV_STYLE_DROP_SHADOW_RADIUS`).
    DropShadowRadius(drop_shadow_radius): i32, val, [EXT_DRAW], INT0;
    /// Drop shadow horizontal offset (`LV_STYLE_DROP_SHADOW_OFFSET_X`).
    DropShadowOffsetX(drop_shadow_offset_x): i32, val, [EXT_DRAW], INT0;
    /// Drop shadow vertical offset (`LV_STYLE_DROP_SHADOW_OFFSET_Y`).
    DropShadowOffsetY(drop_shadow_offset_y): i32, val, [EXT_DRAW], INT0;
    /// Drop shadow color (`LV_STYLE_DROP_SHADOW_COLOR`).
    DropShadowColor(drop_shadow_color): Color, val, [], BLACK;
    /// Drop shadow opacity (`LV_STYLE_DROP_SHADOW_OPA`).
    DropShadowOpa(drop_shadow_opa): Opa, val, [EXT_DRAW], TRANSP;
    /// Drop shadow blur quality (`LV_STYLE_DROP_SHADOW_QUALITY`).
    DropShadowQuality(drop_shadow_quality): BlurQuality, val, [], e(BlurQuality::Precision as u8);
    // ---- Blur -------------------------------------------------------------------------
    /// Blur radius of the part (`LV_STYLE_BLUR_RADIUS`).
    BlurRadius(blur_radius): i32, val, [], INT0;
    /// Blur what is behind the part instead of the part itself (`LV_STYLE_BLUR_BACKDROP`).
    BlurBackdrop(blur_backdrop): bool, val, [], FALSE;
    /// Blur quality (`LV_STYLE_BLUR_QUALITY`).
    BlurQuality(blur_quality): BlurQuality, val, [], e(BlurQuality::Auto as u8);
    // ---- Image ------------------------------------------------------------------------
    /// Image opacity (`LV_STYLE_IMAGE_OPA`).
    ImageOpa(image_opa): Opa, val, [], COVER;
    /// Image recolor (`LV_STYLE_IMAGE_RECOLOR`).
    ImageRecolor(image_recolor): Color, val, [], BLACK;
    /// Image recolor intensity (`LV_STYLE_IMAGE_RECOLOR_OPA`).
    ImageRecolorOpa(image_recolor_opa): Opa, val, [], TRANSP;
    /// Colors made transparent in images (`LV_STYLE_IMAGE_COLORKEY`).
    ImageColorkey(image_colorkey): &'static ImageColorkey, val, [], NONE;
    // ---- Line -------------------------------------------------------------------------
    /// Line width (`LV_STYLE_LINE_WIDTH`).
    LineWidth(line_width): i32, val, [EXT_DRAW], INT0;
    /// Dash length (`LV_STYLE_LINE_DASH_WIDTH`).
    LineDashWidth(line_dash_width): i32, val, [], INT0;
    /// Gap between dashes (`LV_STYLE_LINE_DASH_GAP`).
    LineDashGap(line_dash_gap): i32, val, [], INT0;
    /// Rounded line ends (`LV_STYLE_LINE_ROUNDED`).
    LineRounded(line_rounded): bool, val, [], FALSE;
    /// Line color (`LV_STYLE_LINE_COLOR`).
    LineColor(line_color): Color, val, [], BLACK;
    /// Line opacity (`LV_STYLE_LINE_OPA`).
    LineOpa(line_opa): Opa, val, [], COVER;
    // ---- Arc --------------------------------------------------------------------------
    /// Arc width (`LV_STYLE_ARC_WIDTH`).
    ArcWidth(arc_width): i32, val, [EXT_DRAW], INT0;
    /// Rounded arc ends (`LV_STYLE_ARC_ROUNDED`).
    ArcRounded(arc_rounded): bool, val, [], FALSE;
    /// Arc color (`LV_STYLE_ARC_COLOR`).
    ArcColor(arc_color): Color, val, [], BLACK;
    /// Arc opacity (`LV_STYLE_ARC_OPA`).
    ArcOpa(arc_opa): Opa, val, [], COVER;
    /// Image drawn along the arc (`LV_STYLE_ARC_IMAGE_SRC`).
    ArcImageSrc(arc_image_src): &'static ImageSource, val, [], NONE;
    // ---- Text -------------------------------------------------------------------------
    /// Text color, inherited (`LV_STYLE_TEXT_COLOR`).
    TextColor(text_color): Color, val, [INHERITABLE], BLACK;
    /// Text opacity, inherited (`LV_STYLE_TEXT_OPA`).
    TextOpa(text_opa): Opa, val, [INHERITABLE], COVER;
    /// Font, inherited (`LV_STYLE_TEXT_FONT`); the default comes from `StyleDefaults::font`.
    TextFont(text_font): &'static Font, val, [INHERITABLE LAYOUT], StyleValue::Font(&twine_text::EMPTY_FONT);
    /// Extra space between letters, inherited (`LV_STYLE_TEXT_LETTER_SPACE`).
    TextLetterSpace(text_letter_space): i32, val, [INHERITABLE LAYOUT], INT0;
    /// Extra space between lines, inherited (`LV_STYLE_TEXT_LINE_SPACE`).
    TextLineSpace(text_line_space): i32, val, [INHERITABLE LAYOUT], INT0;
    /// Underline/strikethrough, inherited (`LV_STYLE_TEXT_DECOR`).
    TextDecor(text_decor): TextDecor, val, [INHERITABLE], e(TextDecor::empty().bits());
    /// Horizontal text alignment, inherited (`LV_STYLE_TEXT_ALIGN`).
    TextAlign(text_align): TextAlign, val, [INHERITABLE LAYOUT], e(TextAlign::Auto as u8);
    /// Text outline color (`LV_STYLE_TEXT_OUTLINE_STROKE_COLOR`).
    TextOutlineStrokeColor(text_outline_stroke_color): Color, val, [], BLACK;
    /// Text outline width (`LV_STYLE_TEXT_OUTLINE_STROKE_WIDTH`).
    TextOutlineStrokeWidth(text_outline_stroke_width): i32, val, [], INT0;
    /// Text outline opacity (`LV_STYLE_TEXT_OUTLINE_STROKE_OPA`).
    TextOutlineStrokeOpa(text_outline_stroke_opa): Opa, val, [], TRANSP;
    /// Trims the space above/below text by font metrics, inherited (`LV_STYLE_TEXT_LEADING_TRIM`).
    TextLeadingTrim(text_leading_trim): TextLeadingTrim, val, [INHERITABLE LAYOUT], e(TextLeadingTrim::None as u8);
    // ---- Miscellaneous ----------------------------------------------------------------
    /// Generic length, e.g. of scale ticks (`LV_STYLE_LENGTH`).
    Length(length): i32, val, [EXT_DRAW], INT0;
    /// Corner radius; `RADIUS_CIRCLE` for fully round (`LV_STYLE_RADIUS`).
    Radius(radius): i32, val, [], INT0;
    /// Offset of radial items (`LV_STYLE_RADIAL_OFFSET`).
    RadialOffset(radial_offset): i32, val, [], INT0;
    /// Clip children to the rounded corners (`LV_STYLE_CLIP_CORNER`).
    ClipCorner(clip_corner): bool, val, [], FALSE;
    /// Opacity of the part (`LV_STYLE_OPA`).
    Opa(opa): Opa, val, [], COVER;
    /// Opacity of the object and its children rendered as one layer (`LV_STYLE_OPA_LAYERED`).
    OpaLayered(opa_layered): Opa, val, [LAYER], COVER;
    /// Color filter, inherited (`LV_STYLE_COLOR_FILTER_DSC`).
    ColorFilterDsc(color_filter_dsc): &'static ColorFilter, val, [INHERITABLE], NONE;
    /// Color filter intensity, inherited (`LV_STYLE_COLOR_FILTER_OPA`).
    ColorFilterOpa(color_filter_opa): Opa, val, [INHERITABLE], TRANSP;
    /// Animation template used by some widgets (`LV_STYLE_ANIM`).
    Anim(anim): &'static AnimTemplate, val, [], NONE;
    /// Animation duration in ms used by some widgets (`LV_STYLE_ANIM_DURATION`).
    AnimDuration(anim_duration): u32, val, [], INT0;
    /// Transitions to run when entering the state (`LV_STYLE_TRANSITION`).
    Transition(transition): &'static TransitionDsc, val, [], NONE;
    /// How the part blends with what is below (`LV_STYLE_BLEND_MODE`).
    BlendMode(blend_mode): BlendMode, val, [LAYER], e(BlendMode::Normal as u8);
    /// Layout of the children (`LV_STYLE_LAYOUT`).
    Layout(layout): LayoutKind, val, [LAYOUT], e(LayoutKind::None as u8);
    /// Base text direction, inherited (`LV_STYLE_BASE_DIR`).
    BaseDir(base_dir): BaseDir, val, [INHERITABLE LAYOUT], e(BaseDir::Ltr as u8);
    /// A8/L8 image masking the object (`LV_STYLE_BITMAP_MASK_SRC`).
    BitmapMaskSrc(bitmap_mask_src): &'static ImageSource, val, [LAYER], NONE;
    /// Recolor of everything the part draws (`LV_STYLE_RECOLOR`).
    Recolor(recolor): Color, val, [], BLACK;
    /// Recolor intensity (`LV_STYLE_RECOLOR_OPA`).
    RecolorOpa(recolor_opa): Opa, val, [], TRANSP;
    /// Encoder rotation multiplier, 256 = 1.0 (`LV_STYLE_ROTARY_SENSITIVITY`).
    RotarySensitivity(rotary_sensitivity): u32, val, [], StyleValue::Int(256);
    // ---- Flex -------------------------------------------------------------------------
    /// Flex direction and wrapping (`LV_STYLE_FLEX_FLOW`).
    FlexFlow(flex_flow): FlexFlow, val, [LAYOUT], e(FlexFlow::Row as u8);
    /// Placement on the main axis (`LV_STYLE_FLEX_MAIN_PLACE`).
    FlexMainPlace(flex_main_place): FlexAlign, val, [LAYOUT], e(FlexAlign::Start as u8);
    /// Placement on the cross axis (`LV_STYLE_FLEX_CROSS_PLACE`).
    FlexCrossPlace(flex_cross_place): FlexAlign, val, [LAYOUT], e(FlexAlign::Start as u8);
    /// Placement of the tracks (`LV_STYLE_FLEX_TRACK_PLACE`).
    FlexTrackPlace(flex_track_place): FlexAlign, val, [LAYOUT], e(FlexAlign::Start as u8);
    /// Share of the free main-axis space (`LV_STYLE_FLEX_GROW`).
    FlexGrow(flex_grow): u8, val, [LAYOUT], INT0;
    // ---- Grid -------------------------------------------------------------------------
    /// Column template (`LV_STYLE_GRID_COLUMN_DSC_ARRAY`).
    GridColumnDscArray(grid_column_dsc_array): &'static [GridTrack], val, [LAYOUT], NONE;
    /// Row template (`LV_STYLE_GRID_ROW_DSC_ARRAY`).
    GridRowDscArray(grid_row_dsc_array): &'static [GridTrack], val, [LAYOUT], NONE;
    /// Column track alignment (`LV_STYLE_GRID_COLUMN_ALIGN`).
    GridColumnAlign(grid_column_align): GridAlign, val, [LAYOUT], e(GridAlign::Start as u8);
    /// Row track alignment (`LV_STYLE_GRID_ROW_ALIGN`).
    GridRowAlign(grid_row_align): GridAlign, val, [LAYOUT], e(GridAlign::Start as u8);
    /// Cell column (`LV_STYLE_GRID_CELL_COLUMN_POS`).
    GridCellColumnPos(grid_cell_column_pos): i32, val, [LAYOUT], INT0;
    /// Cell column span (`LV_STYLE_GRID_CELL_COLUMN_SPAN`).
    GridCellColumnSpan(grid_cell_column_span): i32, val, [LAYOUT], StyleValue::Int(1);
    /// Horizontal alignment in the cell (`LV_STYLE_GRID_CELL_X_ALIGN`).
    GridCellXAlign(grid_cell_x_align): GridAlign, val, [LAYOUT], e(GridAlign::Start as u8);
    /// Cell row (`LV_STYLE_GRID_CELL_ROW_POS`).
    GridCellRowPos(grid_cell_row_pos): i32, val, [LAYOUT], INT0;
    /// Cell row span (`LV_STYLE_GRID_CELL_ROW_SPAN`).
    GridCellRowSpan(grid_cell_row_span): i32, val, [LAYOUT], StyleValue::Int(1);
    /// Vertical alignment in the cell (`LV_STYLE_GRID_CELL_Y_ALIGN`).
    GridCellYAlign(grid_cell_y_align): GridAlign, val, [LAYOUT], e(GridAlign::Start as u8);
}

impl PropId {
    /// The property's metadata.
    #[inline]
    #[must_use]
    pub fn meta(self) -> &'static PropMeta {
        &PROP_META[self as usize - 1]
    }

    /// Lookup group: `id >> 4` (see [`Style::has_group`](crate::Style::has_group)).
    #[inline]
    #[must_use]
    pub const fn group(self) -> u8 {
        self as u8 >> 4
    }

    /// Bit of this property's group in a `has_group` mask.
    #[inline]
    #[must_use]
    pub const fn group_bit(self) -> u16 {
        1 << self.group()
    }

    /// Property name (`"BgColor"`).
    #[must_use]
    pub fn name(self) -> &'static str {
        self.meta().name
    }
}

impl core::fmt::Display for PropId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

// A style property must stay small (12 bytes on 32-bit targets): styles are scanned linearly
// and live in flash.
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<StyleProp>() <= 12);

#[cfg(test)]
mod tests {
    use super::generated_tests::ROWS;
    use super::*;

    #[test]
    fn prop_count_le_128() {
        // LVGL v9.6 has 129 built-in properties: more than 128, so the group mask is a `u16`
        // (groups 0..=8), see DEVIATIONS.
        assert_eq!(PROP_COUNT, 129);
        assert!(u8::try_from(PROP_COUNT).is_ok());
        assert!(
            usize::from(PropId::ALL[PROP_COUNT - 1].group()) < 16,
            "group mask is u16"
        );
        for (i, id) in PropId::ALL.iter().enumerate() {
            assert_eq!(*id as usize, i + 1);
            assert_eq!(PropId::from_u8(i as u8 + 1), Some(*id));
        }
        assert_eq!(PropId::from_u8(0), None);
        assert_eq!(PropId::from_u8(PROP_COUNT as u8 + 1), None);
    }

    /// A value of the right variant for every property, including those whose default is
    /// `None`.
    pub(crate) fn sample(id: PropId) -> StyleValue {
        static GRAD: Gradient = Gradient::new(twine_render::GradKind::Ver, &[]);
        static IMG: ImageSource = ImageSource::Symbol("s");
        static KEY: ImageColorkey = ImageColorkey {
            low: Color::BLACK,
            high: Color::WHITE,
        };
        static FILTER: ColorFilter = ColorFilter::SHADE;
        static ANIM: AnimTemplate =
            AnimTemplate::new(twine_core::Duration::ms(1), twine_anim::Easing::Linear);
        static TR: TransitionDsc = TransitionDsc::new(
            &[PropId::BgColor],
            twine_core::Duration::ms(1),
            twine_anim::Easing::Linear,
        );
        static TRACKS: [GridTrack; 2] = [GridTrack::Px(10), GridTrack::Fr(1)];
        match id.meta().type_name {
            "&'static Gradient" => StyleValue::Grad(&GRAD),
            "&'static ImageSource" => StyleValue::Image(&IMG),
            "&'static ImageColorkey" => StyleValue::Colorkey(&KEY),
            "&'static ColorFilter" => StyleValue::ColorFilter(&FILTER),
            "&'static AnimTemplate" => StyleValue::AnimTemplate(&ANIM),
            "&'static TransitionDsc" => StyleValue::Transition(&TR),
            "&'static [GridTrack]" => StyleValue::GridTracks(&TRACKS),
            _ => id.meta().default,
        }
    }

    #[test]
    fn every_prop_has_meta_and_roundtrips() {
        assert_eq!(ROWS.len(), PROP_COUNT);
        for &(id, snake) in ROWS {
            let m = id.meta();
            assert_eq!(m.snake_name, snake);
            assert_eq!(m.name, alloc::format!("{id:?}"));
            assert_eq!(m.group, id as u8 >> 4);
            let v = sample(id);
            assert!(!v.is_none(), "{id:?} has no sample");
            let p = StyleProp::from_value(id, v).unwrap_or_else(|| panic!("{id:?}: {v:?} does not fit"));
            assert_eq!(p.id(), id);
            assert_eq!(p.value(), v, "{id:?}");
            // A value of another kind is rejected.
            let wrong = if matches!(v, StyleValue::Bool(_)) {
                StyleValue::Int(0)
            } else {
                StyleValue::Bool(true)
            };
            assert!(
                StyleProp::from_value(id, wrong).is_none(),
                "{id:?} accepts {wrong:?}"
            );
        }
    }

    #[test]
    fn inherited_set_matches_lvgl_list() {
        // LVGL v9.6.0 src/misc/lv_style.c: properties with LV_STYLE_PROP_FLAG_INHERITABLE.
        let lvgl = [
            PropId::TextColor,
            PropId::TextOpa,
            PropId::TextFont,
            PropId::TextLetterSpace,
            PropId::TextLineSpace,
            PropId::TextDecor,
            PropId::TextAlign,
            PropId::ColorFilterDsc,
            PropId::ColorFilterOpa,
            PropId::BaseDir,
            PropId::TextLeadingTrim,
        ];
        for id in PropId::ALL {
            assert_eq!(id.meta().inherited, lvgl.contains(&id), "{id:?}");
        }
    }

    #[test]
    fn layout_and_ext_draw_flags_match_lvgl() {
        use PropFlags as F;
        // Copied from LVGL v9.6.0 src/misc/lv_style.c `lv_style_builtin_prop_flag_lookup_table`
        // (properties missing there have no flags).
        let lvgl: &[(PropId, PropFlags)] = &[
            (PropId::Width, F::LAYOUT),
            (PropId::MinWidth, F::LAYOUT),
            (PropId::MaxWidth, F::LAYOUT),
            (PropId::Height, F::LAYOUT),
            (PropId::MinHeight, F::LAYOUT),
            (PropId::MaxHeight, F::LAYOUT),
            (PropId::Length, F::EXT_DRAW),
            (PropId::X, F::LAYOUT),
            (PropId::Y, F::LAYOUT),
            (PropId::Align, F::LAYOUT),
            (PropId::TransformWidth, F::EXT_DRAW.union(F::TRANSFORM)),
            (PropId::TransformHeight, F::EXT_DRAW.union(F::TRANSFORM)),
            (PropId::TranslateX, F::LAYOUT.union(F::PARENT_LAYOUT)),
            (PropId::TranslateY, F::LAYOUT.union(F::PARENT_LAYOUT)),
            (
                PropId::TransformScaleX,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformScaleY,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformSkewX,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformSkewY,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformRotation,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (PropId::PadTop, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PadBottom, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PadLeft, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PadRight, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PadRow, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PadColumn, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginTop, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginBottom, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginLeft, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginRight, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::BgImageSrc, F::EXT_DRAW),
            (PropId::BorderWidth, F::LAYOUT),
            (PropId::OutlineWidth, F::EXT_DRAW),
            (PropId::OutlineOpa, F::EXT_DRAW),
            (PropId::OutlinePad, F::EXT_DRAW),
            (PropId::ShadowWidth, F::EXT_DRAW),
            (PropId::ShadowOffsetX, F::EXT_DRAW),
            (PropId::ShadowOffsetY, F::EXT_DRAW),
            (PropId::ShadowSpread, F::EXT_DRAW),
            (PropId::ShadowOpa, F::EXT_DRAW),
            (PropId::LineWidth, F::EXT_DRAW),
            (PropId::ArcWidth, F::EXT_DRAW),
            (PropId::TextColor, F::INHERITABLE),
            (PropId::TextOpa, F::INHERITABLE),
            (PropId::TextFont, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::TextLetterSpace, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::TextLineSpace, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::TextDecor, F::INHERITABLE),
            (PropId::TextAlign, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::OpaLayered, F::LAYER),
            (PropId::ColorFilterDsc, F::INHERITABLE),
            (PropId::ColorFilterOpa, F::INHERITABLE),
            (PropId::BlendMode, F::LAYER),
            (PropId::Layout, F::LAYOUT),
            (PropId::BaseDir, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::BitmapMaskSrc, F::LAYER),
            (PropId::DropShadowRadius, F::EXT_DRAW),
            (PropId::DropShadowOffsetX, F::EXT_DRAW),
            (PropId::DropShadowOffsetY, F::EXT_DRAW),
            (PropId::DropShadowOpa, F::EXT_DRAW),
            (PropId::FlexFlow, F::LAYOUT),
            (PropId::FlexMainPlace, F::LAYOUT),
            (PropId::FlexCrossPlace, F::LAYOUT),
            (PropId::FlexTrackPlace, F::LAYOUT),
            (PropId::FlexGrow, F::LAYOUT),
            (PropId::GridColumnDscArray, F::LAYOUT),
            (PropId::GridRowDscArray, F::LAYOUT),
            (PropId::GridColumnAlign, F::LAYOUT),
            (PropId::GridRowAlign, F::LAYOUT),
            (PropId::GridCellRowSpan, F::LAYOUT),
            (PropId::GridCellRowPos, F::LAYOUT),
            (PropId::GridCellColumnSpan, F::LAYOUT),
            (PropId::GridCellColumnPos, F::LAYOUT),
            (PropId::GridCellXAlign, F::LAYOUT),
            (PropId::GridCellYAlign, F::LAYOUT),
            (PropId::TextLeadingTrim, F::INHERITABLE.union(F::LAYOUT)),
        ];
        for id in PropId::ALL {
            let want = lvgl
                .iter()
                .find(|(p, _)| *p == id)
                .map_or(F::empty(), |(_, f)| *f);
            let m = id.meta();
            assert_eq!(m.flags, want, "{id:?}");
            assert_eq!(m.layout, want.contains(F::LAYOUT), "{id:?}");
            assert_eq!(m.ext_draw, want.contains(F::EXT_DRAW), "{id:?}");
        }
    }

    #[test]
    fn defaults_match_lvgl() {
        use StyleValue as V;
        // LVGL v9.6.0 src/misc/lv_style.c `lv_style_prop_get_default` (every other property: 0 /
        // NULL). Width/Height: LVGL takes them from the widget class; Twine defaults to Content.
        let non_zero: &[(PropId, StyleValue)] = &[
            (PropId::Width, V::Length(Length::Content)),
            (PropId::Height, V::Length(Length::Content)),
            (PropId::TransformScaleX, V::Scale(Scale(256))),
            (PropId::TransformScaleY, V::Scale(Scale(256))),
            (PropId::BgColor, V::Color(Color::WHITE)),
            (PropId::BgGradColor, V::Color(Color::BLACK)),
            (PropId::BorderColor, V::Color(Color::BLACK)),
            (PropId::ShadowColor, V::Color(Color::BLACK)),
            (PropId::OutlineColor, V::Color(Color::BLACK)),
            (PropId::ArcColor, V::Color(Color::BLACK)),
            (PropId::LineColor, V::Color(Color::BLACK)),
            (PropId::TextColor, V::Color(Color::BLACK)),
            (PropId::DropShadowColor, V::Color(Color::BLACK)),
            (PropId::ImageRecolor, V::Color(Color::BLACK)),
            (PropId::Recolor, V::Color(Color::BLACK)),
            (PropId::Opa, V::Opa(Opa::COVER)),
            (PropId::OpaLayered, V::Opa(Opa::COVER)),
            (PropId::BorderOpa, V::Opa(Opa::COVER)),
            (PropId::TextOpa, V::Opa(Opa::COVER)),
            (PropId::ImageOpa, V::Opa(Opa::COVER)),
            (PropId::BgGradOpa, V::Opa(Opa::COVER)),
            (PropId::BgMainOpa, V::Opa(Opa::COVER)),
            (PropId::BgImageOpa, V::Opa(Opa::COVER)),
            (PropId::OutlineOpa, V::Opa(Opa::COVER)),
            (PropId::LineOpa, V::Opa(Opa::COVER)),
            (PropId::ArcOpa, V::Opa(Opa::COVER)),
            (PropId::ShadowOpa, V::Opa(Opa::COVER)),
            (PropId::BgGradStop, V::Int(255)),
            (PropId::BorderSide, V::Enum(0x0F)),
            (PropId::TextFont, V::Font(&twine_text::EMPTY_FONT)),
            (PropId::MaxWidth, V::Length(Length::Px((1 << 29) - 1))),
            (PropId::MaxHeight, V::Length(Length::Px((1 << 29) - 1))),
            (PropId::RotarySensitivity, V::Int(256)),
            (PropId::DropShadowQuality, V::Enum(2)),
            (PropId::GridCellRowSpan, V::Int(1)),
            (PropId::GridCellColumnSpan, V::Int(1)),
            // LVGL's zero of an enum is its first value; in Twine's encoding that is only
            // different for `TextAlign` (LVGL `LV_TEXT_ALIGN_AUTO` = 0).
            (PropId::TextAlign, V::Enum(TextAlign::Auto as u8)),
        ];
        for id in PropId::ALL {
            let d = id.meta().default;
            if let Some((_, want)) = non_zero.iter().find(|(p, _)| *p == id) {
                assert_eq!(d, *want, "{id:?}");
                continue;
            }
            let zero = match d {
                V::None | V::Int(0) | V::Length(Length::Px(0)) | V::Bool(false) | V::Enum(0) => true,
                V::Color(c) => c == Color::BLACK,
                V::Opa(o) => o == Opa::TRANSP,
                V::Angle(a) => a.0 == 0,
                _ => false,
            };
            assert!(zero, "{id:?}: default {d:?} is not LVGL's 0/NULL");
        }
        assert_eq!(PropId::Align.meta().default.as_align(), Some(Align::Default));
        assert_eq!(PropId::BaseDir.meta().default.as_base_dir(), Some(BaseDir::Ltr));
        assert_eq!(
            PropId::FlexFlow.meta().default.as_flex_flow(),
            Some(FlexFlow::Row)
        );
        assert_eq!(PropId::Layout.meta().default.as_layout(), Some(LayoutKind::None));
    }

    #[test]
    fn group_is_id_shift_4() {
        for id in PropId::ALL {
            assert_eq!(id.group(), id as u8 >> 4);
            assert_eq!(id.meta().group, id.group());
            assert_eq!(id.group_bit(), 1u16 << (id as u8 >> 4));
        }
        assert_eq!(PropId::Width.group(), 0);
        assert_eq!(PropId::GridCellYAlign.group(), 8);
    }

    #[test]
    fn size_of_style_prop() {
        // 12 bytes on 32-bit targets (checked at compile time there); two words + tag on 64-bit.
        assert!(core::mem::size_of::<StyleProp>() <= 3 * core::mem::size_of::<usize>());
        assert!(core::mem::size_of::<StyleValue>() <= 3 * core::mem::size_of::<usize>());
    }
}
