//! Design systems: a theme as `const` data — mode tables of [design
//! elements](twine_style::design) and [recipes](crate::Recipe) — described by a
//! [`DesignSystem`] impl, optionally extending a base theme ([`ThemeBase`]), and installed as
//! the zero-sized [`Theme<D>`]; and [`Variant`], the typed front end of theme classes
//! (`ButtonVariant`).

use core::marker::PhantomData;

use twine_core::Size;
use twine_style::ThemeMode;
use twine_style::design::{self, ElementTable};
use twine_text::Font;

use crate::{RecipeRef, ThemeCx, ThemeHook, Widget, WidgetClass};

/// A theme described as `const` data (design `docs/plan/r7-theming-design.md` §2.5): the
/// [design element](twine_style::design) table of each mode it ships and the
/// [recipes](crate::Recipe) of the widget classes it styles. Implemented on a unit struct and
/// installed as [`Theme<Self>`](Theme), which costs no heap and runs no start-up code.
///
/// **Pay for what you use.** An image contains exactly what the impl names: the mode tables
/// of [`MODES`](Self::MODES) and the recipes [`recipe`](Self::recipe) can return (by default
/// those of [`RECIPES`](Self::RECIPES)). There is no registry of every mode or recipe of a
/// design language — a shipped design language provides its tables and recipes as separate
/// `static`s (and an `ALL_RECIPES` list as a convenience), and the application lists the ones
/// it uses. `twine_testing::assert_theme_covers` checks that every widget class a UI creates
/// has a recipe, and `cargo xtask theme-recipes` prints the recipes a UI uses.
///
/// **Customization is const:** tune a base's element sets in `const` tables (`ElementTable`
/// builders over the base's sets), override a recipe by listing your own (the first recipe
/// for a class wins), and **extend a base theme** with [`Base`](Self::Base): a class the
/// design system has no recipe for is styled by the base (`type Base = Lvgl;` — the LVGL
/// look for every widget the design system does not restyle), and the base's per-display
/// [metrics](ThemeHook::metrics) are used. `type Base = NoBase;` for a design system of its
/// own. Nothing is built at run time.
///
/// **Lineage fallback:** [`Theme<D>`] walks the node's theme class
/// [`lineage`](WidgetClass::lineage), most specific first, and styles the node with the first
/// class the design system has a recipe for or the base knows, so a variant or role neither
/// knows (`button.tinted`) looks like its base class (`button`).
///
/// ```
/// use twine_core::Color;
/// use twine_engine::{DesignSystem, OBJ_CLASS, Recipe, Theme, ThemeHook, recipe};
/// use twine_style::design::{self, ElementTable};
/// use twine_style::{Selector, ThemeMode};
///
/// static LIGHT: ElementTable = ElementTable::new()
///     .color(design::SURFACE, Color::WHITE)
///     .font(design::FONT_BODY, &twine_text::EMPTY_FONT);
/// static NIGHT: ElementTable = ElementTable::new().color(design::SURFACE, Color::hex(0x200000));
///
/// recipe! {
///     static PANEL for OBJ_CLASS { Selector::MAIN => { bg: design::SURFACE } }
/// }
///
/// /// A clinic's design system: two modes, one recipe.
/// struct Clinic;
/// impl DesignSystem for Clinic {
///     type Base = twine_engine::NoBase;
///     const NAME: &'static str = "clinic";
///     const MODES: &'static [(ThemeMode, &'static ElementTable)] =
///         &[(ThemeMode::Light, &LIGHT), (ThemeMode::Night, &NIGHT)];
///     const RECIPES: &'static [&'static Recipe] = &[&PANEL];
/// }
///
/// static CLINIC: Theme<Clinic> = Theme::new();
/// assert_eq!(CLINIC.name(), "clinic");
/// assert_eq!(CLINIC.modes(), &[ThemeMode::Light, ThemeMode::Night]); // the first is the start mode
/// assert_eq!(CLINIC.recipe(&OBJ_CLASS).map(|r| r.name()), Some("PANEL"));
/// assert_eq!(core::mem::size_of::<Theme<Clinic>>(), 0);
/// ```
///
/// Extending a base, with one recipe overridden and the base's other recipes kept:
///
/// ```
/// # use twine_core::Color;
/// # use twine_engine::{DesignSystem, NoBase, OBJ_CLASS, Recipe, Theme, ThemeHook, WidgetClass, recipe};
/// # use twine_style::design::{self, ElementTable};
/// # use twine_style::{Selector, ThemeMode};
/// # static LIGHT: ElementTable = ElementTable::new();
/// static CARD: WidgetClass = WidgetClass::new("card").base(&OBJ_CLASS);
/// recipe! {
///     static PANEL for OBJ_CLASS { Selector::MAIN => { bg: design::SURFACE } }
///     static CARD_LOOK for CARD { Selector::MAIN => { radius: 8 } }
/// }
/// struct Clinic;
/// impl DesignSystem for Clinic {
///     type Base = NoBase;
///     const NAME: &'static str = "clinic";
///     const MODES: &'static [(ThemeMode, &'static ElementTable)] = &[(ThemeMode::Light, &LIGHT)];
///     const RECIPES: &'static [&'static Recipe] = &[&PANEL, &CARD_LOOK];
/// }
/// recipe! {
///     static RED_PANEL for OBJ_CLASS { Selector::MAIN => { bg: Color::RED } }
/// }
/// /// The clinic's look with red panels.
/// struct Ward;
/// impl DesignSystem for Ward {
///     type Base = Theme<Clinic>;
///     const NAME: &'static str = "ward";
///     const MODES: &'static [(ThemeMode, &'static ElementTable)] = Clinic::MODES;
///     const RECIPES: &'static [&'static Recipe] = &[&RED_PANEL];
/// }
/// static WARD: Theme<Ward> = Theme::new();
/// assert_eq!(WARD.recipe(&OBJ_CLASS).map(|r| r.name()), Some("RED_PANEL")); // overridden
/// assert_eq!(WARD.recipe(&CARD).map(|r| r.name()), Some("CARD_LOOK")); // the base's
/// ```
pub trait DesignSystem: Sync + 'static {
    /// The theme this design system extends: a class the design system has no recipe for is
    /// styled by it, and its per-display [metrics](ThemeHook::metrics) are the display's (a
    /// built-in theme, e.g. `twine_theme::Lvgl`, or another design system's [`Theme<D>`]).
    /// [`NoBase`] for a design system of its own.
    type Base: ThemeBase;

    /// A short name for logs and audit reports ([`ThemeHook::name`]).
    const NAME: &'static str;

    /// The modes the theme supports, each with its design element table (`const`, in flash),
    /// in the order a mode switch cycles through them; the first is the mode the theme starts
    /// in. List only the modes the product shows: an unlisted table is not linked, and a
    /// switch to an unlisted mode is refused (the display keeps its mode). Each mode at most
    /// once, at most [`ThemeMode::ALL`]`.len()` entries (checked at compile time). A table
    /// should define every standard element
    /// ([`ElementTable::missing_standard`]); the display's default font is the first table's
    /// `design::FONT_BODY`.
    const MODES: &'static [(ThemeMode, &'static ElementTable)];

    /// The recipes the default [`recipe`](Self::recipe) chooses from (default: none). The
    /// first recipe for a class wins.
    const RECIPES: &'static [&'static crate::Recipe] = &[];

    /// The recipe for exactly `class` (no lineage walk: [`Theme<D>`] walks the lineage and
    /// asks for each class), or `None` (then the [`Base`](Self::Base) is asked). The default
    /// searches [`RECIPES`](Self::RECIPES) ([`Recipe::find`](crate::Recipe::find)); override
    /// it to compute the choice. Called when a node is created or re-themed, never while
    /// drawing; must not panic.
    #[must_use]
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        crate::Recipe::find(Self::RECIPES, class)
    }
}

