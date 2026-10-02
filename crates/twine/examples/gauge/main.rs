//! `cargo xtask sim gauge` (or `cargo run -p twine --example gauge`): the gauge widget of
//! the custom widget guide (`twine::guide::custom_widgets`), written outside Twine with its
//! own class, parts, drawing, theme registration and typed view.
//!
//! Two gauges: a setpoint you adjust with the keys (`Left`/`Right`, or an encoder in edit
//! mode) and a load that drifts on its own and turns red above 80 % (an application state,
//! `State::custom`). Keys: `Tab` moves the focus, F12 cycles the theme mode (light, dark,
//! night, high contrast): the gauges follow through their design elements.

// The gauge is shared with `tests/gauge.rs` and the guide's doctests; this example does not
// use every item of it.
#[allow(dead_code)]
mod gauge;

use gauge::{GAUGE_CLASS, INDICATOR, NEEDLE, gauge, style_gauge};
use twine::prelude::*;
use twine_sim::SimConfig;

/// The application's alarm state (shown as `ALARM` in tree dumps).
const ALARM: State = State::custom::<0>();

fn app(cx: Scope) -> impl View {
    State::set_custom_name(ALARM, "ALARM");
    let setpoint = cx.signal(21);
    let load = cx.signal(35);
    // A stand-in for a sensor: the load climbs and falls back.
    cx.interval(Duration::ms(2000), move || {
        load.update(|l| *l = if *l >= 90 { 30 } else { *l + 20 });
    });
    row((
        column((
            gauge(setpoint)
                .range(10..=30)
                .on_change(move |v| setpoint.set(v))
                .test_id("setpoint"),
            label(text!("Setpoint {} C", setpoint.get())),
        ))
        .gap(8)
        .padding(4) // room for the focus ring (children are clipped to their parent)
        .align_items(CrossAlign::Center),
        column((
            gauge(load)
                .state(ALARM, move || load.get() > 80)
                .on_state(ALARM, |s| s.part(INDICATOR, |p| p.arc_color(design::DANGER)))
                .part(NEEDLE, |s| s.line_color(design::PRIMARY))
                .test_id("load"),
            label(text!("Load {} %", load.get())),
        ))
        .gap(8)
        .padding(4) // room for the focus ring (children are clipped to their parent)
        .align_items(CrossAlign::Center),
    ))
    .gap(12)
    .padding(12)
    .size(Length::pct(100), Length::pct(100))
    .justify(MainAlign::Center)
}

fn main() {
    let theme = DefaultTheme::builder().class(&GAUGE_CLASS, style_gauge).build();
    twine_sim::run(SimConfig::new(320, 240).title("Gauge").scale(2).theme(theme), app);
}
