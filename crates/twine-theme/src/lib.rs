//! # twine-theme
//!
//! Themes for the Twine GUI library: they give every widget its default look, exactly like
//! LVGL's themes (`src/themes`). A theme is a `static` — **one theme per display, fixed at
//! compile time** (design `docs/plan/r7-theming-design.md` §2.0a) — given to a display when
//! it is created (`AppConfig::theme(&THEME)` / `DisplayBuilder::theme(&THEME)` in
//! `twine-view`, a [`DisplayTheme`](twine_engine::DisplayTheme) in the engine). The engine
//! then calls it for every node it creates, before the widget's own `init`, and the theme adds
//! styles with the lowest priority, so anything an application sets wins.
//!
//! | Theme | Cargo feature | LVGL | Look |
//! |-------|---------------|------|------|
//! | [`Lvgl`] | `lvgl` (default) | `lv_theme_default` | material-like, light, dark, night or high contrast, primary/secondary colors, rounded cards and buttons with shadows, focus outlines, transitions, DPI-scaled |
//! | `Simple` | `simple` | `lv_theme_simple` | flat grey tones, minimal styling |
//! | `Mono` | `mono` | `lv_theme_mono` | pure black and white for 1-bit displays |
//!
//! A theme is **data**: its styles are `static`s (`lvgl::styles`, `mono::styles`,
//! `simple::styles`), its colors, fonts and motion one `'static` [design
//! element](twine_style::design) table per [`ThemeMode`] (`lvgl::LIGHT`, …), and its per-class
//! looks [`ComputedRecipe`](twine_engine::ComputedRecipe)s (`lvgl::ALL_RECIPES`, …, or the
//! ones the application lists with `.recipes(&[..])`). Only the
//! LVGL theme computes something at run time: the sizes it scales by the display's DPI and
//! size class, a small table per display built once when the display is created
//! (`lvgl::metrics`). Nothing is allocated when the mode switches.
//!
//! ```
//! use twine_style::ThemeMode;
//! use twine_theme::{Lvgl, Mono, Simple, ThemeHook};
//!
//! static THEME: Lvgl = Lvgl::new(); // LVGL's look in all four modes
//! assert_eq!(ThemeHook::modes(&THEME).len(), 4);
//! static OLED: Mono = Mono::new();
//! static PLAIN: Simple = Simple::new();
//! assert_eq!(ThemeHook::modes(&PLAIN).len(), 4);
//! # let _ = &OLED;
//! ```
//!
//! ## Customization
//!
//! Everything is `const` (nothing is built at run time):
//!
//! - **Tune** a theme's element set and list the modes the product shows:
//!   `static LIGHT: ElementTable = Lvgl::SET.primary(Palette::Teal.main()).table(ThemeMode::Light);`
//!   then `static THEME: Lvgl = Lvgl::new().modes(&[(ThemeMode::Light, &LIGHT)]);` — an
//!   unlisted mode's table is not linked. Application elements and overrides of single
//!   elements are `ElementTable`'s const builders on such a table.
//! - **Trim** a theme to the widget classes the product creates:
//!   `static THEME: Lvgl = Lvgl::new().recipes(&[&lvgl::OBJ, &lvgl::LABEL, &lvgl::BUTTON]);`
//!   — only the listed per-class recipes and the styles they use are linked (default: the
//!   theme's `ALL_RECIPES`). `twine_testing::assert_theme_covers` checks the list against a
//!   headless run of the UI, and `cargo xtask theme-recipes` prints the minimal list.
//! - **Extend** a theme with a [`DesignSystem`](twine_engine::DesignSystem) whose
//!   `type Base = Lvgl;` (or `Mono`, `Simple`): its recipes (written with
//!   [`recipe!`](twine_engine::recipe), reusing the theme's public `static` styles as `..`
//!   spreads) restyle the classes it names — custom widgets, variants and roles — and every
//!   other class keeps the base's look.
//!
//! ```
//! use twine_engine::{DesignSystem, Recipe, Theme, ThemeHook, WidgetClass, recipe};
//! use twine_style::design::ElementTable;
//! use twine_style::{Radius, Selector, State, ThemeMode};
//! use twine_theme::{Lvgl, lvgl};
//! use twine_widgets::button::BUTTON_CLASS;
//!
//! /// A key of an on-screen numpad: themed like a button...
//! static KEY_CLASS: WidgetClass = WidgetClass::new("numpad_key").parts(BUTTON_CLASS.parts).base(&BUTTON_CLASS);
//!
//! recipe! {
//!     // ...as a pill.
//!     static KEY for KEY_CLASS {
//!         Selector::MAIN => { ..lvgl::styles::BTN, ..lvgl::styles::BG_COLOR_PRIMARY, radius: Radius::Circle },
//!         State::PRESSED => { ..lvgl::styles::PRESSED },
//!     }
//! }
//! struct Numpad;
//! impl DesignSystem for Numpad {
//!     type Base = Lvgl;
//!     const NAME: &'static str = "numpad";
//!     const MODES: &'static [(ThemeMode, &'static ElementTable)] = lvgl::MODES;
//!     const RECIPES: &'static [&'static Recipe] = &[&KEY];
//! }
//! static THEME: Theme<Numpad> = Theme::new();
//! assert_eq!(THEME.recipe(&KEY_CLASS).map(|r| r.name()), Some("KEY"));
//! assert_eq!(THEME.recipe(&BUTTON_CLASS).map(|r| r.name()), Some("BUTTON")); // LVGL's
//! ```
//!
//! ## Widget classes
//!
//! Themes recognise widget classes by **identity** (`&'static WidgetClass`, a pointer
//! comparison — this crate depends on `twine-widgets` and `twine-widgets-ext` for their class
//! statics), never by name. A class a theme does not know is styled like the nearest class of
//! its [`WidgetClass::base`](twine_engine::WidgetClass::base) chain it knows, so a custom
//! widget with `base: Some(&BUTTON_CLASS)` looks like a button in every theme, and so do
//! variants (`ButtonVariant::Tinted`) and roles
//! ([`Engine::set_theme_class`](twine_engine::Engine::set_theme_class)). Matching runs when a
//! node is created or re-themed — never while drawing — and allocates nothing.
//!
//! ## Theme modes for regulated displays
//!
//! | Theme | `Light` | `Dark` | `Night` | `HighContrast` |
//! |-------|---------|--------|---------|----------------|
//! | [`Lvgl`] | LVGL | LVGL | warm amber on near-black, ≤ 30 % luminance, blue ≤ red / 2 | white on black, yellow accent, opaque cyan focus ring |
//! | `Mono` | black on white | white on black | white on black | black on white |
//! | `Simple` | LVGL | flat greys inverted | dim warm amber, ≤ 30 % luminance, blue ≤ red / 2 | black on white, grey buttons with black text |
//!
//! A mode a theme does not list is refused: the display keeps its mode (with a warning).
//! Each mode specifies minimum WCAG contrast ratios ([`ThemeMode::min_contrast`]:
//! text 4.5:1, 7:1 in `HighContrast`; text on accents 3:1 in `Light`/`Dark`, 4.5:1 at night,
//! 7:1 in `HighContrast`; focus ring and disabled content 3:1 at night, 4.5:1 in
//! `HighContrast`), and every built-in theme meets them with its default colors (tested with
//! [`ElementTable::check_contrast`](twine_style::design::ElementTable::check_contrast)). The
//! ratios (`Mono`: 21:1 for every pair in every mode):
//!
//! | Theme, mode | on-surface / surface | / background | / surface-variant | muted / surface | on-primary / primary | on-secondary / secondary | focus ring / surface | disabled content / surface |
//! |-------------|------|------|------|------|------|------|------|------|
//! | lvgl, `Light` | 16.10 | 14.77 | 12.20 | 4.61 | 3.12 | 3.68 | 1.75 | 3.95 |
//! | lvgl, `Dark` | 13.61 | 17.21 | 12.33 | 7.56 | 3.12 | 3.68 | 2.14 | 6.33 |
//! | lvgl, `Night` | 6.15 | 6.63 | 5.04 | 4.87 | 4.92 | 4.93 | 6.09 | 3.53 |
//! | lvgl, `HighContrast` | 21.00 | 21.00 | 10.37 | 18.10 | 14.87 | 21.00 | 13.65 | 11.42 |
//! | simple, `Light` | 6.19 | 5.68 | 4.69 | 4.61 | 7.84 | 6.19 | 2.68 | 2.61 |
//! | simple, `Dark` | 12.63 | 14.19 | 8.62 | 6.22 | 4.61 | 14.19 | 6.22 | 5.02 |
//! | simple, `Night` | 6.15 | 6.63 | 5.70 | 4.87 | 4.89 | 6.63 | 4.87 | 3.53 |
//! | simple, `HighContrast` | 21.00 | 21.00 | 15.91 | 16.10 | 7.84 | 21.00 | 21.00 | 8.32 |
//!
//! (`Light`/`Dark` focus rings are LVGL's 50 % translucent primary and are not specified.)
//!
//! [`Palette`] is LVGL's material palette with identical values; DPI scaling is
//! [`twine_style::dpx`] (also used by the engine for `Length::dp` values).
//!
//! ## Features
//!
//! - `lvgl` (default), `mono`, `simple`: the themes. A theme's code and data are linked only
//!   when the application names it.
//! - `assets` (default): the default element sets (`Lvgl::SET`, `Mono::SET`, `Simple::SET`)
//!   and tables use the built-in Montserrat 14 font of `twine-assets`. Without it they have
//!   the empty font: give the set a font.
//! - `std`, `log`, `defmt`: as in every Twine crate.
//! - `safety`: the safety profile (enabled by the `twine` facade's `safety` feature, see
//!   `docs/safety/profile.md` in the repository): the profile's lints.
#![no_std]
#![forbid(unsafe_code)]
// The safety profile's lints (`docs/safety/profile.md` § Lints): denied in library code under
// the `safety` feature (`cargo xtask ci` stage `clippy-safety`).
#![cfg_attr(
    all(feature = "safety", not(test)),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::arithmetic_side_effects,
    )
)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

#[cfg(feature = "lvgl")]
pub mod lvgl;
#[cfg(any(feature = "lvgl", feature = "mono", feature = "simple"))]
mod modes;
#[cfg(feature = "mono")]
pub mod mono;
mod palette;
#[cfg(any(feature = "lvgl", feature = "mono", feature = "simple"))]
mod recipes;
#[cfg(feature = "simple")]
pub mod simple;
mod theme;

#[cfg(feature = "lvgl")]
pub use lvgl::{DisplaySize, Lvgl, LvglSet};
#[cfg(feature = "mono")]
pub use mono::{Mono, MonoSet};
pub use palette::{Palette, Tone};
#[cfg(feature = "simple")]
pub use simple::{Simple, SimpleSet};
pub use theme::{FontScale, Theme};
pub use twine_engine::{ThemeCx, ThemeHook};
pub use twine_style::ThemeMode;
