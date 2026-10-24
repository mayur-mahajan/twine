//! Engine-side themes: the object-safe [`ThemeHook`] trait, the [`ThemeCx`] a theme styles a
//! node through, [`DisplayTheme`] (the theme a display is created with) and the engine API
//! that applies the theme to every new node (LVGL `lv_theme_apply`) and switches its mode.
//!
//! **One theme per display, fixed at compile time** (design `docs/plan/r7-theming-design.md`
//! §2.0a): a theme is a `&'static dyn ThemeHook` — a `static` in flash, no heap, nothing
//! built at run time — given when the display is created
//! ([`Engine::add_display`](crate::Engine::add_display) and its siblings take a
//! [`DisplayTheme`]). There is no call that installs, replaces or removes a theme later, so no
//! code for switching themes is linked. The **mode** (light, dark, night, high contrast)
//! switches at run time ([`Engine::set_theme_mode`]) among the modes the theme lists.
//!
//! The engine does not depend on `twine-theme`: that crate's `Theme` trait extends
//! [`ThemeHook`] with fonts.
//!
//! A theme supplies the values of the [design elements](twine_style::design) in two tables:
//! a `'static` table per [`ThemeMode`] it lists ([`ThemeHook::design`]: colours, fonts,
//! motion — in flash) and an optional per-display table ([`ThemeHook::metrics`]: values that
//! depend on the display's DPI and size, computed once when the display is created). The
//! display merges the two (the mode's table wins) into one indexed array of finished values
//! ([`FlatElements`]) that the resolver reads. A mode switch refills that array in place and
//! re-resolves every node of the display once, without re-applying the theme's styles and
//! without allocating.
//!
//! A node is themed as its **theme class**: its widget's class, or a class derived from it
//! that the application set — a variant (`ButtonVariant::Tinted`) or a role
//! (`.role(&METRIC_TILE)`) — with [`Engine::set_theme_class`]. A theme styles a theme class it
//! does not know like the nearest class of its lineage it knows. Themes built from data
//! implement [`DesignSystem`](crate::DesignSystem) and are installed as
//! [`Theme<D>`](crate::Theme); their per-class looks are [recipes](crate::Recipe).

use alloc::boxed::Box;

use twine_core::Size;
use twine_style::design::{DesignValue, Element, ElementRef, ElementTable, ElementType, FlatElements};
use twine_style::{EntryKind, Selector, StyleEntry, StyleRef, StyleValue, ThemeMode, dpx};
use twine_text::Font;

use crate::{DisplayId, Engine, NodeId, RecipeRef, Tree, WidgetClass, fmt_node_id};

/// The empty table a display without a start-mode table merges its metrics with.
static EMPTY_TABLE: ElementTable = ElementTable::new();

