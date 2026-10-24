//! Multi-panel apps: several independent UIs side by side in one window
//! ([`SimApp::panels`](crate::SimApp::panels), [`run_panels`](crate::run_panels)).
//!
//! A product can drive several displays from one MCU, each with its own UI — for example a
//! safety-critical display next to a non-critical one, each in its own **partition**
//! (`twine_reactive::Runtime::take_partition`, feature `partitions`). A multi-panel app
//! simulates exactly that: every [`SimPanel`] is a separate `Ui` with **its own engine**, its own
//! emulated panel ([`SimDisplay`]: own flushes, bus timing and dirty areas),
//! its own simulated input devices and the reactive runtime of the token it was given. The
//! window only places the panels' pixels side by side ([`PanelLayout`]); nothing else is
//! shared, so what one panel does (redraws, faults, slow updates, a stall) never touches
//! another's engine.
//!
//! **Input routing.** The pointer goes to the panel under it, in that panel's coordinates; a
//! press **focuses** the panel and keeps the pointer there until it is released (a drag
//! leaving the panel stays with it). The keyboard (keypad) and the mouse wheel and middle
//! button (encoder) go to the **focused** panel — the first one until another is clicked.
//! Headless scripts route the same way (`tap` a panel to focus it). Engine hotkeys (F2, F3,
//! F4, F8, F12) act on the focused panel.
//!
//! **Stepping.** Every tick steps each running panel's `Ui` with the shared simulator clock;
//! each wakes the window through its own waker. A panel's [`on_frame`](SimPanel::on_frame) hook
//! runs before its update (~60 times per second while any hook is set) with the keys routed
//! to it ([`PanelCx`]); it can stop and resume the panel ([`PanelCx::set_running`]) to
//! simulate a stalled execution context.
//!
//! ```no_run
//! use twine_reactive::Runtime;
//! use twine_sim::{SimConfig, SimPanel, run_panels};
//! use twine_view::prelude::*;
//!
//! let left = SimPanel::new(160, 120, Runtime::take().unwrap(), |_| label("left"));
//! let right = SimPanel::new(160, 120, Runtime::current_thread(), |_| label("right"));
//! run_panels(SimConfig::new(320, 120).title("Two panels").scale(2), vec![left, right]);
//! ```

use std::time::Instant as StdInstant;

use twine_core::{Instant, Point, Rect, Rotation};
use twine_engine::{BufferMode, BufferSpec, DisplayId, Engine, EngineConfig, Wake};
use twine_hal::Key;
use twine_reactive::{Runtime, Scope};
use twine_view::{AppConfig, UiCore, UiError};

use crate::app::{EngineInputs, SimError, register_inputs, sim_hires_timer};
use crate::config::SimConfig;
use crate::display::SimDisplay;
use crate::input::SimDevices;

/// Pixels between two panels (and around them) in the window, filled with [`GAP_COLOR`].
pub const PANEL_GAP: u16 = 4;

/// The colour of the window between the panels (`0x00RRGGBB`).
pub const GAP_COLOR: u32 = 0x0020_2020;

/// How the panels of a multi-panel app are placed in the window.
///
/// ```
/// use twine_core::Rect;
/// use twine_sim::PanelLayout;
///
/// let (rects, size) = PanelLayout::Row.arrange(&[(160, 120), (100, 80)]);
/// assert_eq!(rects, [Rect::new(4, 4, 164, 124), Rect::new(168, 4, 268, 84)]);
/// assert_eq!(size, (272, 128));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelLayout {
    /// Left to right, top-aligned (default).
    #[default]
    Row,
    /// Top to bottom, left-aligned.
    Column,
}

impl PanelLayout {
    /// The rectangle of each panel of size `(width, height)` in the window, and the window size,
    /// with [`PANEL_GAP`] pixels between and around the panels. Saturates at `u16::MAX`;
    /// never panics.
    #[must_use]
    pub fn arrange(self, sizes: &[(u16, u16)]) -> (Vec<Rect>, (u16, u16)) {
        let gap = i32::from(PANEL_GAP);
        let mut rects = Vec::with_capacity(sizes.len());
        let (mut along, mut across) = (gap, 0i32);
        for &(w, h) in sizes {
            let (w, h) = (i32::from(w), i32::from(h));
            let r = match self {
                PanelLayout::Row => Rect::new(along, gap, along.saturating_add(w), gap.saturating_add(h)),
                PanelLayout::Column => Rect::new(gap, along, gap.saturating_add(w), along.saturating_add(h)),
            };
            let (len, depth) = match self {
                PanelLayout::Row => (w, h),
                PanelLayout::Column => (h, w),
            };
            along = along.saturating_add(len).saturating_add(gap);
            across = across.max(depth);
            rects.push(r);
        }
        let across = across.saturating_add(2 * gap);
        let size = |v: i32| u16::try_from(v).unwrap_or(u16::MAX);
        let window = match self {
            PanelLayout::Row => (size(along), size(across)),
            PanelLayout::Column => (size(across), size(along)),
        };
        (rects, window)
    }
}

