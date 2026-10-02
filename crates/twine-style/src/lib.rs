//! # twine-style
//!
//! The style system of the Twine GUI library, independent of the widget tree: the complete
//! LVGL v9 style property catalogue, flash-resident and heap styles, selectors, the exact LVGL
//! resolution (precedence and inheritance) algorithm over an abstract [`StyleSource`], and
//! transitions ([`Transition`], property sets [`Props`]) with interpolation.
//!
//! No floating point and no allocation during resolution.
//!
//! ## Defining styles
//!
//! ```
//! use twine_core::{Color, Opa};
//! use twine_style::{Length, PropId, Style, StyleBuf, StyleValue, style};
//!
//! // In flash: a `static` built by the `style!` macro.
//! static CARD: Style = style! { bg_color: Color::WHITE, bg_opacity: Opa::COVER, radius: 8, padding: 12 };
//! // On the heap, e.g. built by a theme at run time.
//! let accent = StyleBuf::new().bg_color(Color::hex(0x2196F3)).border_width(2);
//! assert_eq!(CARD.get(PropId::PaddingTop), Some(StyleValue::Length(Length::Px(12))));
//! assert_eq!(accent.get(PropId::BorderWidth), Some(StyleValue::Length(Length::Px(2))));
//! ```
//!
//! Styles compose: `..BASE` in `style!` spreads another `static`/`const` style (merged at
//! compile time, later keys win, still in flash); [`Style::merge`] and
//! [`StyleBuf::extend_from`] compose at run time.
//!
//! ```
//! use twine_core::Color;
//! use twine_style::{PropId, Style, StyleValue, style};
//!
//! static BUTTON: Style = style! { bg_color: Color::WHITE, radius: 8, padding: 12 };
//! static DANGER_BUTTON: Style = style! { ..BUTTON, bg_color: Color::RED };
//! assert_eq!(DANGER_BUTTON.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
//! assert_eq!(DANGER_BUTTON.props().len(), BUTTON.props().len()); // overridden in place
//! ```
//!
//! A card that shrinks slightly and darkens while pressed, with a transition (the widget and
//! view parts that attach these to a node come from the engine and view crates):
//!
//! ```
//! use twine_core::{Color, Duration, Opa, Scale};
//! use twine_style::{Length, Props, Radius, Style, Transition, style};
//!
//! pub static CARD: Style = style! {
//!     bg_color: Color::WHITE,
//!     bg_opacity: Opa::COVER,
//!     radius: Radius::Px(8),
//!     padding: Length::dp(12),  // shorthand → padding_top/bottom/left/right; 12 dp scale with the DPI
//!     shadow_width: 12,
//!     shadow_opacity: Opa::P30,
//! };
//! // Pressing animates what the pressed style changes (background and scale): no list needed.
//! pub static SMOOTH: Transition = Transition::all(Duration::ms(150)).ease_out();
//! pub static CARD_PRESSED: Style =
//!     style! { bg_color: Color::hex(0xEEEEEE), transform_scale: Scale::pct(98), transition: &SMOOTH };
//! // Or name the animated properties by group.
//! pub static FADE: Transition = Transition::of(Props::BG.union(Props::OPACITY), Duration::ms(200));
//! # assert_eq!(CARD_PRESSED.props().len(), 4);
//! ```
//!
//! ## Design elements
//!
//! Instead of a fixed value, a color, length, radius, opacity or font property can name a
//! [design element](design) (`design::SURFACE`, `design::SPACE_M`, …, or an application's own):
//! the display's theme gives it a value for its current [`ThemeMode`], substituted when the
//! style is resolved. Switching the mode re-resolves the styles; nothing is rebuilt.
//!
//! ```
//! use twine_style::{StyleBuf, design};
//! let card = StyleBuf::new().bg(design::SURFACE).border(1, design::OUTLINE).padding(design::SPACE_M);
//! # assert_eq!(card.len(), 9);
//! ```
//!
//! ## Precedence
//!
//! A node carries a list of [`StyleEntry`]s, each with a [`Selector`] (part + states) and a
//! kind. [`resolve`] walks them in priority order (the outcome is LVGL 9's):
//!
//! ```text
//!  entries of the node (highest priority first)            value of prop P for part X
//!  ┌──────────────────────────────┐
//!  │ Transition (part X)          │── has P ─────────────────────► wins immediately
//!  ├──────────────────────────────┤
//!  │ Local      (part, states)    │  every entry whose part is X and whose states are all
//!  │ Normal     latest added first│  active: the highest state precedence wins,
//!  │ Normal     …                 │  equal weight → the earlier entry; an entry whose states
//!  │ Theme      …                 │  equal the node's state stops the search
//!  └──────────────────────────────┘
//!         │ not found
//!         ▼
//!  inherited prop (text, base dir, color filter…)?
//!     yes → own Main part (if X ≠ Main), then each ancestor's Main part, same rules
//!     no  → default (PropMeta::default; Font → StyleDefaults::font)
//!
//!  the value found is a design element → the theme's value (StyleSource::design_value;
//!  undefined → the default above); a Dp length → pixels (StyleSource::dpi)
//! ```
//!
//! ### State precedence
//!
//! Among the entries whose states are all active, the selector with the higher state
//! precedence wins, whatever its kind (a theme's pressed style beats a local default-state
//! property). The states are ranked by [`State::PRECEDENCE`]; **the highest-ranked state in
//! which two selectors differ decides**, so one state outranks every combination of the
//! states below it, and adding a state to a selector makes it more specific:
//!
//! | Rank (low → high) | States |
//! |-------------------|--------|
//! | – | no state (`DEFAULT`, or [`State::ANY`] in a selector) |
//! | 0–4 | `ALT`, `CHECKED`, `FOCUSED`, `FOCUS_KEY`, `EDITED` |
//! | 5–8 | `HOVERED`, `PRESSED`, `SCROLLED`, `DISABLED` |
//! | 9–12 | the application's states [`State::custom::<0..4>()`](State::custom) |
//!
//! | Node state | Matching selectors | Winner |
//! |------------|--------------------|--------|
//! | `PRESSED \| CHECKED` | `CHECKED`, `PRESSED` | `PRESSED` |
//! | `PRESSED \| CHECKED` | `PRESSED`, `PRESSED \| CHECKED` | `PRESSED \| CHECKED` |
//! | `DISABLED \| PRESSED \| CHECKED` | `DISABLED`, `PRESSED \| CHECKED` | `DISABLED` |
//! | `DISABLED \| ALARM` | `DISABLED`, `ALARM` (`custom::<0>()`) | `ALARM` |
//!
//! Equal state sets fall back to the entry order above. This is LVGL's order, so the built-in
//! themes look as in LVGL; the order is a documented list, not a property of the bit values
//! (the bits are laid out to match it, checked at compile time, so a comparison is one integer
//! compare: [`Selector::weight`]). The full rule and more examples: [`State`].
//!
//! ## Modules at a glance
//!
//! | Item | Role | LVGL |
//! |------|------|------|
//! | [`PropId`], [`StyleProp`], [`StyleValue`], [`PROP_META`], [`PROP_NAMES`] | the 129 properties, typed values, metadata (hot) and names (diagnostics) | `lv_style_prop_t`, `lv_style_value_t` |
//! | [`Style`], [`StyleBuf`], [`StyleRef`], [`style!`], [`StyleContainer`] | containers and their composition (`..BASE` spread, [`Style::merge`], [`StyleBuf::extend_from`]) | `lv_style_t`, `LV_STYLE_CONST_INIT` |
//! | [`Part`], [`State`] ([`State::custom`], [`State::PRECEDENCE`]), [`Selector`] | selectors, application states, state precedence | `lv_part_t`, `lv_state_t`, `lv_style_selector_t` |
//! | [`resolve`], [`StyleSource`] | resolution | `lv_obj_get_style_prop` |
//! | [`design`] ([`ThemeMode`], [`design::ElementTable`], [`design::ContrastPair`], `design::SURFACE`, …) | design elements: named values the theme supplies per mode, and each mode's contrast requirements | — |
//! | [`GridTracks`], [`SharedTracks`], [`TracksRef`], [`resolve_grid_tracks`] | grid templates in styles (`'static`, or run-time ones owned by their container) | `lv_style_set_grid_*_dsc_array` |
//! | [`Transition`], [`Props`], [`TransitionRef`], [`interpolate`] | transitions: what animates (a property set or the properties a state change alters) and how ([`AnimSpec`]) | `lv_style_transition_dsc_t`, `trans_anim_cb` |
//!
//! ## Features
//!
//! `log` / `defmt` select the logging backend (target `"twine::style"`); `std` enables std-only
//! conveniences of the dependencies.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod design;
mod macros;
mod prop;
mod props;
mod resolve;
mod selector;
mod shorthand;
mod style;
mod table;
#[cfg(test)]
mod test_util;
mod tracks;
mod transition;
mod value;
mod value_types;

