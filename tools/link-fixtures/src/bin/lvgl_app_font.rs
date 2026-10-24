//! The LVGL theme in its light mode with the application's own font (Montserrat 20) for every
//! text size, and a label and a button: the theme's default font (Montserrat 14) is not
//! referenced and may not be linked.
#![no_std]
#![no_main]

use twine::prelude::*;
use twine::style::design::ElementTable;
use twine::theme::FontScale;

/// LVGL's light table with the application's font.
static LIGHT: ElementTable = Lvgl::SET
    .fonts(FontScale::uniform(&fonts::MONTSERRAT_20))
    .table(ThemeMode::Light);
/// The LVGL theme listing that table and the recipes of the UI's classes only.
static THEME: Lvgl = Lvgl::new()
    .modes(&[(ThemeMode::Light, &LIGHT)])
    .recipes(&[&lvgl::OBJ, &lvgl::LABEL, &lvgl::BUTTON]);

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new().theme(&THEME), |_| {
        column((label("Hello"), button(label("OK"))))
    })
}