/// The modes of `tables` in order, checked: each mode at most once (and so at most four).
/// Evaluated at compile time for every design system in use (`Theme::modes`), so a violation
/// is a compile error, never a run-time panic.
const fn mode_list(tables: &[(ThemeMode, &'static ElementTable)]) -> ([ThemeMode; 4], usize) {
    let mut out = [ThemeMode::Light; 4];
    let mut slots: &mut [ThemeMode] = &mut out;
    let mut rest = tables;
    while let [(mode, _), tail @ ..] = rest {
        let mut later = tail;
        while let [(m, _), more @ ..] = later {
            // PANIC: compile time only (evaluated in a `const` block): a design system that
            // lists a mode twice does not compile.
            assert!(*m as u8 != *mode as u8, "DesignSystem::MODES lists a mode twice");
            later = more;
        }
        // Distinct modes never outnumber the slots (one per `ThemeMode`).
        if let [slot, more @ ..] = slots {
            *slot = *mode;
            slots = more;
        }
        rest = tail;
    }
    (out, tables.len())
}

/// A theme a [`DesignSystem`] can extend ([`DesignSystem::Base`]): a theme type with one
/// `'static` instance. Implemented by the built-in themes of `twine-theme` (their default
/// configuration), by every [`Theme<D>`] (so design systems extend each other) and by
/// [`NoBase`].
///
/// ```
/// use twine_engine::{NoBase, ThemeBase, ThemeCx, ThemeHook, WidgetClass};
///
/// /// A theme in code that design systems can extend.
/// struct House;
/// impl ThemeHook for House {
///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
///     fn name(&self) -> &'static str { "house" }
/// }
/// impl ThemeBase for House {
///     const THEME: &'static Self = &House;
/// }
/// assert_eq!(House::THEME.name(), "house");
/// assert_eq!(NoBase::THEME.name(), "none");
/// ```
pub trait ThemeBase: ThemeHook + Sync + 'static {
    /// The instance a design system extending this theme falls back to.
    const THEME: &'static Self;
}

/// The base of a [`DesignSystem`] that extends no theme (`type Base = NoBase;`): styles
/// nothing, has no design elements, no metrics and the empty font.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBase;

impl ThemeHook for NoBase {
    fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}

    fn font_normal(&self) -> &'static Font {
        &twine_text::EMPTY_FONT
    }

    fn name(&self) -> &'static str {
        "none"
    }

    fn modes(&self) -> &'static [ThemeMode] {
        &[]
    }
}

