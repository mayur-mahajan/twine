//! A label in an application font generated without hinting and without text gamma
//! (`include_font!` with `hinting = None, gamma = Linear`) and no theme: the default text-gamma
//! tables (`twine_text::gamma::DEFAULT`) may not be linked (R6.S03: a font generated without
//! the options links nothing new).
#![no_std]
#![no_main]

use twine::prelude::*;

twine::include_font!(
    static PLAIN = "../../assets/fonts/Montserrat-Medium.ttf",
    size = 14,
    chars = ['A'..='Z', 'a'..='z'],
    hinting = None,
    gamma = Linear,
);

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new(), |_| column((label("Hello").font(&PLAIN),)))
}
