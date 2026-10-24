//! # twine-demos
//!
//! Demo applications of the Twine GUI library, shared by the desktop simulator examples and
//! the firmware of the supported boards. Each demo is a module with
//! `pub fn app(cx: Scope) -> impl View` — or `app(cx, ports)` when it talks to other tasks or
//! interrupts ([`thermostat::Ports`]: the caller owns the cross-context objects); all of them
//! are `no_std` and build for every embedded target.
//!
//! | Demo | Shows |
//! |------|-------|
//! | [`calibration`] | touch calibration for resistive touch (raw readings through a channel) |
//! | [`counter`] | signals, `text!`, dynamic properties (`disabled`), click handlers |
//! | [`thermostat`] | data from another task or interrupt ([`Latest`](twine::prelude::Latest)), commands out ([`Outbox`](twine::prelude::Outbox)), the ports pattern ([`thermostat::Ports`]), tweens, memos |
//! | [`todos`] | the store pattern: app state as a `Copy` struct of signals and memos provided as context ([`todos::Todos`]), components reading it, actions as methods, commands from outside through ports; a keyed list |
//! | [`controls`] | every basic control; value widgets sharing signals (two-way bindings) |
//! | [`text_input`] | text fields, the on-screen keyboard, a spinbox and a rich-text preview |
//! | [`selection`] | dropdown, roller, list, menu, tabview, tileview, window and message box |
//! | `lottie` | Lottie animations with play/pause, loop and a two-way frame scrubber (feature `lottie`) |
//! | `vector` | paths, gradients, strokes, SVG icons, an animated path (feature `vector`) |
//! | `multilang` | translations (`tr!`), right-to-left and Arabic text, font fallback, a file image (feature `multilang`) |
//!
//! ## One configuration everywhere
//!
//! [`config`] is the demos' [`AppConfig`]: the firmware mains give
//! it to `Ui::builder(..).app_config(..)`, the simulator examples to
//! `SimConfig::app_config(..)` and the tests to `TestUi::app_config(..)`, so all three run the
//! same engine configuration, theme, motion and budgets — and draw the same pixels.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod assets;
pub mod calibration;
pub mod controls;
pub mod counter;
#[cfg(feature = "lottie")]
pub mod lottie;
#[cfg(feature = "multilang")]
pub mod multilang;
pub mod selection;
pub mod text_input;
pub mod thermostat;
pub mod todos;
#[cfg(feature = "vector")]
pub mod vector;

use twine::prelude::{AppConfig, ThemeHook, lvgl};

/// The configuration every demo ships with: the LVGL theme ([`lvgl::THEME`]: light first, all
/// four modes, every recipe) and the default engine configuration, motion and budgets, as the
/// simulator examples run it (`SimConfig::new(..).app_config(twine_demos::config())`). The
/// firmware and the demos' tests take the same configuration with the demo's own theme
/// ([`config_with_theme`]`(&counter::THEME)`, …: the LVGL theme with only the recipes that
/// demo uses, so the firmware does not link the others), the OLED boards with their
/// monochrome theme; a board adds its hooks such as `hires_timer`.
///
/// Builds nothing but the configuration (the theme is a `static`); never panics.
///
/// ```
/// use twine::prelude::*;
///
/// let cfg = twine_demos::config();
/// assert_eq!(cfg.theme.map(|t| t.name()), Some("lvgl"));
/// assert_eq!(cfg.motion, Motion::Full);
/// ```
pub fn config() -> AppConfig {
    config_with_theme(&lvgl::THEME)
}

/// [`config`] with `theme` instead of the LVGL theme: what a board whose display needs another
/// theme (the OLED boards' monochrome one) runs. A theme is linked only when it is named, so
/// a board that passes its own theme here links neither the LVGL theme nor its tables.
///
/// Builds nothing but the configuration; never panics.
///
/// ```
/// use twine::prelude::*;
///
/// /// The LVGL theme in dark mode only (the other modes' tables are not linked).
/// static DARK: Lvgl = Lvgl::new().modes(&[(ThemeMode::Dark, &lvgl::DARK)]);
/// let cfg = twine_demos::config_with_theme(&DARK);
/// assert_eq!(cfg.theme.map(|t| t.modes().len()), Some(1));
/// assert_eq!(cfg.motion, twine_demos::config().motion);
/// ```
pub fn config_with_theme(theme: &'static dyn ThemeHook) -> AppConfig {
    AppConfig::new().theme(theme)
}
