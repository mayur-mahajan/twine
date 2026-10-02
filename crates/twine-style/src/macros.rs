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
/// `Duration`; a bare integer does not compile. Color, opacity, font, length and radius
/// properties also take a [design element](crate::design) the theme supplies
/// (`bg: design::SURFACE`, `padding: design::SPACE_M`). Later entries override earlier ones; unknown
/// names (including the former LVGL-style names such as `pad_all` or `bg_opa`) are compile
/// errors. The result is a `const` expression,
/// so styles can be `static` (in flash).
///
/// Because the result is `const`, the two properties whose values live behind a reference take
/// `'static` data only: `transition:` a `&'static Transition` (e.g. a reference to a
/// `static`), and `grid_column_tracks:` / `grid_row_tracks:` a `'static` slice or array of
/// [`GridTrack`](crate::GridTrack)s (e.g. `&COLUMNS` for a `static COLUMNS: [GridTrack; 3]`).
/// A transition or track list built at run time (a `Transition` value, a `Vec` from
/// [`grid_tracks!`](crate::grid_tracks)) goes through the [`StyleBuf`](crate::StyleBuf)
/// builder or the `twine-view` modifiers, which hold the value.
///
/// # Composition: `..BASE`
///
/// `..BASE` spreads another style: its properties, then the entries after it, which override
/// them (`style! { ..BUTTON, bg: Color::RED }`). `BASE` is any `const`-evaluable `Style` (a
/// `static` or `const` item, or a reference to one), usually itself written with `style!`.
///
/// - **Later wins:** keys and spreads apply in order; a later value of a property replaces an
///   earlier one, whether it comes from a key or a spread (`style! { radius: 2, ..BASE }`
///   takes `BASE`'s radius if it has one). Spread order matters.
/// - **Several and nested spreads** are allowed (`style! { ..A, ..B, padding: 4 }`; `A` may be
///   a spread style itself).
/// - **Compile time, no duplicates:** with a spread the result is merged while compiling: each
///   property is stored once (at the position of its first occurrence, with its last value,
///   like [`StyleBuf::set`](crate::StyleBuf::set)), in a new `const` array. Nothing runs or
///   allocates at run time, and lookups scan no overridden entries; the cost is flash for the
///   copied properties (12 bytes each on 32-bit targets).
/// - Inside a spread `style!`, values cannot name generic parameters or `Self` (they are
///   evaluated in `const` items). Without a spread, the entries are stored as written.
///
/// For styles only known at run time use [`Style::merge`](crate::Style::merge) and
/// [`StyleBuf::extend_from`](crate::StyleBuf::extend_from).
///
/// ```
/// use twine_core::Color;
/// use twine_style::{Length, PropId, Radius, Style, StyleValue, style};
///
/// static BUTTON: Style = style! { bg: Color::WHITE, radius: 8, padding: 12 };
/// static DANGER: Style = style! { ..BUTTON, bg: Color::RED };
/// static SMALL_DANGER: Style = style! { ..DANGER, padding: 4, width: 60 }; // nested
/// const _: () = assert!(SMALL_DANGER.props().len() == 8); // computed while compiling
///
/// assert_eq!(SMALL_DANGER.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
/// assert_eq!(SMALL_DANGER.get(PropId::Radius), Some(StyleValue::Length(Length::Px(8))));
/// assert_eq!(SMALL_DANGER.get(PropId::PaddingTop), Some(StyleValue::Length(Length::Px(4))));
/// assert_eq!(BUTTON.get(PropId::BgColor), Some(StyleValue::Color(Color::WHITE))); // unchanged
/// ```
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
///
/// // Themed: the values come from the display's theme when the style is resolved.
/// use twine_style::design;
/// static THEMED: Style = style! {
///     bg: design::SURFACE,
///     text_color: design::ON_SURFACE,
///     border: (1, design::OUTLINE),
///     padding: design::SPACE_M,
///     radius: design::RADIUS_M,
///     font: design::FONT_BODY,
/// };
/// assert_eq!(THEMED.get(PropId::BgColor), Some(StyleValue::Element(design::SURFACE.erase())));
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
        $crate::__style_props!({} [] $($body)*)
    };
}

/// Accumulates the property list of `style!` (tt-muncher). Each entry goes through
/// `__style_shorthand!` (generated from the shorthand table), which continues the muncher.
/// Parenthesized values are passed as tokens so a shorthand can take a tuple of parameters.
///
/// State: `{segments}` (one token tree: the finished parts of a composed style, each a
/// parenthesized `&[StyleProp]` expression: the entries before a spread, then the spread
/// style's properties) and `[entries]` (the entries since the last spread).
#[doc(hidden)]
#[macro_export]
macro_rules! __style_props {
    // Done, no spread: the entries as written (later duplicates win at lookup).
    ({} [$($acc:expr),*] $(,)?) => { $crate::Style::new(&[$($acc),*]) };

    // Done after a spread: merged at compile time.
    ({$($seg:tt)+} [$($acc:expr),*] $(,)?) => {
        $crate::__style_compose!($($seg)* (&[$($acc),*]))
    };

    // A spread: `..BASE` (any `const` expression of type `Style` or `&Style`).
    ({$($seg:tt)*} [$($acc:expr),*] .. $base:expr $(, $($rest:tt)*)?) => {
        $crate::__style_props!({$($seg)* (&[$($acc),*]) ($base.props())} [] $($($rest)*)?)
    };

    // A parenthesized value: a tuple of shorthand parameters (or a parenthesized expression).
    ($segs:tt [$($acc:expr),*] $name:ident : ( $($inner:tt)* ) $(, $($rest:tt)*)?) => {
        $crate::__style_shorthand!($name, ($($inner)*), $segs [$($acc),*] $($($rest)*)?)
    };

    // Any other value.
    ($segs:tt [$($acc:expr),*] $name:ident : $v:expr $(, $($rest:tt)*)?) => {
        $crate::__style_shorthand!($name, ($v), $segs [$($acc),*] $($($rest)*)?)
    };
}

/// The `const` merge of a `style!` with spreads: every segment's properties, each property
/// once (first position, last value; see `__private::merge_props`), in a `const` array the
/// returned `Style` borrows (promoted to `'static`, so the style stays in flash).
#[doc(hidden)]
#[macro_export]
macro_rules! __style_compose {
    ($($seg:tt)*) => {{
        const __TWINE_PARTS: &[&[$crate::StyleProp]] = &[$($seg),*];
        const __TWINE_LEN: ::core::primitive::usize = $crate::__private::merged_len(__TWINE_PARTS);
        const __TWINE_PROPS: [$crate::StyleProp; __TWINE_LEN] =
            $crate::__private::merge_props::<__TWINE_LEN>(__TWINE_PARTS);
        $crate::Style::new(&__TWINE_PROPS)
    }};
}
