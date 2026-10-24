//! The per-class recipes a built-in theme uses: its `ALL_RECIPES` by default, or the list the
//! application gives `.recipes(&[..])` (`Lvgl::recipes`, `Mono::recipes`, `Simple::recipes`),
//! so the recipes of widget classes it never creates are not linked.

use twine_engine::{ComputedRecipe, RecipeRef, WidgetClass};

/// The recipe of the nearest class of `class`'s theme-class
/// [lineage](WidgetClass::lineage) that `list` has a recipe for (the first one for that
/// class). A short scan when a node is created or re-themed, never while drawing; never
/// panics.
pub(crate) fn find(
    list: &'static [&'static ComputedRecipe],
    class: &'static WidgetClass,
) -> Option<RecipeRef> {
    class
        .lineage()
        .find_map(|c| list.iter().find_map(|r| r.for_class(c)))
}