impl ThemeBase for NoBase {
    const THEME: &'static Self = &NoBase;
}

/// A [`DesignSystem`] installed as a theme: zero-sized, no heap, no start-up code, `Sync`, so
/// it is a `static` (`static THEME: Theme<Clinic> = Theme::new();`) given to a display when
/// it is created (`AppConfig::theme(&THEME)`, [`DisplayTheme`](crate::DisplayTheme)).
/// Implements [`ThemeHook`]:
///
/// - [`apply`](ThemeHook::apply): walks the node's theme-class lineage (most specific first)
///   to the first class `D` has a recipe for ([`DesignSystem::recipe`], added with
///   [`ThemeCx::add_recipe`]) or the [base](DesignSystem::Base) has one for (the base then
///   styles the node); a lineage neither knows is left to the base.
/// - [`design`](ThemeHook::design): the listed table of the mode (a pointer swap on a mode
///   switch; nothing is built or allocated); `None` for an unlisted mode.
/// - [`metrics`](ThemeHook::metrics): the base's.
/// - [`modes`](ThemeHook::modes): the listed modes; the first is the default start mode.
/// - [`font_normal`](ThemeHook::font_normal): the first table's `design::FONT_BODY`, else
///   the base's font.
///
/// See [`DesignSystem`] for examples.
// NOTE(R7.S07): `Theme<D>` joins the prelude when the `Theme` trait of `twine-theme` goes.
pub struct Theme<D: DesignSystem>(PhantomData<fn() -> D>);

