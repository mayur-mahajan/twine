//! Recipes, design systems and theme classes (R7.S03): a `Theme<D>` styles nodes with the
//! recipes its `DesignSystem` lists (static style entries), along the theme class's lineage;
//! `Engine::set_theme_class` re-themes a node as a variant or role, refuses unrelated classes
//! and is bounded by `Limits::max_theme_classes`; mode switches use the listed tables only.

use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::fault::{CapacityFault, FaultKind};
use twine_core::{Color, Limits, Opa};
use twine_engine::{
    ComputedRecipe, DesignSystem, NoBase, NodeId, OBJ_CLASS, Obj, Recipe, RecipeRef, Theme, ThemeCx,
    ThemeHook, WidgetClass, recipe,
};
use twine_style::design::{self, ElementTable};
use twine_style::{EntryKind, Part, PropId, Selector, State, StyleRef, ThemeMode};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

static TILE: WidgetClass = WidgetClass::new("tile").base(&OBJ_CLASS);
static ALERT_TILE: WidgetClass = WidgetClass::new("tile.alert").base(&TILE);
static UNKNOWN_TILE: WidgetClass = WidgetClass::new("tile.unknown").base(&TILE);
static UNRELATED: WidgetClass = WidgetClass::new("unrelated");

static LIGHT: ElementTable = ElementTable::new()
    .color(design::SURFACE, Color::WHITE)
    .color(design::DANGER, Color::RED)
    .font(design::FONT_BODY, &twine_text::EMPTY_FONT);
static NIGHT: ElementTable = ElementTable::new()
    .color(design::SURFACE, Color::hex(0x0020_0000))
    .color(design::DANGER, Color::hex(0x0080_0000));

recipe! {
    static OBJ for OBJ_CLASS {
        Selector::MAIN => { bg: design::SURFACE },
    }

    static TILE_RECIPE for TILE {
        Part::Main => { bg: Color::BLUE, radius: 4 },
        State::PRESSED => { bg_opacity: Opa::pct(50) },
    }

    static ALERT for ALERT_TILE {
        Selector::MAIN => { bg: design::DANGER },
    }
}

/// A clinic's design system: two of the four modes, three recipes.
struct Clinic;
impl DesignSystem for Clinic {
    type Base = NoBase;
    const NAME: &'static str = "clinic";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] =
        &[(ThemeMode::Light, &LIGHT), (ThemeMode::Night, &NIGHT)];
    const RECIPES: &'static [&'static Recipe] = &[&OBJ, &TILE_RECIPE, &ALERT];
}

static CLINIC: Theme<Clinic> = Theme::new();

fn harness() -> EngineHarness {
    EngineHarness::new(64, 32).theme(&CLINIC)
}

fn obj(h: &mut EngineHarness) -> NodeId {
    let screen = h.screen();
    h.engine_mut().create(screen, Box::new(Obj)).unwrap()
}

fn bg(h: &EngineHarness, n: NodeId) -> Color {
    h.engine().style_color(n, Part::Main, PropId::BgColor)
}

/// The styles of `n`'s theme entries, in order.
fn theme_styles(h: &EngineHarness, n: NodeId) -> Vec<(Selector, StyleRef)> {
    let node = h.engine().tree().node(n).unwrap();
    node.styles()
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Theme)
        .map(|e| (e.selector, e.style.clone()))
        .collect()
}

#[test]
fn recipe_entries_are_static_styles() {
    let mut h = harness();
    let n = obj(&mut h);
    h.engine_mut().set_theme_class(n, &TILE);
    let styles = theme_styles(&h, n);
    assert_eq!(styles.len(), TILE_RECIPE.entries.len());
    for (want_sel, want) in TILE_RECIPE.entries {
        // The recipe's own `static` style: no copy, no reference count.
        assert!(styles.iter().any(|(sel, style)| sel == want_sel
            && matches!(style, StyleRef::Static(s) if core::ptr::eq(*s, *want))));
    }
    assert_eq!(bg(&h, n), Color::BLUE);
}