/// Mounts a panel's application on its engine and display.
type MountFn = Box<dyn FnOnce(&mut Engine, DisplayId, AppConfig) -> Result<UiCore, UiError>>;

/// Runs before a panel's update (see [`SimPanel::on_frame`]).
pub type PanelHook = Box<dyn FnMut(&mut PanelCx<'_>)>;

/// One panel of a multi-panel app: a declarative UI on its own engine and emulated display
/// (see the [module docs](self)). Built with [`new`](Self::new), configured with the builder
/// methods, then handed to [`SimApp::panels`](crate::SimApp::panels) or
/// [`run_panels`](crate::run_panels), which build its engine and mount the app.
///
/// ```
/// use twine_reactive::Runtime;
/// use twine_sim::SimPanel;
/// use twine_view::prelude::*;
///
/// let panel = SimPanel::new(160, 120, Runtime::current_thread(), |_| label("hi"))
///     .title("main")
///     .app_config(AppConfig::new().theme(&lvgl::THEME).mode(ThemeMode::Dark));
/// assert_eq!(panel.size(), (160, 120));
/// assert_eq!(panel.name(), "main");
/// ```
pub struct SimPanel {
    width: u16,
    height: u16,
    title: String,
    config: AppConfig,
    mount: MountFn,
    on_frame: Option<PanelHook>,
}

impl std::fmt::Debug for SimPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimPanel")
            .field("size", &(self.width, self.height))
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl SimPanel {
    /// A `width × height` panel running `app` on the runtime `rt` — a partition's token
    /// (`Runtime::take_partition`) for independent partitions, or the same token for several
    /// panels of one runtime. The application is mounted with the panel's
    /// [`AppConfig`] ([`app_config`](Self::app_config); default [`AppConfig::new`]), whose
    /// partition is set to `rt`'s. Nothing is built before the app starts.
    #[must_use]
    pub fn new<V: twine_view::View>(
        width: u16,
        height: u16,
        rt: Runtime,
        app: impl FnOnce(Scope) -> V + 'static,
    ) -> Self {
        let mount: MountFn = Box::new(move |engine, display, mut config| {
            config.partition = rt.partition();
            UiCore::mount_configured(rt, engine, display, &config, app)
        });
        Self {
            width,
            height,
            title: format!("panel ({})", rt.partition()),
            config: AppConfig::new(),
            mount,
            on_frame: None,
        }
    }

    /// The panel's name in logs (default: its partition).
    #[must_use]
    pub fn title(mut self, title: &str) -> Self {
        title.clone_into(&mut self.title);
        self
    }

    /// The application's configuration (engine, limits, theme, fault hook, …); the panel's
    /// engine also gets the simulator's high-resolution timer.
    #[must_use]
    pub fn app_config(mut self, config: AppConfig) -> Self {
        self.config = config;
        self
    }

    /// Runs `hook` before every update of the panel (about 60 times per second while the app
    /// runs, also while the panel is stopped), with the keys routed to the panel since the last
    /// call: host-side code of the simulation, e.g. a sensor feeding a `Latest`, fault
    /// injection on keys, or stopping the panel ([`PanelCx::set_running`]).
    #[must_use]
    pub fn on_frame(mut self, hook: impl FnMut(&mut PanelCx<'_>) + 'static) -> Self {
        self.on_frame = Some(Box::new(hook));
        self
    }

    /// The panel's size in pixels.
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// The panel's name ([`title`](Self::title)).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.title
    }
}

/// What a panel's [`on_frame`](SimPanel::on_frame) hook can see and do.
#[derive(Debug)]
pub struct PanelCx<'a> {
    index: usize,
    engine: &'a mut Engine,
    ui: &'a mut UiCore,
    keys: &'a [Key],
    running: &'a mut bool,
    now: Instant,
}

