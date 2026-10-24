//! Recipes: a widget class's look as data ([`Recipe`], [`recipe!`](crate::recipe)), or as a
//! function for looks that depend on where the node is ([`ComputedRecipe`]); [`RecipeRef`]
//! names either. A theme adds a recipe to a node with [`ThemeCx::add_recipe`].
//!
//! A recipe is **presentation only** (design `docs/plan/r7-theming-design.md` §2.1): a static
//! recipe is a list of `(selector, &'static Style)` pairs, so a theme built from recipes can
//! add styles and nothing else — no behaviour, content or event handling.

use twine_style::{Part, Selector, State, Style};

use crate::{ThemeCx, WidgetClass};

/// The look of one widget class as data: a [`Style`] per [`Selector`] (part and states), all
/// `'static` (in flash). Written with [`recipe!`](crate::recipe), which builds every style
/// with [`style!`](twine_style::style) (so `..BASE` spreads compose styles at compile time).
///
/// A theme adds a recipe to a node with [`ThemeCx::add_recipe`]: one theme style entry per
/// selector, each a [`StyleRef::Static`](twine_style::StyleRef::Static) (no reference
/// counting, no allocation beyond the node's entry list). One flattened style per selector
/// keeps the node's entry list short, which is what property resolution scans.
///
/// The fields are public so a recipe can also be written by hand in a `static`
/// ([`Recipe::new`]); entries apply in order (among a node's theme styles the later added
/// wins, so put the default-state styles first).
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_engine::{OBJ_CLASS, Recipe, recipe};
/// use twine_style::{PropId, Selector, State, Style, StyleValue, design, style};
///
/// static BASE: Style = style! { radius: 6, padding: 8 };
///
/// recipe! {
///     /// A panel: the theme's surface, darker while pressed.
///     pub static PANEL for OBJ_CLASS {
///         Selector::MAIN => { ..BASE, bg: design::SURFACE },
///         State::PRESSED => { bg_opacity: Opa::pct(80) },
///     }
/// }
///
/// assert_eq!(PANEL.name, "PANEL");
/// assert!(PANEL.class.is(&OBJ_CLASS));
/// assert_eq!(PANEL.entries.len(), 2);
/// assert_eq!(PANEL.entries[1].0, Selector::state(State::PRESSED));
/// assert_eq!(PANEL.entries[0].1.get(PropId::PaddingTop), BASE.get(PropId::PaddingTop));
/// ```
pub struct Recipe {
    /// The recipe's name (the `static`'s name with [`recipe!`](crate::recipe)): shown in logs,
    /// coverage reports (`twine_testing::assert_theme_covers`) and `cargo xtask
    /// theme-recipes`. Not an identity.
    pub name: &'static str,
    /// The widget class the recipe styles (a theme class: a widget's class, a variant or a
    /// role, see [`Engine::set_theme_class`](crate::Engine::set_theme_class)).
    pub class: &'static WidgetClass,
    /// The styles, in the order they are added to a node.
    pub entries: &'static [(Selector, &'static Style)],
}

impl Recipe {
    /// A recipe named `name` for `class` with `entries` (what [`recipe!`](crate::recipe)
    /// writes).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_engine::{OBJ_CLASS, Recipe};
    /// use twine_style::{Selector, Style, style};
    ///
    /// static RED: Style = style! { bg: Color::RED };
    /// static ENTRIES: [(Selector, &Style); 1] = [(Selector::MAIN, &RED)];
    /// static RED_OBJ: Recipe = Recipe::new("RED_OBJ", &OBJ_CLASS, &ENTRIES);
    /// assert_eq!(RED_OBJ.entries.len(), 1);
    /// ```
    #[must_use]
    pub const fn new(
        name: &'static str,
        class: &'static WidgetClass,
        entries: &'static [(Selector, &'static Style)],
    ) -> Self {
        Self { name, class, entries }
    }

    /// `Some(self)` if the recipe is for exactly `class` (an identity comparison: a recipe for
    /// `BUTTON_CLASS` does not match a class derived from it — the theme's lineage walk does
    /// that, see [`DesignSystem`](crate::DesignSystem)). Chains in a
    /// [`DesignSystem::recipe`](crate::DesignSystem::recipe) written by hand:
    /// `CARD.for_class(c).or_else(|| BUTTON.for_class(c))`. Never panics.
    ///
    /// ```
    /// use twine_engine::{OBJ_CLASS, Recipe, WidgetClass};
    /// static PANEL: WidgetClass = WidgetClass::new("panel").base(&OBJ_CLASS);
    /// static OBJ: Recipe = Recipe::new("OBJ", &OBJ_CLASS, &[]);
    /// assert!(OBJ.for_class(&OBJ_CLASS).is_some());
    /// assert!(OBJ.for_class(&PANEL).is_none());
    /// ```
    #[inline]
    #[must_use]
    pub fn for_class(&'static self, class: &WidgetClass) -> Option<RecipeRef> {
        self.class.is(class).then_some(RecipeRef::Static(self))
    }

    /// The first recipe of `list` for exactly `class` (earlier entries win, so an
    /// application's override is listed before the base theme's recipes). O(`list`), never
    /// panics.
    ///
    /// ```
    /// use twine_engine::{OBJ_CLASS, Recipe};
    /// static MINE: Recipe = Recipe::new("MINE", &OBJ_CLASS, &[]);
    /// static BASE: Recipe = Recipe::new("BASE", &OBJ_CLASS, &[]);
    /// let found = Recipe::find(&[&MINE, &BASE], &OBJ_CLASS).unwrap();
    /// assert_eq!(found.name(), "MINE");
    /// ```
    #[must_use]
    pub fn find(list: &[&'static Recipe], class: &WidgetClass) -> Option<RecipeRef> {
        list.iter()
            .find(|r| r.class.is(class))
            .map(|&r| RecipeRef::Static(r))
    }
}

