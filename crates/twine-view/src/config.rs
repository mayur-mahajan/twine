//! [`AppConfig`]: the application's configuration, shared by the firmware, the simulator and
//! the tests.

use twine_anim::Motion;
use twine_core::audit::{Audit, AuditSink, AuditValue, KeyValues};
use twine_core::{Limits, Partition, Rotation};
use twine_engine::{
    DisplayId, DisplayTheme, Engine, EngineConfig, EngineError, FaultHook, ThemeHook, ThemeMode,
};
use twine_reactive::{Runtime, WakerLease};

use crate::error::UiError;

/// Everything about a UI that is the **application's** choice rather than the board's: the
/// engine configuration, the memory bounds ([`Limits`]), the theme, the motion preference,
/// the rotation the screens are designed for and the fault hook.
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
/// The fields are public; the chainable setters are the convenient way to write one. `Clone`
/// is cheap (the theme is a `&'static` reference). [`audit_report`](Self::audit_report) dumps the
/// whole configuration — limits, engine settings, caches, timeouts, theme, enabled features
/// and profile flags — as machine-readable evidence for an audit.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::DisplayInfo;
/// use twine_testing::{MemoryDisplay, MockClock};
/// use twine_view::prelude::*;
///
/// /// The product's configuration (in a crate shared by firmware, simulator and tests).
/// static THEME: Lvgl = Lvgl::new();
///
/// fn config() -> AppConfig {
///     AppConfig::new()
///         .theme(&THEME)
///         .mode(ThemeMode::Dark)
///         .motion(Motion::Reduced)
///         .limits(Limits::DEFAULT.max_nodes(512))
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
/// assert_eq!(ui.engine().limits().max_nodes, 512);
/// assert_eq!(ui.engine().theme(ui.display()).map(|t| t.name()), Some("lvgl"));
/// assert_eq!(ui.engine().theme_mode(ui.display()), ThemeMode::Dark);
/// ```
#[derive(Clone)]
#[must_use]
pub struct AppConfig {
    /// The engine configuration (refresh period, flush timeout and policy, caches, input
    /// timings, idle timeout, …). Default: [`EngineConfig::default`].
    pub engine: EngineConfig,
    /// The memory bounds of the engine, the reactive runtime, the `Ui`'s queues, the
    /// navigation stack and the waker pool (see [`Limits`]), applied when the `Ui` is built
    /// (before anything is created, so preallocated storage is reserved first). Default
    /// [`Limits::DEFAULT`]. The runtime's share applies to the `Ui`'s partition only (every
    /// partition has its own runtime and bounds); the waker pool's share is process-wide (per
    /// thread with `std`): the last `Ui` built sets it.
    pub limits: Limits,
    /// The partition this configuration is for ([`Partition`], ADR-001): named in the audit
    /// report (`partition.index`) so each partition's evidence names itself. The `Ui` builders
    /// set it from the runtime token ([`UiBuilder::runtime`](crate::UiBuilder::runtime));
    /// [`apply_limits`](Self::apply_limits) refuses a configuration for another partition
    /// than the token's. Default [`Partition::MAIN`].
    pub partition: Partition,
    /// The theme of the (first) display: a `static` (one theme per display, fixed at compile
    /// time), attached when the display is created. Default `None`: no theme — set one, or
    /// the widgets get only their own styles (on every host alike). Further displays of a
    /// multi-display `Ui` bring their own ([`DisplayBuilder::theme`](crate::DisplayBuilder::theme)).
    pub theme: Option<&'static dyn ThemeHook>,
    /// The mode the (first) display starts in. `None` (default): the theme's first listed
    /// mode ([`ThemeHook::modes`]). Switch it at run time with `ThemeHandle::set_mode` /
    /// [`Engine::set_theme_mode`].
    pub theme_mode: Option<ThemeMode>,
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
    /// The function called for every fault (see [`Engine::set_fault_hook`]). Default `None`.
    pub fault_hook: Option<FaultHook>,
}

