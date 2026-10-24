//! A design system (R7.S03) built from a shipped design language that lists one of its two mode
//! tables and two of its three recipes, with a button variant: no LVGL (or other built-in)
//! theme, no unlisted table or recipe may be linked.
#![no_std]
#![no_main]

use twine::engine::Theme;
use twine::prelude::*;
use twine_link_fixtures::design;

/// The application's design system: the language's light mode, screens and buttons.
struct App;

impl DesignSystem for App {
    type Base = twine::engine::NoBase;
    const NAME: &'static str = "app";
    const MODES: &'static [(ThemeMode, &'static twine::style::design::ElementTable)] =
        &[(ThemeMode::Light, &design::LIGHT)];
    const RECIPES: &'static [&'static Recipe] = &[&design::SCREEN, &design::BUTTON];
}

/// The application's theme.
static THEME: Theme<App> = Theme::new();

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new().theme(&THEME), |_| {
        column((label("Hello"), button(label("OK")).variant(ButtonVariant::Tinted)))
    })
}