/// What the engine needs from a theme (LVGL `lv_theme_t`): styling a node when it is created,
/// the default font, and the [design element](twine_style::design) values of each
/// [`ThemeMode`] it lists (the theme's colors: `design::PRIMARY`, `design::SECONDARY`, …;
/// LVGL's `color_primary` / `color_secondary`).
///
/// A theme is a `static` (`&'static dyn ThemeHook`, design §2.0a): it is given to a display
/// when the display is created ([`DisplayTheme`]) and never replaced; nothing in it is built
/// at run time except the optional per-display [`metrics`](Self::metrics) table.
///
/// A theme adds styles with [`ThemeCx::add_style`] / [`ThemeCx::add_recipe`]; they get the
/// lowest priority of all styles (after normal and local styles), exactly like LVGL theme
/// styles. The engine calls [`apply`](Self::apply) once for every node created on a display
/// with a theme, after the node is linked into the tree and **before**
/// [`Widget::init`](crate::Widget::init), and again for a node whose
/// [theme class](Engine::set_theme_class) changes.
///
/// A theme recognises classes by **identity** ([`WidgetClass::is`], a pointer comparison —
/// never by [`name`](WidgetClass::name)) and should style a class it does not know like the
/// nearest class of its [`lineage`](WidgetClass::lineage) it knows (its
/// [`base`](WidgetClass::base) chain), so custom widgets can be themed like built-in ones.
///
/// A theme also gives the [design elements](twine_style::design) their values, per
/// [`ThemeMode`] ([`modes`](Self::modes), [`design`](Self::design)) and per display
/// ([`metrics`](Self::metrics)); styles that use design elements follow
/// [`Engine::set_theme_mode`] without being re-applied.
///
/// ```
/// use twine_core::{Color, ColorFormat};
/// use twine_engine::{DisplayTheme, Engine, EngineConfig, OBJ_CLASS, Obj, ThemeCx, ThemeHook, WidgetClass};
/// use twine_hal::DisplayInfo;
/// use twine_style::{Part, PropId, Selector, Style, style};
/// use twine_text::Font;
///
/// static RED_BG: Style = style! { bg: Color::RED };
///
/// /// Every plain object red (screens excepted).
/// struct Red;
/// impl ThemeHook for Red {
///     fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
///         // Classes are compared by identity, never by name.
///         if class.is(&OBJ_CLASS) && cx.parent().is_some() {
///             cx.add_style(Selector::MAIN, &RED_BG);
///         }
///     }
///     fn font_normal(&self) -> &'static Font {
///         &twine_text::EMPTY_FONT
///     }
/// }
/// static RED: Red = Red;
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
/// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&RED))).unwrap();
/// let screen = e.active_screen(d).unwrap();
/// let n = e.create(screen, Box::new(Obj)).unwrap();
/// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::RED);
/// ```
pub trait ThemeHook {
    /// Adds the theme's styles to the node of `cx`, whose theme class is `class`.
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass);

    /// The default font of text on the display (the default `Font`).
    fn font_normal(&self) -> &'static Font;

    /// A short name for logs (default `"theme"`).
    fn name(&self) -> &'static str {
        "theme"
    }

    /// The modes the theme supports, in the order a mode switch cycles through them
    /// ([`ThemeMode::next_in`]): exactly the modes for which [`design`](Self::design) returns
    /// a table. The first is the mode a display starts in unless its [`DisplayTheme`] names
    /// another. Default: [`ThemeMode::Light`] only.
    ///
    /// The engine does not need it to switch modes ([`Engine::set_theme_mode`] asks
    /// [`design`](Self::design)); it is what applications and tools (twine-sim's `F12`) offer
    /// the user ([`Engine::theme_modes`]).
    ///
    /// ```
    /// use twine_engine::{ThemeCx, ThemeHook, WidgetClass};
    /// use twine_style::ThemeMode;
    ///
    /// struct Plain;
    /// impl ThemeHook for Plain {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// }
    /// assert_eq!(Plain.modes(), &[ThemeMode::Light]);
    /// ```
    fn modes(&self) -> &[ThemeMode] {
        &[ThemeMode::Light]
    }

    /// The values of the [design elements](twine_style::design) in `mode`: a `'static`
    /// table (`const`, in flash, no heap), or `None` if the theme does not support `mode`
    /// (the default: a theme without design elements).
    ///
    /// The engine asks when a display is created (for its start mode) and on every
    /// [`Engine::set_theme_mode`]; a mode switch is a pointer swap and allocates nothing. A
    /// table should define every standard element its [`metrics`](Self::metrics) table does
    /// not ([`ElementTable::missing_standard`]); an element defined by neither resolves to the
    /// property's default.
    ///
    /// `None` for the start mode is accepted: the display takes that mode without a table, so
    /// every design element its metrics table does not define resolves to its property's
    /// default (with a warning once per element kind).
    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        let _ = mode;
        None
    }

    /// The display's own design element values: elements whose values depend on the display
    /// (sizes scaled by its DPI, paddings by its size class — the LVGL theme's), the same in
    /// every mode. Computed **once, when the display is created** (before the application
    /// builds its UI, so before `seal`), kept by the display and never rebuilt: a mode switch
    /// does not touch it. An element the mode's [`design`](Self::design) table does not define
    /// is taken from here: the display merges the mode's table over this one into a single
    /// indexed array of finished values ([`FlatElements`], refilled in place on a mode switch),
    /// which is what the resolver reads. Default `None` (a theme whose values do not depend on
    /// the display).
    ///
    /// Parameters: `dpi` is the display's [`DisplayInfo::dpi`](twine_hal::DisplayInfo::dpi)
    /// (dots per inch; [`DEFAULT_DPI`](twine_style::DEFAULT_DPI) unless the driver says
    /// otherwise), `resolution` the display's logical size in pixels when it is created.
    /// Allocates what the returned table holds (its application and private slots); never
    /// called while drawing.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Size};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::design::{ElementTable, LengthElement};
    /// use twine_style::{Length, dpx};
    ///
    /// /// A theme-private gap: 10 px at 160 DPI, scaled for the display.
    /// const GAP: LengthElement = LengthElement::private(0);
    /// struct Scaled;
    /// impl ThemeHook for Scaled {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn metrics(&self, dpi: u16, _: Size) -> Option<ElementTable> {
    ///         Some(ElementTable::new().with(GAP, Length::Px(dpx(10, dpi))))
    ///     }
    /// }
    /// static SCALED: Scaled = Scaled;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8).with_dpi(320);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&SCALED))).unwrap();
    /// assert_eq!(e.design_value(d, GAP), Some(Length::Px(20)));
    /// ```
    fn metrics(&self, dpi: u16, resolution: Size) -> Option<ElementTable> {
        let _ = (dpi, resolution);
        None
    }

    /// Describes the theme for an audit report (`AppConfig::audit_report`, section `theme`):
    /// the hook a certification file uses to pin the exact theme a product runs. The default
    /// writes `theme.name` ([`name`](Self::name)), the first listed mode and the supported
    /// modes' count; a theme can add entries (a content digest of its tables, its version).
    /// Must not allocate; never panics.
    ///
    /// ```
    /// use twine_core::audit::{AuditSink, AuditValue, KeyValues};
    /// use twine_engine::{ThemeCx, ThemeHook, WidgetClass};
    ///
    /// struct Plain;
    /// impl ThemeHook for Plain {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn name(&self) -> &'static str { "plain" }
    ///     fn audit(&self, out: &mut dyn AuditSink) {
    ///         out.entry("theme", "name", AuditValue::Text(self.name()));
    ///         out.entry("theme", "version", AuditValue::Uint(3));
    ///     }
    /// }
    /// struct Report<'a>(&'a dyn ThemeHook);
    /// impl twine_core::audit::Audit for Report<'_> {
    ///     fn audit(&self, out: &mut dyn AuditSink) { self.0.audit(out) }
    /// }
    /// assert_eq!(KeyValues(&Report(&Plain)).to_string(), "theme.name=plain\ntheme.version=3\n");
    /// ```
    // NOTE(R7.S10): themes add `theme.digest` (`ThemeDigest`, CRC32 over their tables) here.
    fn audit(&self, out: &mut dyn twine_core::audit::AuditSink) {
        use twine_core::audit::AuditValue;
        let first = self.modes().first().copied().unwrap_or(ThemeMode::Light);
        out.entry("theme", "name", AuditValue::Text(self.name()));
        out.entry("theme", "mode", AuditValue::Text(first.name()));
        out.entry("theme", "modes", AuditValue::from_usize(self.modes().len()));
    }

    /// The recipe the theme styles nodes of theme class `class` with — after its lineage
    /// fallback, so for a variant it does not know, the recipe of the variant's base — or
    /// `None`: no recipe for `class` or any of its bases, or a theme that styles in code
    /// ([`apply`](Self::apply)) and has no recipes (the default).
    ///
    /// What coverage checks read (`twine_testing::assert_theme_covers`, `cargo xtask
    /// theme-recipes`) and what a [`DesignSystem`](crate::DesignSystem) extending this theme
    /// falls back to; a theme built from recipes answers what its `apply` adds
    /// ([`Theme<D>`](crate::Theme) and the built-in themes do). Called outside drawing only;
    /// must not panic.
    ///
    /// ```
    /// use twine_engine::{OBJ_CLASS, ThemeCx, ThemeHook, WidgetClass};
    ///
    /// struct Plain;
    /// impl ThemeHook for Plain {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// }
    /// assert!(Plain.recipe(&OBJ_CLASS).is_none());
    /// ```
    fn recipe(&self, class: &'static WidgetClass) -> Option<RecipeRef> {
        let _ = class;
        None
    }
}