impl core::fmt::Debug for Recipe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Recipe")
            .field("name", &self.name)
            .field("class", &self.class.name)
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// A recipe computed when a node is themed: a plain function that adds styles through the
/// [`ThemeCx`], for looks that depend on the node's place (its parent's class, its index, the
/// display's DPI) — what a static [`Recipe`] cannot express. Prefer a static recipe where the
/// look is fixed: it is data in flash and costs no code.
///
/// The function must only add styles ([`ThemeCx::add_style`], [`ThemeCx::add_recipe`]): a
/// theme is presentation only.
///
/// ```
/// use twine_core::Color;
/// use twine_engine::{ComputedRecipe, OBJ_CLASS, ThemeCx};
/// use twine_style::{Selector, Style, style};
///
/// static ROOT: Style = style! { bg: Color::WHITE };
/// static NESTED: Style = style! { bg: Color::hex(0xEEEEEE) };
///
/// /// Screens white, nested containers grey.
/// fn panel(cx: &mut ThemeCx<'_>) {
///     let style = if cx.parent().is_some() { &NESTED } else { &ROOT };
///     cx.add_style(Selector::MAIN, style);
/// }
/// static PANEL: ComputedRecipe = ComputedRecipe::new("PANEL", &OBJ_CLASS, panel);
/// assert_eq!(PANEL.name, "PANEL");
/// ```
pub struct ComputedRecipe {
    /// The recipe's name (logs, coverage reports).
    pub name: &'static str,
    /// The widget class the recipe styles.
    pub class: &'static WidgetClass,
    /// Adds the styles to the node of the context.
    pub apply: fn(&mut ThemeCx<'_>),
}

impl ComputedRecipe {
    /// A computed recipe named `name` for `class`.
    #[must_use]
    pub const fn new(name: &'static str, class: &'static WidgetClass, apply: fn(&mut ThemeCx<'_>)) -> Self {
        Self { name, class, apply }
    }

    /// `Some(self)` if the recipe is for exactly `class` (see [`Recipe::for_class`]).
    ///
    /// ```
    /// use twine_engine::{ComputedRecipe, OBJ_CLASS};
    /// static NONE: ComputedRecipe = ComputedRecipe::new("NONE", &OBJ_CLASS, |_| {});
    /// assert!(NONE.for_class(&OBJ_CLASS).is_some());
    /// ```
    #[inline]
    #[must_use]
    pub fn for_class(&'static self, class: &WidgetClass) -> Option<RecipeRef> {
        self.class.is(class).then_some(RecipeRef::Computed(self))
    }
}

impl core::fmt::Debug for ComputedRecipe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComputedRecipe")
            .field("name", &self.name)
            .field("class", &self.class.name)
            .finish_non_exhaustive()
    }
}

/// A recipe as a theme hands it to [`ThemeCx::add_recipe`]: static data ([`Recipe`]) or a
/// function ([`ComputedRecipe`]). `Copy`, one pointer plus a tag; both forms are `'static`
/// (themes are fixed at compile time: no run-time recipes).
///
/// ```
/// use twine_engine::{ComputedRecipe, OBJ_CLASS, Recipe, RecipeRef};
/// static DATA: Recipe = Recipe::new("DATA", &OBJ_CLASS, &[]);
/// static CODE: ComputedRecipe = ComputedRecipe::new("CODE", &OBJ_CLASS, |_| {});
/// let a = RecipeRef::from(&DATA);
/// let b = RecipeRef::from(&CODE);
/// assert_eq!((a.name(), b.name()), ("DATA", "CODE"));
/// assert!(a.class().is(&OBJ_CLASS) && !a.ptr_eq(b) && a.ptr_eq(a));
/// ```
#[derive(Clone, Copy)]
pub enum RecipeRef {
    /// A recipe in flash.
    Static(&'static Recipe),
    /// A recipe computed per node.
    Computed(&'static ComputedRecipe),
}

impl RecipeRef {
    /// The recipe's name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            RecipeRef::Static(r) => r.name,
            RecipeRef::Computed(r) => r.name,
        }
    }

