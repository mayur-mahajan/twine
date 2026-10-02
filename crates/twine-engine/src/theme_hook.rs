//! Engine-side themes: the object-safe [`ThemeHook`] trait, the [`ThemeCx`] a theme styles a
//! node through, and the engine API that installs a theme on a display
//! ([`Engine::set_theme`]) and applies it to every new node (LVGL `lv_theme_apply`).
//!
//! The engine does not depend on `twine-theme`: that crate's `Theme` trait extends
//! [`ThemeHook`] with fonts and colors.
//!
//! A theme also supplies the values of the [design elements](twine_style::design) for each
//! [`ThemeMode`] it supports ([`ThemeHook::design`]). The engine keeps the table of the
//! display's current mode; [`Engine::set_theme_mode`] swaps it and re-resolves every node of the
//! display once, without re-applying the theme's styles.

use alloc::rc::Rc;

use twine_core::Size;
use twine_style::design::{DesignValue, Element, ElementRef, ElementTable, ElementType};
use twine_style::{EntryKind, Selector, StyleEntry, StyleRef, StyleValue, ThemeMode, dpx};
use twine_text::Font;

use crate::{DisplayId, Engine, NodeId, Tree, WidgetClass, fmt_node_id};

/// What the engine needs from a theme (LVGL `lv_theme_t`): styling a node when it is created,
/// the default font, and the [design element](twine_style::design) values of each
/// [`ThemeMode`] (the theme's colors: `design::PRIMARY`, `design::SECONDARY`, …; LVGL's
/// `color_primary` / `color_secondary`).
///
/// A theme adds styles with [`ThemeCx::add_style`]; they get the lowest priority of all
/// styles (after normal and local styles), exactly like LVGL theme styles. The engine calls
/// [`apply`](Self::apply) once for every node created on a display with a theme, after the
/// node is linked into the tree and **before** [`Widget::init`](crate::Widget::init), and
/// again for every node of the display when the theme is replaced.
///
/// A theme recognises classes by **identity** ([`WidgetClass::is`], a pointer comparison —
/// never by [`name`](WidgetClass::name)) and should style a class it does not know like the
/// nearest class of its [`lineage`](WidgetClass::lineage) it knows (its
/// [`base`](WidgetClass::base) chain), so custom widgets can be themed like built-in ones.
///
/// A theme also gives the [design elements](twine_style::design) their values, per
/// [`ThemeMode`] ([`design`](Self::design), [`mode`](Self::mode)); styles that use design
/// elements follow [`Engine::set_theme_mode`] without being re-applied. A theme that wraps
/// another (adds styles on top of it) must forward `mode`, `modes` and `design` to it, or the
/// wrapped theme's design elements resolve to the properties' defaults.
///
/// ```
/// use std::rc::Rc;
/// use twine_core::Color;
/// use twine_engine::{Engine, EngineConfig, OBJ_CLASS, Obj, ThemeCx, ThemeHook, WidgetClass};
/// use twine_style::{Part, PropId, Selector, StyleBuf};
/// use twine_text::Font;
///
/// struct Red(Rc<StyleBuf>);
/// impl ThemeHook for Red {
///     fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
///         // Classes are compared by identity, never by name.
///         if class.is(&OBJ_CLASS) && cx.parent().is_some() {
///             cx.add_style(Selector::MAIN, self.0.clone());
///         }
///     }
///     fn font_normal(&self) -> &'static Font {
///         &twine_text::EMPTY_FONT
///     }
/// }
///
/// # use twine_core::{ColorFormat, Rect};
/// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
/// # struct Panel(Option<DrawBufferMem>);
/// # impl DisplayDriver for Panel {
/// #     type Error = ();
/// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(32, 16, ColorFormat::Rgb565) }
/// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
/// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
/// # }
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let buf: &'static mut [u8] = Box::leak(vec![0u8; 32 * 2 * 16].into_boxed_slice());
/// let d = e.add_display(Panel(None), twine_engine::BufferMode::partial_single(buf)).unwrap();
/// e.set_theme(d, Rc::new(Red(Rc::new(StyleBuf::new().bg_color(Color::RED)))));
/// let screen = e.active_screen(d).unwrap();
/// let n = e.create(screen, Box::new(Obj)).unwrap();
/// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::RED);
/// ```
pub trait ThemeHook {
    /// Adds the theme's styles to the node of `cx`, whose widget class is `class`.
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass);

    /// The default font of text on the display (the default `Font`).
    fn font_normal(&self) -> &'static Font;

    /// A short name for logs (default `"theme"`).
    fn name(&self) -> &'static str {
        "theme"
    }

    /// The mode the theme starts in when it is installed ([`Engine::set_theme`]). Default
    /// [`ThemeMode::Light`].
    /// ```
    /// use std::rc::Rc;
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    ///
    /// /// A theme that starts dark.
    /// struct Night;
    /// impl ThemeHook for Night {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn mode(&self) -> ThemeMode { ThemeMode::Dark }
    /// }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64).unwrap();
    /// e.set_theme(d, Rc::new(Night));
    /// assert_eq!(e.theme_mode(d), ThemeMode::Dark);
    /// ```
    fn mode(&self) -> ThemeMode {
        ThemeMode::Light
    }

    /// The modes the theme supports, in the order a mode switch cycles through them
    /// ([`ThemeMode::next_in`]): exactly the modes for which [`design`](Self::design) returns
    /// a table. Default: only the theme's starting [`mode`](Self::mode).
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
    fn modes(&self) -> &'static [ThemeMode] {
        let m = self.mode();
        let i = ThemeMode::ALL.iter().position(|&x| x == m).unwrap_or(0);
        &ThemeMode::ALL[i..=i]
    }

    /// The values of the [design elements](twine_style::design) in `mode` on a display with
    /// `dpi` and logical `resolution`, or `None` if the theme does not support `mode` (the
    /// default: a theme without design elements).
    ///
    /// The engine asks when the theme is installed and on every
    /// [`Engine::set_theme_mode`]. Build each table once and return the same `Rc` afterwards
    /// (e.g. cached per mode and display class): a mode switch then allocates nothing. A table
    /// should define every standard element ([`ElementTable::missing_standard`]); an element
    /// it lacks resolves to the property's default.
    ///
    /// Parameters: `dpi` is the display's [`DisplayInfo::dpi`](twine_hal::DisplayInfo::dpi)
    /// (dots per inch; [`DEFAULT_DPI`](twine_style::DEFAULT_DPI) unless the driver says
    /// otherwise), `resolution` the display's logical size in pixels (after rotation), so a
    /// theme can pick spacings and type sizes per display class.
    ///
    /// `None` **when the theme is installed** (for its starting [`mode`](Self::mode)) is
    /// accepted: the display takes that mode without a table, so every design element resolves
    /// to its property's default (with a warning once per element kind), and
    /// [`Engine::set_theme_mode`] keeps refusing modes it returns `None` for.
    fn design(&self, mode: ThemeMode, dpi: u16, resolution: Size) -> Option<Rc<ElementTable>> {
        let _ = (mode, dpi, resolution);
        None
    }
}

