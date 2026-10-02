//! [`AppConfig`]: the application's configuration, shared by the firmware, the simulator and
//! the tests.

use alloc::rc::Rc;

use twine_anim::Motion;
use twine_core::Rotation;
use twine_engine::{DisplayId, Engine, EngineConfig, EngineError, FaultHook, IntoTheme, ThemeHook};

use crate::engine_queue::DEFAULT_ENGINE_QUEUE_CAPACITY;
use crate::ui::DEFAULT_MESSAGES_PER_CHANNEL;

/// Everything about a UI that is the **application's** choice rather than the board's: the
/// engine configuration, the theme, the motion preference, the rotation the screens are
/// designed for, the channel and engine-queue budgets and the fault hook.
///
/// Write it once, as a function, and hand it to every host that runs the application:
///
/// - firmware: [`UiBuilder::app_config`](crate::UiBuilder::app_config) (and
///   `AsyncUiBuilder::app_config`),
/// - the simulator: `twine_sim::SimConfig::app_config`,
/// - tests: `twine_testing::TestUi::app_config`,
///
/// so the simulator and the tests run the shipped configuration — same engine budgets,
/// theme, motion, rotation and fault policy, hence the same pixels — instead of a copy that
/// drifts. What belongs to the board stays with the board's `main`: the display and input
/// drivers, the draw-buffer **memory** (`draw_buffers!` statics, sized for the panel; the
/// simulator and the tests bring their own, and the engine draws identical pixels whatever
/// the buffer geometry), the runtime token and the platform. Platform hooks that live in
/// [`EngineConfig`] (`mem_info`, `hires_timer`) are set by the board on the value it gets
/// (`cfg.engine.hires_timer = Some(..)`); the simulator installs its own timer.
///
/// The fields are public (an audit can read the whole configuration); the chainable setters
/// are the convenient way to write one. `Clone` is cheap (the theme is shared, not copied).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::DisplayInfo;
/// use twine_testing::{MemoryDisplay, MockClock};
/// use twine_view::prelude::*;
///
/// /// The product's configuration (in a crate shared by firmware, simulator and tests).
/// fn config() -> AppConfig {
///     AppConfig::new()
///         .theme(DefaultTheme::dark())
///         .motion(Motion::Reduced)
///         .engine(EngineConfig { max_nodes: 512, ..EngineConfig::default() })
/// }
///
/// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
/// let ui = Ui::builder(panel)
///     .runtime(Runtime::take().unwrap())
///     .clock(MockClock::new())
///     .buffers(BufferMode::alloc(BufferSpec::default()))
///     .app_config(config())
///     .build(|_| label("hi"));
/// assert_eq!(ui.motion(), Motion::Reduced);
/// assert_eq!(ui.engine().config().max_nodes, 512);
/// assert_eq!(ui.engine().theme(ui.display()).map(|t| t.name()), config().theme.map(|t| t.name()));
/// ```
#[derive(Clone)]
#[must_use]
pub struct AppConfig {
    /// The engine configuration (refresh period, flush timeout and policy, caches, node
    /// budget, input timings, idle timeout, …). Default: [`EngineConfig::default`].
    pub engine: EngineConfig,
    /// The theme of the (first) display. Default `None`: no theme — set one, or the widgets
    /// get only their own styles (on every host alike). Further displays of a multi-display
    /// `Ui` bring their own ([`DisplayBuilder::theme`](crate::DisplayBuilder::theme)).
    pub theme: Option<Rc<dyn ThemeHook>>,
    /// The initial motion preference ([`Motion`]). Default [`Motion::Full`].
    pub motion: Motion,
    /// The rotation the screens are designed for. `None` (default) keeps the display's own
    /// rotation (the one its driver was created with). `Some(r)` different from it is
    /// requested when the UI is built and applied before the first frame is drawn
    /// ([`Engine::set_rotation`]): by the driver in hardware (`MADCTL` panels), else in
    /// software — which needs [`UiBuilder::reserve_rotation`](crate::UiBuilder::reserve_rotation),
    /// or the rotation is refused with [`FaultKind::DisplayControl`](twine_core::fault::FaultKind::DisplayControl).
    /// The simulator emulates it in hardware; `TestUi` rotates its in-memory panel.
    pub rotation: Option<Rotation>,
    /// Messages each channel delivers per update (see
    /// [`UiBuilder::messages_per_channel`](crate::UiBuilder::messages_per_channel)). Default
    /// [`DEFAULT_MESSAGES_PER_CHANNEL`].
    pub messages_per_channel: usize,
    /// Capacity of the engine-command queue (see
    /// [`UiBuilder::engine_queue_capacity`](crate::UiBuilder::engine_queue_capacity)). Default
    /// [`DEFAULT_ENGINE_QUEUE_CAPACITY`].
    pub engine_queue_capacity: usize,
    /// The function called for every fault (see [`Engine::set_fault_hook`]). Default `None`.
    pub fault_hook: Option<FaultHook>,
}