impl PanelCx<'_> {
    /// The panel's position in the app (0 = first).
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The keys pressed while the panel had the focus, since the previous call.
    #[must_use]
    pub fn keys(&self) -> &[Key] {
        self.keys
    }

    /// The simulator time of this frame.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.now
    }

    /// The panel's engine.
    pub fn engine(&mut self) -> &mut Engine {
        self.engine
    }

    /// The panel's `UiCore` and its engine (most `UiCore` methods take the engine).
    pub fn ui(&mut self) -> (&mut UiCore, &mut Engine) {
        (self.ui, self.engine)
    }

    /// Stops (`false`) or resumes the panel's updates: a stopped panel keeps its last frame and
    /// its heartbeat stands still, like a UI whose execution context stalled. Its hook still
    /// runs.
    pub fn set_running(&mut self, running: bool) {
        *self.running = running;
    }

    /// Whether the panel is updated.
    #[must_use]
    pub fn is_running(&self) -> bool {
        *self.running
    }
}

/// A built panel.
pub(crate) struct Panel {
    pub(crate) engine: Engine,
    pub(crate) display: DisplayId,
    ui: Option<UiCore>,
    pub(crate) devices: SimDevices,
    inputs: EngineInputs,
    title: String,
    pub(crate) rect: Rect,
    on_frame: Option<PanelHook>,
    running: bool,
    keys: Vec<Key>,
    pub(crate) perf_overlay: bool,
    /// The panel's last update asked to be woken at once.
    busy: bool,
}

impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(ui) = self.ui.take() {
            ui.dispose(&mut self.engine);
        }
    }
}

impl Panel {
    /// The panel's emulated display.
    pub(crate) fn sim_display(&self) -> Option<&SimDisplay> {
        self.engine.driver::<SimDisplay>(self.display)
    }

    /// The panel's `UiCore`.
    pub(crate) fn ui(&self) -> Option<&UiCore> {
        self.ui.as_ref()
    }
}

/// Where window input goes: the pointer to the panel under it (captured by the panel it was
/// pressed on until released), keys and the encoder to the focused panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Router {
    /// The panel the keypad and encoder feed (the last one pressed on).
    pub(crate) focused: usize,
    /// The panel holding the pressed pointer.
    captured: Option<usize>,
}

/// A pointer event in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerEvent {
    Press(Point),
    Move(Point),
    Release,
}

impl Router {
    /// The panel and the panel-local point a pointer event goes to (`None`: outside every
    /// panel and not captured). A press focuses and captures the panel; a release ends the
    /// capture.
    pub(crate) fn pointer(&mut self, rects: &[Rect], ev: PointerEvent) -> Option<(usize, Point)> {
        let local = |i: usize, p: Point| {
            rects.get(i).map(|r| {
                // Clamped into the panel: a captured drag leaving it stays at its edge.
                let x = p.x.clamp(r.x0, r.x1.saturating_sub(1)).saturating_sub(r.x0);
                let y = p.y.clamp(r.y0, r.y1.saturating_sub(1)).saturating_sub(r.y0);
                (i, Point::new(x, y))
            })
        };
        let under = |p: Point| rects.iter().position(|r| r.contains(p));
        match ev {
            PointerEvent::Press(p) => {
                let i = under(p)?;
                self.focused = i;
                self.captured = Some(i);
                local(i, p)
            }
            PointerEvent::Move(p) => match self.captured {
                Some(i) => local(i, p),
                None => under(p).and_then(|i| local(i, p)),
            },
            PointerEvent::Release => self
                .captured
                .take()
                .and_then(|i| rects.get(i).map(|r| (i, r.origin()))),
        }
    }
}

/// A multi-panel program.
pub(crate) struct PanelsProgram {
    pub(crate) panels: Vec<Panel>,
    pub(crate) size: (u16, u16),
    pub(crate) router: Router,
}