    /// The class the recipe styles.
    #[must_use]
    pub fn class(self) -> &'static WidgetClass {
        match self {
            RecipeRef::Static(r) => r.class,
            RecipeRef::Computed(r) => r.class,
        }
    }

    /// Whether both name the same recipe (an identity comparison).
    #[must_use]
    pub fn ptr_eq(self, other: RecipeRef) -> bool {
        match (self, other) {
            (RecipeRef::Static(a), RecipeRef::Static(b)) => core::ptr::eq(a, b),
            (RecipeRef::Computed(a), RecipeRef::Computed(b)) => core::ptr::eq(a, b),
            _ => false,
        }
    }
}

impl From<&'static Recipe> for RecipeRef {
    fn from(r: &'static Recipe) -> Self {
        RecipeRef::Static(r)
    }
}

impl From<&'static ComputedRecipe> for RecipeRef {
    fn from(r: &'static ComputedRecipe) -> Self {
        RecipeRef::Computed(r)
    }
}

impl core::fmt::Debug for RecipeRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "RecipeRef({})", self.name())
    }
}

/// Converts the selector of a [`recipe!`](crate::recipe) arm into a [`Selector`] in `const`
/// context: a `Selector` as is, a `Part` in every state, a `State` of the `Main` part
/// (inherent methods on concrete instantiations; no trait needed).
#[doc(hidden)]
pub struct __SelectorArg<T>(pub T);

impl __SelectorArg<Selector> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> Selector {
        self.0
    }
}

impl __SelectorArg<Part> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> Selector {
        Selector::part(self.0)
    }
}

impl __SelectorArg<State> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> Selector {
        Selector::state(self.0)
    }
}

/// Declares a `static` [`Recipe`]: the look of a widget class as data, one
/// [`style!`](twine_style::style) per selector.
///
/// ```text
/// recipe! {
///     /// docs
///     pub static NAME for CLASS {
///         SELECTOR => { style! body },
///         …
///     }
/// }
/// ```
///
/// - `CLASS` is a `WidgetClass` `static` (a widget's class, a variant or a role; written
///   without `&`).
/// - Each `SELECTOR` is a `const` [`Selector`], [`Part`] (every state) or [`State`] (of the
///   `Main` part): `Selector::MAIN`, `Part::Knob`, `State::PRESSED`,
///   `Selector::part(Part::Indicator).with_state(State::CHECKED)`.
/// - Each body is what [`style!`](twine_style::style) takes, `..BASE` spreads included, so
///   recipes share styles and override single properties at compile time.
///
/// The whole recipe is `const` (in flash); nothing runs at start-up. The name
/// ([`Recipe::name`]) is the `static`'s name. Several recipes can be declared in one
/// invocation.
///
/// ```
/// use twine_core::{Color, Duration, Opa};
/// use twine_engine::{OBJ_CLASS, WidgetClass, recipe};
/// use twine_style::{Part, Selector, State, Style, Transition, design, style};
///
/// static GAUGE: WidgetClass = WidgetClass::new("gauge").parts(&[Part::Main, Part::Indicator]).base(&OBJ_CLASS);
/// static CARD: Style = style! { bg: design::SURFACE, radius: design::RADIUS_M, padding: design::SPACE_M };
/// static PRESS: Transition = Transition::all(Duration::ms(80));
///
/// recipe! {
///     /// The gauge: a card with a primary indicator.
///     pub static GAUGE_RECIPE for GAUGE {
///         Part::Main => { ..CARD },
///         Part::Indicator => { bg: design::PRIMARY },
///         State::PRESSED => { bg_opacity: Opa::pct(80), transition: &PRESS },
///         Selector::part(Part::Indicator).with_state(State::DISABLED) => { bg: design::DISABLED },
///     }
///
///     /// A borderless variant.
///     pub static FLAT_GAUGE for GAUGE {
///         Part::Indicator => { bg: Color::BLACK },
///     }
/// }
/// assert_eq!(GAUGE_RECIPE.entries.len(), 4);
/// assert_eq!(FLAT_GAUGE.name, "FLAT_GAUGE");
/// ```
///
/// A selector that is not a `Selector`, `Part` or `State` does not compile:
///
/// ```compile_fail
/// use twine_engine::{OBJ_CLASS, recipe};
/// recipe! { static BAD for OBJ_CLASS { 3 => { radius: 2 } } }
/// ```
#[macro_export]
macro_rules! recipe {
    ($(
        $(#[$meta:meta])*
        $vis:vis static $name:ident for $class:path {
            $($sel:expr => { $($body:tt)* }),* $(,)?
        }
    )+) => {$(
        $(#[$meta])*
        $vis static $name: $crate::Recipe = $crate::Recipe {
            name: ::core::stringify!($name),
            class: &$class,
            entries: &[$((
                $crate::__private::SelectorArg($sel).get(),
                &$crate::__private::style! { $($body)* },
            )),*],
        };
    )+};
}
