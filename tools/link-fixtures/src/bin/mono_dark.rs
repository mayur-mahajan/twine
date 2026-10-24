//! The monochrome theme in its dark mode with a label and a button: no other theme, no other
//! mode's table, and no recipe or style of a widget class it never creates, may be linked.
#![no_std]
#![no_main]

use twine::prelude::*;

/// The mono theme listing its dark table and the recipes of the UI's classes only.
static THEME: Mono = Mono::new()
    .modes(&[(ThemeMode::Dark, &mono::DARK)])
    .recipes(&[&mono::OBJ, &mono::LABEL, &mono::BUTTON]);

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new().theme(&THEME), |_| {
        column((label("Hello"), button(label("OK"))))
    })
}