impl PanelsProgram {
    /// Builds every panel: an engine with the simulator's timer, a [`SimDisplay`] in `cfg`'s
    /// format and bus speed, the simulated devices of `cfg.input`, then the panel's app.
    pub(crate) fn build(
        cfg: &SimConfig,
        panels: Vec<SimPanel>,
        layout: PanelLayout,
    ) -> Result<Self, SimError> {
        let sizes: Vec<(u16, u16)> = panels.iter().map(SimPanel::size).collect();
        let (rects, size) = layout.arrange(&sizes);
        let mut built = Vec::with_capacity(panels.len());
        for (p, rect) in panels.into_iter().zip(rects) {
            let mut engine = Engine::new(EngineConfig {
                hires_timer: Some(sim_hires_timer),
                ..p.config.engine
            })?;
            engine.set_fault_hook(p.config.fault_hook);
            let display = SimDisplay::new(p.width, p.height, cfg.format, Rotation::Deg0)
                .with_bus_hz(cfg.bus_hz)
                .with_mono_colors(cfg.mono_colors.0, cfg.mono_colors.1);
            let display = engine.add_display(
                display,
                BufferMode::alloc(BufferSpec::default()),
                p.config.display_theme(),
            )?;
            let devices = SimDevices::new();
            let inputs = register_inputs(&mut engine, display, cfg, &devices)?;
            let ui = (p.mount)(&mut engine, display, p.config)?;
            log::info!(target: "twine::sim", "panel {}: {}x{} at {}", p.title, p.width, p.height, rect.origin());
            built.push(Panel {
                engine,
                display,
                ui: Some(ui),
                devices,
                inputs,
                title: p.title,
                rect,
                on_frame: p.on_frame,
                running: true,
                keys: Vec::new(),
                perf_overlay: false,
                busy: false,
            });
        }
        Ok(Self {
            panels: built,
            size,
            router: Router::default(),
        })
    }

    /// The panels' rectangles in the window.
    pub(crate) fn rects(&self) -> Vec<Rect> {
        self.panels.iter().map(|p| p.rect).collect()
    }

    /// The devices of panel `i`.
    pub(crate) fn devices(&self, i: usize) -> Option<&SimDevices> {
        self.panels.get(i).map(|p| &p.devices)
    }

    /// The focused panel's devices.
    pub(crate) fn focused_devices(&self) -> Option<&SimDevices> {
        self.devices(self.router.focused)
    }

    /// Whether any panel has an `on_frame` hook (the app then ticks at the frame rate).
    pub(crate) fn has_hooks(&self) -> bool {
        self.panels.iter().any(|p| p.on_frame.is_some())
    }

    /// One step of every panel at `now`: notifies its engine of its devices' changes, runs its
    /// hook, updates it if it runs. Returns the earliest wake-up of all.
    pub(crate) fn step(&mut self, now: Instant) -> Wake {
        let mut wake = Wake::Idle;
        for (index, p) in self.panels.iter_mut().enumerate() {
            let (keys, changes) = {
                let mut s = p.devices.state.borrow_mut();
                (s.take_raw_keys(), s.take_changes())
            };
            p.keys.extend(keys);
            let ids = p.inputs;
            for (changed, id) in [
                (changes.pointer, ids.pointer),
                (changes.keypad, ids.keypad),
                (changes.encoder, ids.encoder),
            ] {
                if let Some(id) = id.filter(|_| changed) {
                    p.engine.notify_input(id);
                }
            }
            if let (Some(hook), Some(ui)) = (&mut p.on_frame, &mut p.ui) {
                let keys = std::mem::take(&mut p.keys);
                let mut cx = PanelCx {
                    index,
                    engine: &mut p.engine,
                    ui,
                    keys: &keys,
                    running: &mut p.running,
                    now,
                };
                hook(&mut cx);
            } else {
                p.keys.clear();
            }
            let w = match (&mut p.ui, p.running) {
                (Some(ui), true) => ui.update(&mut p.engine, now),
                _ => Wake::Idle,
            };
            p.busy = w == Wake::Now;
            wake = wake.min(w);
        }
        wake
    }

    /// Whether a panel asked to be updated again at once.
    pub(crate) fn any_busy(&self) -> bool {
        self.panels.iter().any(|p| p.busy)
    }

    /// When the oldest emulated transfer of any panel completes.
    pub(crate) fn busy_until(&self) -> Option<StdInstant> {
        self.panels
            .iter()
            .filter_map(|p| p.sim_display()?.busy_until())
            .min()
    }

    /// Whether any panel changed since the last call (every panel's flag is taken).
    pub(crate) fn take_dirty(&mut self) -> bool {
        let mut any = false;
        for p in &mut self.panels {
            let d = p.display;
            any |= p
                .engine
                .driver_mut::<SimDisplay>(d)
                .is_some_and(|s| s.take_dirty().is_some());
        }
        any
    }