impl<D: DesignSystem> Theme<D> {
    /// The theme of design system `D`.
    #[must_use]
    pub const fn new() -> Self {
        Theme(PhantomData)
    }

    /// The recipe for the first class of `class`'s lineage that `D` or its base knows, and
    /// whether it is `D`'s own (`true`) or the base's.
    fn choose(class: &'static WidgetClass) -> Option<(RecipeRef, bool)> {
        let base = <D::Base as ThemeBase>::THEME;
        class.lineage().find_map(|c| {
            D::recipe(c)
                .map(|r| (r, true))
                .or_else(|| base.recipe(c).filter(|r| r.class().is(c)).map(|r| (r, false)))
        })
    }
}

impl<D: DesignSystem> Default for Theme<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: DesignSystem> core::fmt::Debug for Theme<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Theme({})", D::NAME)
    }
}

impl<D: DesignSystem> ThemeHook for Theme<D> {
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        match Self::choose(class) {
            Some((r, true)) => cx.add_recipe(r),
            _ => <D::Base as ThemeBase>::THEME.apply(cx, class),
        }
    }

    fn font_normal(&self) -> &'static Font {
        D::MODES
            .first()
            .and_then(|(_, t)| t.get(design::FONT_BODY))
            .unwrap_or_else(|| <D::Base as ThemeBase>::THEME.font_normal())
    }

    fn name(&self) -> &'static str {
        D::NAME
    }

    fn modes(&self) -> &'static [ThemeMode] {
        let (list, n) = const { &mode_list(D::MODES) };
        list.get(..*n).unwrap_or(&[])
    }

    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        D::MODES.iter().find(|&&(m, _)| m == mode).map(|&(_, t)| t)
    }

    fn metrics(&self, dpi: u16, resolution: Size) -> Option<ElementTable> {
        <D::Base as ThemeBase>::THEME.metrics(dpi, resolution)
    }

    fn recipe(&self, class: &'static WidgetClass) -> Option<RecipeRef> {
        Self::choose(class).map(|(r, _)| r)
    }
}

impl<D: DesignSystem> ThemeBase for Theme<D> {
    const THEME: &'static Self = &Theme(PhantomData);
}

/// A typed choice among the theme classes of one widget type: the value of the view's
/// `.variant(..)` (constant or reactive). Each value names a [`WidgetClass`] derived from the
/// widget's class (its [`base`](WidgetClass::base) chain reaches it), set with
/// [`Engine::set_theme_class`](crate::Engine::set_theme_class); a theme that does not know
/// the variant styles the node like the widget's class (lineage fallback).
///
/// `Widget` ties the variant to its widget type, so a button variant on a slider does not
/// compile.
///
/// ```
/// use twine_engine::{OBJ_CLASS, Obj, Variant, WidgetClass};
///
/// static OUTLINED: WidgetClass = WidgetClass::new("obj.outlined").base(&OBJ_CLASS);
///
/// /// The variants of a plain object.
/// #[derive(Clone, Copy)]
/// enum Panel { Plain, Outlined }
/// impl Variant for Panel {
///     type Widget = Obj;
///     fn class(self) -> &'static WidgetClass {
///         match self { Panel::Plain => &OBJ_CLASS, Panel::Outlined => &OUTLINED }
///     }
/// }
/// assert!(Panel::Outlined.class().is_a(&OBJ_CLASS));
/// assert!(Panel::Plain.class().is(&OBJ_CLASS));
/// ```
pub trait Variant: Copy + 'static {
    /// The widget type whose views take this variant.
    type Widget: Widget;

    /// The theme class of this variant (the widget's own class for the default variant).
    fn class(self) -> &'static WidgetClass;
}
