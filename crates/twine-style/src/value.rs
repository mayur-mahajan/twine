//! [`StyleValue`]: the resolved value of a style property, and [`PropValue`] conversions.

use core::fmt;

use twine_anim::AnimTemplate;
use twine_core::{Angle, Color, Opa, Scale};
use twine_image::ImageSource;
use twine_render::Gradient;
use twine_text::Font;

use crate::transition::TransitionDsc;
use crate::value_types::{
    Align, BaseDir, BlendMode, BlurQuality, BorderSide, ColorFilter, Dir, FlexAlign, FlexFlow, GradDir,
    GridAlign, GridTrack, ImageColorkey, LayoutKind, Length, ScrollSnap, ScrollbarMode, StyleEnum, TextAlign,
    TextDecor, TextLeadingTrim,
};

/// The value of a style property as returned by lookups and resolution: one variant per value
/// type. `Copy` and at most three words, so lookups never allocate.
///
/// Enum-like values (alignments, flags, modes) are stored as [`StyleValue::Enum`] codes; read
/// them with [`StyleValue::get`] or the typed accessors (`as_align`, `as_flex_flow`, …).
/// References are compared by address (fonts, transitions, color filters) or by address and
/// then content (images, gradients, grid templates, templates, color keys).
///
/// ```
/// use twine_core::Color;
/// use twine_style::{Align, StyleValue};
///
/// let v = StyleValue::Color(Color::RED);
/// assert_eq!(v.as_color(), Some(Color::RED));
/// assert_eq!(v.as_i32(), None);
/// assert_eq!(StyleValue::from(Align::Center).as_align(), Some(Align::Center));
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub enum StyleValue {
    /// No value: the default of reference-typed properties (LVGL `NULL`).
    #[default]
    None,
    /// An integer (pixels, counts, raw LVGL numbers).
    Int(i32),
    /// A size or position.
    Length(Length),
    /// A color.
    Color(Color),
    /// An opacity.
    Opa(Opa),
    /// An angle (0.1°).
    Angle(Angle),
    /// A scale factor (256 = 1.0).
    Scale(Scale),
    /// A font.
    Font(&'static Font),
    /// An image source.
    Image(&'static ImageSource),
    /// A gradient descriptor.
    Grad(&'static Gradient),
    /// A transition descriptor.
    Transition(&'static TransitionDsc),
    /// A grid column or row template.
    GridTracks(&'static [GridTrack]),
    /// An enum or flag set, see [`StyleEnum`].
    Enum(u8),
    /// A boolean.
    Bool(bool),
    /// An animation template.
    AnimTemplate(&'static AnimTemplate),
    /// An image color key.
    Colorkey(&'static ImageColorkey),
    /// A color filter.
    ColorFilter(&'static ColorFilter),
}

fn same_or_equal<T: PartialEq + ?Sized>(a: &T, b: &T) -> bool {
    core::ptr::eq(a, b) || a == b
}

impl PartialEq for StyleValue {
    fn eq(&self, other: &Self) -> bool {
        use StyleValue as V;
        match (*self, *other) {
            (V::None, V::None) => true,
            (V::Int(a), V::Int(b)) => a == b,
            (V::Length(a), V::Length(b)) => a == b,
            (V::Color(a), V::Color(b)) => a == b,
            (V::Opa(a), V::Opa(b)) => a == b,
            (V::Angle(a), V::Angle(b)) => a == b,
            (V::Scale(a), V::Scale(b)) => a == b,
            (V::Font(a), V::Font(b)) => core::ptr::eq(a, b),
            (V::Image(a), V::Image(b)) => same_or_equal(a, b),
            (V::Grad(a), V::Grad(b)) => same_or_equal(a, b),
            (V::Transition(a), V::Transition(b)) => core::ptr::eq(a, b),
            (V::GridTracks(a), V::GridTracks(b)) => same_or_equal(a, b),
            (V::Enum(a), V::Enum(b)) => a == b,
            (V::Bool(a), V::Bool(b)) => a == b,
            (V::AnimTemplate(a), V::AnimTemplate(b)) => same_or_equal(a, b),
            (V::Colorkey(a), V::Colorkey(b)) => same_or_equal(a, b),
            (V::ColorFilter(a), V::ColorFilter(b)) => core::ptr::eq(a, b),
            _ => false,
        }
    }
}

impl Eq for StyleValue {}

impl fmt::Display for StyleValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            StyleValue::None => f.write_str("none"),
            StyleValue::Int(v) => write!(f, "{v}"),
            StyleValue::Length(Length::Px(v)) => write!(f, "{v}px"),
            StyleValue::Length(Length::Pct(v)) => write!(f, "{v}%"),
            StyleValue::Length(Length::Content) => f.write_str("content"),
            StyleValue::Color(c) => write!(f, "{c}"),
            StyleValue::Opa(o) => write!(f, "opa {}", o.0),
            StyleValue::Angle(a) => {
                let sign = if a.0 < 0 { "-" } else { "" };
                write!(
                    f,
                    "{sign}{}.{}°",
                    a.0.unsigned_abs() / 10,
                    a.0.unsigned_abs() % 10
                )
            }
            StyleValue::Scale(s) => write!(f, "scale {}", s.0),
            StyleValue::Font(font) => write!(f, "font@{:p} ({} px lines)", font, font.line_height),
            StyleValue::Image(i) => write!(f, "image {i}"),
            StyleValue::Grad(g) => write!(f, "gradient ({} stops)", g.stops().len()),
            StyleValue::Transition(t) => write!(f, "transition ({} props)", t.props.len()),
            StyleValue::GridTracks(t) => write!(f, "{} tracks", t.len()),
            StyleValue::Enum(e) => write!(f, "enum {e}"),
            StyleValue::Bool(b) => write!(f, "{b}"),
            StyleValue::AnimTemplate(a) => write!(f, "anim {}", a.duration),
            StyleValue::Colorkey(k) => write!(f, "colorkey {}..{}", k.low, k.high),
            StyleValue::ColorFilter(_) => f.write_str("color filter"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for StyleValue {
    fn format(&self, f: defmt::Formatter<'_>) {
        match *self {
            StyleValue::Int(v) => defmt::write!(f, "Int({})", v),
            StyleValue::Length(l) => defmt::write!(f, "{}", l),
            StyleValue::Color(c) => defmt::write!(f, "{}", c),
            StyleValue::Opa(o) => defmt::write!(f, "{}", o),
            StyleValue::Angle(a) => defmt::write!(f, "{}", a),
            StyleValue::Scale(s) => defmt::write!(f, "{}", s),
            StyleValue::Enum(e) => defmt::write!(f, "Enum({})", e),
            StyleValue::Bool(b) => defmt::write!(f, "Bool({})", b),
            StyleValue::None => defmt::write!(f, "None"),
            _ => defmt::write!(f, "Ref"),
        }
    }
}

/// A Rust type that can be the payload of a [`StyleProp`](crate::StyleProp) variant and be
/// converted to and from [`StyleValue`].
pub trait PropValue: Copy {
    /// Wraps the value.
    fn into_value(self) -> StyleValue;
    /// Unwraps a value of the matching variant (`None` for any other variant or an
    /// out-of-range code).
    fn from_value(v: StyleValue) -> Option<Self>;
}

macro_rules! prop_value {
    ($t:ty, $variant:ident) => {
        impl PropValue for $t {
            #[inline]
            fn into_value(self) -> StyleValue {
                StyleValue::$variant(self)
            }
            #[inline]
            fn from_value(v: StyleValue) -> Option<Self> {
                match v {
                    StyleValue::$variant(x) => Some(x),
                    _ => None,
                }
            }
        }
    };
}

prop_value!(i32, Int);
prop_value!(Length, Length);
prop_value!(Color, Color);
prop_value!(Opa, Opa);
prop_value!(Angle, Angle);
prop_value!(Scale, Scale);
prop_value!(bool, Bool);
prop_value!(&'static Font, Font);
prop_value!(&'static ImageSource, Image);
prop_value!(&'static Gradient, Grad);
prop_value!(&'static TransitionDsc, Transition);
prop_value!(&'static [GridTrack], GridTracks);
prop_value!(&'static AnimTemplate, AnimTemplate);
prop_value!(&'static ImageColorkey, Colorkey);
prop_value!(&'static ColorFilter, ColorFilter);

macro_rules! prop_value_int {
    ($($t:ty),*) => {$(
        impl PropValue for $t {
            #[inline]
            fn into_value(self) -> StyleValue {
                StyleValue::Int(i32::try_from(self).unwrap_or(i32::MAX))
            }
            #[inline]
            fn from_value(v: StyleValue) -> Option<Self> {
                match v {
                    StyleValue::Int(x) => <$t>::try_from(x).ok(),
                    _ => None,
                }
            }
        }
    )*};
}

prop_value_int!(u8, u32);

macro_rules! prop_value_enum {
    ($($t:ty),* $(,)?) => {$(
        impl PropValue for $t {
            #[inline]
            fn into_value(self) -> StyleValue {
                StyleValue::Enum(self.to_code())
            }
            #[inline]
            fn from_value(v: StyleValue) -> Option<Self> {
                match v {
                    StyleValue::Enum(c) => <$t as StyleEnum>::from_code(c),
                    _ => None,
                }
            }
        }

        impl From<$t> for StyleValue {
            fn from(v: $t) -> Self {
                v.into_value()
            }
        }
    )*};
}

prop_value_enum!(
    Align,
    BaseDir,
    FlexFlow,
    FlexAlign,
    GridAlign,
    LayoutKind,
    GradDir,
    ScrollbarMode,
    ScrollSnap,
    BlurQuality,
    TextLeadingTrim,
    TextAlign,
    BlendMode,
    BorderSide,
    TextDecor,
    Dir,
);

macro_rules! accessors {
    ($($(#[$m:meta])* $name:ident -> $t:ty;)*) => {$(
        $(#[$m])*
        #[inline]
        #[must_use]
        pub fn $name(self) -> Option<$t> {
            <$t as PropValue>::from_value(self)
        }
    )*};
}

impl StyleValue {
    /// The payload as `T` if the variant (and, for enums, the code) matches.
    #[inline]
    #[must_use]
    pub fn get<T: PropValue>(self) -> Option<T> {
        T::from_value(self)
    }

    /// Whether this is [`StyleValue::None`].
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, StyleValue::None)
    }

    accessors! {
        /// `Int` payload.
        as_i32 -> i32;
        /// `Length` payload.
        as_length -> Length;
        /// `Color` payload.
        as_color -> Color;
        /// `Opa` payload.
        as_opa -> Opa;
        /// `Angle` payload.
        as_angle -> Angle;
        /// `Scale` payload.
        as_scale -> Scale;
        /// `Bool` payload.
        as_bool -> bool;
        /// `Font` payload.
        as_font -> &'static Font;
        /// `Image` payload.
        as_image -> &'static ImageSource;
        /// `Grad` payload.
        as_gradient -> &'static Gradient;
        /// `Transition` payload.
        as_transition -> &'static TransitionDsc;
        /// `GridTracks` payload.
        as_grid_tracks -> &'static [GridTrack];
        /// `AnimTemplate` payload.
        as_anim_template -> &'static AnimTemplate;
        /// `Colorkey` payload.
        as_colorkey -> &'static ImageColorkey;
        /// `ColorFilter` payload.
        as_color_filter -> &'static ColorFilter;
        /// An [`Align`] code.
        as_align -> Align;
        /// A [`BaseDir`] code.
        as_base_dir -> BaseDir;
        /// A [`FlexFlow`] code.
        as_flex_flow -> FlexFlow;
        /// A [`FlexAlign`] code.
        as_flex_align -> FlexAlign;
        /// A [`GridAlign`] code.
        as_grid_align -> GridAlign;
        /// A [`LayoutKind`] code.
        as_layout -> LayoutKind;
        /// A [`GradDir`] code.
        as_grad_dir -> GradDir;
        /// A [`BlurQuality`] code.
        as_blur_quality -> BlurQuality;
        /// A [`TextLeadingTrim`] code.
        as_text_leading_trim -> TextLeadingTrim;
        /// A [`TextAlign`] code.
        as_text_align -> TextAlign;
        /// A [`BlendMode`] code.
        as_blend_mode -> BlendMode;
        /// A [`BorderSide`] code.
        as_border_side -> BorderSide;
        /// A [`TextDecor`] code.
        as_text_decor -> TextDecor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_value_eq_by_pointer_for_fonts() {
        static A: Font = crate::test_util::font(1);
        static B: Font = crate::test_util::font(1);
        static I1: ImageSource = ImageSource::Symbol("x");
        static I2: ImageSource = ImageSource::Symbol("x");
        static I3: ImageSource = ImageSource::Symbol("y");
        static T1: TransitionDsc =
            TransitionDsc::new(&[], twine_core::Duration::ZERO, twine_anim::Easing::Linear);
        static T2: TransitionDsc =
            TransitionDsc::new(&[], twine_core::Duration::ZERO, twine_anim::Easing::Linear);
        assert_eq!(StyleValue::Font(&A), StyleValue::Font(&A));
        assert_ne!(StyleValue::Font(&A), StyleValue::Font(&B)); // same content, other font
        assert_eq!(StyleValue::Image(&I1), StyleValue::Image(&I2)); // same source
        assert_ne!(StyleValue::Image(&I1), StyleValue::Image(&I3));
        assert_eq!(StyleValue::Transition(&T1), StyleValue::Transition(&T1));
        assert_ne!(StyleValue::Transition(&T1), StyleValue::Transition(&T2));
        assert_ne!(StyleValue::Int(1), StyleValue::Bool(true));
        assert_eq!(StyleValue::None, StyleValue::default());
    }

    #[test]
    fn typed_accessors() {
        assert_eq!(StyleValue::Int(-4).get::<u32>(), None);
        assert_eq!(StyleValue::Int(300).get::<u8>(), None);
        assert_eq!(StyleValue::Int(30).get::<u8>(), Some(30));
        assert_eq!(StyleValue::Enum(200).as_align(), None);
        assert_eq!(
            StyleValue::from(FlexFlow::ColumnWrap).as_flex_flow(),
            Some(FlexFlow::ColumnWrap)
        );
        assert_eq!(
            StyleValue::Length(Length::Content).as_length(),
            Some(Length::Content)
        );
        assert!(StyleValue::None.is_none());
        assert_eq!(u32::MAX.into_value(), StyleValue::Int(i32::MAX));
    }

    #[test]
    fn display() {
        use std::string::ToString;
        assert_eq!(StyleValue::Length(Length::Pct(50)).to_string(), "50%");
        assert_eq!(StyleValue::Color(Color::RED).to_string(), "#FF0000");
        assert_eq!(StyleValue::Angle(Angle(-15)).to_string(), "-1.5°");
        assert_eq!(StyleValue::Angle(Angle(-5)).to_string(), "-0.5°");
        assert_eq!(StyleValue::None.to_string(), "none");
    }
}
