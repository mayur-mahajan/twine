//! Theme coverage (design `docs/plan/r7-theming-design.md` §2.9.4): which recipe styles each
//! theme class a UI creates — [`theme_coverage`], [`assert_theme_covers`] — and the input of
//! `cargo xtask theme-recipes`, which prints the minimal recipe list of a UI.
//!
//! A [`DesignSystem`](twine_engine::DesignSystem) lists the recipes it links; these checks
//! make that list evidence instead of a guess: a test builds the product's screens headless
//! and asserts that every widget class (variant, role) shown has a recipe in the theme.

use std::fmt;

use twine_engine::{Engine, ObjFlags, RecipeRef, WidgetClass};

/// The environment variable that makes [`assert_theme_covers`] print its report for
/// `cargo xtask theme-recipes` (any value).
pub const RECIPES_ENV: &str = "TWINE_THEME_RECIPES";

/// The prefix of the report lines [`assert_theme_covers`] prints when [`RECIPES_ENV`] is set:
/// `twine-theme-recipes: <class name> <recipe name or ->`, one line per theme class.
pub const RECIPES_LINE: &str = "twine-theme-recipes:";

/// One theme class a UI uses and the recipe its theme styles it with.
#[derive(Clone, Copy, Debug)]
pub struct CoveredClass {
    /// The theme class (a widget's class, a variant or a role).
    pub class: &'static WidgetClass,
    /// The recipe the display's theme uses for it (after its lineage fallback), or `None`
    /// when the theme has no recipe for the class or any of its bases.
    pub recipe: Option<RecipeRef>,
    /// Nodes themed as this class.
    pub nodes: usize,
}

/// The theme classes of every themed node of an engine, each once, in the order first met
/// (displays in order, parents first). See [`theme_coverage`].
#[derive(Clone, Debug, Default)]
pub struct ThemeCoverage {
    /// One entry per (display theme, theme class).
    pub classes: Vec<CoveredClass>,
}

impl ThemeCoverage {
    /// The classes without a recipe.
    pub fn missing(&self) -> impl Iterator<Item = &'static WidgetClass> + '_ {
        self.classes
            .iter()
            .filter(|c| c.recipe.is_none())
            .map(|c| c.class)
    }

    /// Whether every class has a recipe.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing().next().is_none()
    }

    /// The recipes in use, each once, in the order first met: the minimal recipe list of the
    /// UI (what `cargo xtask theme-recipes` prints).
    #[must_use]
    pub fn recipes(&self) -> Vec<RecipeRef> {
        let mut out: Vec<RecipeRef> = Vec::new();
        for r in self.classes.iter().filter_map(|c| c.recipe) {
            if !out.iter().any(|o| o.ptr_eq(r)) {
                out.push(r);
            }
        }
        out
    }
}

/// The report lines (see [`RECIPES_LINE`]).
impl fmt::Display for ThemeCoverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.classes {
            writeln!(
                f,
                "{RECIPES_LINE} {} {}",
                c.class.name,
                c.recipe.map_or("-", RecipeRef::name)
            )?;
        }
        Ok(())
    }
}