#[test]
fn design_system_theme_is_zero_sized_and_lists_its_modes() {
    let t = Theme::<Clinic>::new();
    assert_eq!(core::mem::size_of_val(&t), 0);
    assert_eq!(t.name(), "clinic");
    assert_eq!(t.modes(), &[ThemeMode::Light, ThemeMode::Night]); // the first is the start mode
    assert!(
        t.design(ThemeMode::Night)
            .is_some_and(|x| core::ptr::eq(x, &raw const NIGHT))
    );
    assert!(t.design(ThemeMode::Dark).is_none());
    assert!(
        t.metrics(160, twine_core::Size::new(64, 32)).is_none(),
        "NoBase has no metrics"
    );
    assert!(core::ptr::eq(t.font_normal(), &raw const twine_text::EMPTY_FONT));
}

#[test]
fn mode_switch_uses_the_listed_tables_and_allocates_nothing() {
    let mut h = harness();
    let d = h.display();
    let n = obj(&mut h);
    assert_eq!(bg(&h, n), Color::WHITE);
    let ((), allocs) = count_allocs(|| h.engine_mut().set_theme_mode(d, ThemeMode::Night));
    assert_eq!(
        allocs.allocs, 0,
        "a mode switch of a design system swaps a static table"
    );
    assert_eq!(bg(&h, n), Color::hex(0x0020_0000));
    h.engine_mut().set_theme_mode(d, ThemeMode::Dark); // not listed: refused
    assert_eq!(h.engine().theme_mode(d), ThemeMode::Night);
}

#[test]
fn lineage_fallback_for_unknown_variants() {
    let mut h = harness();
    let n = obj(&mut h);
    // `tile.unknown` has no recipe: styled with its base's (`tile`).
    h.engine_mut().set_theme_class(n, &UNKNOWN_TILE);
    assert_eq!(bg(&h, n), Color::BLUE);
    assert_eq!(
        Theme::<Clinic>::new().recipe(&UNKNOWN_TILE).map(RecipeRef::name),
        Some("TILE_RECIPE")
    );
    // A known variant gets its own recipe, and only it (the base recipe is not stacked).
    h.engine_mut().set_theme_class(n, &ALERT_TILE);
    assert_eq!(bg(&h, n), Color::RED);
    assert_eq!(theme_styles(&h, n).len(), ALERT.entries.len());
    // The widget's own class removes the override.
    h.engine_mut().set_theme_class(n, &OBJ_CLASS);
    assert!(h.engine().theme_class(n).unwrap().is(&OBJ_CLASS));
    assert_eq!(bg(&h, n), Color::WHITE);
}

#[test]
fn role_refused_for_an_unrelated_class() {
    let mut h = harness();
    let n = obj(&mut h);
    h.engine_mut().set_theme_class(n, &TILE);
    let logs = twine_testing::logs::capture(|| h.engine_mut().set_theme_class(n, &UNRELATED));
    assert!(
        logs.iter().any(|l| l.message.contains("is not a obj")),
        "{logs:?}"
    );
    assert!(h.engine().theme_class(n).unwrap().is(&TILE));
    assert_eq!(bg(&h, n), Color::BLUE);
    h.engine().tree().check_invariants().unwrap();
}

/// Counts the theme's `apply` calls.
struct Counting {
    applied: AtomicU32,
}
impl ThemeHook for Counting {
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        self.applied.fetch_add(1, Ordering::Relaxed);
        CLINIC.apply(cx, class);
    }
    fn font_normal(&self) -> &'static twine_text::Font {
        &twine_text::EMPTY_FONT
    }
}

#[test]
fn set_theme_class_is_idempotent() {
    static THEME: Counting = Counting {
        applied: AtomicU32::new(0),
    };
    let mut h = EngineHarness::new(64, 32).theme(&THEME);
    let n = obj(&mut h);
    h.run_until_idle();
    let applied = || THEME.applied.load(Ordering::Relaxed);
    let before = applied();
    h.engine_mut().set_theme_class(n, &TILE);
    assert_eq!(applied(), before + 1, "one re-theming");
    h.run_until_idle();
    h.engine_mut().set_theme_class(n, &TILE);
    h.engine_mut().set_theme_class(n, &UNRELATED); // refused: nothing happens either
    assert_eq!(applied(), before + 1);
    h.assert_idle(); // an unchanged theme class invalidates nothing
}