impl core::fmt::Debug for dyn ThemeHook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ThemeHook({})", self.name())
    }
}

/// Access to one node for a theme's [`ThemeHook::apply`]: its tree position, its display's
/// resolution and DPI, and [`add_style`](Self::add_style).
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

    /// The widget class of the node's parent (themes style e.g. list items differently).
    #[must_use]
    pub fn parent_class(&self) -> Option<&'static WidgetClass> {
        self.parent()
            .and_then(|p| self.tree.node(p))
            .map(crate::Node::class)
    }

    /// The widget class of the node's grandparent.
    #[must_use]
    pub fn grandparent_class(&self) -> Option<&'static WidgetClass> {
        let gp = self.parent().and_then(|p| self.tree.parent(p))?;
        self.tree.node(gp).map(crate::Node::class)
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
            self.root += 1;
            if let Some(&s) = disp.screens.get(k) {
                self.scope = Some(s);
                self.cur = Some(s);
                return Some(s);
            }
            let layer = [disp.bottom_layer, disp.top_layer, disp.sys_layer]
                .get(k - disp.screens.len())
                .copied()?;
            // Continue with the layer's first child (the layer itself is skipped).
            self.scope = Some(layer);
            self.cur = Some(layer);
        }
    }
}

impl Engine {
    /// Installs `theme` on `display` (LVGL `lv_display_set_theme`): every node of the
    /// display's screens loses its theme styles and gets the new theme applied (parents before
    /// children), the display takes the theme's [`mode`](ThemeHook::mode) and its
    /// [design element](twine_style::design) table for that mode, every style cache and extra
    /// draw size is recomputed, the nodes receive `StyleChanged` and are laid out again, and
    /// the whole display is invalidated once. Layers get no theme styles (LVGL removes all
    /// styles of its layers).
    ///
    /// Never panics: an unknown `display` is ignored (a no-op, logged with `warn!`). Cost:
    /// O(nodes of the display) — every node is re-themed and re-resolved — plus whatever the
    /// theme's [`design`](ThemeHook::design) allocates for its table; meant for start-up and
    /// user-initiated theme switches, not per frame. To switch only between light and dark,
    /// [`set_theme_mode`](Self::set_theme_mode) is cheaper (no theme styles re-applied).
    pub fn set_theme(&mut self, display: DisplayId, theme: Rc<dyn ThemeHook>) {
        self.replace_theme(display, Some(theme));
    }

