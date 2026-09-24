//! # twine-theme
//!
//! Themes for the Twine GUI library: they give every widget its default look, exactly like
//! LVGL's themes (`src/themes`). A theme is installed per display with
//! [`Engine::set_theme`](twine_engine::Engine::set_theme); the engine then calls it for every
//! node it creates, before the widget's own `init`, and the theme adds styles with the
//! lowest priority, so anything an application sets wins.
//!
//! | Theme | LVGL | Look |
//! |-------|------|------|
//! | [`DefaultTheme`] | `lv_theme_default` | material-like, light or dark, primary/secondary colors, rounded cards and buttons with shadows, focus outlines, transitions, DPI-scaled |
//! | [`SimpleTheme`] | `lv_theme_simple` | flat grey tones, minimal styling |
//! | [`MonoTheme`] | `lv_theme_mono` | pure black and white for 1-bit displays |
//!
//! The engine only knows the object-safe [`ThemeHook`] (so `twine-engine` does not depend on
//! this crate); [`Theme`] extends it with the fonts and colors widgets and applications ask
//! a theme for. Themes chain like LVGL's `lv_theme_set_parent`: `with_parent` applies another
//! theme first, so the child theme only adds or overrides styles.
//!
//! [`Palette`] is LVGL's material palette with identical values, and [`dpx`] its DPI scaling.
//!
//! ## Example
//!
//! ```
//! use std::rc::Rc;
//! use twine_theme::{DefaultTheme, Palette, Theme, ThemeMode};
//!
//! let light = DefaultTheme::light();
//! assert_eq!(light.color_primary(), Palette::Blue.main());
//! let custom = DefaultTheme::new(Palette::Teal, Palette::Amber, ThemeMode::Dark, &twine_assets::fonts::MONTSERRAT_14);
//! assert_eq!(custom.mode(), ThemeMode::Dark);
//! let _hook: Rc<dyn twine_engine::ThemeHook> = Rc::new(custom);
//! ```
//!
//! ## Features
//!
//! - `assets` (default): the ready-made constructors ([`DefaultTheme::light`],
//!   [`DefaultTheme::dark`], [`SimpleTheme::new`]) use the built-in Montserrat 14 font of
//!   `twine-assets`. Without it, pass a font to the `with_font` constructors.
//! - `std`, `log`, `defmt`: as in every Twine crate.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

pub mod default;
mod dpx;
mod mono;
mod palette;
mod simple;
mod theme;

pub use default::{DefaultTheme, DisplaySize, ThemeMode};
pub use dpx::{DPI_DEF, dpx};
pub use mono::MonoTheme;
pub use palette::Palette;
pub use simple::SimpleTheme;
pub use theme::Theme;
pub use twine_engine::{ThemeCx, ThemeHook};
