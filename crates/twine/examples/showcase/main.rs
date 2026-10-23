//! `cargo xtask sim showcase` (or `cargo run -p twine --example showcase`): a bedside patient
//! monitor on a 480 × 320 touch panel — live ECG, SpO2 pleth and respiration waveforms, vital
//! signs, prioritised alarms, trends and setup. **A demo with a simulated patient, not a
//! medical device.**
//!
//! | Screen | Shows |
//! |--------|-------|
//! | Monitor | three sweep waveforms with an erase gap, HR / SpO2 / RR / NIBP / Temp tiles, a heart-beat indicator |
//! | Alarms | the active alarms by priority, audio paused for 2 min with a countdown, the alarm history |
//! | Trends | HR and SpO2 over 1 h / 4 h / 12 h as plots or a table |
//! | Setup | alarm limits, the patient (on-screen keyboard), Day / Dark / Night, the NIBP interval |
//!
//! A seeded, deterministic patient simulator (`patient.rs`) desaturates a few seconds after
//! start — SpO2 falls to ~85 % (a high-priority alarm), the breathing rate climbs past its limit
//! (medium priority) — and recovers about half a minute later.
//!
//! **How Twine is used** (each file's docs go into detail):
//!
//! - *Signals at 60 Hz without allocation* (`store.rs`): one engine timer feeds the waveforms;
//!   each tick writes three integer signals, and only the bindings reading them run.
//! - *Partial redraw* (`waveform.rs`): a custom widget invalidates only the pixel columns its
//!   sweep head passed — press F2 in the simulator to see the redrawn areas.
//! - *Custom widgets drawn with the renderer* (`waveform.rs`, `trend_plot.rs`, the heart in
//!   `components.rs`): per-column fills, dashed lines and a polygon, integer math only.
//! - *Design elements and theme modes* (`theme.rs`): the whole palette is tokens, so Day /
//!   Dark / Night is one `set_mode`.
//! - *The store pattern* (`store.rs`): the signals behind one `Copy` handle, provided as
//!   context; buffers and state are scope-owned (`StoredValue`), nothing is leaked.
//! - *Keyed lists, memos, tweens, modals, the on-screen keyboard* (`alarms.rs`, `trends.rs`,
//!   `setup.rs`, `chrome.rs`).
//! - *Safety profile*: the HR and SpO2 numerics are `safety_critical` (display integrity), the
//!   alarm history is bounded, and nothing allocates on the 60 Hz path.
//!
//! Keys: F2 tints the redrawn areas, F12 cycles the theme mode, F1 lists the other simulator
//! hotkeys.

// Clinical abbreviations such as SpO2 read better without code formatting in the docs.
#![allow(clippy::doc_markdown)]

mod alarms;
mod chrome;
mod components;
mod monitor;
mod patient;
mod setup;
mod store;
mod theme;
mod trend_plot;
mod trends;
mod waveform;

use twine::prelude::*;
use twine_sim::SimConfig;

use store::Monitor;

/// The app: the status bar, the selected screen, the navigation bar.
fn app(cx: Scope) -> impl View {
    let m = Monitor::new(cx);
    cx.provide(m);
    let page = cx.signal(0usize);
    column((
        chrome::status_bar(m),
        // The four screens on top of each other, all but the selected one hidden: switching
        // is one flag per screen, and every screen keeps its state (and the waveforms keep
        // sweeping underneath, invalidating nothing while hidden).
        container((
            monitor::page(cx).hidden(move || page.get() != 0),
            alarms::page(cx).hidden(move || page.get() != 1),
            trends::page(cx).hidden(move || page.get() != 2),
            setup::page(cx).hidden(move || page.get() != 3),
        ))
        .style(&theme::BARE)
        .fill_width()
        .flex_grow(1)
        .flag(ObjFlags::SCROLLABLE, false),
        chrome::nav_bar(cx, m, page),
    ))
    .fill()
    .gap(0)
    .padding(0)
    .bg_color(design::BACKGROUND)
    .bg_opacity(Opa::COVER)
}

fn main() {
    twine_sim::run(
        SimConfig::new(480, 320)
            .title("Twine bedside monitor (demo)")
            .scale(2)
            .theme(theme::monitor_theme()),
        app,
    );
}