impl core::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppConfig")
            .field("engine", &self.engine)
            .field("theme", &self.theme.as_ref().map(|t| t.name()))
            .field("motion", &self.motion)
            .field("rotation", &self.rotation)
            .field("messages_per_channel", &self.messages_per_channel)
            .field("engine_queue_capacity", &self.engine_queue_capacity)
            .field("fault_hook", &self.fault_hook.is_some())
            .finish()
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl AppConfig {
    /// The defaults: [`EngineConfig::default`], no theme, [`Motion::Full`], the display's own
    /// rotation, [`DEFAULT_MESSAGES_PER_CHANNEL`], [`DEFAULT_ENGINE_QUEUE_CAPACITY`], no fault
    /// hook — exactly what a builder uses when it is given no `AppConfig`.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new();
    /// assert!(cfg.theme.is_none());
    /// assert_eq!(cfg.motion, Motion::Full);
    /// assert_eq!(cfg.messages_per_channel, twine_view::DEFAULT_MESSAGES_PER_CHANNEL);
    /// ```
    pub fn new() -> Self {
        Self {
            engine: EngineConfig::default(),
            theme: None,
            motion: Motion::Full,
            rotation: None,
            messages_per_channel: DEFAULT_MESSAGES_PER_CHANNEL,
            engine_queue_capacity: DEFAULT_ENGINE_QUEUE_CAPACITY,
            fault_hook: None,
        }
    }

    /// Sets [`engine`](Self::engine).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new().engine(EngineConfig { refr_period: Duration::ms(20), ..EngineConfig::default() });
    /// assert_eq!(cfg.engine.refr_period, Duration::ms(20));
    /// ```
    pub fn engine(mut self, engine: EngineConfig) -> Self {
        self.engine = engine;
        self
    }

    /// Sets [`theme`](Self::theme): a theme value or a shared one ([`IntoTheme`]).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new().theme(DefaultTheme::light());
    /// assert!(cfg.theme.is_some());
    /// ```
    pub fn theme(mut self, theme: impl IntoTheme) -> Self {
        self.theme = Some(theme.into_theme());
        self
    }

    /// Clears [`theme`](Self::theme): the display gets no theme.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// assert!(AppConfig::new().theme(DefaultTheme::light()).no_theme().theme.is_none());
    /// ```
    pub fn no_theme(mut self) -> Self {
        self.theme = None;
        self
    }

    /// Sets [`motion`](Self::motion).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// assert_eq!(AppConfig::new().motion(Motion::Reduced).motion, Motion::Reduced);
    /// ```
    pub fn motion(mut self, motion: Motion) -> Self {
        self.motion = motion;
        self
    }

    /// Sets [`rotation`](Self::rotation) to `Some(rotation)`.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// assert_eq!(AppConfig::new().rotation(Rotation::Deg90).rotation, Some(Rotation::Deg90));
    /// ```
    pub fn rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = Some(rotation);
        self
    }

    /// Sets [`messages_per_channel`](Self::messages_per_channel) (`0` counts as `1`).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// assert_eq!(AppConfig::new().messages_per_channel(4).messages_per_channel, 4);
    /// ```
    pub fn messages_per_channel(mut self, n: usize) -> Self {
        self.messages_per_channel = n;
        self
    }

    /// Sets [`engine_queue_capacity`](Self::engine_queue_capacity).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// assert_eq!(AppConfig::new().engine_queue_capacity(64).engine_queue_capacity, 64);
    /// ```
    pub fn engine_queue_capacity(mut self, capacity: usize) -> Self {
        self.engine_queue_capacity = capacity;
        self
    }

    /// Sets [`fault_hook`](Self::fault_hook).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn log_fault(_: &FaultRecord) {}
    /// assert!(AppConfig::new().fault_hook(log_fault).fault_hook.is_some());
    /// ```
    pub fn fault_hook(mut self, hook: FaultHook) -> Self {
        self.fault_hook = Some(hook);
        self
    }

    /// Applies the configuration to an engine and its `display`, for hosts that create the
    /// engine themselves (the simulator, test harnesses, custom loops; the builders call it):
    /// the fault hook, the motion preference, the theme (unless the display already has this
    /// very theme) and the rotation (requested only when it differs from the display's).
    /// [`engine`](Self::engine) is not applied here — it is what the host creates the engine
    /// with ([`Engine::new`]) — and neither are the channel and queue budgets (they belong to
    /// the mount: [`UiCore::mount_configured`](crate::UiCore::mount_configured)).
    ///
    /// Idempotent. Never panics; costs one theme installation (O(nodes of the display)) when
    /// the theme changes, nothing per frame.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`] for an unknown display; [`EngineError::InvalidConfig`]
    /// when a rotation is configured for a framebuffer display (see [`Engine::set_rotation`]).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new().theme(DefaultTheme::light()).motion(Motion::None);
    /// let mut engine = Engine::new(cfg.engine).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = engine.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// cfg.configure_engine(&mut engine, d).unwrap();
    /// assert_eq!(engine.motion(), Motion::None);
    /// assert!(engine.theme(d).is_some());
    /// ```
    pub fn configure_engine(&self, engine: &mut Engine, display: DisplayId) -> Result<(), EngineError> {
        self.configure_engine_wide(engine);
        self.configure_display(engine, display)
    }

    /// The engine-wide part of [`configure_engine`](Self::configure_engine) (before any
    /// display exists, so the fault hook sees the display registration's faults).
    pub(crate) fn configure_engine_wide(&self, engine: &mut Engine) {
        engine.set_fault_hook(self.fault_hook);
        engine.set_motion(self.motion);
    }

    /// The display part of [`configure_engine`](Self::configure_engine): theme and rotation.
    pub(crate) fn configure_display(
        &self,
        engine: &mut Engine,
        display: DisplayId,
    ) -> Result<(), EngineError> {
        let Some(info) = engine.display_info(display) else {
            return Err(EngineError::DisplayNotFound(display));
        };
        if let Some(theme) = &self.theme {
            let same = engine.theme(display).is_some_and(|t| Rc::ptr_eq(t, theme));
            if !same {
                engine.set_theme(display, theme.clone());
            }
        }
        if let Some(r) = self.rotation {
            if r != info.rotation {
                engine.set_rotation(display, r)?;
            }
        }
        Ok(())
    }
}
