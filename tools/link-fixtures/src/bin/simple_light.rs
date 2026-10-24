//! The simple theme in its light mode with a label and a button: no other theme, no other
//! mode's table, and no recipe or style of a widget class it never creates, may be linked.
#![no_std]
#![no_main]

use twine::prelude::*;

/// The simple theme listing its light table and the recipes of the UI's classes only.
static THEME: Simple = Simple::new()
    .modes(&[(ThemeMode::Light, &simple::LIGHT)])
    .recipes(&[&simple::OBJ, &simple::LABEL, &simple::BUTTON]);

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new().theme(&THEME), |_| {
        column((label("Hello"), button(label("OK"))))
    })
}