/// Which recipe styles each theme class of `engine`'s themed nodes (every display with a
/// theme: the nodes of its screens and layers, [`Engine::themed_nodes`]), as the display's
/// theme answers [`ThemeHook::recipe`](twine_engine::ThemeHook::recipe) (the built-in themes
/// answer with their per-class recipes). A theme that styles in code without recipes has all
/// its classes missing. Control-flow wrappers (`ObjFlags::LAYOUT_PASSTHROUGH`: `when`,
/// `for_each`, …) draw nothing and are left out.
///
/// ```
/// use twine_core::Color;
/// use twine_engine::{DesignSystem, OBJ_CLASS, Recipe, Theme, recipe};
/// use twine_style::design::ElementTable;
/// use twine_style::{Selector, ThemeMode};
/// use twine_testing::{EngineHarness, theme_coverage};
///
/// static LIGHT: ElementTable = ElementTable::new();
/// recipe! { static PANEL for OBJ_CLASS { Selector::MAIN => { bg: Color::WHITE } } }
/// struct Tiny;
/// impl DesignSystem for Tiny {
///     type Base = twine_engine::NoBase;
///     const NAME: &'static str = "tiny";
///     const MODES: &'static [(ThemeMode, &'static ElementTable)] = &[(ThemeMode::Light, &LIGHT)];
///     const RECIPES: &'static [&'static Recipe] = &[&PANEL];
/// }
///
/// static TINY: Theme<Tiny> = Theme::new();
/// let h = EngineHarness::new(64, 32).theme(&TINY);
/// let cov = theme_coverage(h.engine());
/// assert!(cov.is_complete()); // the screen is an `obj`
/// assert_eq!(cov.recipes()[0].name(), "PANEL");
/// ```
#[must_use]
pub fn theme_coverage(engine: &Engine) -> ThemeCoverage {
    let mut cov = ThemeCoverage::default();
    for d in engine.displays() {
        let Some(theme) = engine.theme(d) else { continue };
        let first = cov.classes.len();
        for n in engine.themed_nodes(d) {
            // A `LAYOUT_PASSTHROUGH` wrapper (`when`, `for_each`, …) draws nothing: no recipe
            // is needed for it.
            let wrapper = engine
                .tree()
                .node(n)
                .is_some_and(|node| node.flags().contains(ObjFlags::LAYOUT_PASSTHROUGH));
            let Some(class) = engine.theme_class(n).filter(|_| !wrapper) else {
                continue;
            };
            let seen = cov
                .classes
                .get_mut(first..)
                .and_then(|s| s.iter_mut().find(|c| c.class.is(class)));
            match seen {
                Some(c) => c.nodes += 1,
                None => cov.classes.push(CoveredClass {
                    class,
                    recipe: theme.recipe(class),
                    nodes: 1,
                }),
            }
        }
    }
    cov
}

/// Asserts that the theme of every display of `engine` has a recipe for every theme class
/// its nodes use ([`theme_coverage`]): build the product's screens headless (a
/// [`TestUi`](crate::TestUi), an [`EngineHarness`](crate::EngineHarness)) and call it, so a
/// design system's recipe list cannot miss a widget the product shows.
///
/// With the environment variable [`RECIPES_ENV`] set it first prints the report
/// ([`ThemeCoverage`]'s `Display`) to stdout, which `cargo xtask theme-recipes` collects into
/// the minimal recipe list.
///
/// # Panics
/// When a class has no recipe, listing those classes (a test assertion).
///
/// ```
/// use twine_engine::{DesignSystem, OBJ_CLASS, Recipe, Theme};
/// use twine_style::design::ElementTable;
/// use twine_style::ThemeMode;
/// use twine_testing::{EngineHarness, assert_theme_covers};
///
/// static LIGHT: ElementTable = ElementTable::new();
/// static OBJ: Recipe = Recipe::new("OBJ", &OBJ_CLASS, &[]);
/// struct Tiny;
/// impl DesignSystem for Tiny {
///     type Base = twine_engine::NoBase;
///     const NAME: &'static str = "tiny";
///     const MODES: &'static [(ThemeMode, &'static ElementTable)] = &[(ThemeMode::Light, &LIGHT)];
///     const RECIPES: &'static [&'static Recipe] = &[&OBJ];
/// }
/// static TINY: Theme<Tiny> = Theme::new();
/// let h = EngineHarness::new(64, 32).theme(&TINY);
/// assert_theme_covers(h.engine());
/// ```
#[track_caller]
pub fn assert_theme_covers(engine: &Engine) {
    let cov = theme_coverage(engine);
    if std::env::var_os(RECIPES_ENV).is_some() {
        print!("{cov}");
    }
    let missing: Vec<&str> = cov.missing().map(|c| c.name).collect();
    assert!(
        missing.is_empty(),
        "the theme has no recipe for {} (nor for a base class): {}",
        if missing.len() == 1 {
            "this class"
        } else {
            "these classes"
        },
        missing.join(", ")
    );
}
