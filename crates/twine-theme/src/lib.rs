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
//! | [`DefaultTheme`] | `lv_theme_default` | material-like, light, dark, night or high contrast, primary/secondary colors, rounded cards and buttons with shadows, focus outlines, transitions, DPI-scaled |
//! | [`SimpleTheme`] | `lv_theme_simple` | flat grey tones, minimal styling |
//! | [`MonoTheme`] | `lv_theme_mono` | pure black and white for 1-bit displays |
//!
//! Every theme is configured with a builder ([`DefaultTheme::builder`],
//! [`MonoTheme::builder`], [`SimpleTheme::builder`]): colors, starting [`ThemeMode`], fonts
//! ([`FontScale`]), a parent theme, design elements and application classes; anything not
//! set keeps LVGL's default. [`Palette`] colors come in ten [`Tone`]s.
//!
//! The engine only knows the object-safe [`ThemeHook`] (so `twine-engine` does not depend on
//! this crate); [`Theme`] extends it with the fonts applications ask a theme for. Themes chain
//! like LVGL's `lv_theme_set_parent`: a builder's `.parent(..)` applies another theme first, so
//! the child theme only adds or overrides styles.
//!
//! ## Widget classes
//!
//! Themes recognise widget classes by **identity** (`&'static WidgetClass`, a pointer
//! comparison — this crate depends on `twine-widgets` and `twine-widgets-ext` for their class
//! statics), never by name. A class a theme does not know is styled like the nearest class of
//! its [`WidgetClass::base`](twine_engine::WidgetClass::base) chain it knows, so a custom
//! widget with `base: Some(&BUTTON_CLASS)` looks like a button in every theme. Applications
//! style their own classes (or add to built-in ones) with the builders' `.class(&MY_CLASS,
//! |cx| ..)`, reusing the theme's styles ([`ClassCx::styles`]; also
//! [`DefaultTheme::styles`], [`MonoTheme::styles`], [`SimpleTheme::styles`]) so custom
//! widgets look native:
//!
//! ```
//! use std::rc::Rc;
//! use twine_engine::WidgetClass;
//! use twine_style::{Part, Selector};
//! use twine_theme::DefaultTheme;
//! use twine_widgets::button::BUTTON_CLASS;
//!
//! /// A key of an on-screen numpad: themed like a button...
//! static KEY_CLASS: WidgetClass = WidgetClass::new("numpad_key").parts(BUTTON_CLASS.parts).base(&BUTTON_CLASS);
//!
//! // ...with the theme's pill radius on top.
//! let theme = DefaultTheme::builder()
//!     .class(&KEY_CLASS, |cx| {
//!         let circle = cx.styles().circle.clone();
//!         cx.add_style(Selector::MAIN, circle);
//!     })
//!     .build();
//! let _hook: Rc<dyn twine_engine::ThemeHook> = Rc::new(theme);
//! ```
//!
//! Matching runs when a node is created or re-themed — never while drawing — and costs one
//! pointer comparison per class the theme knows (at most [`MAX_CLASS_DEPTH`] classes of a
//! lineage), plus one per application registration; it allocates nothing.
//!
//! [`MAX_CLASS_DEPTH`]: twine_engine::MAX_CLASS_DEPTH
//!
//! Every built-in theme defines the standard [design elements](twine_style::design)
//! (`design::SURFACE`, `design::PRIMARY`, `design::SPACE_M`, …) in each [`ThemeMode`] it
//! supports and uses them in its own styles, so the mode switches at run time
//! ([`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode), cycling through
//! [`ThemeHook::modes`]) without re-applying a style. Applications add their own elements (or
//! override standard ones) with the builders' `.element(..)` / `.element_in(mode, ..)`.
//!
//! ## Theme modes for regulated displays
//!
//! | Theme | `Light` | `Dark` | `Night` | `HighContrast` |
//! |-------|---------|--------|---------|----------------|
//! | [`DefaultTheme`] | LVGL | LVGL | warm amber on near-black, ≤ 30 % luminance, blue ≤ red / 2 | white on black, yellow accent, opaque cyan focus ring |
//! | [`MonoTheme`] | black on white | white on black | white on black | black on white |
//! | [`SimpleTheme`] | LVGL | — | — | black on white, grey buttons with black text |
//!
//! A mode a theme does not support is refused: the display keeps its mode (with a warning).
//! Each mode specifies minimum WCAG contrast ratios ([`ThemeMode::min_contrast`]:
//! text 4.5:1, 7:1 in `HighContrast`; text on accents 3:1 in `Light`/`Dark`, 4.5:1 at night,
//! 7:1 in `HighContrast`; focus ring and disabled content 3:1 at night, 4.5:1 in
//! `HighContrast`), and every built-in theme meets them with its default colors (tested with
//! [`ElementTable::check_contrast`](twine_style::design::ElementTable::check_contrast)). The
//! ratios (`MonoTheme`: 21:1 for every pair in every mode):
//!
//! | Theme, mode | on-surface / surface | / background | / surface-variant | muted / surface | on-primary / primary | on-secondary / secondary | focus ring / surface | disabled content / surface |
//! |-------------|------|------|------|------|------|------|------|------|
//! | default, `Light` | 16.10 | 14.77 | 12.20 | 4.61 | 3.12 | 3.68 | 1.75 | 3.95 |
//! | default, `Dark` | 13.61 | 17.21 | 12.33 | 7.56 | 3.12 | 3.68 | 2.14 | 6.33 |
//! | default, `Night` | 6.15 | 6.63 | 5.04 | 4.87 | 4.92 | 4.93 | 6.09 | 3.53 |
//! | default, `HighContrast` | 21.00 | 21.00 | 10.37 | 18.10 | 14.87 | 21.00 | 13.65 | 11.42 |
//! | simple, `Light` | 6.19 | 5.68 | 4.69 | 4.61 | 7.84 | 6.19 | 2.68 | 2.61 |
//! | simple, `HighContrast` | 21.00 | 21.00 | 15.91 | 16.10 | 7.84 | 21.00 | 21.00 | 8.32 |
//!
//! (`Light`/`Dark` focus rings are LVGL's 50 % translucent primary and are not specified.)
//!
//! [`Palette`] is LVGL's material palette with identical values; DPI scaling is
//! [`twine_style::dpx`] (also used by the engine for `Length::dp` values).
//!
//! ## Example
//!
//! ```
//! use std::rc::Rc;
//! use twine_theme::{DefaultTheme, FontScale, Palette, ThemeHook, ThemeMode, Tone};
//!
//! let light = DefaultTheme::light();
//! assert_eq!(light.primary(), Palette::Blue.main());
//! let font = &twine_assets::fonts::MONTSERRAT_14;
//! let custom = DefaultTheme::builder()
//!     .primary(Palette::Teal)
//!     .secondary(Palette::Amber.tone(Tone::D2))
//!     .mode(ThemeMode::Dark)
//!     .fonts(FontScale::uniform(font))
//!     .build();
//! assert_eq!(custom.mode(), ThemeMode::Dark);
//! let _hook: Rc<dyn twine_engine::ThemeHook> = Rc::new(custom);
//! ```
//!
//! ## Features
//!
//! - `assets` (default): the builders start with the built-in Montserrat 14 font of
//!   `twine-assets`, and the shortcuts [`DefaultTheme::light`], [`DefaultTheme::dark`] and
//!   [`SimpleTheme::new`] exist. Without it, set a font with the builders' `fonts` / `font`
//!   (a theme built without one draws no text and logs a warning).
//! - `std`, `log`, `defmt`: as in every Twine crate.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

mod class;
pub mod default;
mod design;
mod mono;
mod palette;
mod simple;
mod theme;

pub use class::ClassCx;
pub use default::{DefaultTheme, DefaultThemeBuilder, DisplaySize, ThemeMode};
pub use mono::{MonoStyles, MonoTheme, MonoThemeBuilder};
pub use palette::{Palette, Tone};
pub use simple::{SimpleStyles, SimpleTheme, SimpleThemeBuilder};
pub use theme::{FontScale, Theme};
pub use twine_engine::{ThemeCx, ThemeHook};
