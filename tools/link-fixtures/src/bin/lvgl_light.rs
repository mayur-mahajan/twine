//! The LVGL theme in its light mode only, with a label and a button and the theme's own font:
//! no other theme, no other mode's table, no recipe or style of a widget class it never
//! creates, no other font or icon may be linked.
#![no_std]
#![no_main]

use twine::prelude::*;

/// The LVGL theme listing its light table and the recipes of the classes the UI creates
/// only (screen and column: `OBJ`; `LABEL`; `BUTTON` — `cargo xtask theme-recipes`).
static THEME: Lvgl = Lvgl::new()
    .modes(&[(ThemeMode::Light, &lvgl::LIGHT)])
    .recipes(&[&lvgl::OBJ, &lvgl::LABEL, &lvgl::BUTTON]);

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new().theme(&THEME), |_| {
        column((label("Hello"), button(label("OK"))))
    })
}
