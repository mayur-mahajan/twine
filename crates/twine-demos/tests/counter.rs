//! The `counter` demo with its theme trimmed to the recipes it uses (`counter::THEME`, what the
//! firmware links): every class it creates has a recipe, and it draws exactly like the full
//! LVGL theme.

use twine_demos::counter::{self, app};
use twine_testing::{TestUi, assert_theme_covers, by_id};

#[test]
fn counter_theme_covers_the_demo_and_draws_like_the_full_theme() {
    let mut trimmed = TestUi::new(320, 240)
        .app_config(twine_demos::config_with_theme(&counter::THEME))
        .mount(app);
    trimmed.run_until_idle();
    assert_theme_covers(&trimmed.engine());
    let mut full = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    full.run_until_idle();
    assert_eq!(
        trimmed.harness_mut().panel_rgb888(),
        full.harness_mut().panel_rgb888()
    );
    assert_eq!(trimmed.find(by_id("count")).text(), "Clicked 0 times");
}
