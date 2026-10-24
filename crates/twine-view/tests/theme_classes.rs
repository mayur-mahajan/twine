//! Variants and roles in views (R7.S03): `.variant(..)` and `.role(..)` set the node's theme
//! class; a reactive variant re-themes the node once per change; a theme that does not know a
//! variant (the LVGL theme) shows its widget's look; `assert_theme_covers` checks a design
//! system's recipe list against a UI.

use std::sync::atomic::{AtomicU32, Ordering};

use twine_engine::{OBJ_CLASS, ThemeCx, ThemeHook};
use twine_style::design::ElementTable;
use twine_testing::{TestUi, by_id, theme_coverage};
use twine_view::prelude::*;
use twine_widgets::button::{BUTTON_CLASS, BUTTON_DESTRUCTIVE_CLASS, BUTTON_GRAY_CLASS};
use twine_widgets::label::LABEL_CLASS;

/// Counts `apply` calls per class name.
struct Counting {
    buttons: AtomicU32,
}
impl ThemeHook for Counting {
    fn apply(&self, _: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if class.is_a(&BUTTON_CLASS) {
            self.buttons.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn font_normal(&self) -> &'static Font {
        &twine_text::EMPTY_FONT
    }
}

#[test]
fn a_reactive_variant_reapplies_the_theme_once_per_change() {
    static THEME: Counting = Counting {
        buttons: AtomicU32::new(0),
    };
    let theme = &THEME;
    let mut t = TestUi::new(120, 60).theme(theme).mount(|cx| {
        let danger = cx.signal(false);
        cx.provide(danger);
        button(label("Erase"))
            .variant(move || {
                if danger.get() {
                    ButtonVariant::Destructive
                } else {
                    ButtonVariant::Gray
                }
            })
            .test_id("b")
    });
    t.run_until_idle();
    let b = t.find(by_id("b")).id();
    assert!(t.engine().theme_class(b).unwrap().is(&BUTTON_GRAY_CLASS));
    let after_build = theme.buttons.load(Ordering::Relaxed);
    assert_eq!(
        after_build, 2,
        "created as a button, re-themed once as the gray variant"
    );

    let danger = t.root_scope().use_context::<Signal<bool>>().unwrap();
    danger.set(true);
    t.run_until_idle();
    assert!(t.engine().theme_class(b).unwrap().is(&BUTTON_DESTRUCTIVE_CLASS));
    assert_eq!(
        theme.buttons.load(Ordering::Relaxed),
        after_build + 1,
        "one re-theming per change"
    );

    danger.set(true); // unchanged: no binding run, nothing re-themed
    t.run_until_idle();
    assert_eq!(theme.buttons.load(Ordering::Relaxed), after_build + 1);
}

#[test]
fn lvgl_theme_shows_unknown_variants_as_buttons() {
    let mut t = TestUi::new(200, 80).theme(&lvgl::THEME).mount(|_| {
        row((
            button(label("A")).test_id("filled"),
            button(label("B"))
                .variant(ButtonVariant::Tinted)
                .test_id("tinted"),
        ))
    });
    t.run_until_idle();
    let (a, b) = (t.find(by_id("filled")).id(), t.find(by_id("tinted")).id());
    let e = t.engine();
    for prop in [
        PropId::BgColor,
        PropId::Radius,
        PropId::ShadowWidth,
        PropId::PaddingTop,
    ] {
        assert_eq!(
            e.style_prop(a, Part::Main, prop),
            e.style_prop(b, Part::Main, prop),
            "{prop:?}"
        );
    }
}

/// A role for a container.
static METRIC_TILE: WidgetClass = WidgetClass::new("metric_tile").base(&OBJ_CLASS);

#[test]
fn a_role_themes_a_container_and_an_unrelated_role_is_refused() {
    let mut t = TestUi::new(120, 60).mount(|_| {
        column((
            column(()).role(&METRIC_TILE).test_id("tile"),
            label("x").role(&METRIC_TILE).test_id("label"), // a label is not an `obj`
        ))
    });
    t.run_until_idle();
    let (tile, lbl) = (t.find(by_id("tile")).id(), t.find(by_id("label")).id());
    assert!(t.engine().theme_class(tile).unwrap().is(&METRIC_TILE));
    assert!(t.engine().theme_class(lbl).unwrap().is(&LABEL_CLASS));
}

static LIGHT: ElementTable = ElementTable::new();
recipe! {
    static SCREEN for OBJ_CLASS { Selector::MAIN => { bg: design::SURFACE } }
    static BUTTON for BUTTON_CLASS { Selector::MAIN => { bg: design::PRIMARY } }
    static LABEL for LABEL_CLASS { Selector::MAIN => { text_color: design::ON_SURFACE } }
}

/// Recipes for the screen and buttons only.
struct NoLabels;
impl DesignSystem for NoLabels {
    type Base = twine_engine::NoBase;
    const NAME: &'static str = "no-labels";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = &[(ThemeMode::Light, &LIGHT)];
    const RECIPES: &'static [&'static Recipe] = &[&SCREEN, &BUTTON];
}

/// The same with labels.
struct Complete;
impl DesignSystem for Complete {
    type Base = twine_engine::NoBase;
    const NAME: &'static str = "complete";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = NoLabels::MODES;
    const RECIPES: &'static [&'static Recipe] = &[&SCREEN, &BUTTON, &LABEL];
}

static NO_LABELS: twine_engine::Theme<NoLabels> = twine_engine::Theme::new();
static COMPLETE: twine_engine::Theme<Complete> = twine_engine::Theme::new();

fn ui(t: TestUi) -> TestUi {
    t.mount(|_| {
        column((
            button(label("OK")),
            button(label("Cancel")).variant(ButtonVariant::Plain),
        ))
    })
}

#[test]
fn coverage_finds_classes_without_a_recipe() {
    let t = ui(TestUi::new(120, 60).theme(&NO_LABELS));
    let cov = theme_coverage(&t.engine());
    let missing: Vec<&str> = cov.missing().map(|c| c.name).collect();
    assert_eq!(missing, ["label"]);
    let recipes: Vec<&str> = cov.recipes().iter().map(|r| r.name()).collect();
    assert_eq!(
        recipes,
        ["SCREEN", "BUTTON"],
        "the plain variant falls back to BUTTON"
    );
    let e = t.engine();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        twine_testing::assert_theme_covers(&e);
    }));
    assert!(failed.is_err());

    let t = ui(TestUi::new(120, 60).theme(&COMPLETE));
    twine_testing::assert_theme_covers(&t.engine());
    assert!(
        theme_coverage(&t.engine())
            .to_string()
            .contains("twine-theme-recipes: button.plain BUTTON\n")
    );
}
