//! `cargo xtask sim core_widgets`: the core widgets with LVGL's default theme — labels in every
//! long mode (wrap, dots, scroll, circular scroll, clip, a selection), buttons (normal,
//! checkable, disabled) and images (a static RGB565A8 logo, the same logo decoded from QOI, a
//! recolored copy, a symbol and a logo rotating with anti-aliasing).
//!
//! Keys: F12 toggles the light and dark theme. F2 tints the refreshed areas (only the scrolling
//! labels and the rotating logo are redrawn), F4 shows the frame rate, F6 pauses the time
//! (then nothing is refreshed and the CPU load is 0 %).
//!
//! `RUST_LOG=twine::example=info` logs button clicks.

use std::rc::Rc;

use twine_examples::core_widgets::{H, W, build, engine_config};
use twine_sim::{SimConfig, run_engine};
use twine_theme::DefaultTheme;

fn main() {
    let cfg = SimConfig::new(W, H)
        .title("core widgets")
        .scale(2)
        .engine_config(engine_config())
        .theme_toggle(Rc::new(DefaultTheme::light()), Rc::new(DefaultTheme::dark()));
    run_engine(cfg, |engine| {
        build(engine);
    });
}