/// Theme classes are bounded by `Limits::max_theme_classes` (a `ThemeClasses` capacity fault),
/// freed with their node and preallocated.
///
/// req: SAF-008
#[test]
fn theme_classes_are_bounded_and_freed_with_their_node() {
    let mut h = harness();
    h.engine_mut()
        .set_limits(&Limits::DEFAULT.max_theme_classes(1).preallocated())
        .unwrap();
    let (a, b) = (obj(&mut h), obj(&mut h));
    h.engine_mut().set_theme_class(a, &TILE);
    h.engine_mut().take_faults();
    h.engine_mut().set_theme_class(b, &TILE); // the table is full
    assert!(h.engine().theme_class(b).unwrap().is(&OBJ_CLASS));
    let f = h.engine().last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(
        CapacityFault::from_code(f.code),
        Some(CapacityFault::ThemeClasses)
    );
    assert_eq!(f.node, Some(b));
    // Deleting `a` drops its entry: `b` fits now, and setting it allocates nothing
    // (reserved by `preallocate`).
    h.engine_mut().delete(a).unwrap();
    let ((), allocs) = count_allocs(|| {
        h.engine_mut().set_theme_class(b, &OBJ_CLASS); // no override: nothing stored
    });
    assert_eq!(allocs.allocs, 0);
    h.engine_mut().set_theme_class(b, &TILE);
    assert!(h.engine().theme_class(b).unwrap().is(&TILE));
    h.engine().tree().check_invariants().unwrap();
}

/// A recipe whose look depends on the node's parent.
fn nested(cx: &mut ThemeCx<'_>) {
    static NESTED: twine_style::Style = twine_style::style! { bg: Color::GREEN };
    if cx.parent_class().is_some_and(|c| c.is(&TILE)) {
        cx.add_style(Selector::MAIN, &NESTED);
    }
}
static NESTED: ComputedRecipe = ComputedRecipe::new("NESTED", &UNKNOWN_TILE, nested);

/// Extends `Clinic` (`type Base`), with a computed recipe and an override of `tile`.
struct Ward;
impl DesignSystem for Ward {
    type Base = Theme<Clinic>;
    const NAME: &'static str = "ward";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = Clinic::MODES;
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        NESTED
            .for_class(class)
            .or_else(|| class.is(&TILE).then_some(RecipeRef::Static(&ALERT)))
    }
}
static WARD: Theme<Ward> = Theme::new();

#[test]
fn base_fallback_overrides_and_computed_recipes() {
    let mut h = EngineHarness::new(64, 32).theme(&WARD);
    let parent = obj(&mut h);
    h.engine_mut().set_theme_class(parent, &TILE);
    assert_eq!(
        bg(&h, parent),
        Color::RED,
        "Ward overrides tile with the alert recipe"
    );
    let child = h.engine_mut().create(parent, Box::new(Obj)).unwrap();
    h.engine_mut().set_theme_class(child, &UNKNOWN_TILE);
    assert_eq!(bg(&h, child), Color::GREEN, "the computed recipe saw its parent");
    let n = obj(&mut h);
    assert_eq!(bg(&h, n), Color::WHITE, "obj: Clinic's, the base's recipe");
    assert_eq!(WARD.recipe(&OBJ_CLASS).map(RecipeRef::name), Some("OBJ"));
    // `tile.alert`: Clinic knows it exactly, Ward knows its base `tile` — the nearer wins.
    assert_eq!(WARD.recipe(&ALERT_TILE).map(RecipeRef::name), Some("ALERT"));
}

#[test]
fn reapply_theme_replaces_the_theme_styles() {
    let mut h = harness();
    let n = obj(&mut h);
    h.engine_mut().set_theme_class(n, &TILE);
    h.engine_mut().reapply_theme(n);
    h.engine_mut().reapply_theme(n);
    assert_eq!(theme_styles(&h, n).len(), TILE_RECIPE.entries.len());
    // The screen's layers get no theme styles.
    let d = h.display();
    let top = h.engine().top_layer(d).unwrap();
    h.engine_mut().reapply_theme(top);
    assert!(theme_styles(&h, top).is_empty());
}
