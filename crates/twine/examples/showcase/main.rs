//! `cargo xtask sim showcase` (or `cargo run -p twine --example showcase`): a small tabbed
//! app that tours Twine on a 480 × 320 panel — the kind of UI a thermostat, a printer or a
//! handheld instrument ships.
//!
//! | Tab | Shows |
//! |-----|-------|
//! | Dashboard | live data from a timer: an arc gauge, a sparkline chart, a tweened value, a memo |
//! | Controls | value widgets sharing one signal (two-way bindings), a dropdown and a roller |
//! | Tasks | a keyed list (`for_each`): add, check and delete rows; a derived count |
//! | Settings | light/dark theme mode at run time, slide animations on/off |
//!
//! Keys: `Tab` moves the focus, F12 cycles the theme mode, F2 tints the redrawn areas (only
//! what changed is drawn), F1 lists the other simulator hotkeys.

mod controls;
mod dashboard;
mod settings;
mod tasks;

use twine::prelude::*;
use twine_sim::SimConfig;

/// The whole app: one tab view, the bar at the bottom like a handheld's.
fn app(cx: Scope) -> impl View {
    let page = cx.signal(0usize);
    // Shared with the settings tab, which switches the slide animation off.
    let animated = cx.signal(true);

    tabview(
        page,
        (
            tab("Dashboard", dashboard::page(cx)),
            tab("Controls", controls::page(cx)),
            tab("Tasks", tasks::page(cx)),
            tab("Settings", settings::page(cx, animated)),
        ),
    )
    .bar_position(Side::Bottom)
    .bar_size(44)
    .animated(animated)
    .fill()
}

fn main() {
    twine_sim::run(
        SimConfig::new(480, 320)
            .title("Twine showcase")
            .scale(2)
            .theme(DefaultTheme::light()),
        app,
    );
}