pub use design::ThemeMode;
pub use prop::{
    PROP_ALIASES, PROP_COUNT, PROP_META, PROP_NAMES, PropFlags, PropId, PropMeta, PropNames, StyleProp,
};
pub use props::Props;
pub use resolve::{
    EntryKind, EntryTrace, ResolveOptions, StyleDefaults, StyleEntry, StyleSource, TraceEvent, resolve,
    resolve_angle, resolve_as, resolve_bool, resolve_color, resolve_font, resolve_grid_tracks, resolve_i32,
    resolve_length, resolve_local_only, resolve_opa, resolve_scale, resolve_traced, resolve_value,
    resolve_with,
};
pub use selector::{Part, Selector, State};
pub use shorthand::{SHORTHANDS, ShorthandMeta};
pub use style::{Style, StyleBuf, StyleContainer, StyleRef};
#[doc(hidden)]
pub use tracks::__TracksArg;
pub use tracks::{GridTracks, SharedTracks, TrackId, TracksRef};
#[doc(hidden)]
pub use transition::__TransitionArg;
pub use transition::{
    Transition, TransitionId, TransitionRef, TransitionValue, interpolate, is_interpolable,
};
pub use twine_anim::{AnimSpec, Easing};
pub use value::{PropValue, StyleValue};
#[doc(hidden)]
pub use value_types::{__DurationArg, __LengthArg, __RadiusArg};
pub use value_types::{
    Align, Anchor, Axis, BaseDir, BlendMode, BlurQuality, BorderSide, COORD_MAX, ColorFilter, CrossAlign,
    DurationMs, FlexDirection, FlexFlow, GradDir, Gradient, GridAlign, GridSpan, GridTrack, ImageColorkey,
    LayoutKind, Length, MainAlign, Radius, ScrollSnap, ScrollbarMode, Side, Sides, StyleEnum, TextAlign,
    TextDecor, TextLeadingTrim,
};
pub use value_types::{DEFAULT_DPI, REFERENCE_DPI, dpx};