impl core::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppConfig")
            .field("engine", &self.engine)
            .field("limits", &self.limits)
            .field("partition", &self.partition)
            .field("theme", &self.theme.map(ThemeHook::name))
            .field("theme_mode", &self.theme_mode)
            .field("motion", &self.motion)
            .field("rotation", &self.rotation)
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
    /// The defaults: [`EngineConfig::default`], [`Limits::DEFAULT`], no theme,
    /// [`Motion::Full`], the display's own rotation, no fault hook — exactly what a builder
    /// uses when it is given no `AppConfig`.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new();
    /// assert!(cfg.theme.is_none());
    /// assert_eq!(cfg.motion, Motion::Full);
    /// assert_eq!(cfg.limits, Limits::DEFAULT);
    /// ```
    pub fn new() -> Self {
        Self {
            engine: EngineConfig::default(),
            limits: Limits::DEFAULT,
            partition: Partition::MAIN,
            theme: None,
            theme_mode: None,
            motion: Motion::Full,
            rotation: None,
            fault_hook: None,
        }
    }

    /// Sets [`partition`](Self::partition): the partition this configuration is reviewed
    /// for. Needed only when the configuration is applied by hand
    /// ([`apply_limits`](Self::apply_limits), [`UiCore::mount_configured`](crate::UiCore::mount_configured))
    /// to a runtime of another partition than [`Partition::MAIN`]; the builders set it from
    /// their token.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let p = Partition::ALL[Partition::COUNT - 1];
    /// let cfg = AppConfig::new().partition(p);
    /// assert!(cfg.audit_report().to_string().contains(&format!("partition.index={}\n", p.index())));
    /// ```
    pub fn partition(mut self, p: Partition) -> Self {
        self.partition = p;
        self
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

    /// Sets [`limits`](Self::limits).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new().limits(Limits::DEFAULT.max_nodes(300).engine_queue(32));
    /// assert_eq!((cfg.limits.max_nodes, cfg.limits.engine_queue), (300, 32));
    /// ```
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets [`theme`](Self::theme): a `static` theme (`static THEME: Lvgl = Lvgl::new();`).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// static THEME: Lvgl = Lvgl::new();
    /// let cfg = AppConfig::new().theme(&THEME);
    /// assert_eq!(cfg.theme.map(|t| t.name()), Some("lvgl"));
    /// ```
    pub fn theme(mut self, theme: &'static dyn ThemeHook) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Sets [`theme_mode`](Self::theme_mode): the mode the display starts in (a mode the
    /// theme does not list starts in its first mode instead, with a warning).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// static THEME: Lvgl = Lvgl::new();
    /// let cfg = AppConfig::new().theme(&THEME).mode(ThemeMode::Night);
    /// assert_eq!(cfg.theme_mode, Some(ThemeMode::Night));
    /// ```
    pub fn mode(mut self, mode: ThemeMode) -> Self {
        self.theme_mode = Some(mode);
        self
    }

    /// Clears [`theme`](Self::theme): the display gets no theme.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// static THEME: Lvgl = Lvgl::new();
    /// assert!(AppConfig::new().theme(&THEME).no_theme().theme.is_none());
    /// ```
    pub fn no_theme(mut self) -> Self {
        self.theme = None;
        self
    }

    /// The theme the (first) display is created with: [`theme`](Self::theme) starting in
    /// [`theme_mode`](Self::theme_mode). What hosts that create the engine themselves pass to
    /// [`Engine::add_display`] and its siblings (the builders, the simulator and the test
    /// harness do). `None` without a theme.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// static THEME: Lvgl = Lvgl::new();
    /// let cfg = AppConfig::new().theme(&THEME).mode(ThemeMode::Dark);
    /// let mut engine = Engine::new(cfg.engine).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = engine.add_display(panel, BufferMode::alloc(BufferSpec::default()), cfg.display_theme()).unwrap();
    /// assert_eq!(engine.theme_mode(d), ThemeMode::Dark);
    /// ```
    #[must_use]
    pub fn display_theme(&self) -> Option<DisplayTheme> {
        self.theme.map(|theme| DisplayTheme {
            theme,
            mode: self.theme_mode,
        })
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
    /// the fault hook, the motion preference and the rotation (requested only when it differs
    /// from the display's). The theme is not applied here: it is attached when the display
    /// is created ([`display_theme`](Self::display_theme)).
    /// [`engine`](Self::engine) is not applied here — it is what the host creates the engine
    /// with ([`Engine::new`]) — and neither are the [`limits`](Self::limits) (they need the
    /// runtime too: [`apply_limits`](Self::apply_limits), which
    /// [`UiCore::mount_configured`](crate::UiCore::mount_configured) calls).
    ///
    /// Idempotent. Never panics; costs nothing per frame.
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
    /// static THEME: Lvgl = Lvgl::new();
    /// let cfg = AppConfig::new().theme(&THEME).motion(Motion::None);
    /// let mut engine = Engine::new(cfg.engine).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = engine.add_display(panel, BufferMode::alloc(BufferSpec::default()), cfg.display_theme()).unwrap();
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

    /// Puts `engine` into `rt`'s partition ([`Engine::set_partition`]: its growth is then
    /// noted in that partition's seal domain) and applies [`limits`](Self::limits): the
    /// engine's share ([`Engine::set_limits`]), the reactive runtime's
    /// ([`Runtime::set_limits`], for `rt`'s partition only) and the waker pool's heap bound
    /// ([`WakerLease::set_heap_limit`] from [`Limits::max_heap_wakers`]; process-wide, shared
    /// by every partition — the last `Ui` built sets it). With
    /// [`Limits::preallocate`] this reserves the bounded storage, so call it before anything
    /// is created (the builders and [`UiCore::mount_configured`](crate::UiCore::mount_configured)
    /// do). The `Ui`'s own bounds (engine queue, messages per channel) are applied by the
    /// mount. Idempotent; allocates only the reservation; never panics.
    ///
    /// # Errors
    /// [`UiError::Engine`] with [`EngineError::InvalidConfig`] when [`Limits::validate`]
    /// refuses the limits (nothing is applied) — in the `safety` profile also when they are
    /// not preallocated —, or when the configuration is for another partition than `rt`'s
    /// ([`partition`](Self::partition): its audit evidence would name the wrong partition).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cfg = AppConfig::new().limits(Limits::DEFAULT.max_nodes(64).max_scopes(32).preallocated());
    /// let mut engine = Engine::new(cfg.engine).unwrap();
    /// cfg.apply_limits(Runtime::take().unwrap(), &mut engine).unwrap();
    /// assert_eq!(engine.limits().max_nodes, 64);
    /// assert_eq!(engine.partition(), Partition::MAIN);
    /// assert!(cfg.limits(Limits::DEFAULT.max_nodes(0)).apply_limits(Runtime::current_thread(), &mut engine).is_err());
    /// ```
    pub fn apply_limits(&self, rt: Runtime, engine: &mut Engine) -> Result<(), UiError> {
        if self.partition != rt.partition() {
            twine_core::warn!(
                target: "twine::view",
                "apply_limits: the configuration is for {}, the runtime is {}",
                self.partition,
                rt.partition()
            );
            return Err(UiError::Engine(EngineError::InvalidConfig(
                "the configuration is for another partition than the runtime",
            )));
        }
        // The engine belongs to the token's partition (its seal domain), before anything grows.
        engine.set_partition(rt.partition());
        engine.set_limits(&self.limits)?;
        // Validated by the engine above: the runtime accepts the same value.
        if let Err(twine_core::Error::InvalidArgument(msg)) = rt.set_limits(&self.limits) {
            return Err(UiError::Engine(EngineError::InvalidConfig(msg)));
        }
        WakerLease::set_heap_limit(self.limits.max_heap_wakers);
        Ok(())
    }

    /// The complete configuration as `section.key=value` lines (see [`twine_core::audit`]
    /// for the format): `Display`, and `defmt::Format` with the `defmt` feature; or visit
    /// the entries with [`Audit::audit`] to store or transmit them without text. Sections, in
    /// order:
    ///
    /// | Section | Entries |
    /// |---------|---------|
    /// | `audit` | `format` (the format version, `1`) |
    /// | `build` | `twine_version`, the profile and feature flags (`safety`, `no_panic_runtime`, `debug_checks`, `async`, `std`, …), `partitions` (the number of partitions the build has, [`Partition::COUNT`]) |
    /// | `partition` | `index`: the partition the configuration is for ([`partition`](Self::partition)) |
    /// | `limits` | every [`Limits`] field ([`Limits::summary`]) |
    /// | `engine` | every [`EngineConfig`] field: refresh, flush timeout and policy, cache budgets, input timings, idle timeout |
    /// | `fixed` | compile-time capacities: displays, input devices, focus groups, static waker slots |
    /// | `view` | motion, rotation, fault hook |
    /// | `theme` | `theme.set`, `theme.start_mode` (when [`theme_mode`](Self::theme_mode) is set), then the theme's own entries ([`ThemeHook::audit`]: name, modes; R7 themes add their digest) |
    /// | `audit` | `digest_crc32`, last: the CRC-32 of the text of every line before it ([`twine_core::audit::digest`]) — one number to compare with the reviewed configuration's |
    ///
    /// The report is the configuration, not the state: it does not depend on what the
    /// application did. Nothing is allocated; the formatting code is linked only when this is
    /// used.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// static THEME: Lvgl = Lvgl::new();
    /// let cfg = AppConfig::new()
    ///     .limits(Limits::DEFAULT.max_nodes(300).preallocated())
    ///     .theme(&THEME);
    /// let report = cfg.audit_report().to_string();
    /// assert!(report.starts_with("audit.format=1\n"));
    /// assert!(report.contains("limits.max_nodes=300\n"));
    /// assert!(report.contains("limits.preallocate=true\n"));
    /// assert!(report.contains("engine.flush_timeout_us=500000\n"));
    /// assert!(report.contains("build.safety=false\n"));
    /// assert!(report.contains(&format!("build.partitions={}\n", Partition::COUNT)));
    /// assert!(report.contains("partition.index=0\n"));
    /// assert!(report.contains("theme.set=true\n"));
    /// // The last line digests the others.
    /// let (body, last) = report.trim_end().rsplit_once('\n').unwrap();
    /// let crc = twine_core::crc::Crc32::of(format!("{body}\n").as_bytes());
    /// assert_eq!(last, format!("audit.digest_crc32={}", crc.value()));
    /// ```
    pub fn audit_report(&self) -> KeyValues<'_, AppConfig> {
        KeyValues(self)
    }

    /// The display part of [`configure_engine`](Self::configure_engine): the rotation.
    pub(crate) fn configure_display(
        &self,
        engine: &mut Engine,
        display: DisplayId,
    ) -> Result<(), EngineError> {
        let Some(info) = engine.display_info(display) else {
            return Err(EngineError::DisplayNotFound(display));
        };
        if let Some(r) = self.rotation {
            if r != info.rotation {
                engine.set_rotation(display, r)?;
            }
        }
        Ok(())
    }
}

impl Audit for AppConfig {
    /// The entries of [`audit_report`](AppConfig::audit_report), in its order, ending with
    /// `audit.digest_crc32`: the CRC-32 ([`twine_core::audit::digest`]) of every line before it.
    fn audit(&self, out: &mut dyn AuditSink) {
        let body = Entries(self);
        body.audit(out);
        out.entry(
            "audit",
            "digest_crc32",
            AuditValue::Uint(u64::from(twine_core::audit::digest(&body).value())),
        );
    }
}

/// The entries of an [`AppConfig`]'s audit report before its digest.
struct Entries<'a>(&'a AppConfig);

