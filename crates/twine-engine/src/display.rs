//! Displays: [`BufferMode`], the object-safe driver wrappers, screens and layers.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

use twine_core::{ColorFormat, Rect, Rotation};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, FramebufferDisplay};

use crate::refresh::Refresher;
use crate::{
    DisplayId, Engine, EngineError, EventCode, EventParam, InvalidateReason, NodeId, Obj, ObjFlags, Widget,
    fmt_node_id,
};

/// Maximum number of displays per engine.
pub const MAX_DISPLAYS: usize = 4;

/// The draw buffers of a display (LVGL's render modes).
///
/// | Mode | Driver | Behaviour |
/// |------|--------|-----------|
/// | `Partial { a, b: None }` | [`DisplayDriver`] | render a chunk, flush, wait, repeat |
/// | `Partial { a, b: Some(_) }` | [`DisplayDriver`] | ping-pong: render into one buffer while the other is flushed (DMA) |
/// | `Full` | [`FramebufferDisplay`] with two framebuffers | render dirty areas into the back buffer, present, sync areas |
/// | `Direct` | [`FramebufferDisplay`] | render dirty areas in place |
///
/// Partial buffers hold whole rows (`len` a multiple of `width × bytes per pixel`, at least
/// one row) and start 4-byte aligned.
///
/// ```
/// use twine_engine::BufferMode;
/// let a: &'static mut [u8] = Box::leak(vec![0u8; 320 * 2 * 40].into_boxed_slice());
/// let b: &'static mut [u8] = Box::leak(vec![0u8; 320 * 2 * 40].into_boxed_slice());
/// let mode = BufferMode::partial_double(a, b);
/// assert!(matches!(mode, BufferMode::Partial { b: Some(_), .. }));
/// ```
#[derive(Debug)]
pub enum BufferMode {
    /// One or two partial draw buffers.
    Partial {
        /// The first buffer.
        a: DrawBufferMem,
        /// The optional second buffer (DMA ping-pong).
        b: Option<DrawBufferMem>,
    },
    /// Two full framebuffers owned by a [`FramebufferDisplay`].
    Full,
    /// One full framebuffer owned by a [`FramebufferDisplay`], rendered in place.
    Direct,
}

impl BufferMode {
    /// One partial buffer.
    #[must_use]
    pub fn partial_single(buf: &'static mut [u8]) -> Self {
        BufferMode::Partial {
            a: DrawBufferMem::new(buf),
            b: None,
        }
    }

    /// Two partial buffers (rendering overlaps flushing).
    #[must_use]
    pub fn partial_double(a: &'static mut [u8], b: &'static mut [u8]) -> Self {
        BufferMode::Partial {
            a: DrawBufferMem::new(a),
            b: Some(DrawBufferMem::new(b)),
        }
    }

    /// Double framebuffer with area sync (framebuffer displays).
    #[must_use]
    pub const fn full() -> Self {
        BufferMode::Full
    }

    /// Single framebuffer rendered in place (framebuffer displays).
    #[must_use]
    pub const fn direct() -> Self {
        BufferMode::Direct
    }
}