impl core::fmt::Debug for dyn ThemeHook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ThemeHook({})", self.name())
    }
}

/// The theme a display is created with: a `&'static` theme (fixed at compile time, design
/// §2.0a) and the mode the display starts in. Passed to
/// [`Engine::add_display`](crate::Engine::add_display),
/// [`add_chunked_display`](crate::Engine::add_chunked_display) and
/// [`add_framebuffer_display`](crate::Engine::add_framebuffer_display) (and built by the view
/// layer's `DisplayBuilder::theme` / `AppConfig::theme` + `.mode(..)`). `Copy`, `const`; a
/// display keeps its theme for its lifetime (its mode switches with
/// [`Engine::set_theme_mode`]).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
/// use twine_hal::DisplayInfo;
/// use twine_style::ThemeMode;
/// use twine_style::design::ElementTable;
///
/// struct DayNight;
/// static TABLE: ElementTable = ElementTable::new();
/// impl ThemeHook for DayNight {
///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
///     fn modes(&self) -> &'static [ThemeMode] { &[ThemeMode::Light, ThemeMode::Night] }
///     fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
///         self.modes().contains(&mode).then_some(&TABLE)
///     }
/// }
/// static THEME: DayNight = DayNight;
/// const NIGHT_START: DisplayTheme = DisplayTheme::new(&THEME).mode(ThemeMode::Night);
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
/// let d = e.add_chunked_display(info, 64, Some(NIGHT_START)).unwrap();
/// assert_eq!(e.theme_mode(d), ThemeMode::Night);
/// assert_eq!(e.theme(d).map(|t| t.modes().len()), Some(2));
/// ```
#[derive(Clone, Copy)]
pub struct DisplayTheme {
    /// The theme.
    pub theme: &'static dyn ThemeHook,
    /// The mode the display starts in: `None` = the theme's first listed mode
    /// ([`ThemeHook::modes`]). A mode the theme does not list starts as its first mode
    /// instead, with a warning.
    pub mode: Option<ThemeMode>,
}

impl DisplayTheme {
    /// `theme`, starting in its first listed mode.
    #[must_use]
    pub const fn new(theme: &'static dyn ThemeHook) -> Self {
        Self { theme, mode: None }
    }

    /// Starts in `mode` instead of the theme's first listed mode (builder style, `const`).
    #[must_use]
    pub const fn mode(mut self, mode: ThemeMode) -> Self {
        self.mode = Some(mode);
        self
    }
}

impl core::fmt::Debug for DisplayTheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayTheme")
            .field("theme", &self.theme.name())
            .field("mode", &self.mode)
            .finish()
    }
}

/// Access to one node for a theme's [`ThemeHook::apply`]: its tree position, its display's
/// resolution and DPI, [`add_style`](Self::add_style) and [`add_recipe`](Self::add_recipe).
///
/// Styles added here do not refresh anything by themselves: the engine refreshes the node once
/// after the theme ran (new nodes are laid out and drawn anyway).
pub struct ThemeCx<'a> {
    tree: &'a mut Tree,
    node: NodeId,
    display: DisplayId,
    dpi: u16,
    resolution: Size,
    added: u16,
}

impl core::fmt::Debug for ThemeCx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ThemeCx")
            .field("node", &self.node)
            .field("display", &self.display)
            .field("dpi", &self.dpi)
            .finish_non_exhaustive()
    }
}

impl<'a> ThemeCx<'a> {
    /// A context for `node` of `display` (with its DPI and logical resolution).
    pub fn new(tree: &'a mut Tree, node: NodeId, display: DisplayId, dpi: u16, resolution: Size) -> Self {
        Self {
            tree,
            node,
            display,
            dpi,
            resolution,
            added: 0,
        }
    }

