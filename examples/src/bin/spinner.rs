//! `cargo xtask sim spinner -- --bus-hz 62500000`: one 60 × 60 spinner on a 320 × 240 RGB565
//! display with two 40-row draw buffers — the smooth-animation benchmark. Press F4 for the
//! performance overlay (fps, CPU, dirty pixels per frame) and F2 to see that only the arc's
//! changing segments are redrawn.
//!
//! Options:
//! - `--bus-hz <bits/s>`: emulate the display bus speed (e.g. `62500000` for 62.5 MHz SPI).

use twine_assets::fonts::MONTSERRAT_12;
use twine_engine::EngineConfig;
use twine_hal::BufferSpec;
use twine_sim::SimConfig;
use twine_view::prelude::*;

fn app(_cx: Scope) -> impl View {
    stack(spinner().size(60, 60))
        .size(Length::pct(100), Length::pct(100))
        .bg_opa(Opa::TRANSP)
        .border_width(0)
}

fn main() {
    let mut bus_hz = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bus-hz" => {
                let Some(Ok(hz)) = args.next().map(|v| v.parse::<u32>()) else {
                    eprintln!("--bus-hz needs a number of bits per second");
                    std::process::exit(2);
                };
                bus_hz = Some(hz);
            }
            other => {
                eprintln!("unknown option {other} (options: --bus-hz <bits/s>)");
                std::process::exit(2);
            }
        }
    }
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("Spinner")
            .scale(3)
            .buffers(BufferSpec::PartialDouble { rows: 40 })
            // The performance overlay's font.
            .engine_config(EngineConfig {
                default_font: Some(&MONTSERRAT_12),
                ..EngineConfig::default()
            })
            .bus_hz(bus_hz),
        app,
    );
}
