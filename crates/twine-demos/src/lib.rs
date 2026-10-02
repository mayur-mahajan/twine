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
#[cfg(feature = "vector")]
pub mod vector;

use twine::prelude::{AppConfig, DefaultTheme, IntoTheme};

/// The configuration every demo ships with: LVGL's light default theme and the default engine
/// configuration, motion and budgets. Shared by the firmware (`Ui::builder(..).app_config(
/// twine_demos::config())`, which adds its board hooks such as `hires_timer`), the simulator
/// examples (`SimConfig::new(..).app_config(twine_demos::config())`) and the tests
/// (`TestUi::new(..).app_config(twine_demos::config())`). A board may refine it; the OLED
/// boards take it with their monochrome theme ([`config_with_theme`]).
///
/// Builds the theme (one `Rc`) on every call; call it once per UI. Never panics.
///
/// ```
/// use twine::prelude::*;
///
/// let cfg = twine_demos::config();
/// assert!(cfg.theme.is_some());
/// assert_eq!(cfg.motion, Motion::Full);
/// ```
pub fn config() -> AppConfig {
    config_with_theme(DefaultTheme::light())
}

/// [`config`] with `theme` instead of the default theme: what a board whose display needs
/// another theme (the OLED boards' monochrome one) runs. Prefer it to replacing the theme of
/// [`config`] afterwards: a theme that is built is linked — LVGL's default theme is ~14 KiB of
/// flash — even when it is replaced before the UI is built.
///
/// Builds nothing but the configuration; never panics.
///
/// ```
/// use twine::prelude::*;
///
/// let cfg = twine_demos::config_with_theme(MonoTheme::builder().mode(ThemeMode::Dark).build());
/// assert_eq!(cfg.theme.as_ref().map(|t| t.name()), Some("mono"));
/// assert_eq!(cfg.motion, twine_demos::config().motion);
/// ```
pub fn config_with_theme(theme: impl IntoTheme) -> AppConfig {
    AppConfig::new().theme(theme)
}