impl Audit for Entries<'_> {
    fn audit(&self, out: &mut dyn AuditSink) {
        let this = self.0;
        out.entry("audit", "format", AuditValue::Uint(1));
        out.entry(
            "build",
            "twine_version",
            AuditValue::Text(env!("CARGO_PKG_VERSION")),
        );
        out.entry("build", "safety", AuditValue::Bool(twine_core::SAFETY_PROFILE));
        out.entry("build", "view_async", AuditValue::Bool(cfg!(feature = "async")));
        out.entry("build", "view_vector", AuditValue::Bool(cfg!(feature = "vector")));
        out.entry("build", "view_svg", AuditValue::Bool(cfg!(feature = "svg")));
        out.entry("build", "std", AuditValue::Bool(cfg!(feature = "std")));
        out.entry("build", "partitions", AuditValue::from_usize(Partition::COUNT));
        twine_reactive::audit_features(out);
        twine_engine::audit_features(out);
        out.entry(
            "partition",
            "index",
            AuditValue::Uint(u64::from(this.partition.index())),
        );
        this.limits.audit(out);
        this.engine.audit(out);
        out.entry(
            "fixed",
            "max_displays",
            AuditValue::from_usize(twine_engine::MAX_DISPLAYS),
        );
        out.entry(
            "fixed",
            "max_inputs",
            AuditValue::from_usize(twine_engine::MAX_INPUTS),
        );
        out.entry(
            "fixed",
            "max_focus_groups",
            AuditValue::from_usize(twine_engine::MAX_GROUPS),
        );
        out.entry(
            "fixed",
            "static_wakers",
            AuditValue::from_usize(WakerLease::STATIC_SLOTS),
        );
        out.entry(
            "view",
            "motion",
            AuditValue::Text(match this.motion {
                Motion::Full => "full",
                Motion::Reduced => "reduced",
                Motion::None => "none",
            }),
        );
        out.entry(
            "view",
            "rotation_deg",
            this.rotation
                .map_or(AuditValue::Absent, |r| AuditValue::Uint(u64::from(r.degrees()))),
        );
        out.entry("view", "fault_hook", AuditValue::Bool(this.fault_hook.is_some()));
        out.entry("theme", "set", AuditValue::Bool(this.theme.is_some()));
        if let Some(mode) = this.theme_mode {
            out.entry("theme", "start_mode", AuditValue::Text(mode.name()));
        }
        if let Some(theme) = this.theme {
            theme.audit(out);
        }
    }
}
