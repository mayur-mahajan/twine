//! `cargo xtask sim controls`: every basic control on one screen. The slider, the bar and the
//! arc share one value; the switch, the checkbox, the LED and the power button share one
//! on/off state. Drag the slider (F2 shows that only the bound widgets redraw), or use Tab,
//! the arrow keys, Enter and the mouse wheel (encoder) to operate every control. F4 shows the
//! performance overlay.

use twine::assets::fonts::MONTSERRAT_12;
use twine::prelude::AppConfig;
use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("Controls")
            .scale(3)
            .app_config(perf_overlay_font(twine_demos::config())),
        twine_demos::controls::app,
    );
}

/// The shipped configuration plus the performance overlay's font (F4; simulator only).
fn perf_overlay_font(mut cfg: AppConfig) -> AppConfig {
    cfg.engine.default_font = Some(&MONTSERRAT_12);
    cfg
}