/// Object-safe view of a [`DisplayDriver`] (errors mapped to [`EngineError::Driver`]).
pub(crate) trait FlushBackend {
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), EngineError>;
    fn poll_flush(&mut self) -> Option<DrawBufferMem>;
    fn wait_vsync(&mut self);
    fn idle(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: DisplayDriver + 'static> FlushBackend for D {
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), EngineError> {
        DisplayDriver::begin_flush(self, area, buf).map_err(|e| {
            let err = EngineError::driver(&e);
            if let EngineError::Driver(msg) = &err {
                twine_core::error!(target: "twine::driver", "flush {} failed: {}", area, msg.as_str());
            }
            err
        })
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        DisplayDriver::poll_flush(self)
    }
    fn wait_vsync(&mut self) {
        DisplayDriver::wait_vsync(self);
    }
    fn idle(&mut self) {
        DisplayDriver::idle(self);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Object-safe view of a [`FramebufferDisplay`].
pub(crate) trait FbBackend {
    fn present(&mut self, index: u8) -> Result<(), EngineError>;
    fn present_done(&mut self) -> bool;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: FramebufferDisplay + 'static> FbBackend for D {
    fn present(&mut self, index: u8) -> Result<(), EngineError> {
        FramebufferDisplay::present(self, index).map_err(|e| {
            let err = EngineError::driver(&e);
            if let EngineError::Driver(msg) = &err {
                twine_core::error!(target: "twine::driver", "present {} failed: {}", index, msg.as_str());
            }
            err
        })
    }
    fn present_done(&mut self) -> bool {
        FramebufferDisplay::present_done(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The driver of a display.
pub(crate) enum Backend {
    Flush(Box<dyn FlushBackend>),
    Framebuffer(Box<dyn FbBackend>),
}

impl Backend {
    fn as_any(&self) -> &dyn Any {
        match self {
            Backend::Flush(b) => b.as_any(),
            Backend::Framebuffer(b) => b.as_any(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        match self {
            Backend::Flush(b) => b.as_any_mut(),
            Backend::Framebuffer(b) => b.as_any_mut(),
        }
    }
}

/// One display: driver, refresher, layers and screens.
pub(crate) struct Display {
    pub(crate) id: DisplayId,
    pub(crate) backend: Backend,
    pub(crate) info: DisplayInfo,
    pub(crate) refresher: Refresher,
    pub(crate) bottom_layer: NodeId,
    pub(crate) top_layer: NodeId,
    pub(crate) sys_layer: NodeId,
    pub(crate) active_screen: NodeId,
    pub(crate) prev_screen: Option<NodeId>,
    pub(crate) screens: Vec<NodeId>,
    /// A screen load animation in progress (LVGL `scr_to_load`).
    pub(crate) screen_load: Option<crate::screen_anim::ScreenLoadRun>,
    /// The previous screen is drawn above the active one (LVGL `draw_prev_over_act`).
    pub(crate) draw_prev_over_act: bool,
    #[cfg(feature = "perf-monitor")]
    pub(crate) perf_overlay: Option<NodeId>,
    /// The display's theme (LVGL `lv_display_set_theme`).
    pub(crate) theme: Option<alloc::rc::Rc<dyn crate::ThemeHook>>,
}

impl Display {
    /// Whether `root` is currently shown (a layer, the active or the previous screen).
    pub(crate) fn shows_root(&self, root: NodeId) -> bool {
        root == self.active_screen
            || root == self.bottom_layer
            || root == self.top_layer
            || root == self.sys_layer
            || self.prev_screen == Some(root)
    }

    /// Whether `root` belongs to this display (a layer or a screen).
    pub(crate) fn owns_root(&self, root: NodeId) -> bool {
        root == self.bottom_layer
            || root == self.top_layer
            || root == self.sys_layer
            || self.screens.contains(&root)
    }

    /// The logical screen area.
    pub(crate) fn area(&self) -> Rect {
        self.info.area()
    }
}

impl Engine {
    /// Registers a display with an embedded frame memory (SPI/i80 panels). `buffers` must be
    /// [`BufferMode::Partial`]. Creates the display's bottom layer, first screen, top layer and
    /// system layer, and schedules a full redraw. The first display becomes the default one.
    pub fn add_display(
        &mut self,
        driver: impl DisplayDriver + 'static,
        buffers: BufferMode,
    ) -> Result<DisplayId, EngineError> {
        let BufferMode::Partial { a, b } = buffers else {
            return Err(EngineError::BufferModeMismatch);
        };
        if self.displays.len() >= MAX_DISPLAYS {
            return Err(EngineError::TooManyDisplays);
        }
        let info = DisplayDriver::info(&driver);
        let refresher = Refresher::new_partial(&info, a, b)?;
        self.push_display(Backend::Flush(Box::new(driver)), info, refresher)
    }

    /// Registers a memory-mapped display (LTDC, RGB, Linux fb). `buffers` must be
    /// [`BufferMode::Full`] (the driver must hand out two framebuffers) or
    /// [`BufferMode::Direct`]. Each framebuffer must hold exactly `width × height` pixels.
    /// Software rotation is not available in these modes.
    #[allow(clippy::needless_pass_by_value)] // symmetric with `add_display`, which consumes its buffers
    pub fn add_framebuffer_display(
        &mut self,
        mut driver: impl FramebufferDisplay + 'static,
        buffers: BufferMode,
    ) -> Result<DisplayId, EngineError> {
        let full = match buffers {
            BufferMode::Full => true,
            BufferMode::Direct => false,
            BufferMode::Partial { .. } => return Err(EngineError::BufferModeMismatch),
        };
        if self.displays.len() >= MAX_DISPLAYS {
            return Err(EngineError::TooManyDisplays);
        }
        let info = FramebufferDisplay::info(&driver);
        if info.rotation != Rotation::Deg0 && !info.hw_rotation {
            return Err(EngineError::InvalidConfig(
                "software rotation needs partial buffers",
            ));
        }
        if info.format == ColorFormat::I1 && info.width % 8 != 0 {
            return Err(EngineError::InvalidConfig(
                "I1 framebuffer width must be a multiple of 8",
            ));
        }
        let fbs = FramebufferDisplay::framebuffers(&mut driver).ok_or(EngineError::BufferModeMismatch)?;
        let refresher = Refresher::new_framebuffer(&info, fbs, full)?;
        self.push_display(Backend::Framebuffer(Box::new(driver)), info, refresher)
    }

    fn push_display(
        &mut self,
        backend: Backend,
        info: DisplayInfo,
        refresher: Refresher,
    ) -> Result<DisplayId, EngineError> {
        let id = DisplayId(self.displays.len() as u8);
        let area = info.area();
        let layer = |engine: &mut Engine, clickable: bool| -> Result<NodeId, EngineError> {
            let n = engine.tree.create(None, Box::new(Obj))?;
            if let Some(node) = engine.tree.node_mut(n) {
                node.coords = area;
                if !clickable {
                    node.flags.remove(ObjFlags::CLICKABLE);
                }
            }
            Ok(n)
        };
        let bottom_layer = layer(self, false)?;
        let active_screen = layer(self, true)?;
        let top_layer = layer(self, false)?;
        let sys_layer = layer(self, false)?;
        self.displays.push(Display {
            id,
            backend,
            info,
            refresher,
            bottom_layer,
            top_layer,
            sys_layer,
            active_screen,
            prev_screen: None,
            screens: alloc::vec![active_screen],
            screen_load: None,
            draw_prev_over_act: false,
            #[cfg(feature = "perf-monitor")]
            perf_overlay: None,
            theme: None,
        });
        if self.default_display.is_none() {
            self.default_display = Some(id);
        }
        twine_core::info!(
            target: "twine::engine",
            "display {} added: {}x{} {} rotation {:?}{}",
            id,
            info.width,
            info.height,
            info.format,
            info.rotation,
            if info.hw_rotation { " (hw)" } else { "" }
        );
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        Ok(id)
    }

    fn display(&self, d: DisplayId) -> Option<&Display> {
        self.displays.get(d.index())
    }

    /// The first registered display.
    #[must_use]
    pub fn default_display(&self) -> Option<DisplayId> {
        self.default_display
    }

    /// Every registered display.
    pub fn displays(&self) -> impl Iterator<Item = DisplayId> + '_ {
        self.displays.iter().map(|d| d.id)
    }

    /// The description of `display`.
    #[must_use]
    pub fn display_info(&self, display: DisplayId) -> Option<DisplayInfo> {
        self.display(display).map(|d| d.info)
    }

    /// The driver of `display` as `D` (e.g. to read a test display's pixels).
    #[must_use]
    pub fn driver<D: 'static>(&self, display: DisplayId) -> Option<&D> {
        self.display(display)?.backend.as_any().downcast_ref::<D>()
    }

    /// The driver of `display` as `&mut D`.
    pub fn driver_mut<D: 'static>(&mut self, display: DisplayId) -> Option<&mut D> {
        self.displays
            .get_mut(display.index())?
            .backend
            .as_any_mut()
            .downcast_mut::<D>()
    }

    /// Creates a new (inactive) screen on `display`: a root [`Obj`] covering the display.
    pub fn create_screen(&mut self, display: DisplayId) -> Result<NodeId, EngineError> {
        let area = self
            .display(display)
            .ok_or(EngineError::DisplayNotFound(display))?
            .area();
        let s = self.tree.create(None, Box::new(Obj))?;
        if let Some(n) = self.tree.node_mut(s) {
            n.coords = area;
        }
        self.displays[display.index()].screens.push(s);
        self.apply_theme_on_create(s);
        Ok(s)
    }

    /// Makes `screen` the active screen of its display (instantly) and redraws the display
    /// (`ScreenUnloadStart`, `ScreenLoadStart`, `ScreenLoaded`, `ScreenUnloaded`). A screen load
    /// animation in progress is finished first. Same as
    /// [`load_screen_anim`](Self::load_screen_anim) with [`ScreenAnim::None`](crate::ScreenAnim::None).
    pub fn load_screen(&mut self, screen: NodeId) {
        self.load_screen_anim(screen, crate::ScreenAnim::None);
    }

    /// The instant screen switch of display `d` (LVGL `load_new_screen`).
    pub(crate) fn load_screen_now(&mut self, d: usize, screen: NodeId) {
        let old = self.displays[d].active_screen;
        if old == screen {
            return;
        }
        self.send_event(old, EventCode::ScreenUnloadStart, EventParam::None);
        self.send_event(screen, EventCode::ScreenLoadStart, EventParam::None);
        // Handlers may have loaded another screen or deleted this one.
        let Some(d) = self.displays.iter().position(|x| x.screens.contains(&screen)) else {
            return;
        };
        self.displays[d].active_screen = screen;
        let (id, area) = (self.displays[d].id, self.displays[d].area());
        twine_core::info!(target: "twine::engine", "display {}: screen {} loaded", id, fmt_node_id(screen));
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        // A press on the old screen must not continue on the new one.
        self.input_reset(None, None);
        self.send_event(screen, EventCode::ScreenLoaded, EventParam::None);
        if self.tree.contains(old) {
            self.send_event(old, EventCode::ScreenUnloaded, EventParam::None);
        }
    }

    /// The active screen of `display`.
    #[must_use]
    pub fn active_screen(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.active_screen)
    }

    /// The screens of `display`.
    #[must_use]
    pub fn screens(&self, display: DisplayId) -> &[NodeId] {
        self.display(display).map_or(&[], |d| &d.screens)
    }

    /// The layer below every screen (LVGL `lv_layer_bottom`).
    #[must_use]
    pub fn bottom_layer(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.bottom_layer)
    }

    /// The layer above the screens, for popups (LVGL `lv_layer_top`).
    #[must_use]
    pub fn top_layer(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.top_layer)
    }

    /// The topmost layer, for the performance overlay and cursors (LVGL `lv_layer_sys`).
    #[must_use]
    pub fn sys_layer(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.sys_layer)
    }

    /// The display `node` belongs to (walks to the root).
    #[must_use]
    pub fn display_of(&self, node: NodeId) -> Option<DisplayId> {
        let root = self.tree.root_of(node)?;
        self.displays.iter().find(|d| d.owns_root(root)).map(|d| d.id)
    }

    /// Calls `wait_vsync` on the driver before the first flush of every frame of `display`
    /// (tear-effect synchronization).
    pub fn set_display_vsync(&mut self, display: DisplayId, on: bool) {
        match self.displays.get_mut(display.index()) {
            Some(d) => d.refresher.vsync = on,
            None => {
                twine_core::warn!(target: "twine::engine", "set_display_vsync: display {} not found", display);
            }
        }
    }

    /// The bytes of framebuffer `index` of a framebuffer display (`None` for other displays or
    /// while the buffer is lent out).
    #[must_use]
    pub fn framebuffer(&self, display: DisplayId, index: u8) -> Option<&[u8]> {
        self.display(display)?.refresher.framebuffer(index)
    }

    /// Creates a node for `widget` as the last child of `parent`, then calls `Widget::init`.
    /// Unknown parents log `warn!` and fail with [`EngineError::NodeNotFound`].
    pub fn create(&mut self, parent: NodeId, widget: Box<dyn Widget>) -> Result<NodeId, EngineError> {
        if !self.tree.contains(parent) {
            twine_core::warn!(target: "twine::engine", "create: parent {} not found", fmt_node_id(parent));
            return Err(EngineError::NodeNotFound(parent));
        }
        let id = self.tree.create(Some(parent), widget)?;
        self.mark_layout(id, crate::LayoutDirty::SELF);
        // LVGL `lv_obj_class_init_obj`: the theme first, then the constructor.
        self.apply_theme_on_create(id);
        self.init_widget(id);
        self.invalidate(id, InvalidateReason::Create);
        self.group_auto_add(id);
        if self.tree.contains(id) {
            self.send_event(id, EventCode::Create, EventParam::None);
        }
        if self.tree.contains(id) && self.tree.contains(parent) {
            // `target` is the new child, the handlers of the parent run.
            self.dispatch(id, parent, EventCode::ChildCreated, EventParam::None);
        }
        Ok(id)
    }

    /// Creates a root node that belongs to no display (LVGL `lv_obj_create(NULL)` before the
    /// screen is used). Useful for tests and for building trees off-screen.
    pub fn create_root(&mut self, widget: Box<dyn Widget>) -> Result<NodeId, EngineError> {
        let id = self.tree.create(None, widget)?;
        self.init_widget(id);
        self.group_auto_add(id);
        if self.tree.contains(id) {
            self.send_event(id, EventCode::Create, EventParam::None);
        }
        Ok(id)
    }

    /// The widget of `id` as `W` (`None` for unknown ids or other widget types).
    #[must_use]
    pub fn widget<W: Widget>(&self, id: NodeId) -> Option<&W> {
        self.tree.get::<W>(id)
    }

    /// Calls `f` with the widget of `id` as `&mut W` and a [`WidgetCx`](crate::WidgetCx) for
    /// the node: the way to call a widget's setters (LVGL `lv_<widget>_set_*`). Returns `None`
    /// (and logs `warn!`) when the node does not exist or holds another widget type.
    ///
    /// While `f` runs the widget is taken out of its node (like during `Widget::init`).
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// assert_eq!(e.with_widget_mut(n, |_o: &mut Obj, cx| cx.node()), Some(n));
    /// ```
    pub fn with_widget_mut<W: Widget, R>(
        &mut self,
        id: NodeId,
        f: impl FnOnce(&mut W, &mut crate::WidgetCx<'_>) -> R,
    ) -> Option<R> {
        let Some(n) = self.tree.node_mut(id) else {
            twine_core::warn!(target: "twine::engine", "with_widget_mut: node {} not found", fmt_node_id(id));
            return None;
        };
        if !n.widget.is::<W>() {
            twine_core::warn!(
                target: "twine::engine",
                "with_widget_mut: node {} is a {}, not the requested widget type",
                fmt_node_id(id),
                n.class().name
            );
            return None;
        }
        let mut w: Box<dyn Widget> = core::mem::replace(&mut n.widget, Box::new(crate::obj::Detached));
        let r = w
            .downcast_mut::<W>()
            .map(|w| f(w, &mut crate::WidgetCx::new(self, id)));
        if let Some(n) = self.tree.node_mut(id) {
            n.widget = w;
        }
        r
    }

    fn init_widget(&mut self, id: NodeId) {
        let Some(n) = self.tree.node_mut(id) else {
            return;
        };
        let mut w: Box<dyn Widget> = core::mem::replace(&mut n.widget, Box::new(crate::obj::Detached));
        w.init(&mut crate::WidgetCx::new(self, id));
        if let Some(n) = self.tree.node_mut(id) {
            n.widget = w;
        }
    }

    /// Deletes `id` and its subtree. Every node of the subtree receives `Delete` (children
    /// before their parent) while it still exists; then the nodes leave their focus groups,
    /// the area is invalidated, the nodes are freed and the parent receives `ChildDeleted`
    /// (with the deleted node as target). Layers and active screens cannot be deleted. The
    /// animations and style transitions of the deleted nodes stop.
    pub fn delete(&mut self, id: NodeId) -> Result<(), EngineError> {
        if !self.tree.contains(id) {
            twine_core::warn!(target: "twine::engine", "delete: node {} not found", fmt_node_id(id));
            return Err(EngineError::NodeNotFound(id));
        }
        for d in &self.displays {
            if id == d.bottom_layer || id == d.top_layer || id == d.sys_layer || id == d.active_screen {
                twine_core::warn!(target: "twine::engine", "delete: {} is a layer or the active screen", fmt_node_id(id));
                return Err(EngineError::InvalidConfig(
                    "cannot delete a layer or the active screen",
                ));
            }
        }
        let order = self.tree.post_order(id);
        for &n in &order {
            if self.tree.contains(n) {
                self.send_event(n, EventCode::Delete, EventParam::None);
            }
        }
        if !self.tree.contains(id) {
            return Ok(()); // a `Delete` handler deleted it already
        }
        for &n in &order {
            if self.tree.node(n).is_some_and(|x| x.group.is_some()) {
                self.group_remove(n);
            }
        }
        if !self.tree.contains(id) {
            return Ok(());
        }
        let parent = self.tree.parent(id);
        self.invalidate_subtree(id, InvalidateReason::Delete);
        let deleted = self.tree.delete(id)?;
        self.anims_forget_nodes(&deleted);
        self.screen_anims_forget(&deleted);
        for d in &mut self.displays {
            d.screens.retain(|s| *s != id);
            if d.prev_screen == Some(id) {
                d.prev_screen = None;
            }
            #[cfg(feature = "perf-monitor")]
            if d.perf_overlay.is_some_and(|p| !self.tree.contains(p)) {
                d.perf_overlay = None;
            }
        }
        if let Some(p) = parent.filter(|p| self.tree.contains(*p)) {
            self.mark_layout(p, crate::LayoutDirty::CHILDREN);
            self.layout.readjust.push(p);
            self.scrollbar_invalidate_tracks(p);
            self.dispatch(id, p, EventCode::ChildDeleted, EventParam::None);
        }
        Ok(())
    }
}
