//! `TWINE_SIM_BUS_HZ=62500000 cargo xtask sim spinner`: one 60 × 60 spinner on a 320 × 240 RGB565
//! display with two 40-row draw buffers — the smooth-animation benchmark. Press F4 for the
//! performance overlay (fps, CPU, dirty pixels per frame) and F2 to see that only the arc's
//! changing segments are redrawn.
//!
//! `TWINE_SIM_BUS_HZ=<bits/s>` emulates the display bus speed (e.g. `62500000` for 62.5 MHz SPI).

use twine::assets::fonts::MONTSERRAT_12;
use twine::engine::EngineConfig;
use twine::view::prelude::*;
use twine_sim::SimConfig;

fn app(_cx: Scope) -> impl View {
    stack(spinner().size(60, 60))
        .size(Length::pct(100), Length::pct(100))
        .bg_opacity(Opa::TRANSP)
        .border_width(0)
}

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("Spinner")
            .scale(3)
            .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 40 }))
            .theme(DefaultTheme::light())
            // The performance overlay's font.
            .engine_config(EngineConfig {
                default_font: Some(&MONTSERRAT_12),
                ..EngineConfig::default()
            }),
        app,
    );
}