/// Names used by the macros generated from the property table (`style!`, the `twine-view`
/// modifiers): payload types under one path and the default values of the table. Not a
/// public API.
#[doc(hidden)]
pub mod __private {
    pub use alloc::vec::Vec;
    pub use twine_anim::AnimSpec;
    pub use twine_core::{Angle, Color, Duration, Fraction, Insets, Opa, Point, Scale};
    pub use twine_image::ImageSource;
    pub use twine_render::ShadowDsc;
    pub use twine_text::Font;

    pub use crate::design::{ColorValue, FontValue, LengthValue, OpacityValue, RadiusValue};
    pub use crate::style::{merge_props, merged_len};
    pub use crate::tracks::TracksRef;
    pub use crate::transition::TransitionRef;
    pub use crate::value_types::{
        Align, BaseDir, BlendMode, BlurQuality, BorderSide, ColorFilter, CrossAlign, DurationMs, FlexFlow,
        GradDir, Gradient, GridAlign, GridSpan, GridTrack, ImageColorkey, LayoutKind, Length, MainAlign,
        Radius, TextAlign, TextDecor, TextLeadingTrim,
    };

    use crate::StyleValue as V;
    use crate::value_types::COORD_MAX;

    /// `Opa::COVER` (constant of the `bg`, `border` and `outline` shorthands).
    pub const OPA_COVER: OpacityValue = crate::design::DesignValue::Fixed(Opa::COVER);

    const fn e(code: u8) -> V {
        V::Enum(code)
    }

    // Defaults of the property table (LVGL `lv_style_prop_get_default`).
    pub const INT0: V = V::Int(0);
    pub const INT1: V = V::Int(1);
    pub const INT255: V = V::Int(255);
    pub const PX0: V = V::Length(Length::Px(0));
    pub const CONTENT: V = V::Length(Length::Content);
    pub const COORD_MAX_PX: V = V::Length(Length::Px(COORD_MAX));
    pub const BLACK: V = V::Color(Color::BLACK);
    pub const WHITE: V = V::Color(Color::WHITE);
    pub const COVER: V = V::Opa(Opa::COVER);
    pub const TRANSP: V = V::Opa(Opa::TRANSP);
    pub const NONE: V = V::None;
    pub const FALSE: V = V::Bool(false);
    pub const SCALE_ONE: V = V::Scale(Scale::ONE);
    pub const ANGLE0: V = V::Angle(Angle::ZERO);
    pub const FONT_EMPTY: V = V::Font(&twine_text::EMPTY_FONT);
    pub const ALIGN_DEFAULT: V = e(Align::Default as u8);
    pub const GRAD_DIR_NONE: V = e(GradDir::None as u8);
    pub const BORDER_SIDE_FULL: V = e(BorderSide::FULL.0);
    pub const BLUR_PRECISION: V = e(BlurQuality::Precision as u8);
    pub const BLUR_AUTO: V = e(BlurQuality::Auto as u8);
    pub const TEXT_DECOR_NONE: V = e(TextDecor::empty().bits());
    pub const TEXT_ALIGN_AUTO: V = e(TextAlign::Auto as u8);
    pub const LEADING_TRIM_NONE: V = e(TextLeadingTrim::None as u8);
    pub const BLEND_NORMAL: V = e(BlendMode::Normal as u8);
    pub const LAYOUT_NONE: V = e(LayoutKind::None as u8);
    pub const BASE_DIR_LTR: V = e(BaseDir::Ltr as u8);
    pub const FLEX_ROW: V = e(FlexFlow::ROW.to_lvgl());
    pub const FLEX_START: V = e(MainAlign::Start as u8);
    pub const CROSS_START: V = e(CrossAlign::Start as u8);
    pub const GRID_START: V = e(GridAlign::Start as u8);
}