    /// The node being styled.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The node's parent (`None` for screens).
    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        self.tree.parent(self.node)
    }

    /// The class the node's parent is themed as (themes style e.g. list items differently):
    /// its [theme class](Engine::set_theme_class) — a variant or role — if it has one, else
    /// its widget's class.
    #[must_use]
    pub fn parent_class(&self) -> Option<&'static WidgetClass> {
        self.parent().and_then(|p| self.tree.theme_class(p))
    }

    /// The class the node's grandparent is themed as (see
    /// [`parent_class`](Self::parent_class)).
    #[must_use]
    pub fn grandparent_class(&self) -> Option<&'static WidgetClass> {
        let gp = self.parent().and_then(|p| self.tree.parent(p))?;
        self.tree.theme_class(gp)
    }

    /// The node's position among its siblings.
    #[must_use]
    pub fn index(&self) -> Option<usize> {
        self.tree.index(self.node)
    }

    /// The tree (read-only).
    #[must_use]
    pub fn tree(&self) -> &Tree {
        self.tree
    }

    /// Adds `style` for `selector` as a theme style (lowest priority; among theme styles the
    /// latest added wins, as with LVGL's `lv_obj_add_style` in a theme's apply callback).
    pub fn add_style(&mut self, selector: Selector, style: impl Into<StyleRef>) {
        if let Some(n) = self.tree.node_mut(self.node) {
            n.styles
                .insert(StyleEntry::new(selector, style.into(), EntryKind::Theme));
            self.added = self.added.saturating_add(1);
        }
    }

    /// Adds `recipe` (a [`Recipe`](crate::Recipe) or a [`ComputedRecipe`](crate::ComputedRecipe))
    /// to the node: a static recipe's entries in order, each as a theme style
    /// ([`add_style`](Self::add_style) with a
    /// [`StyleRef::Static`](twine_style::StyleRef::Static): no reference counting); a computed
    /// recipe's function runs with this context. Allocates only the node's entry list growth;
    /// never panics.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, OBJ_CLASS, Obj, ThemeCx, ThemeHook, WidgetClass, recipe};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::{Part, PropId, Selector, State};
    ///
    /// recipe! {
    ///     static PANEL for OBJ_CLASS {
    ///         Selector::MAIN => { bg: Color::BLUE },
    ///         State::PRESSED => { bg: Color::RED },
    ///     }
    /// }
    /// struct Panels;
    /// impl ThemeHook for Panels {
    ///     fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
    ///         if class.is(&OBJ_CLASS) && cx.parent().is_some() {
    ///             cx.add_recipe(&PANEL);
    ///         }
    ///     }
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// }
    /// static PANELS: Panels = Panels;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&PANELS))).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// assert_eq!(e.tree().node(n).unwrap().styles().len(), 2);
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::BLUE);
    /// e.add_state(n, State::PRESSED);
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::RED);
    /// ```
    pub fn add_recipe(&mut self, recipe: impl Into<RecipeRef>) {
        match recipe.into() {
            RecipeRef::Static(r) => {
                for &(selector, style) in r.entries {
                    self.add_style(selector, StyleRef::Static(style));
                }
            }
            RecipeRef::Computed(r) => (r.apply)(self),
        }
    }

    /// `px` pixels at 160 DPI scaled to the display's DPI (LVGL `LV_DPX_CALC`):
    /// `(px · dpi + 80) / 160`, at least 1 for positive `px`; negative values are scaled
    /// symmetrically.
    #[must_use]
    pub fn dpx(&self, px: i32) -> i32 {
        dpx(px, self.dpi)
    }

    /// The display's DPI.
    #[must_use]
    pub fn dpi(&self) -> u16 {
        self.dpi
    }

    /// The display's logical resolution.
    #[must_use]
    pub fn resolution(&self) -> Size {
        self.resolution
    }

    /// The display the node belongs to.
    #[must_use]
    pub fn display(&self) -> DisplayId {
        self.display
    }
}

/// Walks the themed nodes of one display, parents first: every node of its screens and every
/// node on its layers (not the layers themselves, which get no theme styles). Holds no borrow
/// of the engine between steps (the caller may mutate it) and allocates nothing.
struct ThemedWalk {
    d: usize,
    root: usize,
    scope: Option<NodeId>,
    cur: Option<NodeId>,
}

impl ThemedWalk {
    fn new(d: usize) -> Self {
        Self {
            d,
            root: 0,
            scope: None,
            cur: None,
        }
    }

    /// The next node (`None` when done). A node deleted since the previous step ends its
    /// subtree's walk.
    fn next(&mut self, e: &Engine) -> Option<NodeId> {
        loop {
            if let (Some(scope), Some(cur)) = (self.scope, self.cur) {
                self.cur = e.next_in_subtree(scope, cur);
                if self.cur.is_some() {
                    return self.cur;
                }
            }
            let disp = e.displays.get(self.d)?;
            let k = self.root;
            self.root = self.root.saturating_add(1);
            if let Some(&s) = disp.screens.get(k) {
                self.scope = Some(s);
                self.cur = Some(s);
                return Some(s);
            }
            // `k ≥ screens.len()` here (the screen lookup failed).
            let layer = [disp.bottom_layer, disp.top_layer, disp.sys_layer]
                .get(k.saturating_sub(disp.screens.len()))
                .copied()?;
            // Continue with the layer's first child (the layer itself is skipped).
            self.scope = Some(layer);
            self.cur = Some(layer);
        }
    }
}

/// The themed nodes of a display, parents first ([`Engine::themed_nodes`]): every node of its
/// screens and every node on its layers, not the layers themselves (they get no theme
/// styles). Allocates nothing.
pub struct ThemedNodes<'a> {
    engine: &'a Engine,
    walk: ThemedWalk,
}

impl Iterator for ThemedNodes<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        self.walk.next(self.engine)
    }
}

