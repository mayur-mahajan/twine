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
//! use twine_style::{PropId, Style, StyleBuf, StyleValue, style};
//!
//! // In flash: a `static` built by the `style!` macro.
//! static CARD: Style = style! { bg_color: Color::WHITE, bg_opa: Opa::COVER, radius: 8, pad_all: 12 };
//! // On the heap, e.g. built by a theme at run time.
//! let accent = StyleBuf::new().bg_color(Color::hex(0x2196F3)).border_width(2);
//! assert_eq!(CARD.get(PropId::PadTop), Some(StyleValue::Int(12)));
//! assert_eq!(accent.get(PropId::BorderWidth), Some(StyleValue::Int(2)));
//! ```
//!
//! A card that shrinks slightly and darkens while pressed, with a transition (the widget and
//! view parts that attach these to a node come from the engine and view crates):
//!
//! ```
//! use twine_core::{Color, Duration, Opa};
//! use twine_style::{Easing, PropId, Style, TransitionDsc, style};
//!
//! pub static CARD: Style = style! {
//!     bg_color: Color::WHITE,
//!     bg_opa: Opa::COVER,
//!     radius: 8,
//!     pad_all: 12,              // shorthand → pad_top/bottom/left/right
//!     shadow_width: 12,
//!     shadow_opa: Opa::P30,
//! };
//! pub static CARD_PRESSED: Style = style! { bg_color: Color::hex(0xEEEEEE), transform_scale: 250 };
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
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod macros;
mod prop;
mod resolve;
mod selector;
mod style;
#[cfg(test)]
mod test_util;
mod transition;
mod value;
mod value_types;

pub use prop::{PROP_COUNT, PROP_META, PropFlags, PropId, PropMeta, StyleProp};
pub use resolve::{
    EntryKind, EntryTrace, ResolveOptions, StyleDefaults, StyleEntry, StyleSource, TraceEvent, resolve,
    resolve_angle, resolve_as, resolve_bool, resolve_color, resolve_font, resolve_i32, resolve_length,
    resolve_local_only, resolve_opa, resolve_scale, resolve_traced, resolve_with,
};
pub use selector::{Part, Selector, State};
pub use style::{Style, StyleBuf, StyleRef};
pub use transition::{TransitionDsc, interpolate, is_interpolable};
pub use twine_anim::{AnimTemplate, Easing};
pub use value::{PropValue, StyleValue};
#[doc(hidden)]
pub use value_types::{__LengthArg, __ScaleArg};
pub use value_types::{
    Align, BaseDir, BlendMode, BlurQuality, BorderSide, COORD_MAX, ColorFilter, Dir, FlexAlign, FlexFlow,
    GradDir, Gradient, GridAlign, GridTrack, ImageColorkey, LayoutKind, Length, RADIUS_CIRCLE, ScrollSnap,
    ScrollbarMode, StyleEnum, TextAlign, TextDecor, TextLeadingTrim,
};
