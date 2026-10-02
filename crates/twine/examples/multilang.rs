//! `cargo xtask sim multilang`: people cards in eight languages (English, German, French,
//! Chinese, Japanese, Hebrew, Arabic, Persian). The dropdown or the ▶ button switches the
//! header language (`tr!`, no rebuild) and scrolls to that language's card; Hebrew, Arabic and
//! Persian lay out right to left and Arabic script is shaped. The heading is a runtime
//! TrueType font; the English avatar is read from a memory file system (`A:`).

use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(480, 320)
            .title("multilang")
            .scale(2)
            // The configuration the firmware ships (theme, engine, motion).
            .app_config(twine_demos::config()),
        twine_demos::multilang::app,
    );
}