impl core::fmt::Debug for ThemedNodes<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ThemedNodes")
            .field("display", &self.walk.d)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// The theme `display` was created with (`None` without a theme or for an unknown
    /// display). Never panics; O(1).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    ///
    /// struct Plain;
    /// impl ThemeHook for Plain {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn name(&self) -> &'static str { "plain" }
    /// }
    /// static PLAIN: Plain = Plain;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let bare = e.add_chunked_display(info, 64, None).unwrap();
    /// let themed = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&PLAIN))).unwrap();
    /// assert!(e.theme(bare).is_none());
    /// assert_eq!(e.theme(themed).map(|t| t.name()), Some("plain"));
    /// ```
    #[must_use]
    pub fn theme(&self, display: DisplayId) -> Option<&'static dyn ThemeHook> {
        self.displays.get(display.index())?.theme
    }

    /// Switches the theme of `display` to `mode` (e.g. light → dark): the display takes the
    /// theme's `'static` [design element](twine_style::design) table for `mode`
    /// ([`ThemeHook::design`]: a pointer swap), and every node of the display is re-resolved
    /// once (style caches, extra draw sizes, layout, one `StyleChanged` event each) and the
    /// display is redrawn. The theme's styles are **not** re-applied: everything styled with
    /// design elements changes, nothing else does, so no view is rebuilt and nothing is
    /// allocated. The display's [metrics](ThemeHook::metrics) table is kept.
    ///
    /// Idempotent: the current mode does nothing. **Fallback:** if `display` does not exist,
    /// has no theme, or its theme does not support `mode` ([`ThemeHook::design`] returns
    /// `None`: a mode the theme does not list), the display **keeps its current
    /// mode and table** — nothing changes on screen — and a warning is logged
    /// (`"twine::style"`). Never panics. [`theme_modes`](Self::theme_modes) lists the modes
    /// that will be accepted.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, Obj, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::design::{self, ElementTable};
    /// use twine_style::{Part, PropId, Selector, StyleProp, ThemeMode};
    ///
    /// /// A theme that only supplies design elements, in two modes (const tables, in flash).
    /// struct Tiny;
    /// static LIGHT: ElementTable = ElementTable::new().color(design::SURFACE, Color::WHITE);
    /// static DARK: ElementTable = ElementTable::new().color(design::SURFACE, Color::BLACK);
    /// impl ThemeHook for Tiny {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn modes(&self) -> &'static [ThemeMode] { &[ThemeMode::Light, ThemeMode::Dark] }
    ///     fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
    ///         match mode {
    ///             ThemeMode::Light => Some(&LIGHT),
    ///             ThemeMode::Dark => Some(&DARK),
    ///             _ => None,
    ///         }
    ///     }
    /// }
    /// static TINY: Tiny = Tiny;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::Rgb565);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&TINY))).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(design::SURFACE.into()));
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::WHITE);
    /// e.set_theme_mode(d, ThemeMode::Dark);
    /// assert_eq!(e.theme_mode(d), ThemeMode::Dark);
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::BLACK);
    /// e.set_theme_mode(d, ThemeMode::Night); // not listed: kept Dark (and warned)
    /// assert_eq!(e.theme_mode(d), ThemeMode::Dark);
    /// ```
    pub fn set_theme_mode(&mut self, display: DisplayId, mode: ThemeMode) {
        let d = display.index();
        let Some(disp) = self.displays.get(d) else {
            twine_core::warn!(target: "twine::style", "set_theme_mode: display {} not found", display);
            return;
        };
        if disp.theme_mode == mode {
            return;
        }
        let Some(theme) = disp.theme else {
            twine_core::warn!(target: "twine::style", "set_theme_mode: display {} has no theme", display);
            return;
        };
        let Some(table) = theme.design(mode) else {
            twine_core::warn!(
                target: "twine::style",
                "set_theme_mode: theme {} has no {:?} mode",
                theme.name(),
                mode
            );
            return;
        };
        let Some(disp) = self.displays.get_mut(d) else {
            return;
        };
        disp.theme_mode = mode;
        disp.design = Some(table);
        if let Some(flat) = disp.elements.as_mut() {
            // In place: the room for every listed mode was reserved when the display was
            // created.
            flat.fill(disp.metrics.as_deref(), table, disp.info.dpi);
        }
        disp.design_epoch = disp.design_epoch.wrapping_add(1);
        twine_core::info!(target: "twine::style", "display {}: theme mode {:?}", display, mode);
        self.restyle_display(d);
    }

    /// The modes the theme of `display` supports ([`ThemeHook::modes`]; empty for a display
    /// without a theme or an unknown display). Cycle through them with
    /// [`ThemeMode::next_in`]:
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::ElementTable;
    ///
    /// /// A theme with a light and a night table.
    /// struct DayNight;
    /// static TABLE: ElementTable = ElementTable::new();
    /// impl ThemeHook for DayNight {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn modes(&self) -> &'static [ThemeMode] { &[ThemeMode::Light, ThemeMode::Night] }
    ///     fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
    ///         self.modes().contains(&mode).then_some(&TABLE)
    ///     }
    /// }
    /// static DAY_NIGHT: DayNight = DayNight;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let bare = e.add_chunked_display(info, 64, None).unwrap();
    /// assert!(e.theme_modes(bare).is_empty()); // no theme
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&DAY_NIGHT))).unwrap();
    /// let next = e.theme_mode(d).next_in(e.theme_modes(d));
    /// e.set_theme_mode(d, next);
    /// assert_eq!(e.theme_mode(d), ThemeMode::Night);
    /// ```
    #[must_use]
    pub fn theme_modes(&self, display: DisplayId) -> &'static [ThemeMode] {
        self.theme(display).map_or(&[], |t| t.modes())
    }

    /// The theme mode of `display` ([`ThemeMode::Light`] for a display without a theme or an
    /// unknown display).
    /// Never panics; O(1).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64, None).unwrap();
    /// assert_eq!(e.theme_mode(d), ThemeMode::Light); // no theme
    /// ```
    #[must_use]
    pub fn theme_mode(&self, display: DisplayId) -> ThemeMode {
        self.displays
            .get(display.index())
            .map_or(ThemeMode::Light, |d| d.theme_mode)
    }

    /// A number that changes whenever the theme mode (and so the design element table) of
    /// `display` changes ([`set_theme_mode`](Self::set_theme_mode)): lets a cache of design
    /// element values (e.g. a reactive layer) notice that it is stale. 0 for an unknown
    /// display.
    ///
    /// Compare for **inequality** only: the counter wraps around at `u32::MAX` (after 2³²
    /// changes it repeats a value, which a cache refreshed at least once in that span never
    /// observes). Never panics; O(1).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::ElementTable;
    ///
    /// struct Two;
    /// static TABLE: ElementTable = ElementTable::new();
    /// impl ThemeHook for Two {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn modes(&self) -> &'static [ThemeMode] { &[ThemeMode::Light, ThemeMode::Dark] }
    ///     fn design(&self, _: ThemeMode) -> Option<&'static ElementTable> { Some(&TABLE) }
    /// }
    /// static TWO: Two = Two;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&TWO))).unwrap();
    /// let seen = e.design_epoch(d);
    /// e.set_theme_mode(d, ThemeMode::Dark);
    /// assert_ne!(e.design_epoch(d), seen); // a cached design value is stale
    /// ```
    #[must_use]
    pub fn design_epoch(&self, display: DisplayId) -> u32 {
        self.displays.get(display.index()).map_or(0, |d| d.design_epoch)
    }

    /// The [design element](twine_style::design) table of `display`'s current mode (its
    /// theme's `'static` table; `None` without a theme, for a theme without design elements,
    /// or for an unknown display). Elements the display's [metrics](ThemeHook::metrics) table
    /// defines are not in it: [`design_value`](Self::design_value) reads both. Never panics;
    /// O(1).
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::{self, ElementTable};
    ///
    /// struct Brand;
    /// static BRAND: ElementTable = ElementTable::new().color(design::PRIMARY, Color::RED);
    /// impl ThemeHook for Brand {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn design(&self, _: ThemeMode) -> Option<&'static ElementTable> { Some(&BRAND) }
    /// }
    /// static THEME: Brand = Brand;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let bare = e.add_chunked_display(info, 64, None).unwrap();
    /// assert!(e.design_table(bare).is_none()); // no theme
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&THEME))).unwrap();
    /// assert_eq!(e.design_table(d).and_then(|t| t.get(design::PRIMARY)), Some(Color::RED));
    /// ```
    #[must_use]
    pub fn design_table(&self, display: DisplayId) -> Option<&'static ElementTable> {
        self.displays.get(display.index())?.design
    }

    /// The value `display`'s theme gives `element` in its current mode: the mode's table
    /// first, then the display's [metrics](ThemeHook::metrics) table (`None` if neither
    /// defines it). Never panics; allocates nothing.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::design::{self, ElementTable};
    /// use twine_style::ThemeMode;
    ///
    /// struct Brand;
    /// static BRAND: ElementTable = ElementTable::new().color(design::PRIMARY, Color::RED);
    /// impl ThemeHook for Brand {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn design(&self, _: ThemeMode) -> Option<&'static ElementTable> { Some(&BRAND) }
    /// }
    /// static THEME: Brand = Brand;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&THEME))).unwrap();
    /// assert_eq!(e.design_value(d, design::PRIMARY), Some(Color::RED));
    /// assert_eq!(e.design_value(d, design::DANGER), None);
    /// ```
    #[must_use]
    pub fn design_value<T: ElementType>(&self, display: DisplayId, element: Element<T>) -> Option<T> {
        self.displays
            .get(display.index())?
            .elements
            .as_ref()?
            .get(element)
    }

    /// The display whose design elements styles of `id` resolve with: the display showing
    /// `id`, else the default display. With one display this is a direct read.
    #[inline]
    fn node_display(&self, id: NodeId) -> Option<&crate::display::Display> {
        if let [d] = self.displays.as_slice() {
            return Some(d);
        }
        self.display_of(id)
            .or(self.default_display)
            .and_then(|d| self.displays.get(d.index()))
    }

    /// The value of `element` for `id` (the [`StyleSource::design_value`] of the engine): one
    /// indexed lookup in the node's display's table (the mode's, merged over the metrics).
    ///
    /// [`StyleSource::design_value`]: twine_style::StyleSource::design_value
    #[inline]
    pub(crate) fn node_design_value(&self, id: NodeId, element: ElementRef) -> Option<StyleValue> {
        self.node_display(id)?.elements.as_ref()?.value(element)
    }

    /// The default font of `display`: its theme's [`ThemeHook::font_normal`], else
    /// [`EngineConfig::default_font`](crate::EngineConfig::default_font), else
    /// [`twine_text::EMPTY_FONT`].
    #[must_use]
    pub fn default_font(&self, display: DisplayId) -> &'static Font {
        self.displays
            .get(display.index())
            .and_then(|d| d.theme)
            .map(ThemeHook::font_normal)
            .or(self.config.default_font)
            .unwrap_or(&twine_text::EMPTY_FONT)
    }

    /// The theme of the display showing `id` (on a screen or a layer), if any.
    #[must_use]
    pub fn theme_of(&self, id: NodeId) -> Option<&'static dyn ThemeHook> {
        let root = self.tree.root_of(id)?;
        self.displays.iter().find(|d| d.owns_root(root))?.theme
    }

    /// The value of a [design value](twine_style::design::DesignValue) for node `id`: a fixed
    /// value as is, an element looked up in the design element tables of the display showing
    /// `id` (its theme's table for the current mode, then the display's metrics table; with
    /// one display a direct read). `None` when the element is not defined there (no theme, or
    /// a theme without it): the caller picks its fallback.
    ///
    /// For widgets that keep a color (or length, …) of their own outside the style system,
    /// like an LED's color: resolve it when the widget is created and again on
    /// [`EventCode::StyleChanged`](crate::EventCode::StyleChanged) (sent to every node when the
    /// theme mode changes), so it follows the theme like a style property. One or two table
    /// reads; allocates nothing, never panics.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, Obj, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::{self, ColorValue, ElementTable};
    /// # struct Tiny;
    /// # static TABLE: ElementTable = ElementTable::new().color(design::PRIMARY, Color::BLUE);
    /// # impl ThemeHook for Tiny {
    /// #     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    /// #     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// #     fn design(&self, _: ThemeMode) -> Option<&'static ElementTable> { Some(&TABLE) }
    /// # }
    /// # static TINY: Tiny = Tiny;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&TINY))).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// assert_eq!(e.resolve_design_value(n, ColorValue::from(design::PRIMARY)), Some(Color::BLUE));
    /// assert_eq!(e.resolve_design_value(n, ColorValue::from(Color::RED)), Some(Color::RED));
    /// assert_eq!(e.resolve_design_value(n, ColorValue::from(design::DANGER)), None);
    /// ```
    #[must_use]
    pub fn resolve_design_value<T: ElementType>(&self, id: NodeId, value: DesignValue<T>) -> Option<T> {
        match value {
            DesignValue::Fixed(v) => Some(v),
            DesignValue::Element(e) => self.node_display(id)?.elements.as_ref()?.get(e),
        }
    }

    /// The nodes of `display` its theme styles, parents first: every node of its screens and
    /// every node on its layers (not the layers themselves). Empty for an unknown display.
    /// Allocates nothing; never panics. For tools and tests that inspect a theme's coverage
    /// (`twine_testing::assert_theme_covers`).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64, None).unwrap();
    /// let screen = e.active_screen(d).unwrap();
    /// let n = e.create(screen, Box::new(Obj)).unwrap();
    /// assert_eq!(e.themed_nodes(d).collect::<Vec<_>>(), [screen, n]);
    /// ```
    #[must_use]
    pub fn themed_nodes(&self, display: DisplayId) -> ThemedNodes<'_> {
        ThemedNodes {
            engine: self,
            walk: ThemedWalk::new(display.index()),
        }
    }

    /// Themes node `id` as `class` from now on (a variant or a role, design
    /// `docs/plan/r7-theming-design.md` §2.4): its theme styles are removed and the display's
    /// theme is applied again with `class` instead of the widget's class
    /// ([`reapply_theme`](Self::reapply_theme)). A theme that does not know `class` styles the
    /// node like the nearest class of `class`'s [`lineage`](WidgetClass::lineage) it knows,
    /// so an unknown variant looks like its widget. Passing the widget's own class removes the
    /// override.
    ///
    /// `class` must be the widget's class or derive from it ([`WidgetClass::is_a`]): a theme
    /// class only changes the look, never what the node is. Otherwise the call is refused with
    /// a warning and nothing changes. Idempotent: the current theme class does nothing (no
    /// re-theming, no invalidation), so a reactive variant may set it on every change.
    ///
    /// The override lives in a side table read only when the theme is applied (creation,
    /// re-theming), bounded by [`Limits::max_theme_classes`](twine_core::Limits::max_theme_classes):
    /// a node beyond it keeps its look and the call raises
    /// [`CapacityFault::ThemeClasses`](twine_core::fault::CapacityFault::ThemeClasses). The
    /// entry is dropped when the node is deleted. Never panics; an unknown node is ignored
    /// (logged). Cost: one re-theming of the node (O(its theme styles)); call it once at
    /// creation, or when the variant changes.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig, OBJ_CLASS, Obj, WidgetClass};
    /// use twine_hal::DisplayInfo;
    ///
    /// static TILE: WidgetClass = WidgetClass::new("tile").base(&OBJ_CLASS);
    /// static OTHER: WidgetClass = WidgetClass::new("other");
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64, None).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.set_theme_class(n, &TILE);
    /// assert!(e.theme_class(n).unwrap().is(&TILE));
    /// e.set_theme_class(n, &OTHER); // not derived from `obj`: refused
    /// assert!(e.theme_class(n).unwrap().is(&TILE));
    /// e.set_theme_class(n, &OBJ_CLASS); // the widget's own class: the override goes
    /// assert!(e.theme_class(n).unwrap().is(&OBJ_CLASS));
    /// ```
    pub fn set_theme_class(&mut self, id: NodeId, class: &'static WidgetClass) {
        let Some(own) = self.tree.node(id).map(crate::Node::class) else {
            twine_core::warn!(target: "twine::style", "set_theme_class: node {} not found", fmt_node_id(id));
            return;
        };
        if !class.is_a(own) {
            twine_core::warn!(
                target: "twine::style",
                "set_theme_class: {} is not a {} (node {})",
                class.name,
                own.name,
                fmt_node_id(id)
            );
            return;
        }
        let table = &mut self.tree.theme_classes;
        let pos = table.iter().position(|(n, _)| *n == id);
        match pos {
            Some(i) if class.is(own) => {
                table.swap_remove(i);
            }
            Some(i) => match table.get_mut(i) {
                Some(entry) if entry.1.is(class) => return,
                Some(entry) => entry.1 = class,
                None => return,
            },
            None if class.is(own) => return,
            None => {
                if table.len() >= usize::from(self.limits.max_theme_classes) {
                    self.capacity_refused(twine_core::fault::CapacityFault::ThemeClasses, Some(id));
                    return;
                }
                if table.len() == table.capacity() {
                    twine_core::alloc_guard::note(
                        self.tree.partition(),
                        twine_core::alloc_guard::AllocSite::ThemeClass,
                    );
                }
                self.tree.theme_classes.push((id, class));
            }
        }
        twine_core::debug!(
            target: "twine::style",
            "node {}: theme class {}",
            fmt_node_id(id),
            class.name
        );
        self.reapply_theme(id);
    }

    /// The class node `id` is themed as: its [theme class](Self::set_theme_class) if it has
    /// one, else its widget's class; `None` for an unknown node. Never panics.
    #[must_use]
    pub fn theme_class(&self, id: NodeId) -> Option<&'static WidgetClass> {
        self.tree.theme_class(id)
    }

    /// Re-themes node `id`: removes its theme styles and applies its display's theme again
    /// with the node's [theme class](Self::theme_class), then refreshes the node once (style
    /// caches, extra draw size, layout, redraw, one `StyleChanged` event). Its own, normal and
    /// local styles are kept; its children are not re-themed (a change of inherited
    /// properties still reaches them, but a theme that styles children by their parent's
    /// class — [`ThemeCx::parent_class`] — sees the new class only for children created
    /// afterwards). What [`set_theme_class`](Self::set_theme_class) runs;
    /// also for a theme whose look depends on something it reads in
    /// [`apply`](ThemeHook::apply) that changed.
    ///
    /// A node without a themed display (no theme, or a layer itself) only loses its theme
    /// styles. Never panics; an unknown node is ignored (logged). Allocates only if the
    /// node's style list grows.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat};
    /// use twine_engine::{DisplayTheme, Engine, EngineConfig, Obj, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::{Selector, Style, style};
    ///
    /// static RED: Style = style! { bg: Color::RED };
    /// struct Red;
    /// impl ThemeHook for Red {
    ///     fn apply(&self, cx: &mut ThemeCx<'_>, _: &'static WidgetClass) {
    ///         cx.add_style(Selector::MAIN, &RED);
    ///     }
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// }
    /// static THEME: Red = Red;
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let info = DisplayInfo::new(32, 16, ColorFormat::L8);
    /// let d = e.add_chunked_display(info, 64, Some(DisplayTheme::new(&THEME))).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.reapply_theme(n);
    /// assert_eq!(e.tree().node(n).unwrap().styles().len(), 1); // replaced, not added twice
    /// ```
    pub fn reapply_theme(&mut self, id: NodeId) {
        let Some(node) = self.tree.node_mut(id) else {
            twine_core::warn!(target: "twine::style", "reapply_theme: node {} not found", fmt_node_id(id));
            return;
        };
        node.styles.remove_where(|e| e.kind == EntryKind::Theme);
        if let Some(d) = self.themed_display_of(id) {
            self.apply_theme_to(id, d);
        }
        self.refresh_style(id, twine_style::Part::Any, None);
    }

    /// The index of the display whose theme styles `id` (a node of its screens or on one of
    /// its layers, not a layer itself), if that display has a theme.
    fn themed_display_of(&self, id: NodeId) -> Option<usize> {
        let root = self.tree.root_of(id)?;
        let d = self.displays.iter().position(|d| d.owns_root(root))?;
        let disp = self.displays.get(d)?;
        let is_layer = [disp.bottom_layer, disp.top_layer, disp.sys_layer].contains(&id);
        (disp.theme.is_some() && !is_layer).then_some(d)
    }

    /// Attaches `theme` to the display being created (`Display` fields, before its first
    /// screen is themed): the start mode's table, the display's metrics table (computed once,
    /// here), and the engine's default font. Called by `push_display` only, so a display's
    /// theme is never replaced.
    pub(crate) fn attach_theme(disp: &mut crate::display::Display, theme: DisplayTheme) {
        let t = theme.theme;
        let first = t.modes().first().copied().unwrap_or(ThemeMode::Light);
        let mut mode = theme.mode.unwrap_or(first);
        let mut design = t.design(mode);
        if design.is_none() && mode != first {
            twine_core::warn!(
                target: "twine::style",
                "theme {} has no {:?} mode: display {} starts in {:?}",
                t.name(),
                mode,
                disp.id,
                first
            );
            mode = first;
            design = t.design(mode);
        }
        let area = disp.area();
        disp.metrics = t
            .metrics(disp.info.dpi, Size::new(area.width(), area.height()))
            .map(Box::new);
        // One indexed array to look up: the mode's table over the metrics, with room for every
        // listed mode, so a mode switch refills it without allocating.
        let mut flat = FlatElements::new();
        let metrics = disp.metrics.as_deref();
        for table in t.modes().iter().filter_map(|&m| t.design(m)) {
            flat.reserve(metrics, table);
        }
        flat.fill(metrics, design.unwrap_or(&EMPTY_TABLE), disp.info.dpi);
        disp.elements = Some(flat);
        disp.theme = Some(t);
        disp.theme_mode = mode;
        disp.design = design;
        twine_core::info!(
            target: "twine::style",
            "display {}: theme {}, mode {:?}",
            disp.id,
            t.name(),
            mode
        );
    }

    /// Re-resolves every themed node of display index `d` after its design element table
    /// changed: all caches at once (inherited values too), then per node what
    /// `refresh_style` does, without per-node invalidation (the display is invalidated once),
    /// then one `StyleChanged` event per node. Allocates nothing.
    fn restyle_display(&mut self, d: usize) {
        self.tree.epoch = self.tree.epoch.wrapping_add(1);
        let mut walk = ThemedWalk::new(d);
        while let Some(n) = walk.next(self) {
            if let Some(node) = self.tree.node(n) {
                node.style_cache.invalidate();
            }
            self.refresh_ext_draw(n);
            self.mark_layout(n, crate::LayoutDirty::SELF);
        }
        if let Some((display, area)) = self.displays.get(d).map(|x| (x.id, x.area())) {
            self.invalidate_area(display, area, crate::InvalidateReason::StyleChange);
        }
        let mut walk = ThemedWalk::new(d);
        while let Some(n) = walk.next(self) {
            self.send_event(n, crate::EventCode::StyleChanged, crate::EventParam::None);
        }
    }

    /// Recomputes the engine-wide default font (the default display's theme font).
    pub(crate) fn update_theme_font(&mut self) {
        self.theme_font = self
            .default_display
            .and_then(|d| self.displays.get(d.index()))
            .and_then(|d| d.theme)
            .map(ThemeHook::font_normal);
    }

    /// Applies the theme of display index `d` (if any) to `id` without refreshing anything.
    pub(crate) fn apply_theme_to(&mut self, id: NodeId, d: usize) {
        let Some(disp) = self.displays.get(d) else {
            return;
        };
        let Some(theme) = disp.theme else {
            return;
        };
        let (display, dpi) = (disp.id, disp.info.dpi);
        let area = disp.area();
        let Some(class) = self.tree.theme_class(id) else {
            return;
        };
        let mut cx = ThemeCx::new(
            &mut self.tree,
            id,
            display,
            dpi,
            Size::new(area.width(), area.height()),
        );
        theme.apply(&mut cx, class);
        twine_core::trace!(
            target: "twine::style",
            "theme {} applied to {} ({}, {} styles)",
            theme.name(),
            fmt_node_id(id),
            class.name,
            cx.added
        );
    }

    /// Applies the theme of the display of the new node `id` (called by `create` before
    /// `Widget::init`, and for every new screen).
    pub(crate) fn apply_theme_on_create(&mut self, id: NodeId) {
        let Some(root) = self.tree.root_of(id) else {
            return;
        };
        // Nodes of screens and nodes on layers (LVGL themes every object it creates).
        let Some(d) = self.displays.iter().position(|d| d.owns_root(root)) else {
            return;
        };
        if self.displays.get(d).is_none_or(|x| x.theme.is_none()) {
            return;
        }
        self.apply_theme_to(id, d);
        if let Some(n) = self.tree.node(id) {
            n.style_cache.invalidate();
        }
        self.refresh_ext_draw(id);
    }
}

#[cfg(test)]
mod tests {
    use super::dpx;

    #[test]
    fn dpx_rounds_like_lvgl() {
        assert_eq!(dpx(1, 130), 1);
        assert_eq!(dpx(10, 160), 10);
        assert_eq!(dpx(10, 320), 20);
        assert_eq!(dpx(-5, 160), -5);
        assert_eq!(dpx(0, 300), 0);
        assert_eq!(dpx(12, 130), 10);
    }
}
