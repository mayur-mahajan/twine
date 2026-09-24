//! `cargo xtask sim controls`: every basic control on one screen. The slider, the bar and the
//! arc share one value; the switch, the checkbox, the LED and the power button share one
//! on/off state. Drag the slider (F2 shows that only the bound widgets redraw), or use Tab,
//! the arrow keys, Enter and the mouse wheel (encoder) to operate every control. F4 shows the
//! performance overlay.

use twine_assets::fonts::MONTSERRAT_12;
use twine_engine::EngineConfig;
use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("Controls")
            .scale(3)
            // The performance overlay's font.
            .engine_config(EngineConfig {
                default_font: Some(&MONTSERRAT_12),
                ..EngineConfig::default()
            }),
        twine_demos::controls::app,
    );
}
