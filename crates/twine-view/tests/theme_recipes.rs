//! Per-class pay-for-use of the built-in themes (R7.S04b): a theme trimmed with
//! `.recipes(&[..])` to the classes a UI creates covers that UI (`assert_theme_covers`) and
//! draws it pixel for pixel like the full theme; a class whose recipe is not listed gets no
//! theme styles. The UI is the one of the link fixtures (`tools/link-fixtures`: `lvgl_light`,
//! `mono_dark`, `simple_light`), whose images must not link the other recipes.

use twine_engine::{ComputedRecipe, ThemeHook};
use twine_testing::{TestUi, assert_theme_covers, theme_coverage};
use twine_theme::{Lvgl, Mono, Simple, lvgl, mono, simple};
use twine_view::prelude::*;

/// The link fixtures' UI: a column (a plain object) with a label and a button.
fn ui(theme: &'static dyn ThemeHook) -> TestUi {
    let mut t = TestUi::new(160, 80)
        .theme(theme)
        .mount(|_| column((label("Hello"), button(label("OK")))));
    t.run_until_idle();
    t
}

/// The fixtures' recipe lists (what `cargo xtask theme-recipes` prints for the UI).
static LVGL_USED: &[&ComputedRecipe] = &[&lvgl::OBJ, &lvgl::LABEL, &lvgl::BUTTON];
static MONO_USED: &[&ComputedRecipe] = &[&mono::OBJ, &mono::LABEL, &mono::BUTTON];
static SIMPLE_USED: &[&ComputedRecipe] = &[&simple::OBJ, &simple::LABEL, &simple::BUTTON];

static LVGL_TRIMMED: Lvgl = Lvgl::new().recipes(LVGL_USED);
static MONO_TRIMMED: Mono = Mono::new().recipes(MONO_USED);
static SIMPLE_TRIMMED: Simple = Simple::new().recipes(SIMPLE_USED);
static MONO_FULL: Mono = Mono::new();
static SIMPLE_FULL: Simple = Simple::new();

fn assert_same_pixels(full: &'static dyn ThemeHook, trimmed: &'static dyn ThemeHook) {
    let mut a = ui(full);
    let mut b = ui(trimmed);
    assert_theme_covers(&b.engine());
    assert_eq!(
        a.harness_mut().panel_rgb888(),
        b.harness_mut().panel_rgb888(),
        "{} trimmed to the UI's recipes draws differently",
        full.name()
    );
}

#[test]
fn a_theme_trimmed_to_the_used_recipes_covers_and_draws_the_ui_unchanged() {
    assert_same_pixels(&lvgl::THEME, &LVGL_TRIMMED);
    assert_same_pixels(&MONO_FULL, &MONO_TRIMMED);
    assert_same_pixels(&SIMPLE_FULL, &SIMPLE_TRIMMED);
}

#[test]
fn the_minimal_list_is_what_coverage_reports() {
    let t = ui(&lvgl::THEME);
    let used: Vec<&str> = theme_coverage(&t.engine())
        .recipes()
        .iter()
        .map(|r| r.name())
        .collect();
    let mut expected: Vec<&str> = LVGL_USED.iter().map(|r| r.name).collect();
    let mut sorted = used.clone();
    sorted.sort_unstable();
    expected.sort_unstable();
    assert_eq!(sorted, expected);
}

#[test]
fn an_unlisted_class_has_no_recipe_and_coverage_says_so() {
    static LABELS_ONLY: Lvgl = Lvgl::new().recipes(&[&lvgl::OBJ, &lvgl::LABEL]);
    let t = ui(&LABELS_ONLY);
    let cov = theme_coverage(&t.engine());
    let missing: Vec<&str> = cov.missing().map(|c| c.name).collect();
    assert_eq!(missing, ["button"]);
    let e = t.engine();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| assert_theme_covers(&e)));
    assert!(r.is_err(), "a UI with an unlisted class fails the coverage check");
}

#[test]
fn variants_use_the_listed_base_recipe() {
    let mut t = TestUi::new(160, 80)
        .theme(&LVGL_TRIMMED)
        .mount(|_| column((button(label("OK")).variant(ButtonVariant::Tinted),)));
    t.run_until_idle();
    assert_theme_covers(&t.engine());
}
