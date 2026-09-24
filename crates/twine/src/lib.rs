//! # twine
//!
//! Twine: a declarative, signal-based, low-power GUI library for embedded devices with the
//! feature set of LVGL, `no_std` + `alloc`.
//!
//! This is the facade crate: it re-exports every Twine crate under a short module name and
//! gathers everything an application needs in [`prelude`].
//!
//! ```
//! use twine::prelude::*;
//!
//! pub fn counter(cx: Scope) -> impl View {
//!     let count = cx.signal(0u32);
//!     column((
//!         label(text!("Clicked {} times", count.get())).test_id("count"),
//!         button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
//!     ))
//!     .gap(12)
//!     .padding(16)
//!     .align_items(FlexAlign::Center)
//! }
//! # let _ = counter;
//! ```
//!
//! An application is a function run **once** that describes its widgets; dynamic values are
//! fine-grained bindings, so a change updates exactly the affected widgets and redraws
//! exactly their pixels, and an idle UI uses no CPU at all.
//!
//! ## Features
//!
//! - default: `color-rgb565`, `color-rgb565-swapped`, `img-qoi`, `perf-monitor`, `log`.
//! - `std` (host, simulator, tests), `log`, `defmt`, `debug-checks`, `test-ids`.
//! - `color-*`: pixel formats; `img-*`: image decoders; `montserrat-*`, `unscii-*`, …: the
//!   built-in fonts of [`fonts`].
#![no_std]

pub use twine_anim as anim;
pub use twine_assets as assets;
pub use twine_core as core;
pub use twine_engine as engine;
pub use twine_hal as hal;
pub use twine_image as image;
pub use twine_layout as layout;
pub use twine_reactive as reactive;
pub use twine_render as render;
pub use twine_style as style;
pub use twine_text as text;
pub use twine_theme as theme;
pub use twine_view as view;
pub use twine_widgets as widgets;
pub use twine_widgets_ext as widgets_ext;

/// The built-in fonts (each behind its cargo feature).
pub use twine_assets::fonts;

/// Everything an application needs: `use twine::prelude::*;`.
///
/// Views, modifiers, control flow, the reactive primitives, styles, colors, geometry, time,
/// animation, themes, navigation, the `Ui` runtime and the built-in [`fonts`].
///
/// ```
/// use twine::prelude::*;
///
/// pub static CARD: Style = style! { bg_color: Color::WHITE, bg_opa: Opa::COVER, radius: 8, pad_all: 12 };
///
/// fn card(title: &'static str) -> impl View {
///     container(label(title)).style(&CARD)
/// }
/// # let _ = card;
/// ```
pub mod prelude {
    pub use twine_assets::fonts;
    pub use twine_view::prelude::*;
}
