//! A label and a button without a theme: no theme of any kind may be linked.
#![no_std]
#![no_main]

use twine::prelude::*;

/// The entry point (`link.x`).
#[unsafe(no_mangle)]
pub extern "C" fn Reset() -> ! {
    twine_link_fixtures::run(AppConfig::new(), |_| {
        column((label("Hello"), button(label("OK"))))
    })
}
