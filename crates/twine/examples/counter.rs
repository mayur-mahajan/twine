//! `cargo xtask sim counter`: the counter of the API guide. Each click changes one label (F2
//! tints the refreshed area: only the label and the pressed button); between clicks the UI
//! does nothing (0 % CPU).

use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("Counter")
            .scale(2)
            // The configuration the firmware ships (theme, engine, motion).
            .app_config(twine_demos::config()),
        twine_demos::counter::app,
    );
}
