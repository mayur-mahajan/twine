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
//!     .align_items(CrossAlign::Center)
//! }
//! # let _ = counter;
//! ```
//!
//! An application is a function run **once** that describes its widgets; dynamic values are
//! fine-grained bindings, so a change updates exactly the affected widgets and redraws
//! exactly their pixels, and an idle UI uses no CPU at all.
//!
//! ## Guides
//!
//! - [Writing a custom widget](guide::custom_widgets): a gauge with its own class and parts,
//!   drawing, keypad/encoder input, theming with design elements and theme modes, a typed
//!   view, animations within the motion preference, and tests (the `gauge` example).
//!
//! ## Features
//!
//! - default: `color-rgb565`, `color-rgb565-swapped`, `img-qoi`, `perf-monitor`, `log`.
//! - `std` (host, simulator, tests: the thread-local reactive runtime), `log`, `defmt`
//!   (logging backends), `perf-monitor` (on-screen performance overlay), `debug-checks`
//!   (expensive engine invariant checks), `test-ids` (keep test ids in release builds).
//! - `async`: the async runtime (`AsyncUi`, `Ui::builder_async`) and the async HAL traits.
//! - `color-*`: pixel formats the renderer is compiled for (`color-i1` also compiles `L8`);
//!   `img-*`: image decoders (`qoi`, `png`, `jpeg`, `gif`, `bmp`, `lz4`).
//! - `montserrat-*`, `unscii-*`, `dejavu-*`, `source-han-*`, `all-fonts`: the built-in fonts
//!   of [`fonts`].
//! - `bidi`, `arabic-shaping`: right-to-left and Arabic/Persian text; `ttf`: runtime TrueType
//!   fonts; `fs`: the file system (`fs` module) and file images; `vector`: vector graphics
//!   (`vector` module, `vector_canvas`); `svg`: SVG images drawn as vectors (implies
//!   `vector`).
//! - `full`: every optional, platform-neutral part above (all pixel formats, decoders, fonts,
//!   `vector`, `svg`, `ttf`, `fs`, `bidi`, `arabic-shaping`, `async`, `perf-monitor`) — for
//!   hosts, simulators, tests and documentation. It leaves out `std`, the logging backends and
//!   the debugging aids; firmware enables only what it uses.
//!
//! ## Examples
//!
//! The runnable desktop examples (simulator windows) live in this crate's `examples/`
//! directory: `cargo run -p twine --example <name>` (e.g. `counter`, `thermostat`, `gauge`), or
//! `cargo xtask sim <name>` (also headless: `--headless`, `--script <file>`). Board examples
//! are in the repository's `firmware/` directory.
#![no_std]
#![forbid(unsafe_code)]

pub use twine_anim as anim;
pub use twine_assets as assets;
pub use twine_core as core;
pub use twine_engine as engine;
#[cfg(feature = "fs")]
pub use twine_fs as fs;
pub use twine_hal as hal;
pub use twine_image as image;
pub use twine_layout as layout;
pub use twine_reactive as reactive;
pub use twine_render as render;
pub use twine_style as style;
pub use twine_text as text;
pub use twine_theme as theme;
#[cfg(feature = "vector")]
pub use twine_vector as vector;
pub use twine_view as view;
pub use twine_widgets as widgets;
pub use twine_widgets_ext as widgets_ext;

/// The built-in fonts (each behind its cargo feature).
pub use twine_assets::fonts;

pub mod guide;

/// Everything an application needs: `use twine::prelude::*;`.
///
/// Views, modifiers, control flow, the reactive primitives, styles, colors, geometry, time,
/// animation, themes, navigation, the `Ui` runtime and the built-in [`fonts`].
///
/// ```
/// use twine::prelude::*;
///
/// pub static CARD: Style = style! { bg_color: Color::WHITE, bg_opacity: Opa::COVER, radius: 8, padding: 12 };
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
