//! # twine-style
//!
//! The style system of the Twine GUI library, independent of the widget tree: the complete
//! LVGL v9 style property catalogue, flash-resident and heap styles, selectors, the exact LVGL
//! resolution (precedence and inheritance) algorithm over an abstract [`StyleSource`], and
//! transition descriptors with interpolation.
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
//! A card that shrinks slightly and darkens while pressed, with a transition (the widget and
//! view parts that attach these to a node come from the engine and view crates):
//!
//! ```
//! use twine_core::{Color, Duration, Opa, Scale};
//! use twine_style::{Easing, Length, PropId, Radius, Style, TransitionDsc, style};
//!
//! pub static CARD: Style = style! {
//!     bg_color: Color::WHITE,
//!     bg_opacity: Opa::COVER,
//!     radius: Radius::Px(8),
//!     padding: Length::dp(12),  // shorthand → padding_top/bottom/left/right; 12 dp scale with the DPI
//!     shadow_width: 12,
//!     shadow_opacity: Opa::P30,
//! };
//! pub static CARD_PRESSED: Style = style! { bg_color: Color::hex(0xEEEEEE), transform_scale: Scale::pct(98) };
//! pub static SMOOTH: TransitionDsc = TransitionDsc::new(
//!     &[PropId::BgColor, PropId::TransformScaleX, PropId::TransformScaleY],
//!     Duration::ms(150),
//!     Easing::EaseOut,
//! );
//! # assert_eq!(CARD_PRESSED.props().len(), 3);
//! ```
//!
//! ## Precedence (identical to LVGL 9)
//!
//! A node carries a list of [`StyleEntry`]s, each with a [`Selector`] (part + states) and a
//! kind. [`resolve`] walks them in priority order:
//!
//! ```text
//!  entries of the node (highest priority first)            value of prop P for part X
//!  ┌──────────────────────────────┐
//!  │ Transition (part X)          │── has P ─────────────────────► wins immediately
//!  ├──────────────────────────────┤
//!  │ Local      (part, states)    │  every entry whose part is X and whose states are all
//!  │ Normal     latest added first│  active: the highest weight (state bits) wins,
//!  │ Normal     …                 │  equal weight → the earlier entry; an entry whose states
//!  │ Theme      …                 │  equal the node's state stops the search
//!  └──────────────────────────────┘
//!         │ not found
//!         ▼
//!  inherited prop (text, base dir, color filter…)?
//!     yes → own Main part (if X ≠ Main), then each ancestor's Main part, same rules
//!     no  → default (PropMeta::default; TextFont → StyleDefaults::font)
//! ```
//!
//! Weights are the numeric state bits, so `DISABLED` (0x200) beats `PRESSED | CHECKED`
//! (0x84), and a state style beats a local default style: weight dominates kind.
//!
//! ## Modules at a glance
//!
//! | Item | Role | LVGL |
//! |------|------|------|
//! | [`PropId`], [`StyleProp`], [`StyleValue`], [`PROP_META`] | the 129 properties, typed values, metadata | `lv_style_prop_t`, `lv_style_value_t` |
//! | [`Style`], [`StyleBuf`], [`StyleRef`], [`style!`] | containers | `lv_style_t`, `LV_STYLE_CONST_INIT` |
//! | [`Part`], [`State`], [`Selector`] | selectors | `lv_part_t`, `lv_state_t`, `lv_style_selector_t` |
//! | [`resolve`], [`StyleSource`] | resolution | `lv_obj_get_style_prop` |
//! | [`TransitionDsc`], [`interpolate`] | transitions | `lv_style_transition_dsc_t`, `trans_anim_cb` |
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

mod macros;
mod prop;
mod resolve;
mod selector;
mod shorthand;
mod style;
mod table;
#[cfg(test)]
mod test_util;
mod transition;
mod value;
mod value_types;

pub use prop::{PROP_ALIASES, PROP_COUNT, PROP_META, PropFlags, PropId, PropMeta, StyleProp};
pub use resolve::{
    EntryKind, EntryTrace, ResolveOptions, StyleDefaults, StyleEntry, StyleSource, TraceEvent, resolve,
    resolve_angle, resolve_as, resolve_bool, resolve_color, resolve_font, resolve_i32, resolve_length,
    resolve_local_only, resolve_opa, resolve_scale, resolve_traced, resolve_with,
};
pub use selector::{Part, Selector, State};
pub use shorthand::{SHORTHANDS, ShorthandMeta};
pub use style::{Style, StyleBuf, StyleRef};
pub use transition::{TransitionDsc, interpolate, is_interpolable};
pub use twine_anim::{AnimTemplate, Easing};
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
    pub use twine_anim::AnimTemplate;
    pub use twine_core::{Angle, Color, Duration, Fraction, Insets, Opa, Point, Scale};
    pub use twine_image::ImageSource;
    pub use twine_render::ShadowDsc;
    pub use twine_text::Font;

    pub use crate::transition::TransitionDsc;
    pub use crate::value_types::{
        Align, BaseDir, BlendMode, BlurQuality, BorderSide, ColorFilter, CrossAlign, DurationMs, FlexFlow,
        GradDir, Gradient, GridAlign, GridSpan, GridTrack, ImageColorkey, LayoutKind, Length, MainAlign,
        Radius, TextAlign, TextDecor, TextLeadingTrim,
    };

    use crate::StyleValue as V;
    use crate::value_types::COORD_MAX;

    /// `Opa::COVER` (constant of the `bg`, `border` and `outline` shorthands).
    pub const OPA_COVER: Opa = Opa::COVER;

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
