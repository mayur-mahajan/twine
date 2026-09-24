//! The `style!` macro.

/// Builds a `const` [`Style`](crate::Style) from `name: value` pairs.
///
/// Every property is accepted under its `snake_case` name (`bg_color`, `pad_top`, …; see
/// [`PropId`](crate::PropId)), plus these shorthands:
///
/// | Shorthand | Expands to |
/// |-----------|------------|
/// | `pad_all: v` | `pad_top`, `pad_bottom`, `pad_left`, `pad_right` |
/// | `pad_hor: v` / `pad_ver: v` | left + right / top + bottom padding |
/// | `pad_gap: v` | `pad_row` + `pad_column` |
/// | `margin_all`, `margin_hor`, `margin_ver` | like the padding shorthands |
/// | `size: (w, h)` | `width` + `height` |
/// | `transform_scale: s` | `transform_scale_x` + `transform_scale_y` |
/// | `border: (w, c)` | `border_width` + `border_color` |
///
/// Length properties (`width`, `x`, `translate_x`, `size`, …) accept integers (pixels) or any
/// [`Length`](crate::Length) expression; scale properties accept integers (256 = 1.0) or a
/// `Scale`. Later entries override earlier ones; unknown names are compile errors. The result
/// is a `const` expression, so styles can be `static` (in flash).
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::{Length, PropId, Style, StyleValue, style};
///
/// static CARD: Style = style! {
///     bg_color: Color::WHITE,
///     bg_opa: Opa::COVER,
///     radius: 8,
///     pad_all: 12,
///     width: Length::pct(50),
///     height: 40,
/// };
/// assert_eq!(CARD.get(PropId::PadLeft), Some(StyleValue::Int(12)));
/// assert_eq!(CARD.get(PropId::Height), Some(StyleValue::Length(Length::Px(40))));
/// ```
#[macro_export]
macro_rules! style {
    ($($body:tt)*) => {
        $crate::Style::new(&$crate::__style_props!([] $($body)*))
    };
}

/// Accumulates the property list of `style!` (tt-muncher).
#[doc(hidden)]
#[macro_export]
macro_rules! __style_props {
    // Done.
    ([$($acc:expr),*] $(,)?) => { [$($acc),*] };

    // Shorthands.
    ([$($acc:expr),*] pad_all : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)*
            $crate::StyleProp::PadTop($v), $crate::StyleProp::PadBottom($v),
            $crate::StyleProp::PadLeft($v), $crate::StyleProp::PadRight($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] pad_hor : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::StyleProp::PadLeft($v), $crate::StyleProp::PadRight($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] pad_ver : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::StyleProp::PadTop($v), $crate::StyleProp::PadBottom($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] pad_gap : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::StyleProp::PadRow($v), $crate::StyleProp::PadColumn($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] margin_all : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)*
            $crate::StyleProp::MarginTop($v), $crate::StyleProp::MarginBottom($v),
            $crate::StyleProp::MarginLeft($v), $crate::StyleProp::MarginRight($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] margin_hor : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::StyleProp::MarginLeft($v), $crate::StyleProp::MarginRight($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] margin_ver : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::StyleProp::MarginTop($v), $crate::StyleProp::MarginBottom($v)] $($($rest)*)?)
    };
    ([$($acc:expr),*] size : ($w:expr, $h:expr $(,)?) $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)*
            $crate::StyleProp::Width($crate::__to_length!($w)),
            $crate::StyleProp::Height($crate::__to_length!($h))] $($($rest)*)?)
    };
    ([$($acc:expr),*] transform_scale : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)*
            $crate::StyleProp::TransformScaleX($crate::__style_wrap!(scale, $v)),
            $crate::StyleProp::TransformScaleY($crate::__style_wrap!(scale, $v))] $($($rest)*)?)
    };
    ([$($acc:expr),*] border : ($w:expr, $c:expr $(,)?) $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::StyleProp::BorderWidth($w), $crate::StyleProp::BorderColor($c)] $($($rest)*)?)
    };

    // Any property.
    ([$($acc:expr),*] $name:ident : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!([$($acc,)* $crate::__style_prop!($name, $v)] $($($rest)*)?)
    };
}

/// Converts an integer (pixels) or a [`Length`](crate::Length) expression into a `Length` in
/// `const` context (used by `style!` for length properties).
///
/// ```
/// use twine_style::{__to_length, Length};
/// const A: Length = __to_length!(10);
/// const B: Length = __to_length!(Length::pct(20));
/// assert_eq!((A, B), (Length::Px(10), Length::Pct(20)));
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __to_length {
    ($v:expr) => {
        $crate::__LengthArg($v).get()
    };
}
