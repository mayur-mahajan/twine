//! Engine-side themes: the object-safe [`ThemeHook`] trait, the [`ThemeCx`] a theme styles a
//! node through, and the engine API that installs a theme on a display
//! ([`Engine::set_theme`]) and applies it to every new node (LVGL `lv_theme_apply`).
//!
//! The engine does not depend on `twine-theme`: that crate's `Theme` trait extends
//! [`ThemeHook`] with fonts and colors.

use alloc::rc::Rc;

use twine_core::{Color, Size};
use twine_style::{EntryKind, Selector, StyleEntry, StyleRef};
use twine_text::Font;

use crate::{DisplayId, Engine, NodeId, Tree, WidgetClass, fmt_node_id};

/// LVGL's default primary color (`lv_palette_main(LV_PALETTE_BLUE)`), used without a theme.
pub const DEFAULT_COLOR_PRIMARY: Color = Color::hex(0x0021_96F3);
/// LVGL's default secondary color (`lv_palette_main(LV_PALETTE_RED)`), used without a theme.
pub const DEFAULT_COLOR_SECONDARY: Color = Color::hex(0x00F4_4336);

/// What the engine needs from a theme (LVGL `lv_theme_t`): styling a node when it is created,
/// the default font and the primary / secondary colors widgets take (an LED's default color).
///
/// A theme adds styles with [`ThemeCx::add_style`]; they get the lowest priority of all
/// styles (after normal and local styles), exactly like LVGL theme styles. The engine calls
/// [`apply`](Self::apply) once for every node created on a display with a theme, after the
/// node is linked into the tree and **before** [`Widget::init`](crate::Widget::init), and
/// again for every node of the display when the theme is replaced.
///
/// ```
/// use std::rc::Rc;
/// use twine_core::Color;
/// use twine_engine::{Engine, EngineConfig, Obj, ThemeCx, ThemeHook, WidgetClass};
/// use twine_style::{Part, PropId, Selector, StyleBuf};
/// use twine_text::Font;
///
/// struct Red(Rc<StyleBuf>);
/// impl ThemeHook for Red {
///     fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
///         if class.name == "obj" && cx.parent().is_some() {
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

    /// The default font of text on the display (the default `TextFont`).
    fn font_normal(&self) -> &'static Font;

    /// A short name for logs (default `"theme"`).
    fn name(&self) -> &'static str {
        "theme"
    }

    /// The primary color (buttons, sliders, focus outlines; LVGL `color_primary`). Default
    /// [`DEFAULT_COLOR_PRIMARY`].
    fn color_primary(&self) -> Color {
        DEFAULT_COLOR_PRIMARY
    }

    /// The secondary color (checked buttons, edit outlines; LVGL `color_secondary`). Default
    /// [`DEFAULT_COLOR_SECONDARY`].
    fn color_secondary(&self) -> Color {
        DEFAULT_COLOR_SECONDARY
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

/// `px` pixels at 160 DPI scaled to `dpi` (LVGL `LV_DPX_CALC`): 0 stays 0, positive values
/// are at least 1, negative values are scaled symmetrically.
#[must_use]
pub(crate) fn dpx(px: i32, dpi: u16) -> i32 {
    match px {
        0 => 0,
        p if p > 0 => ((i32::from(dpi) * p + 80) / 160).max(1),
        p => -dpx(-p, dpi),
    }
}

impl Engine {
    /// Installs `theme` on `display` (LVGL `lv_display_set_theme`): every node of the
    /// display's screens loses its theme styles and gets the new theme applied (parents before
    /// children), every style cache and extra draw size is recomputed, the nodes receive
    /// `StyleChanged` and are laid out again, and the whole display is invalidated once.
    /// Layers get no theme styles (LVGL removes all styles of its layers).
    pub fn set_theme(&mut self, display: DisplayId, theme: Rc<dyn ThemeHook>) {
        self.replace_theme(display, Some(theme));
    }

    /// Removes the theme of `display`: the theme styles of its nodes are removed and the
    /// default font falls back to [`EngineConfig::default_font`](crate::EngineConfig::default_font).
    pub fn remove_theme(&mut self, display: DisplayId) {
        self.replace_theme(display, None);
    }

    /// The theme of `display`.
    #[must_use]
    pub fn theme(&self, display: DisplayId) -> Option<&Rc<dyn ThemeHook>> {
        self.displays.get(display.index())?.theme.as_ref()
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

    /// The primary color of the theme of `id`'s display (LVGL `lv_theme_get_color_primary`):
    /// [`DEFAULT_COLOR_PRIMARY`] without a theme.
    #[must_use]
    pub fn color_primary(&self, id: NodeId) -> Color {
        self.theme_of(id)
            .map_or(DEFAULT_COLOR_PRIMARY, |t| t.color_primary())
    }

    /// The secondary color of the theme of `id`'s display (LVGL
    /// `lv_theme_get_color_secondary`): [`DEFAULT_COLOR_SECONDARY`] without a theme.
    #[must_use]
    pub fn color_secondary(&self, id: NodeId) -> Color {
        self.theme_of(id)
            .map_or(DEFAULT_COLOR_SECONDARY, |t| t.color_secondary())
    }

    fn replace_theme(&mut self, display: DisplayId, theme: Option<Rc<dyn ThemeHook>>) {
        let d = display.index();
        if d >= self.displays.len() {
            twine_core::warn!(target: "twine::style", "set_theme: display {} not found", display);
            return;
        }
        let name = theme.as_ref().map_or("none", |t| t.name());
        self.displays[d].theme = theme;
        twine_core::info!(target: "twine::style", "theme set on display {}", display);
        self.update_theme_font();
        // Every node of every screen and every node on a layer (the layers themselves keep no
        // theme styles), parents first.
        let screens = self.themed_roots(d);
        let mut count = 0u32;
        for s in screens {
            let mut cur = Some(s);
            while let Some(n) = cur {
                if let Some(node) = self.tree.node_mut(n) {
                    node.styles.remove_where(|e| e.kind == EntryKind::Theme);
                }
                self.apply_theme_to(n, d);
                count += 1;
                cur = self.next_in_subtree(s, n);
            }
        }
        // All caches at once (inherited values too), then per node what `refresh_style`
        // does, without per-node invalidation: the display is invalidated once below.
        self.tree.epoch = self.tree.epoch.wrapping_add(1);
        let screens = self.themed_roots(d);
        for s in screens {
            let mut cur = Some(s);
            while let Some(n) = cur {
                if let Some(node) = self.tree.node(n) {
                    node.style_cache.invalidate();
                }
                self.refresh_ext_draw(n);
                self.mark_layout(n, crate::LayoutDirty::SELF);
                cur = self.next_in_subtree(s, n);
            }
        }
        let area = self.displays[d].area();
        self.invalidate_area(display, area, crate::InvalidateReason::StyleChange);
        let screens = self.themed_roots(d);
        for s in screens {
            let mut cur = Some(s);
            while let Some(n) = cur {
                self.send_event(n, crate::EventCode::StyleChanged, crate::EventParam::None);
                cur = if self.tree.contains(n) {
                    self.next_in_subtree(s, n)
                } else {
                    None
                };
            }
        }
        twine_core::debug!(target: "twine::style", "theme {} applied to {} nodes", name, count);
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
    /// The roots of the themed subtrees of display `d`: its screens and the children of its
    /// layers (layer roots themselves get no theme styles).
    fn themed_roots(&self, d: usize) -> alloc::vec::Vec<NodeId> {
        let disp = &self.displays[d];
        let mut roots = disp.screens.clone();
        for layer in [disp.bottom_layer, disp.top_layer, disp.sys_layer] {
            roots.extend(self.tree.children(layer));
        }
        roots
    }

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