    /// Removes the theme of `display`: the theme styles of its nodes are removed, the default
    /// font falls back to [`EngineConfig::default_font`](crate::EngineConfig::default_font),
    /// and design elements no longer resolve (they give the properties' defaults). The
    /// display's mode becomes [`ThemeMode::Light`].
    ///
    /// Never panics: an unknown `display` is ignored (a no-op, logged with `warn!`). Cost:
    /// O(nodes of the display), like [`set_theme`](Self::set_theme).
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    ///
    /// struct Plain;
    /// impl ThemeHook for Plain {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64).unwrap();
    /// e.set_theme(d, Rc::new(Plain));
    /// assert!(e.theme(d).is_some());
    /// e.remove_theme(d);
    /// assert!(e.theme(d).is_none());
    /// ```
    pub fn remove_theme(&mut self, display: DisplayId) {
        self.replace_theme(display, None);
    }

    /// The theme of `display`.
    #[must_use]
    pub fn theme(&self, display: DisplayId) -> Option<&Rc<dyn ThemeHook>> {
        self.displays.get(display.index())?.theme.as_ref()
    }

    /// Switches the theme of `display` to `mode` (e.g. light → dark): the display takes the
    /// theme's [design element](twine_style::design) table for `mode`
    /// ([`ThemeHook::design`]), and every node of the display is re-resolved once (style
    /// caches, extra draw sizes, layout, one `StyleChanged` event each) and the display is
    /// redrawn. The theme's styles are **not** re-applied: everything styled with design
    /// elements changes, nothing else does, so no view is rebuilt and nothing is allocated
    /// (once the theme has built its table for `mode`).
    ///
    /// Idempotent: the current mode does nothing. **Fallback:** if `display` does not exist,
    /// has no theme, or its theme does not support `mode` ([`ThemeHook::design`] returns
    /// `None`, e.g. `SimpleTheme` asked for [`ThemeMode::Night`]), the display **keeps its
    /// current mode and table** — nothing changes on screen — and a warning is logged
    /// (`"twine::style"`). Never panics. [`theme_modes`](Self::theme_modes) lists the modes
    /// that will be accepted.
    ///
    /// ```
    /// # use std::rc::Rc;
    /// # use twine_core::{Color, ColorFormat, Rect, Size};
    /// # use twine_engine::{Engine, EngineConfig, Obj, ThemeCx, ThemeHook, WidgetClass};
    /// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
    /// use twine_style::design::{self, ElementTable};
    /// use twine_style::{Part, PropId, Selector, StyleProp, ThemeMode};
    ///
    /// /// A theme that only supplies design elements, in two modes.
    /// struct Tiny { light: Rc<ElementTable>, dark: Rc<ElementTable> }
    /// impl ThemeHook for Tiny {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn design(&self, mode: ThemeMode, _: u16, _: Size) -> Option<Rc<ElementTable>> {
    ///         Some(if mode == ThemeMode::Dark { self.dark.clone() } else { self.light.clone() })
    ///     }
    /// }
    /// # struct Panel(Option<DrawBufferMem>);
    /// # impl DisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(32, 16, ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
    /// # }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 32 * 2 * 16].into_boxed_slice());
    /// # let d = e.add_display(Panel(None), twine_engine::BufferMode::partial_single(buf)).unwrap();
    /// e.set_theme(d, Rc::new(Tiny {
    ///     light: Rc::new(ElementTable::new().with(design::SURFACE, Color::WHITE)),
    ///     dark: Rc::new(ElementTable::new().with(design::SURFACE, Color::BLACK)),
    /// }));
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(design::SURFACE.into()));
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::WHITE);
    /// e.set_theme_mode(d, ThemeMode::Dark);
    /// assert_eq!(e.theme_mode(d), ThemeMode::Dark);
    /// assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::BLACK);
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
        let Some(theme) = disp.theme.as_ref() else {
            twine_core::warn!(target: "twine::style", "set_theme_mode: display {} has no theme", display);
            return;
        };
        let area = disp.area();
        let Some(table) = theme.design(mode, disp.info.dpi, Size::new(area.width(), area.height())) else {
            twine_core::warn!(
                target: "twine::style",
                "set_theme_mode: theme {} has no {:?} mode",
                theme.name(),
                mode
            );
            return;
        };
        let disp = &mut self.displays[d];
        disp.theme_mode = mode;
        disp.design = Some(table);
        disp.design_epoch = disp.design_epoch.wrapping_add(1);
        twine_core::info!(target: "twine::style", "display {}: theme mode {:?}", display, mode);
        self.restyle_display(d);
    }

    /// The modes the theme of `display` supports ([`ThemeHook::modes`]; empty for a display
    /// without a theme or an unknown display). Cycle through them with
    /// [`ThemeMode::next_in`]:
    ///
    /// ```
    /// # use std::rc::Rc;
    /// # use twine_core::{ColorFormat, Rect, Size};
    /// # use twine_engine::{Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
    /// use twine_style::ThemeMode;
    /// use twine_style::design::ElementTable;
    ///
    /// /// A theme with a light and a night table.
    /// struct DayNight(Rc<ElementTable>);
    /// impl ThemeHook for DayNight {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn modes(&self) -> &'static [ThemeMode] { &[ThemeMode::Light, ThemeMode::Night] }
    ///     fn design(&self, mode: ThemeMode, _: u16, _: Size) -> Option<Rc<ElementTable>> {
    ///         self.modes().contains(&mode).then(|| self.0.clone())
    ///     }
    /// }
    /// # struct Panel(Option<DrawBufferMem>);
    /// # impl DisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(32, 16, ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
    /// # }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 32 * 2 * 16].into_boxed_slice());
    /// # let d = e.add_display(Panel(None), twine_engine::BufferMode::partial_single(buf)).unwrap();
    /// assert!(e.theme_modes(d).is_empty()); // no theme
    /// e.set_theme(d, Rc::new(DayNight(Rc::new(ElementTable::new()))));
    /// let next = e.theme_mode(d).next_in(e.theme_modes(d));
    /// e.set_theme_mode(d, next);
    /// assert_eq!(e.theme_mode(d), ThemeMode::Night);
    /// e.set_theme_mode(d, ThemeMode::Dark); // unsupported: kept Night (and warned)
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
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64).unwrap();
    /// assert_eq!(e.theme_mode(d), ThemeMode::Light); // no theme yet
    /// ```
    #[must_use]
    pub fn theme_mode(&self, display: DisplayId) -> ThemeMode {
        self.displays
            .get(display.index())
            .map_or(ThemeMode::Light, |d| d.theme_mode)
    }

    /// A number that changes whenever the theme, the theme mode or the design element table of
    /// `display` changes ([`set_theme`](Self::set_theme), [`remove_theme`](Self::remove_theme),
    /// [`set_theme_mode`](Self::set_theme_mode)): lets a cache of design element values (e.g.
    /// a reactive layer) notice that it is stale. 0 for an unknown display.
    ///
    /// Compare for **inequality** only: the counter wraps around at `u32::MAX` (after 2³²
    /// changes it repeats a value, which a cache refreshed at least once in that span never
    /// observes). Never panics; O(1).
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    ///
    /// struct Plain;
    /// impl ThemeHook for Plain {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64).unwrap();
    /// let seen = e.design_epoch(d);
    /// e.set_theme(d, Rc::new(Plain));
    /// assert_ne!(e.design_epoch(d), seen); // a cached design value is stale
    /// ```
    #[must_use]
    pub fn design_epoch(&self, display: DisplayId) -> u32 {
        self.displays.get(display.index()).map_or(0, |d| d.design_epoch)
    }

    /// The [design element](twine_style::design) table of `display` (its theme's table for
    /// its current mode; `None` without a theme, for a theme without design elements, or for
    /// an unknown display). Never panics; O(1).
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_core::{Color, ColorFormat, Size};
    /// use twine_engine::{Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::ThemeMode;
    /// use twine_style::design::{self, ElementTable};
    ///
    /// struct Brand(Rc<ElementTable>);
    /// impl ThemeHook for Brand {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn design(&self, _: ThemeMode, _: u16, _: Size) -> Option<Rc<ElementTable>> { Some(self.0.clone()) }
    /// }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::L8), 64).unwrap();
    /// assert!(e.design_table(d).is_none()); // no theme
    /// e.set_theme(d, Rc::new(Brand(Rc::new(ElementTable::new().with(design::PRIMARY, Color::RED)))));
    /// assert_eq!(e.design_table(d).and_then(|t| t.get(design::PRIMARY)), Some(Color::RED));
    /// ```
    #[must_use]
    pub fn design_table(&self, display: DisplayId) -> Option<&ElementTable> {
        self.displays.get(display.index())?.design.as_deref()
    }

    /// The value `display`'s theme gives `element` in its current mode (`None` if it does not
    /// define it).
    ///
    /// ```
    /// # use std::rc::Rc;
    /// # use twine_core::{Color, ColorFormat, Rect, Size};
    /// # use twine_engine::{Engine, EngineConfig, ThemeCx, ThemeHook, WidgetClass};
    /// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
    /// use twine_style::design::{self, ElementTable};
    /// use twine_style::ThemeMode;
    ///
    /// struct Brand(Rc<ElementTable>);
    /// impl ThemeHook for Brand {
    ///     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    ///     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    ///     fn design(&self, _: ThemeMode, _: u16, _: Size) -> Option<Rc<ElementTable>> { Some(self.0.clone()) }
    /// }
    /// # struct Panel(Option<DrawBufferMem>);
    /// # impl DisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(32, 16, ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
    /// # }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 32 * 2 * 16].into_boxed_slice());
    /// # let d = e.add_display(Panel(None), twine_engine::BufferMode::partial_single(buf)).unwrap();
    /// assert_eq!(e.design_value(d, design::PRIMARY), None); // no theme yet
    /// e.set_theme(d, Rc::new(Brand(Rc::new(ElementTable::new().with(design::PRIMARY, Color::RED)))));
    /// assert_eq!(e.design_value(d, design::PRIMARY), Some(Color::RED));
    /// ```
    #[must_use]
    pub fn design_value<T: ElementType>(&self, display: DisplayId, element: Element<T>) -> Option<T> {
        self.design_table(display)?.get(element)
    }

    /// The design element table that styles of `id` resolve with: the table of the display
    /// showing `id`, else of the default display. With one display this is a direct read.
    #[must_use]
    pub(crate) fn node_design(&self, id: NodeId) -> Option<&ElementTable> {
        if let [d] = self.displays.as_slice() {
            return d.design.as_deref();
        }
        self.display_of(id)
            .or(self.default_display)
            .and_then(|d| self.displays.get(d.index()))
            .and_then(|d| d.design.as_deref())
    }

    /// The value of `element` for `id` (the [`StyleSource::design_value`] of the engine).
    ///
    /// [`StyleSource::design_value`]: twine_style::StyleSource::design_value
    #[inline]
    pub(crate) fn node_design_value(&self, id: NodeId, element: ElementRef) -> Option<StyleValue> {
        self.node_design(id)?.value(element)
    }

    /// The default font of `display`: its theme's [`ThemeHook::font_normal`], else
    /// [`EngineConfig::default_font`](crate::EngineConfig::default_font), else
    /// [`twine_text::EMPTY_FONT`].
    #[must_use]
    pub fn default_font(&self, display: DisplayId) -> &'static Font {
        self.displays
            .get(display.index())
            .and_then(|d| d.theme.as_ref())
            .map(|t| t.font_normal())
            .or(self.config.default_font)
            .unwrap_or(&twine_text::EMPTY_FONT)
    }

    /// The theme of the display showing `id` (on a screen or a layer), if any.
    #[must_use]
    pub fn theme_of(&self, id: NodeId) -> Option<&Rc<dyn ThemeHook>> {
        let root = self.tree.root_of(id)?;
        self.displays.iter().find(|d| d.owns_root(root))?.theme.as_ref()
    }

    /// The value of a [design value](twine_style::design::DesignValue) for node `id`: a fixed
    /// value as is, an element looked up in the design element table of the display showing
    /// `id` (its theme's table for the current mode; with one display a direct read). `None`
    /// when the element is not defined there (no theme, or a theme without it): the caller
    /// picks its fallback.
    ///
    /// For widgets that keep a color (or length, …) of their own outside the style system,
    /// like an LED's color: resolve it when the widget is created and again on
    /// [`EventCode::StyleChanged`](crate::EventCode::StyleChanged) (sent to every node when the
    /// theme or its mode changes), so it follows the theme like a style property. One table
    /// read; allocates nothing, never panics.
    ///
    /// ```
    /// # use std::rc::Rc;
    /// # use twine_core::{Color, ColorFormat, Rect, Size};
    /// # use twine_engine::{Engine, EngineConfig, Obj, ThemeCx, ThemeHook, WidgetClass};
    /// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
    /// use twine_style::design::{self, ColorValue, ElementTable};
    /// # struct Tiny(Rc<ElementTable>);
    /// # impl ThemeHook for Tiny {
    /// #     fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    /// #     fn font_normal(&self) -> &'static twine_text::Font { &twine_text::EMPTY_FONT }
    /// #     fn design(&self, _: twine_style::ThemeMode, _: u16, _: Size) -> Option<Rc<ElementTable>> { Some(self.0.clone()) }
    /// # }
    /// # struct Panel(Option<DrawBufferMem>);
    /// # impl DisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(32, 16, ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
    /// # }
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 32 * 2 * 16].into_boxed_slice());
    /// # let d = e.add_display(Panel(None), twine_engine::BufferMode::partial_single(buf)).unwrap();
    /// e.set_theme(d, Rc::new(Tiny(Rc::new(ElementTable::new().with(design::PRIMARY, Color::BLUE)))));
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// assert_eq!(e.resolve_design_value(n, ColorValue::from(design::PRIMARY)), Some(Color::BLUE));
    /// assert_eq!(e.resolve_design_value(n, ColorValue::from(Color::RED)), Some(Color::RED));
    /// assert_eq!(e.resolve_design_value(n, ColorValue::from(design::DANGER)), None);
    /// ```
    #[must_use]
    pub fn resolve_design_value<T: ElementType>(&self, id: NodeId, value: DesignValue<T>) -> Option<T> {
        match value {
            DesignValue::Fixed(v) => Some(v),
            DesignValue::Element(e) => self.node_design(id)?.get(e),
        }
    }

    fn replace_theme(&mut self, display: DisplayId, theme: Option<Rc<dyn ThemeHook>>) {
        let d = display.index();
        let Some(disp) = self.displays.get_mut(d) else {
            twine_core::warn!(target: "twine::style", "set_theme: display {} not found", display);
            return;
        };
        let name = theme.as_ref().map_or("none", |t| t.name());
        let area = disp.area();
        let (mode, design) = match &theme {
            Some(t) => {
                let mode = t.mode();
                (
                    mode,
                    t.design(mode, disp.info.dpi, Size::new(area.width(), area.height())),
                )
            }
            None => (ThemeMode::Light, None),
        };
        disp.theme = theme;
        disp.theme_mode = mode;
        disp.design = design;
        disp.design_epoch = disp.design_epoch.wrapping_add(1);
        twine_core::info!(target: "twine::style", "theme set on display {}", display);
        self.update_theme_font();
        // Every node of every screen and every node on a layer (the layers themselves keep no
        // theme styles), parents first.
        let mut walk = ThemedWalk::new(d);
        let mut count = 0u32;
        while let Some(n) = walk.next(self) {
            if let Some(node) = self.tree.node_mut(n) {
                node.styles.remove_where(|e| e.kind == EntryKind::Theme);
            }
            self.apply_theme_to(n, d);
            count += 1;
        }
        self.restyle_display(d);
        twine_core::debug!(target: "twine::style", "theme {} applied to {} nodes", name, count);
    }

    /// Re-resolves every themed node of display index `d` after its theme styles or its design
    /// element table changed: all caches at once (inherited values too), then per node what
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
        let (display, area) = (self.displays[d].id, self.displays[d].area());
        self.invalidate_area(display, area, crate::InvalidateReason::StyleChange);
        let mut walk = ThemedWalk::new(d);
        while let Some(n) = walk.next(self) {
            self.send_event(n, crate::EventCode::StyleChanged, crate::EventParam::None);
        }
    }

    /// Recomputes the engine-wide default font (the default display's theme font).
    fn update_theme_font(&mut self) {
        self.theme_font = self
            .default_display
            .and_then(|d| self.displays.get(d.index()))
            .and_then(|d| d.theme.as_ref())
            .map(|t| t.font_normal());
    }

    /// Applies the theme of display index `d` (if any) to `id` without refreshing anything.
    pub(crate) fn apply_theme_to(&mut self, id: NodeId, d: usize) {
        let Some(disp) = self.displays.get(d) else {
            return;
        };
        let Some(theme) = disp.theme.clone() else {
            return;
        };
        let (display, dpi) = (disp.id, disp.info.dpi);
        let area = disp.area();
        let Some(class) = self.tree.node(id).map(crate::Node::class) else {
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

    /// Applies the theme of the display of the new node `id` (called by `create`, before
    /// `Widget::init`).
    pub(crate) fn apply_theme_on_create(&mut self, id: NodeId) {
        let Some(root) = self.tree.root_of(id) else {
            return;
        };
        // Nodes of screens and nodes on layers (LVGL themes every object it creates).
        let Some(d) = self.displays.iter().position(|d| d.owns_root(root)) else {
            return;
        };
        if self.displays[d].theme.is_none() {
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