    /// The window as `0x00RRGGBB` words: every panel at its rectangle, [`GAP_COLOR`] between.
    pub(crate) fn xrgb_into(&self, out: &mut Vec<u32>) {
        let (w, h) = (usize::from(self.size.0), usize::from(self.size.1));
        out.clear();
        out.resize(w * h, GAP_COLOR);
        let mut scratch = Vec::new();
        for p in &self.panels {
            let Some(d) = p.sim_display() else { continue };
            d.panel_xrgb_into(&mut scratch);
            let (pw, _) = d.panel_size();
            let pw = usize::from(pw);
            if pw == 0 {
                continue;
            }
            let (x0, y0) = (p.rect.x0 as usize, p.rect.y0 as usize);
            for (y, row) in scratch.chunks_exact(pw).enumerate() {
                let at = (y0 + y) * w + x0;
                if let Some(dst) = out.get_mut(at..at + pw) {
                    dst.copy_from_slice(row);
                }
            }
        }
    }

    /// The window as 8-bit RGB (screenshots, headless shots).
    pub(crate) fn rgb888(&self) -> Vec<u8> {
        let mut words = Vec::new();
        self.xrgb_into(&mut words);
        words
            .iter()
            .flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
            .collect()
    }

    /// The focused panel (engine hotkeys act on it).
    pub(crate) fn focused_mut(&mut self) -> Option<&mut Panel> {
        self.panels.get_mut(self.router.focused)
    }

    /// The UI wakers of every panel (registered with the window's waker).
    pub(crate) fn wakers(&self) -> Vec<&'static twine_reactive::UiWaker> {
        self.panels
            .iter()
            .filter_map(|p| p.ui().map(UiCore::waker))
            .collect()
    }

    /// Panel `i`'s name.
    pub(crate) fn title(&self, i: usize) -> &str {
        self.panels.get(i).map_or("", |p| p.title.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects() -> Vec<Rect> {
        PanelLayout::Row.arrange(&[(100, 50), (60, 40)]).0
    }

    #[test]
    fn layouts_place_panels_with_gaps() {
        let (r, size) = PanelLayout::Row.arrange(&[(100, 50), (60, 40)]);
        assert_eq!(r, [Rect::new(4, 4, 104, 54), Rect::new(108, 4, 168, 44)]);
        assert_eq!(size, (172, 58));
        let (r, size) = PanelLayout::Column.arrange(&[(100, 50), (60, 40)]);
        assert_eq!(r, [Rect::new(4, 4, 104, 54), Rect::new(4, 58, 64, 98)]);
        assert_eq!(size, (108, 102));
        let (r, size) = PanelLayout::Row.arrange(&[]);
        assert_eq!(r, []);
        assert_eq!(size, (4, 8));
    }

    #[test]
    fn pointer_goes_to_the_panel_under_it_and_stays_while_pressed() {
        let r = rects();
        let mut router = Router::default();
        assert_eq!(
            router.pointer(&r, PointerEvent::Move(Point::new(110, 10))),
            Some((1, Point::new(2, 6)))
        );
        assert_eq!(
            router.pointer(&r, PointerEvent::Move(Point::new(106, 10))),
            None,
            "the gap"
        );
        assert_eq!(router.focused, 0);
        // A press focuses and captures panel 1; a drag into panel 0 stays with panel 1.
        assert_eq!(
            router.pointer(&r, PointerEvent::Press(Point::new(120, 20))),
            Some((1, Point::new(12, 16)))
        );
        assert_eq!(router.focused, 1);
        assert_eq!(
            router.pointer(&r, PointerEvent::Move(Point::new(10, 10))),
            Some((1, Point::new(0, 6)))
        );
        assert_eq!(router.pointer(&r, PointerEvent::Release).map(|(i, _)| i), Some(1));
        assert_eq!(
            router.pointer(&r, PointerEvent::Release),
            None,
            "nothing captured"
        );
        // A press in the gap focuses nothing.
        assert_eq!(router.pointer(&r, PointerEvent::Press(Point::new(0, 0))), None);
        assert_eq!(router.focused, 1);
        assert_eq!(
            router.pointer(&r, PointerEvent::Press(Point::new(5, 5))),
            Some((0, Point::new(1, 1)))
        );
        assert_eq!(router.focused, 0);
    }
}
