//! The `style!` macro.

/// Builds a `const` [`Style`](crate::Style) from `key: value` pairs.
///
/// The keys are the property names of the style vocabulary (`bg_color`, `padding_top`,
/// `font`, …; see [`PropId`](crate::PropId) and `PROPERTIES.md`), the same names as the
/// [`StyleBuf`](crate::StyleBuf) builder methods and the `twine-view` modifiers, plus the
/// shorthands of [`SHORTHANDS`](crate::SHORTHANDS):
///
/// | Shorthand | Sets |
/// |-----------|------|
/// | `size: (w, h)`, `pos: (x, y)`, `translate: (x, y)` | `width` + `height`, `x` + `y`, `translate_x` + `translate_y` |
/// | `offset: point` | `translate_x` + `translate_y` in pixels |
/// | `padding: v`, `padding_x: v`, `padding_y: v`, `padding_each: insets` | all four / left + right / top + bottom / each side |
/// | `margin`, `margin_x`, `margin_y`, `margin_each` | like the padding shorthands |
/// | `gap: v` | `row_gap` + `column_gap` |
/// | `bg: color` | `bg_color` + `bg_opacity: Opa::COVER` |
/// | `border: (w, color)` | `border_width` + `border_color` + `border_opacity: Opa::COVER` |
/// | `outline: (w, color, offset)` | `outline_width` + `outline_color` + `outline_opacity: Opa::COVER` + `outline_offset` |
/// | `shadow: shadow_dsc`, `shadow_offset: (x, y)` | every shadow property / both offsets |
/// | `transform_scale: s`, `transform_pivot: point` | both scale axes / both pivot coordinates |
/// | `grid_col: span`, `grid_row: span`, `grid_align: (x, y)` | the grid cell placement (a `GridSpan`: first track and count) |
///
/// Length properties (`width`, `x`, `translate_x`, `size`, the spacing properties `padding`,
/// `margin`, `gap`, `border_width`, …) accept integers (pixels) or any
/// [`Length`](crate::Length) expression, e.g. `Length::dp(8)` (density-independent);
/// `radius` accepts integers (pixels) or a [`Radius`](crate::Radius) (`Radius::Circle`).
/// Unit-typed properties take typed values only: opacities an `Opa` (`Opa::pct(50)`), angles
/// an `Angle` (`Angle::deg(30)`), scales a `Scale` (`Scale::pct(98)`), durations a
/// `Duration`; a bare integer does not compile. Later entries override earlier ones; unknown
/// names (including the former LVGL-style names such as `pad_all` or `bg_opa`) are compile
/// errors. The result is a `const` expression,
/// so styles can be `static` (in flash).
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::{Length, PropId, Style, StyleValue, style};
///
/// static CARD: Style = style! {
///     bg: Color::WHITE,
///     bg_opacity: Opa::pct(90),
///     radius: 8,
///     padding: 12,
///     border: (1, Color::hex(0xDDDDDD)),
///     width: Length::pct(50),
///     height: 40,
/// };
/// assert_eq!(CARD.get(PropId::PaddingLeft), Some(StyleValue::Length(Length::Px(12))));
/// assert_eq!(CARD.get(PropId::BgOpacity), Some(StyleValue::Opa(Opa::pct(90))));
/// assert_eq!(CARD.get(PropId::Height), Some(StyleValue::Length(Length::Px(40))));
/// ```
///
/// A bare integer is not an opacity, an angle or a scale:
///
/// ```compile_fail
/// use twine_style::{Style, style};
/// static S: Style = style! { bg_opacity: 128 }; // write Opa::pct(50) or Opa::from_raw(128)
/// ```
///
/// ```compile_fail
/// use twine_style::{Style, style};
/// static S: Style = style! { transform_rotation: 300 }; // write Angle::deg(30)
/// ```
///
/// ```compile_fail
/// use twine_style::{Style, style};
/// static S: Style = style! { transform_scale: 250 }; // write Scale::pct(98)
/// ```
#[macro_export]
macro_rules! style {
    ($($body:tt)*) => {
        $crate::Style::new(&$crate::__style_props!([] $($body)*))
    };
}

/// Accumulates the property list of `style!` (tt-muncher). Each entry goes through
/// `__style_shorthand!` (generated from the shorthand table), which continues the muncher.
/// Parenthesized values are passed as tokens so a shorthand can take a tuple of parameters.
#[doc(hidden)]
#[macro_export]
macro_rules! __style_props {
    // Done.
    ([$($acc:expr),*] $(,)?) => { [$($acc),*] };

    // A parenthesized value: a tuple of shorthand parameters (or a parenthesized expression).
    ([$($acc:expr),*] $name:ident : ( $($inner:tt)* ) $(, $($rest:tt)*)?) => {
        $crate::__style_shorthand!($name, ($($inner)*), [$($acc),*] $($($rest)*)?)
    };

    // Any other value.
    ([$($acc:expr),*] $name:ident : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_shorthand!($name, ($v), [$($acc),*] $($($rest)*)?)
    };
}
