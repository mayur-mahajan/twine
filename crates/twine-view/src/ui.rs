//! The runtime: [`Ui`], [`UiBuilder`] and [`UiCore`].

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use twine_anim::Motion;
use twine_core::fault::{FaultCounts, FaultKind, Faults};
use twine_core::{Duration, Fraction, Instant, Rotation};
use twine_engine::{
    BufferMode, DisplayCmd, DisplayId, Engine, EngineConfig, EngineError, FaultHook, FaultRecord, InputId,
    IntoTheme, StepBudget, Wake,
};
use twine_hal::{Clock, DisplayDriver, FramebufferDisplay, InputDevice, Platform};
use twine_reactive::{Channel, Runtime, Scope, Signal, UiWaker, WakerLease};
use twine_render::DrawAccel;

use crate::access::{EffectCx, EngineAccess};
use crate::build::{BuildCx, MountReport};
use crate::config::AppConfig;
use crate::display::{CommandsSetter, DisplayBuilder, DisplayMut, mount_display};
use crate::engine_queue::{DEFAULT_ENGINE_QUEUE_CAPACITY, EngineQueue};
use crate::error::{BuildError, BuildFailure, BuildFault, CapacityFault, UiError};
use crate::hooks::{StyleAnchor, ThemeState, UiContext, track_theme};
use crate::typestate::{HasBuffers, HasClock, HasRuntime, NoBuffers, NoClock, NoRuntime};
use crate::view::View;

/// The default of [`UiBuilder::messages_per_channel`]: messages delivered per channel per
/// update (the rest waits for the next update, which is requested at once).
pub const DEFAULT_MESSAGES_PER_CHANNEL: usize = 16;

/// The reactive half of a [`Ui`] for hosts that own the engine themselves (test harnesses,
/// simulators, custom loops): the root scope, the waker, and the update cycle run on an
/// engine passed in.
///
/// ```
/// use twine_engine::{Engine, EngineConfig, Obj};
/// use twine_view::{UiCore, prelude::*};
///
/// let mut engine = Engine::new(EngineConfig::default()).unwrap();
/// # struct Nop(Option<twine_hal::DrawBufferMem>);
/// # impl twine_hal::DisplayDriver for Nop {
/// #     type Error = ();
/// #     fn info(&self) -> twine_hal::DisplayInfo { twine_hal::DisplayInfo::new(64, 32, twine_core::ColorFormat::Rgb565) }
/// #     fn begin_flush(&mut self, _: twine_core::Rect, b: twine_hal::DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
/// #     fn poll_flush(&mut self) -> Option<twine_hal::DrawBufferMem> { self.0.take() }
/// # }
/// let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
/// let d = engine.add_display(Nop(None), BufferMode::partial_single(buf)).unwrap();
/// let rt = Runtime::take().unwrap();
/// let mut core = UiCore::mount(rt, &mut engine, d, |_cx| label("hi")).unwrap();
/// let _wake = core.update(&mut engine, twine_core::Instant::from_millis(0));
/// ```
///
/// **Waker.** Every `UiCore` has its own [`UiWaker`], set on its root scope
/// ([`Scope::set_ui_waker`]): channel handlers of this application wake this `UiCore` only.
/// [`mount`](Self::mount) leases one from a process-wide pool ([`WakerLease`]: `static`
/// slots first, no heap for the first few UIs, and returned on drop, so remounting does not
/// grow the heap); [`mount_with_waker`](Self::mount_with_waker) uses a `static` the
/// application declares (no pool, and interrupt handlers can name it before the UI exists).
/// Either way the update cycle reads it through one `&'static` reference. When the pool is
/// exhausted (see [`WakerLease`] § Exhausted pool) the mount still succeeds with the shared
/// fallback waker and raises [`FaultKind::Capacity`] with code
/// [`CapacityFault::WakerPool`](crate::CapacityFault::WakerPool).
///
/// **Runtime.** A `UiCore` belongs to the reactive runtime whose [`Runtime`] token it was
/// mounted with ([`runtime`](Self::runtime)); the token keeps it in that execution context.
///
/// **Engine commands.** Engine side effects issued while the engine is not lent — the
/// cleanups of a scope disposed outside [`update`](Self::update) (animations stopped, timers
/// removed, modals closed), [`AnimController`](crate::AnimController) methods,
/// [`ThemeHandle::set`](crate::ThemeHandle::set), [`ThemeHandle::set_mode`](crate::ThemeHandle::set_mode)
/// and [`MotionHandle::set`](crate::MotionHandle::set) called between updates — are queued on the
/// `UiCore` that owns the scope and applied at the start of the next `update`. The queue is a
/// fixed ring of [`DEFAULT_ENGINE_QUEUE_CAPACITY`] commands allocated at mount
/// ([`set_engine_queue_capacity`](Self::set_engine_queue_capacity)); a full queue drops the
/// command and raises [`FaultKind::Capacity`] with code
/// [`CapacityFault::EngineQueue`](crate::CapacityFault::EngineQueue) at the next update. Pass
/// the engine to [`dispose`](Self::dispose) when taking the application down: a `UiCore`
/// dropped without it cannot stop the animations and timers it leaves in the engine.
///
/// **Several displays.** One engine can drive several displays; [`mount_on`](Self::mount_on)
/// builds a further application on another display of the engine, in a child scope of the
/// root (its own theme, design elements and engine-command queue, see [`Ui::mount_on`]). All
/// of them run in this `UiCore`'s one update cycle, with its one waker: an update renders
/// every display that has something to draw, each with its own dirty areas.
///
/// **Sharing the runtime.** Several `UiCore`s (or `Ui`s) may share one runtime, each with its
/// own engine. Each update runs as its root's update ([`Scope::activate`]): it delivers only
/// its own channel handlers' messages ([`Scope::drain_channels`]), flushes its own effects,
/// and an effect of another `UiCore` that runs meanwhile (a signal shared by both changed)
/// sees no engine — a binding defers itself to its own `UiCore`'s next update and wakes it —
/// so a binding only ever writes into the engine it was built for.
pub struct UiCore {
    rt: Runtime,
    root: Scope,
    display: DisplayId,
    waker: &'static UiWaker,
    /// Engine commands issued without the engine (also provided on `root`).
    queue: Rc<EngineQueue>,
    /// The applications mounted on further displays ([`mount_on`](Self::mount_on)), in mount
    /// order (empty, and unallocated, for one display).
    others: Vec<DisplayApp>,
    /// Returns a pooled waker when the `UiCore` is dropped (`None` for an application's
    /// `static`). Declared after `root`'s disposal in `Drop`.
    lease: Option<WakerLease>,
    /// Capacity of every display's engine-command queue.
    queue_capacity: usize,
    /// Messages delivered per channel per update (at least 1).
    messages_per_channel: usize,
    /// The engine's motion preference as the `Ui`'s reactive layer sees it (also provided on
    /// `root`), and the value last written to it.
    motion: Signal<Motion>,
    motion_seen: Motion,
}

/// An application on a further display of a [`UiCore`] ([`UiCore::mount_on`]).
struct DisplayApp {
    display: DisplayId,
    /// The application's scope: a child of the `UiCore`'s root.
    scope: Scope,
    /// Its engine commands issued without the engine (provided on `scope`).
    queue: Rc<EngineQueue>,
}

impl core::fmt::Debug for UiCore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiCore")
            .field("root", &self.root)
            .field("display", &self.display)
            .field("displays", &(1 + self.others.len()))
            .finish_non_exhaustive()
    }
}

/// Builds `app` on the active screen of `display` in `scope` (inside one batch, with the
/// engine lent to [`EngineAccess`]); the caller has activated the scope's root. Provides the
/// display's [`UiContext`] and style anchor on `scope`. On a build failure the scope is
/// disposed again and the nodes it created are deleted.
fn build_app<V: View>(
    scope: Scope,
    engine: &mut Engine,
    display: DisplayId,
    queue: &Rc<EngineQueue>,
    motion: Signal<Motion>,
    app: impl FnOnce(Scope) -> V,
) -> Result<(), BuildError> {
    let rt = scope.runtime();
    let report = MountReport::begin(scope);
    let theme = scope.signal(ThemeState::of(engine, display));
    scope.provide(UiContext {
        display,
        queue: queue.clone(),
        theme,
        motion,
    });
    let anchor = StyleAnchor::default();
    scope.provide(anchor.clone());
    let parent = engine.active_screen(display);
    let mut built = None;
    EngineAccess::provide(rt, engine, || {
        rt.batch(|| {
            let view = app(scope);
            EngineAccess::with(rt, |e| {
                let parent = if let Some(p) = parent {
                    p
                } else {
                    twine_core::warn!(target: "twine::view", "Ui: display {} has no screen", display);
                    match e.create_root(Box::new(twine_engine::Obj)) {
                        Ok(r) => r,
                        Err(err) => {
                            let cause = BuildFailure::of(&err);
                            e.raise_fault(FaultRecord::new(FaultKind::BuildFailed).code(cause.code()));
                            report.record(BuildFault::new(cause, None));
                            return;
                        }
                    }
                };
                let mut cx = BuildCx::new(e, parent, scope);
                let node = view.build(&mut cx);
                anchor.set(node);
                built = Some(node);
            });
        });
    });
    forward_reactive_faults(rt, engine);
    // The build (including nested builds run by the batch's effects) is over: the report
    // decides; the `BuildFailed` faults raised meanwhile are telemetry.
    if let Err(err) = report.finish().into_result() {
        twine_core::error!(
            target: "twine::view",
            "ui build failed on display {}: {} widget(s) not created",
            display,
            err.failures
        );
        EngineAccess::provide(rt, engine, || scope.dispose());
        if let Some(n) = built.filter(|&n| engine.tree().contains(n)) {
            let _ = engine.delete(n);
        }
        return Err(err);
    }
    if let Some(n) = built.filter(|&n| engine.tree().contains(n)) {
        track_theme(engine, n, display, theme);
    }
    engine.update_layout();
    Ok(())
}

/// Moves the faults the reactive runtime recorded since the last call into the engine's fault
/// stream (one record per kind, with the number of occurrences), so the application sees every
/// fault in one place and through one hook.
fn forward_reactive_faults(rt: Runtime, engine: &mut Engine) {
    let counts = rt.take_faults();
    for kind in counts.kinds().iter() {
        engine.raise_fault(FaultRecord::new(kind).occurrences(counts.get(kind)));
    }
}

impl UiCore {
    /// Builds `app` on the active screen of `display` (inside one batch, with the engine lent
    /// to [`EngineAccess`]), lays it out, and sets a waker leased from the pool
    /// ([`UiWaker::lease`]) on the root scope, so the application's channels wake this UI.
    ///
    /// `rt` is the runtime of the calling context ([`Runtime::take`], or a copy of it); the
    /// `UiCore` keeps it.
    ///
    /// # Errors
    ///
    /// [`BuildError`] when a widget could not be created while `app` was built: the mount owns
    /// a [`BuildReport`](crate::BuildReport) that every [`BuildCx::create`](crate::BuildCx::create)
    /// failure of the application fills, including nested builds that run during the mount
    /// (each failure is also raised as a [`FaultKind::BuildFailed`] fault, as telemetry). The
    /// application is then taken down again: its scope is disposed and the nodes it created
    /// are deleted, so the engine is left as before (apart from the fault stream).
    pub fn mount<V: View>(
        rt: Runtime,
        engine: &mut Engine,
        display: DisplayId,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, BuildError> {
        Self::mount_inner(rt, engine, display, None, DEFAULT_ENGINE_QUEUE_CAPACITY, app)
    }

    /// [`mount`](Self::mount) with the application's own waker (typically a
    /// `static UiWaker`) instead of a pooled one. The waker should not be shared with another
    /// live UI (both would be woken by each other's channels).
    ///
    /// ```
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_view::{UiCore, prelude::*};
    /// # struct Nop(Option<twine_hal::DrawBufferMem>);
    /// # impl twine_hal::DisplayDriver for Nop {
    /// #     type Error = ();
    /// #     fn info(&self) -> twine_hal::DisplayInfo { twine_hal::DisplayInfo::new(64, 32, twine_core::ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: twine_core::Rect, b: twine_hal::DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<twine_hal::DrawBufferMem> { self.0.take() }
    /// # }
    /// static WAKER: UiWaker = UiWaker::new(); // an input interrupt may call `WAKER.wake()`
    /// # let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// # let d = engine.add_display(Nop(None), BufferMode::partial_single(buf)).unwrap();
    /// let rt = Runtime::take().unwrap();
    /// let core = UiCore::mount_with_waker(rt, &mut engine, d, &WAKER, |_cx| label("hi")).unwrap();
    /// assert!(core::ptr::eq(core.waker(), &WAKER));
    /// ```
    ///
    /// # Errors
    ///
    /// As [`mount`](Self::mount).
    pub fn mount_with_waker<V: View>(
        rt: Runtime,
        engine: &mut Engine,
        display: DisplayId,
        waker: &'static UiWaker,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, BuildError> {
        Self::mount_inner(
            rt,
            engine,
            display,
            Some(waker),
            DEFAULT_ENGINE_QUEUE_CAPACITY,
            app,
        )
    }

    /// [`mount`](Self::mount) with the application's [`AppConfig`], for hosts that create the
    /// engine themselves (the simulator, test harnesses, custom loops) and want exactly what
    /// [`UiBuilder::app_config`] gives a [`Ui`]: [`AppConfig::configure_engine`] on `display`
    /// (fault hook, motion, theme, rotation), then the mount with the configured engine-queue
    /// capacity and messages per channel. [`AppConfig::engine`] is what the host creates the
    /// engine with.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::{UiCore, prelude::*};
    ///
    /// let cfg = AppConfig::new().theme(DefaultTheme::light()).messages_per_channel(2);
    /// let mut engine = Engine::new(cfg.engine).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = engine.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// let core = UiCore::mount_configured(Runtime::take().unwrap(), &mut engine, d, &cfg, |_| label("hi")).unwrap();
    /// assert_eq!(core.messages_per_channel(), 2);
    /// assert!(engine.theme(d).is_some());
    /// # core.dispose(&mut engine);
    /// ```
    ///
    /// # Errors
    ///
    /// [`UiError::Engine`] from [`AppConfig::configure_engine`] (unknown display, a rotation
    /// on a framebuffer display); [`UiError::Build`] as [`mount`](Self::mount).
    pub fn mount_configured<V: View>(
        rt: Runtime,
        engine: &mut Engine,
        display: DisplayId,
        config: &AppConfig,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, UiError> {
        config.configure_engine(engine, display)?;
        let mut core = Self::mount_inner(rt, engine, display, None, config.engine_queue_capacity, app)?;
        core.set_messages_per_channel(config.messages_per_channel);
        Ok(core)
    }

    pub(crate) fn mount_inner<V: View>(
        rt: Runtime,
        engine: &mut Engine,
        display: DisplayId,
        app_waker: Option<&'static UiWaker>,
        queue_capacity: usize,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, BuildError> {
        let root = rt.create_root();
        let queue = EngineQueue::new(queue_capacity);
        let motion = root.signal(engine.motion());
        root.activate(|| build_app(root, engine, display, &queue, motion, app))?;
        // The waker is taken only once the build succeeded (a failed mount holds none).
        let (waker, lease) = if let Some(w) = app_waker {
            (w, None)
        } else {
            let lease = UiWaker::lease();
            if lease.is_shared() {
                engine.raise_fault(
                    FaultRecord::new(FaultKind::Capacity)
                        .display(display)
                        .code(CapacityFault::WakerPool.code()),
                );
            }
            (lease.get(), Some(lease))
        };
        root.set_ui_waker(waker);
        queue.set_waker(waker);
        twine_core::info!(
            target: "twine::view",
            "ui built on display {}: {} nodes, {} reactive nodes",
            display,
            engine.tree().len(),
            rt.stats().nodes
        );
        Ok(UiCore {
            rt,
            root,
            display,
            waker,
            queue,
            others: Vec::new(),
            queue_capacity,
            lease,
            messages_per_channel: DEFAULT_MESSAGES_PER_CHANNEL,
            motion,
            motion_seen: engine.motion(),
        })
    }

    /// Builds `app` on a further display of the engine (added by the host, e.g. with
    /// [`Engine::add_display`]), in a new child scope of the root that is returned: the
    /// application gets the display's own theme state, design elements and engine-command
    /// queue (its [`ThemeHandle`](crate::ThemeHandle) and [`use_theme`](crate::use_theme)
    /// work on its display), and runs in this `UiCore`'s update cycle and waker (see
    /// [`UiCore`] § Several displays). Contexts provided on the root scope (by the first
    /// display's application) are visible to it. Allocates the scope, the queue and the views
    /// once; updates allocate nothing more than with one display.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::{UiCore, prelude::*};
    ///
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = |w, h| MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565));
    /// let main = engine.add_display(panel(64, 32), BufferMode::alloc(BufferSpec::default())).unwrap();
    /// let aux = engine.add_display(panel(32, 16), BufferMode::alloc(BufferSpec::default())).unwrap();
    /// let mut core = UiCore::mount(Runtime::take().unwrap(), &mut engine, main, |_| label("main")).unwrap();
    /// let scope = core.mount_on(&mut engine, aux, |_| label("aux")).unwrap();
    /// assert_eq!(core.displays().collect::<Vec<_>>(), [main, aux]);
    /// assert_eq!(core.display_scope(aux), Some(scope));
    /// core.update(&mut engine, Instant::from_millis(0)); // both displays
    /// # core.dispose(&mut engine);
    /// ```
    ///
    /// # Errors
    ///
    /// [`UiError::Engine`] with [`EngineError::DisplayNotFound`] for a display the engine does
    /// not have, [`EngineError::InvalidConfig`] for a display that already runs an
    /// application of this `UiCore`; [`UiError::Build`] when a widget could not be created
    /// (see [`mount`](Self::mount); the scope is disposed and its nodes deleted again, the
    /// other displays' applications are untouched).
    pub fn mount_on<V: View>(
        &mut self,
        engine: &mut Engine,
        display: DisplayId,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<Scope, UiError> {
        if engine.display_info(display).is_none() {
            twine_core::error!(target: "twine::view", "mount_on: display {} not found", display);
            return Err(EngineError::DisplayNotFound(display).into());
        }
        if self.display_scope(display).is_some() {
            twine_core::error!(target: "twine::view", "mount_on: display {} already runs an application", display);
            return Err(
                EngineError::InvalidConfig("the display already runs an application of this Ui").into(),
            );
        }
        let root = self.root;
        let scope = root.child();
        let queue = EngineQueue::new(self.queue_capacity);
        queue.set_waker(self.waker);
        let motion = self.motion;
        root.activate(|| build_app(scope, engine, display, &queue, motion, app))?;
        twine_core::info!(target: "twine::view", "ui mounted on display {}", display);
        self.others.push(DisplayApp {
            display,
            scope,
            queue,
        });
        Ok(scope)
    }

    /// The displays this `UiCore` runs applications on: the one it was mounted on, then the
    /// ones added with [`mount_on`](Self::mount_on), in that order. Allocates nothing; never
    /// panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::{UiCore, prelude::*};
    ///
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let main = engine.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// let core = UiCore::mount(Runtime::take().unwrap(), &mut engine, main, |_| label("main")).unwrap();
    /// assert_eq!(core.displays().collect::<Vec<_>>(), [main]);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn displays(&self) -> impl Iterator<Item = DisplayId> + '_ {
        core::iter::once(self.display).chain(self.others.iter().map(|a| a.display))
    }

    /// The scope of the application on `display`: the root scope for the display the
    /// `UiCore` was mounted on, the child scope [`mount_on`](Self::mount_on) returned for a
    /// further one, `None` for a display without an application of this `UiCore`. Never
    /// panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::{UiCore, prelude::*};
    ///
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let main = engine.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// let core = UiCore::mount(Runtime::take().unwrap(), &mut engine, main, |_| label("main")).unwrap();
    /// assert_eq!(core.display_scope(main), Some(core.root_scope()));
    /// # core.dispose(&mut engine);
    /// ```
    #[must_use]
    pub fn display_scope(&self, display: DisplayId) -> Option<Scope> {
        if display == self.display {
            return Some(self.root);
        }
        self.others.iter().find(|a| a.display == display).map(|a| a.scope)
    }

    /// Whether engine commands wait in any display's queue (one flag read per display).
    #[inline]
    fn queues_pending(&self) -> bool {
        self.queue.is_pending() || self.others.iter().any(|a| a.queue.is_pending())
    }

    /// One update at `now`, in the documented order: 1 time, then the engine commands queued
    /// since the last update (see [`UiCore`] § Engine commands; effects deferred for lack of
    /// an engine run first, as they were requested earlier: e.g. an animation created, then
    /// paused, between updates), 2 channel messages, 3 inputs, 4 timers, 5 animations (2–5
    /// inside one batch, with the engine lent to the application code they run), 6 the effect
    /// flush (bindings), 7 layout, 8 refresh; returns 9 when to run again (the engine's
    /// deadline, or at once while effects, channel messages, engine commands or the waker are
    /// pending).
    #[inline]
    pub fn update(&mut self, engine: &mut Engine, now: Instant) -> Wake {
        self.update_budgeted(engine, now, StepBudget::UNLIMITED)
    }

    /// [`update`](Self::update) whose refresh (8) renders at most `budget` (see
    /// [`StepBudget`]): a frame cut short continues in the next update, and this one returns
    /// [`Wake::Now`]. See [`Ui::update_budgeted`].
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::MemoryDisplay;
    /// # use twine_view::{UiCore, prelude::*};
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = engine.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// let mut core = UiCore::mount(Runtime::take().unwrap(), &mut engine, d, |_cx| label("hi")).unwrap();
    /// let now = Instant::from_millis(0);
    /// assert_eq!(core.update_budgeted(&mut engine, now, StepBudget::chunks(1)), Wake::Now);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn update_budgeted(&mut self, engine: &mut Engine, now: Instant, budget: StepBudget) -> Wake {
        let t0 = engine.config().hires_timer.map(|f| f());
        engine.begin_step(now);
        if self.waker.take() {
            engine.notify_input_all();
        }
        let (rt, root) = (self.rt, self.root);
        // Everything that runs application code runs as this root's update: other roots'
        // handlers are not drained here, and their effects never see this engine.
        let messages = root.activate(|| {
            rt.batch(|| {
                if self.queues_pending() {
                    self.apply_queue(engine);
                }
                self.sync_motion(engine);
                let messages =
                    EngineAccess::provide(rt, engine, || root.drain_channels(self.messages_per_channel));
                engine.read_inputs(now);
                engine.run_timers(now);
                engine.run_anims(now);
                EffectCx::scoped(rt, engine, |ecx| rt.flush_effects_with(ecx));
                messages
            })
        });
        forward_reactive_faults(rt, engine);
        let t1 = engine.config().hires_timer.map(|f| f());
        let wake = engine.finish_step_budgeted(now, budget);
        let busy = root.has_pending_effects()
            || root.any_channel_pending()
            || self.waker.is_set()
            || self.queues_pending();
        let wake = if busy { Wake::Now } else { wake };
        if let (Some(a), Some(b), Some(c)) = (t0, t1, engine.config().hires_timer.map(|f| f())) {
            twine_core::debug!(
                target: "twine::view",
                "update: messages={} app={}us frame={}us -> {:?}",
                messages,
                b.saturating_duration_since(a).as_micros(),
                c.saturating_duration_since(b).as_micros(),
                wake
            );
        } else {
            twine_core::debug!(target: "twine::view", "update: messages={} -> {:?}", messages, wake);
        }
        wake
    }

    /// Keeps the `Ui`'s motion signal in step with the engine (a changed preference set on
    /// the engine directly, or by another `Ui` of the engine): one comparison per update.
    #[inline]
    fn sync_motion(&mut self, engine: &Engine) {
        let m = engine.motion();
        if m != self.motion_seen {
            self.motion_seen = m;
            self.motion.set_if_changed(m);
        }
    }

    /// Sets the engine's motion preference ([`Engine::set_motion`]) and the `Ui`'s tracked
    /// copy ([`MotionHandle::get`](crate::MotionHandle::get)) at once.
    /// Idempotent; never panics, allocates nothing.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::MemoryDisplay;
    /// # use twine_view::{UiCore, prelude::*};
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// let d = engine.add_display(panel, BufferMode::partial_single(buf)).unwrap();
    /// let mut core = UiCore::mount(Runtime::take().unwrap(), &mut engine, d, |_cx| label("hi")).unwrap();
    /// core.set_motion(&mut engine, Motion::Reduced);
    /// assert_eq!(engine.motion(), Motion::Reduced);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn set_motion(&mut self, engine: &mut Engine, motion: Motion) {
        engine.set_motion(motion);
        self.rt.batch(|| self.sync_motion(engine));
    }

    /// Applies the queued engine commands of every display, after the effects deferred
    /// before them. The caller has activated the root.
    #[cold]
    #[inline(never)]
    fn apply_queue(&self, engine: &mut Engine) {
        let rt = self.rt;
        EffectCx::scoped(rt, engine, |ecx| rt.flush_effects_with(ecx));
        self.queue.apply(engine, self.display);
        for a in &self.others {
            a.queue.apply(engine, a.display);
        }
    }

    /// Sets the number of engine commands the `UiCore` can queue (default
    /// [`DEFAULT_ENGINE_QUEUE_CAPACITY`]; see [`UiCore`] § Engine commands). Allocates the new
    /// ring now (configuration time), keeping the queued commands; queuing never allocates.
    /// Shrinking the ring below the number of commands already queued keeps the oldest ones that
    /// fit and drops the rest, reported at the next update like a full queue
    /// ([`FaultKind::Capacity`] with code
    /// [`CapacityFault::EngineQueue`](crate::CapacityFault::EngineQueue), the dropped commands as
    /// occurrences). A capacity of `0` drops every command issued without the engine. Never
    /// panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::MemoryDisplay;
    /// # use twine_view::{UiCore, prelude::*};
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// let d = engine.add_display(panel, BufferMode::partial_single(buf)).unwrap();
    /// let mut core = UiCore::mount(Runtime::take().unwrap(), &mut engine, d, |_cx| label("hi")).unwrap();
    /// core.set_engine_queue_capacity(64);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn set_engine_queue_capacity(&mut self, capacity: usize) {
        self.queue_capacity = capacity;
        self.queue.set_capacity(capacity);
        for a in &self.others {
            a.queue.set_capacity(capacity);
        }
    }

    /// Sets how many messages each channel delivers per update (default
    /// [`DEFAULT_MESSAGES_PER_CHANNEL`]; `0` counts as `1`, so every update makes progress). See
    /// [`UiBuilder::messages_per_channel`]. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::MemoryDisplay;
    /// # use twine_view::{UiCore, prelude::*};
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = engine.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// let mut core = UiCore::mount(Runtime::take().unwrap(), &mut engine, d, |_cx| label("hi")).unwrap();
    /// core.set_messages_per_channel(4);
    /// assert_eq!(core.messages_per_channel(), 4);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn set_messages_per_channel(&mut self, n: usize) {
        self.messages_per_channel = n.max(1);
    }

    /// Messages each channel delivers per update (see
    /// [`set_messages_per_channel`](Self::set_messages_per_channel)).
    #[must_use]
    pub fn messages_per_channel(&self) -> usize {
        self.messages_per_channel
    }

    /// The root scope: the scope of the application on the display the `UiCore` was mounted
    /// on (further displays' applications run in child scopes, see
    /// [`display_scope`](Self::display_scope)). Its waker and channel registrations are the
    /// `UiCore`'s.
    #[must_use]
    pub fn root_scope(&self) -> Scope {
        self.root
    }

    /// The reactive runtime the application runs in (the token it was mounted with).
    #[must_use]
    pub fn runtime(&self) -> Runtime {
        self.rt
    }

    /// Heap bytes of the engine command queues of every display of this `UiCore`.
    pub(crate) fn engine_queue_bytes(&self) -> usize {
        self.queue.heap_bytes() + self.others.iter().map(|d| d.queue.heap_bytes()).sum::<usize>()
    }

    /// The display the `UiCore` was mounted on (see [`displays`](Self::displays) for all).
    #[must_use]
    pub fn display(&self) -> DisplayId {
        self.display
    }

    /// The waker: set by channel sends and [`notify_input`](Self::notify_input); register a
    /// task [`Waker`](core::task::Waker) in it to be woken.
    #[must_use]
    pub fn waker(&self) -> &'static UiWaker {
        self.waker
    }

    /// Tells the next update that an input device changed (callable from anywhere through
    /// the [`waker`](Self::waker)).
    pub fn notify_input(&self) {
        self.waker.wake();
    }

    /// The faults raised since the last call (engine faults and the reactive runtime's, which
    /// are forwarded to the engine first), clearing them. See [`Ui::take_faults`].
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::MemoryDisplay;
    /// # use twine_view::{UiCore, prelude::*};
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// let d = engine.add_display(panel, BufferMode::partial_single(buf)).unwrap();
    /// let core = UiCore::mount(Runtime::take().unwrap(), &mut engine, d, |_cx| label("hi")).unwrap();
    /// engine.raise_fault(FaultRecord::new(FaultKind::Capacity));
    /// assert!(core.take_faults(&mut engine).contains(FaultKind::Capacity));
    /// assert!(core.take_faults(&mut engine).is_empty()); // cleared
    /// # core.dispose(&mut engine);
    /// ```
    pub fn take_faults(&self, engine: &mut Engine) -> Faults {
        forward_reactive_faults(self.rt, engine);
        engine.take_faults()
    }

    /// Disposes the application's scope with the engine lent to its cleanups (after applying
    /// the engine commands still queued), so it leaves no animation, timer or modal behind.
    /// Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_engine::{Engine, EngineConfig};
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::MemoryDisplay;
    /// # use twine_view::{UiCore, prelude::*};
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// let d = engine.add_display(panel, BufferMode::partial_single(buf)).unwrap();
    /// let core = UiCore::mount(Runtime::take().unwrap(), &mut engine, d, |_cx| label("hi")).unwrap();
    /// let root = core.root_scope();
    /// core.dispose(&mut engine);
    /// assert!(!root.is_alive());
    /// ```
    pub fn dispose(mut self, engine: &mut Engine) {
        self.shut_down(engine);
    }

    /// [`dispose`](Self::dispose) in place (for the owners of an engine: `Ui`, `AsyncUi`).
    pub(crate) fn shut_down(&mut self, engine: &mut Engine) {
        let (rt, root) = (self.rt, self.root);
        root.activate(|| {
            if self.queues_pending() {
                rt.batch(|| self.apply_queue(engine));
            }
            if root.is_alive() {
                EngineAccess::provide(rt, engine, || root.dispose());
            }
        });
    }
}

impl Drop for UiCore {
    fn drop(&mut self) {
        if self.root.is_alive() {
            self.root.dispose();
        }
        let unapplied = self.queue.len() + self.others.iter().map(|a| a.queue.len()).sum::<usize>();
        if unapplied > 0 {
            twine_core::warn!(
                target: "twine::view",
                "UiCore dropped without UiCore::dispose: {} engine command(s) (animations, timers, \
                 modals of its scopes) not applied",
                unapplied
            );
        }
        // The root (and with it every channel routed to the waker) is gone: the pooled waker
        // can go back, reset, for the next UI.
        drop(self.lease.take());
    }
}

/// How a display is added to the engine, given its draw buffers. Implemented by [`Partial`]
/// ([`Ui::builder`], [`DisplayBuilder::new`]) and [`Framebuffer`] ([`Ui::builder_fb`],
/// [`DisplayBuilder::framebuffer`]); sealed.
pub trait DisplaySetup: setup_seal::Sealed + 'static {
    /// Adds the display with `buffers` and returns its id.
    ///
    /// # Errors
    /// The engine's errors for the display and buffers (see [`Engine::add_display`]).
    fn add(self, engine: &mut Engine, buffers: BufferMode) -> Result<DisplayId, EngineError>;
}

/// A [`DisplayDriver`] flushed from partial draw buffers ([`Ui::builder`]).
#[derive(Debug)]
pub struct Partial<D>(pub(crate) D);

/// A [`FramebufferDisplay`] ([`Ui::builder_fb`]).
#[derive(Debug)]
pub struct Framebuffer<D>(pub(crate) D);

/// Seals [`DisplaySetup`].
pub(crate) mod setup_seal {
    /// Implemented by the display kinds of this crate only.
    pub trait Sealed {}
}

impl<D> setup_seal::Sealed for Partial<D> {}
impl<D> setup_seal::Sealed for Framebuffer<D> {}

impl<D: DisplayDriver + 'static> DisplaySetup for Partial<D> {
    /// Partial displays bring their buffers explicitly (the builder's typestate requires
    /// them): no heap memory is allocated implicitly.
    fn add(self, engine: &mut Engine, buffers: BufferMode) -> Result<DisplayId, EngineError> {
        engine.add_display(self.0, buffers)
    }
}

impl<D: FramebufferDisplay + 'static> DisplaySetup for Framebuffer<D> {
    /// [`BufferMode::Full`] unless the builder set another mode: the framebuffers belong to
    /// the driver, nothing is allocated.
    fn add(self, engine: &mut Engine, buffers: BufferMode) -> Result<DisplayId, EngineError> {
        engine.add_framebuffer_display(self.0, buffers)
    }
}

/// Attaches the draw accelerator to the engine (keeps the accelerator's concrete type until
/// the engine boxes it once).
type AccelSetter = Box<dyn FnOnce(&mut Engine)>;
/// Adds a further display and mounts its application ([`UiBuilder::display`]).
type ExtraDisplay = Box<dyn FnOnce(&mut Engine, &mut UiCore) -> Result<DisplayId, UiError>>;
/// Adds the first display: its id and its command-channel registration.
pub(crate) type AddedDisplay = (DisplayId, Option<CommandsSetter>);

/// Everything a builder holds besides its typestate parts and its first display: the
/// [`AppConfig`] and the settings that are not part of it. Shared by [`UiBuilder`] and
/// `AsyncUiBuilder`; not generic, so building it is one piece of code in the binary.
#[derive(Default)]
pub(crate) struct Settings {
    pub(crate) app: AppConfig,
    /// Further displays with their applications, mounted after the first one, in order.
    extra: Vec<ExtraDisplay>,
    accel: Option<AccelSetter>,
    waker: Option<&'static UiWaker>,
    /// Installed on the waker after mount (`Platform::notify`).
    pub(crate) notify: Option<fn()>,
    /// Registered as the runtime's interrupt probe (`Platform::in_interrupt`).
    pub(crate) interrupt_probe: Option<fn() -> bool>,
    /// Caller memory for the engine's layer buffer (`Engine::with_layer_buf`); `None`: heap.
    pub(crate) layer_buf: Option<&'static mut [u8]>,
}

impl Settings {
    pub(crate) fn accel(&mut self, accel: impl DrawAccel + 'static) {
        self.accel = Some(Box::new(move |e: &mut Engine| e.set_accel(accel)));
    }

    pub(crate) fn waker(&mut self, waker: &'static UiWaker) {
        self.waker = Some(waker);
    }

    /// Builds the engine and its first display (added by `add_display`) and mounts `app`:
    /// everything of a [`Ui`] but its time source (shared with the async runtime, whose time
    /// source is its `AsyncPlatform`). The configuration is applied around the generic
    /// display and mount steps by non-generic code.
    pub(crate) fn build<V: View>(
        mut self,
        rt: Runtime,
        add_display: impl FnOnce(&mut Engine) -> Result<AddedDisplay, UiError>,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<(Engine, UiCore), UiError> {
        let mut engine = self.new_engine()?;
        let (display, commands) = add_display(&mut engine)?;
        self.app.configure_display(&mut engine, display)?;
        if let Some(set_accel) = self.accel {
            set_accel(&mut engine);
        }
        let mut core = UiCore::mount_inner(
            rt,
            &mut engine,
            display,
            self.waker,
            self.app.engine_queue_capacity,
            app,
        )?;
        core.set_messages_per_channel(self.app.messages_per_channel);
        if let Some(register) = commands {
            register(core.root_scope(), display);
        }
        for mount in self.extra {
            mount(&mut engine, &mut core)?;
        }
        if let Some(notify) = self.notify {
            core.waker().set_notify(notify);
        }
        Ok((engine, core))
    }

    /// The engine, configured (fault hook and motion set before any display is added), with
    /// the caller's layer buffer if one was given.
    fn new_engine(&mut self) -> Result<Engine, UiError> {
        if let Some(probe) = self.interrupt_probe {
            twine_reactive::set_interrupt_probe(Some(probe));
        }
        let mut engine = match self.layer_buf.take() {
            Some(buf) => Engine::with_layer_buf(self.app.engine, buf)?,
            None => Engine::new(self.app.engine)?,
        };
        self.app.configure_engine_wide(&mut engine);
        Ok(engine)
    }
}

/// Configures and builds a [`Ui`] (see [`Ui::builder`]).
///
/// The required parts are type parameters ([`typestate`](crate::typestate)): `R` the reactive
/// runtime ([`runtime`](Self::runtime)), `C` the clock ([`platform`](Self::platform) or
/// [`clock`](Self::clock)), `B` the draw buffers of the first display
/// ([`buffers`](Self::buffers); a framebuffer display starts with [`BufferMode::Full`]).
/// [`build`](Self::build) and [`try_build`](Self::try_build) exist only when all three are
/// provided — a missing one is a compile error naming the call to add. Everything else is
/// optional, in any order; the application's own settings come as one [`AppConfig`]
/// ([`app_config`](Self::app_config)) or one by one.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::DisplayInfo;
/// use twine_testing::{MemoryDisplay, MockClock};
/// use twine_view::prelude::*;
///
/// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
/// let ui = Ui::builder(panel)
///     .runtime(Runtime::take().unwrap()) // required
///     .clock(MockClock::new()) // required (or `.platform(&p)`)
///     .buffers(BufferMode::alloc(BufferSpec::default())) // required for a partial display
///     .app_config(AppConfig::new().theme(DefaultTheme::light()))
///     .build(|_| label("hi"));
/// # let _ = ui;
/// ```
#[must_use]
pub struct UiBuilder<S, R = NoRuntime, C = NoClock, B = NoBuffers> {
    /// The first display and what belongs to it.
    pub(crate) display: DisplayBuilder<S, B>,
    pub(crate) settings: Settings,
    runtime: R,
    clock: C,
}

impl<S, R, C, B> core::fmt::Debug for UiBuilder<S, R, C, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiBuilder")
            .field("display", &self.display)
            .field("app", &self.settings.app)
            .field("extra_displays", &self.settings.extra.len())
            .field("accel", &self.settings.accel.is_some())
            .finish_non_exhaustive()
    }
}

impl<S, B> UiBuilder<S, NoRuntime, NoClock, B> {
    pub(crate) fn new(display: DisplayBuilder<S, B>) -> Self {
        Self {
            display,
            settings: Settings::default(),
            runtime: NoRuntime,
            clock: NoClock,
        }
    }
}

impl<S, R, C, B> UiBuilder<S, R, C, B> {
    /// The application's configuration ([`AppConfig`]: engine configuration, theme, motion,
    /// rotation, channel and queue budgets, fault hook) — the same value the simulator and the
    /// tests are given, so they run what ships. **Replaces** every setting it covers, including
    /// those set before by [`config`](Self::config), [`theme`](Self::theme),
    /// [`motion`](Self::motion), [`rotation`](Self::rotation),
    /// [`messages_per_channel`](Self::messages_per_channel),
    /// [`engine_queue_capacity`](Self::engine_queue_capacity) and
    /// [`fault_hook`](Self::fault_hook); those methods called afterwards refine it (e.g. the
    /// board's `hires_timer`). Never panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// fn config() -> AppConfig {
    ///     AppConfig::new().theme(DefaultTheme::light()).messages_per_channel(4)
    /// }
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .app_config(config())
    ///     .motion(Motion::Reduced) // a refinement after the shared configuration
    ///     .build(|_| label("hi"));
    /// assert_eq!(ui.messages_per_channel(), 4);
    /// assert_eq!(ui.motion(), Motion::Reduced);
    /// ```
    pub fn app_config(mut self, config: AppConfig) -> Self {
        self.settings.app = config;
        self
    }

    /// Adds a further display to the `Ui`, running `app` (see [`Ui::mount_on`], which does the
    /// same on a built `Ui`): `display` brings its own draw buffers (required, checked at
    /// compile time), inputs, theme, rotation reserve and command channel. Displays are
    /// mounted after the first one's application, in the order they were added. One engine,
    /// one update cycle and one waker drive all of them, so one run loop does
    /// ([`run::blocking`](crate::run::blocking), `twine_embassy::run`); each display keeps its
    /// own dirty areas, inputs, theme mode, rotation and power state.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = |w, h| MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565));
    /// let rt = Runtime::take().unwrap();
    /// let speed = rt.create_root().signal(0);
    /// let mut ui = Ui::builder(panel(96, 48))
    ///     .runtime(rt)
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .display(
    ///         DisplayBuilder::new(panel(64, 32)).buffers(BufferMode::alloc(BufferSpec::default())),
    ///         move |_| label(text!("{} km/h", speed.get())), // the cluster's second panel
    ///     )
    ///     .build(move |_| label(text!("speed {}", speed.get())));
    /// speed.set(42); // both displays follow at the next update
    /// ui.update();
    /// ```
    pub fn display<S2: DisplaySetup, B2: HasBuffers + 'static, V: View + 'static>(
        mut self,
        display: DisplayBuilder<S2, B2>,
        app: impl FnOnce(Scope) -> V + 'static,
    ) -> Self {
        self.settings.extra.push(Box::new(move |engine, core| {
            mount_display(core, engine, display, app)
        }));
        self
    }

    /// Applies the [`DisplayCmd`]s sent to `commands` from any context — another task, an
    /// interrupt, another thread or core (brightness, sleep, wake, rotation): each update
    /// delivers them (like any channel, see [`messages_per_channel`](Self::messages_per_channel))
    /// to [`Engine::display_command`] for the `Ui`'s display, and the engine applies them
    /// between frames (see [`Ui::set_rotation`], [`Ui::set_display_sleep`],
    /// [`Ui::set_brightness`]). A send wakes the `Ui`. The application owns the channel (the
    /// ports pattern: a `static` in firmware). A later call replaces the channel.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// static DISPLAY: Channel<DisplayCmd, 4> = Channel::new();
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let mut ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .display_commands(&DISPLAY)
    ///     .build(|_| label("hi"));
    /// DISPLAY.try_send(DisplayCmd::Sleep).unwrap(); // e.g. from a power-management task
    /// ui.update();
    /// assert!(ui.display_asleep());
    /// ```
    pub fn display_commands<const N: usize>(mut self, commands: &'static Channel<DisplayCmd, N>) -> Self {
        self.display.parts.display_commands(commands);
        self
    }

    /// Reserves, at build time, the memory software rotation needs in every rotation
    /// ([`Engine::reserve_rotation`]), so [`Ui::set_rotation`] (and an
    /// [`AppConfig::rotation`]) works without allocating on a panel that cannot rotate in
    /// hardware. Not needed for drivers that rotate in hardware (MIPI DCS panels with
    /// `MADCTL`). Building fails for a framebuffer display.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)); // no hw rotation
    /// let mut ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .reserve_rotation()
    ///     .build(|_| label("hi"));
    /// ui.set_rotation(Rotation::Deg90).unwrap();
    /// ui.update();
    /// assert_eq!(ui.display_info().rotation, Rotation::Deg90);
    /// ```
    pub fn reserve_rotation(mut self) -> Self {
        self.display.parts.reserve_rotation();
        self
    }

    /// The draw buffers of the display. **Required** for [`Ui::builder`] (partial displays):
    /// [`build`](Self::build) does not exist until they are given (a compile error, see
    /// [`typestate`](crate::typestate)) — the `Ui` never allocates heap buffers implicitly.
    /// [`Ui::builder_fb`] starts with [`BufferMode::Full`] (the driver's framebuffers). A
    /// later call replaces them.
    ///
    /// - Firmware: `static` buffers declared with [`draw_buffers!`](crate::draw_buffers),
    ///   passed with [`BufferMode::partial_double_from`] / [`BufferMode::partial_single_from`].
    /// - Heap: [`BufferMode::alloc`], allocated only once the display is accepted.
    ///
    /// Whether the memory fits the panel (size, alignment, mode) is checked when the `Ui` is
    /// built ([`try_build`](Self::try_build)).
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// draw_buffers!(static BUFS: 2 x 8 rows x 64 px @ Rgb565);
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::partial_double_from(BUFS.take().expect("taken once")))
    ///     .build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn buffers(self, m: BufferMode) -> UiBuilder<S, R, C, BufferMode> {
        UiBuilder {
            display: self.display.buffers(m),
            settings: self.settings,
            runtime: self.runtime,
            clock: self.clock,
        }
    }

    /// Adds an input device (any number). Keypads and encoders are attached to a default
    /// focus group.
    ///
    /// The device is fitted to the display when the `Ui` is built
    /// ([`InputDevice::fit_to_display`]): touch drivers take their coordinate transform from the
    /// display's rotation and native size, so pass the driver as constructed — no touch
    /// transform, native size or rotation table in application code.
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    /// use twine_core::{ColorFormat, Point, Rotation};
    /// use twine_hal::{DisplayInfo, InputData, InputDevice, InputKind, PointerData, TouchTransform};
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// /// A touch panel reporting native panel coordinates; shares its transform for the test.
    /// struct Touch(Rc<Cell<TouchTransform>>);
    /// impl InputDevice for Touch {
    ///     fn kind(&self) -> InputKind { InputKind::Pointer }
    ///     fn read(&mut self) -> InputData {
    ///         InputData::Pointer(PointerData { point: self.0.get().apply(10, 20), pressed: false })
    ///     }
    ///     fn fit_to_display(&mut self, info: &DisplayInfo) { self.0.set(TouchTransform::for_display(info)); }
    /// }
    ///
    /// let transform = Rc::new(Cell::new(TouchTransform::PASS_THROUGH));
    /// // A 240 × 320 panel turned to landscape.
    /// let panel = MemoryDisplay::new(DisplayInfo::new(320, 240, ColorFormat::Rgb565).with_rotation(Rotation::Deg90));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .input(Touch(transform.clone()))
    ///     .build(|_| label("hi"));
    /// assert_eq!(transform.get().apply(10, 20), Point::new(299, 10)); // fitted to the rotated display
    /// # drop(ui);
    /// ```
    pub fn input(mut self, d: impl InputDevice + 'static) -> Self {
        self.display.parts.input(d);
        self
    }

    /// Attaches a draw accelerator, taking ownership of it — and with it of the peripheral it
    /// drives (e.g. `twine_accel_stm32::Dma2d::new(PacRegs::new(p.DMA2D)?)`): every rendered
    /// chunk offers its fills, blits and glyph blending to it, and what it declines is drawn in
    /// software (see [`Engine::set_accel`]). A later call replaces it. Default: software only.
    ///
    /// Nothing else is needed after `build` (the accelerator lives in the engine, boxed once;
    /// one dynamic call per accelerated operation).
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_hal::DisplayInfo;
    /// use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// /// An accelerator that declines everything.
    /// struct Decline;
    /// impl DrawAccel for Decline {
    ///     fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult { AccelResult::Unsupported }
    ///     fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult { AccelResult::Unsupported }
    ///     fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult { AccelResult::Unsupported }
    ///     fn wait(&mut self) {}
    /// }
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .accel(Decline)
    ///     .build(|_| label("hi"));
    /// assert!(ui.engine().has_accel());
    /// ```
    pub fn accel(mut self, accel: impl DrawAccel + 'static) -> Self {
        self.settings.accel(accel);
        self
    }

    /// Caller memory for the engine's layer buffer (opacity groups, transforms, blend modes;
    /// ARGB8888) — typically a [`LayerBuffer`](crate::LayerBuffer) static — so that firmware
    /// keeps the renderer's largest scratch buffer out of the heap. Without it the engine
    /// allocates [`EngineConfig::layer_buf_bytes`](twine_engine::EngineConfig::layer_buf_bytes)
    /// on the heap. Like draw buffers, this is memory owned by the board, not part of the
    /// shared [`AppConfig`]. A later call replaces it.
    ///
    /// Shorter than 4 KiB: `build` fails with `UiError::Engine(EngineError::InvalidConfig(..))`
    /// ([`Engine::with_layer_buf`]). Never panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// static LAYER: LayerBuffer<{ 8 * 1024 }> = LayerBuffer::zeroed();
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .layer_buf(LAYER.take().expect("layer buffer taken once"))
    ///     .build(|_| label("hi"));
    /// let m = ui.memory_report();
    /// assert!(m.engine.layer_buf_static);
    /// assert_eq!(m.engine.layer_buf, 8 * 1024);
    /// ```
    pub fn layer_buf(mut self, buf: &'static mut [u8]) -> Self {
        self.settings.layer_buf = Some(buf);
        self
    }

    /// The clock (required, unless [`platform`](Self::platform) provides it): the time of
    /// every update, timer and animation. A later `clock` or `platform` call replaces it.
    /// Boxed once.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let clock = MockClock::new();
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(clock.clone()).build(|_| label("hi"));
    /// clock.advance(Duration::ms(5));
    /// assert_eq!(ui.now(), Instant::from_millis(5));
    /// ```
    pub fn clock(self, c: impl Clock + 'static) -> UiBuilder<S, R, Box<dyn Clock>, B> {
        UiBuilder {
            display: self.display,
            settings: self.settings,
            runtime: self.runtime,
            clock: Box::new(c),
        }
    }

    /// The platform the `Ui` runs on (see [`Platform`]), in one call:
    ///
    /// - a clone of `platform` becomes the `Ui`'s clock (replacing [`clock`](Self::clock)),
    ///   so the deadlines [`Ui::update`] returns are on the clock the loop waits with;
    /// - [`P::notify`](Platform::notify) is installed on the `Ui`'s waker
    ///   ([`UiWaker::set_notify`]), so every wake-up — a channel send, an input interrupt,
    ///   [`Ui::notify_input`] — also ends a [`Platform::wait`] in progress;
    /// - [`P::in_interrupt`](Platform::in_interrupt) becomes the reactive runtime's interrupt
    ///   probe ([`twine_reactive::set_interrupt_probe`]): with `twine-reactive`'s
    ///   `debug-checks` feature, using the runtime from interrupt context then panics.
    ///
    /// The application keeps `platform` and waits with it. Never panics; the clone is boxed
    /// once at build (as a [`clock`](Self::clock) is).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::{DisplayInfo, Platform};
    /// use twine_testing::{MemoryDisplay, MockPlatform};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut platform = MockPlatform::new(); // e.g. `CortexMPlatform::new(timer)`
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).platform(&platform).build(|_| label("hi"));
    /// for _ in 0..3 {
    ///     match ui.update() {
    ///         Wake::Now => {}
    ///         Wake::At(t) => platform.wait(Some(t)),
    ///         Wake::Idle | Wake::IdleFor(_) => platform.wait(None),
    ///     }
    /// }
    /// assert!(platform.waits() >= 1);
    /// ```
    pub fn platform<P: Platform + Clone + 'static>(self, platform: &P) -> UiBuilder<S, R, Box<dyn Clock>, B> {
        let mut b = self.clock(platform.clone());
        b.settings.notify = Some(P::notify);
        b.settings.interrupt_probe = Some(P::in_interrupt);
        b
    }

    /// The theme of the display: a theme value or a shared one ([`IntoTheme`]); sets
    /// [`AppConfig::theme`]. A later call replaces it.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .theme(DefaultTheme::dark())
    ///     .build(|_| label("hi"));
    /// assert!(ui.engine().theme(ui.display()).is_some());
    /// ```
    pub fn theme(mut self, t: impl IntoTheme) -> Self {
        self.settings.app.theme = Some(t.into_theme());
        self
    }

    /// The engine configuration; sets [`AppConfig::engine`].
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .config(EngineConfig { max_nodes: 256, ..EngineConfig::default() })
    ///     .build(|_| label("hi"));
    /// assert_eq!(ui.engine().config().max_nodes, 256);
    /// ```
    pub fn config(mut self, c: EngineConfig) -> Self {
        self.settings.app.engine = c;
        self
    }

    /// The rotation the screens are designed for; sets [`AppConfig::rotation`] (applied
    /// before the first frame, in hardware when the driver can, else in software with
    /// [`reserve_rotation`](Self::reserve_rotation)).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_rotation_control();
    /// let mut ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .rotation(Rotation::Deg90)
    ///     .build(|_| label("hi"));
    /// ui.update(); // applied before the first frame
    /// assert_eq!(ui.display_info().rotation, Rotation::Deg90);
    /// ```
    pub fn rotation(mut self, rotation: Rotation) -> Self {
        self.settings.app.rotation = Some(rotation);
        self
    }

    /// The function called for every fault (see [`Ui::set_fault_hook`]); sets
    /// [`AppConfig::fault_hook`]. Set here, it also sees the faults raised while the display
    /// is added and the application is built.
    /// The hook is a plain `fn` (see [`FaultHook`]): called synchronously for each fault, it must
    /// be short and must not panic.
    ///
    /// ```
    /// use core::sync::atomic::{AtomicU32, Ordering};
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// static FAULTS: AtomicU32 = AtomicU32::new(0);
    /// fn on_fault(r: &FaultRecord) {
    ///     FAULTS.fetch_add(r.occurrences, Ordering::Relaxed);
    /// }
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).fault_hook(on_fault).build(|_| label("hi"));
    /// ui.engine_mut().raise_fault(FaultRecord::new(FaultKind::Capacity));
    /// assert_eq!(FAULTS.load(Ordering::Relaxed), 1);
    /// ```
    pub fn fault_hook(mut self, hook: FaultHook) -> Self {
        self.settings.app.fault_hook = Some(hook);
        self
    }

    /// The `Ui`'s waker: the application's own (typically a `static UiWaker` that interrupt
    /// handlers name directly) instead of one leased from the pool (see
    /// [`UiCore::mount_with_waker`]).
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// static WAKER: UiWaker = UiWaker::new(); // an input interrupt may call `WAKER.wake()`
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).waker(&WAKER).build(|_| label("hi"));
    /// assert!(core::ptr::eq(ui.waker(), &WAKER));
    /// ```
    pub fn waker(mut self, waker: &'static UiWaker) -> Self {
        self.settings.waker(waker);
        self
    }

    /// How many engine commands the `Ui` can queue while the engine is not lent (default
    /// [`DEFAULT_ENGINE_QUEUE_CAPACITY`]; sets [`AppConfig::engine_queue_capacity`]): the
    /// cleanups of scopes disposed and the
    /// [`AnimController`](crate::AnimController) methods, [`ThemeHandle::set`](crate::ThemeHandle::set),
    /// [`ThemeHandle::set_mode`](crate::ThemeHandle::set_mode) and
    /// [`MotionHandle::set`](crate::MotionHandle::set) calls made outside [`Ui::update`],
    /// applied at the start of the next update. The ring is allocated once when the `Ui` is
    /// built; a full queue drops the command and raises [`FaultKind::Capacity`] with code
    /// [`CapacityFault::EngineQueue`](crate::CapacityFault::EngineQueue). A capacity of `0` is
    /// accepted: every such command is then dropped and reported (the engine side effects of
    /// handlers, effects and timers run by the `Ui` are applied at once and never queued). See
    /// [`UiCore`] § Engine commands.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).engine_queue_capacity(64).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn engine_queue_capacity(mut self, capacity: usize) -> Self {
        self.settings.app.engine_queue_capacity = capacity;
        self
    }

    /// How many messages each [`Channel`](twine_reactive::Channel) with an
    /// [`on_message`](Scope::on_message) handler delivers per update (default
    /// [`DEFAULT_MESSAGES_PER_CHANNEL`] = 16; `0` counts as `1`; sets
    /// [`AppConfig::messages_per_channel`]). Messages beyond it stay queued
    /// and the update returns [`Wake::Now`], so a burst is spread over several updates (frames
    /// keep being drawn in between) instead of stalling one. Raise it for bursty producers
    /// with large channels; lower it to bound the time an update spends in handlers. A
    /// [`Latest`](twine_reactive::Latest) always delivers its one newest value. Change it later
    /// with [`Ui::set_messages_per_channel`] ([`UiCore::set_messages_per_channel`]); it costs
    /// nothing per update. Never panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .clock(MockClock::new())
    ///     .messages_per_channel(64)
    ///     .build(|_| label("hi"));
    /// assert_eq!(ui.messages_per_channel(), 64);
    /// ```
    pub fn messages_per_channel(mut self, n: usize) -> Self {
        self.settings.app.messages_per_channel = n;
        self
    }

    /// The initial motion preference (default [`Motion::Full`]; sets [`AppConfig::motion`]):
    /// how much the UI animates, e.g. the platform's "reduce motion" accessibility setting.
    /// Applied before the application is built, so even its first animations honour it;
    /// change it at run time with [`Ui::set_motion`] or [`use_motion`](crate::use_motion).
    /// See [`Motion`] for what each level does to which animation.
    ///
    /// ```no_run
    /// # use twine_view::prelude::*;
    /// # fn app(_cx: Scope) -> impl View { label("hi") }
    /// # fn run(display: impl twine_hal::DisplayDriver + 'static, clock: impl twine_hal::Clock + 'static) {
    /// let ui = Ui::builder(display).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(clock).motion(Motion::Reduced).build(app);
    /// assert_eq!(ui.motion(), Motion::Reduced);
    /// # }
    /// ```
    pub fn motion(mut self, motion: Motion) -> Self {
        self.settings.app.motion = motion;
        self
    }

    /// The reactive runtime of the calling execution context (required: [`build`](Self::build)
    /// does not exist without it — a compile error): the token [`Runtime::take`] returned (or
    /// a copy of it). The `Ui` keeps it; because the token is `!Send`, the `Ui` and every
    /// reactive handle of the application stay in this context — interrupts and other tasks
    /// talk to it through [`Channel`](twine_reactive::Channel) and the
    /// [`waker`](Ui::waker). A later call replaces the token. Never panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let rt = Runtime::take().expect("runtime already taken"); // once, early in `main`
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(rt).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).build(|_| label("hi"));
    /// assert_eq!(ui.runtime(), rt);
    /// ```
    pub fn runtime(self, rt: Runtime) -> UiBuilder<S, Runtime, C, B> {
        UiBuilder {
            display: self.display,
            settings: self.settings,
            runtime: rt,
            clock: self.clock,
        }
    }

    /// Builds the `Ui`: creates the engine, adds the display, the theme and the inputs, then
    /// builds `app` (once) on the active screen. Exists only once the runtime, the clock and
    /// the draw buffers are given ([`typestate`](crate::typestate)).
    ///
    /// # Errors
    /// What only the device can tell: [`UiError::Engine`] with the engine's errors for the
    /// display, buffers and inputs (see [`Engine::add_display`]: a format not compiled in,
    /// buffers that do not fit the panel or are misaligned, too many displays or nodes) and
    /// for an [`AppConfig::rotation`] on a framebuffer display. Heap buffers
    /// ([`BufferMode::alloc`]) are allocated only once the display has been accepted.
    /// [`UiError::Build`]: a widget of `app` could not be created (e.g. more nodes than
    /// [`EngineConfig::max_nodes`](twine_engine::EngineConfig::max_nodes)); see
    /// [`UiCore::mount`]. The fault hook set with [`fault_hook`](Self::fault_hook) has seen
    /// every fault by then.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// // An A8 panel: no renderer draws A8, so the engine refuses the display at run time.
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::A8));
    /// let built = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .try_build(|_| label("hi"));
    /// assert!(matches!(built, Err(UiError::Engine(_))));
    /// ```
    pub fn try_build<V: View>(self, app: impl FnOnce(Scope) -> V) -> Result<Ui, UiError>
    where
        S: DisplaySetup,
        R: HasRuntime,
        C: HasClock,
        B: HasBuffers,
    {
        let clock = self.clock.into_part();
        let display = self.display;
        let (engine, core) = self
            .settings
            .build(self.runtime.into_part(), |e| display.add(e), app)?;
        Ok(Ui { engine, core, clock })
    }

    /// [`try_build`](Self::try_build), panicking on an error.
    ///
    /// # Panics
    /// When the engine rejects the display, buffers or inputs, or when a widget of `app`
    /// cannot be created (see [`try_build`](Self::try_build) § Errors). A missing runtime,
    /// clock or buffers is a compile error, not a panic.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .build(|_| label("hi"));
    /// ui.update();
    /// ```
    pub fn build<V: View>(self, app: impl FnOnce(Scope) -> V) -> Ui
    where
        S: DisplaySetup,
        R: HasRuntime,
        C: HasClock,
        B: HasBuffers,
    {
        match self.try_build(app) {
            Ok(ui) => ui,
            Err(e) => crate::error::build_failed("Ui", &e),
        }
    }
}

/// The declarative UI runtime: the engine, the application's root scope and the clock.
///
/// Call [`update`](Self::update) whenever the returned [`Wake`] asks for it; an idle UI
/// returns [`Wake::Idle`] and needs no CPU until an input interrupt or a channel message
/// ([`waker`](Self::waker)). [`run::blocking`](crate::run::blocking) is that loop over a
/// [`Platform`] (bare metal, an RTOS task, a host thread); `twine_embassy::run` the one for
/// embassy. A loop of your own is a few lines:
///
/// ```no_run
/// # use twine_view::prelude::*;
/// # fn app(_cx: Scope) -> impl View { label("hi") }
/// fn run(display: impl twine_hal::DisplayDriver + 'static, clock: impl twine_hal::Clock + 'static) -> ! {
///     let mut ui = Ui::builder(display).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(clock).theme(DefaultTheme::light()).build(app);
///     loop {
///         match ui.update() {
///             Wake::Idle | Wake::IdleFor(_) => { /* sleep until an interrupt */ }
///             Wake::At(_t) => { /* sleep until `t` or an interrupt */ }
///             Wake::Now => {}
///         }
///     }
/// }
/// ```
pub struct Ui {
    engine: Engine,
    core: UiCore,
    clock: Box<dyn Clock>,
}

impl core::fmt::Debug for Ui {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ui")
            .field("engine", &self.engine)
            .field("core", &self.core)
            .finish_non_exhaustive()
    }
}

impl Ui {
    /// A builder for a display flushed from partial draw buffers.
    ///
    /// The builder needs a runtime, a clock and draw buffers before it can build (see
    /// [`UiBuilder`] and [`typestate`](crate::typestate)).
    pub fn builder<D: DisplayDriver + 'static>(display: D) -> UiBuilder<Partial<D>> {
        UiBuilder::new(DisplayBuilder::new(display))
    }

    /// A builder for a memory-mapped framebuffer display (`Full` / `Direct` buffer modes).
    ///
    /// The builder starts with [`BufferMode::Full`] (the driver's framebuffers) and needs a
    /// runtime and a clock before it can build.
    pub fn builder_fb<D: FramebufferDisplay + 'static>(
        display: D,
    ) -> UiBuilder<Framebuffer<D>, NoRuntime, NoClock, BufferMode> {
        UiBuilder::new(DisplayBuilder::framebuffer(display))
    }

    /// One update (see [`UiCore::update`]) at the clock's current time. Returns when to run
    /// again; [`run::blocking`](crate::run::blocking) is the loop that honours it.
    pub fn update(&mut self) -> Wake {
        let now = self.clock.now();
        self.core.update(&mut self.engine, now)
    }

    /// One update that renders at most `budget` ([`StepBudget`]): the hook for schedulers
    /// that bound how long a step may take (an RTOS time slice, a super-loop with other
    /// duties). When the frame is cut short the update returns [`Wake::Now`] and the next
    /// update — budgeted or not — continues it; the finished frame has exactly the pixels of
    /// the same frame rendered by one [`update`](Self::update). Each update still runs the
    /// whole update cycle before rendering (messages, inputs, timers, animations, bindings,
    /// layout), so the UI stays responsive while a large frame is drawn.
    ///
    /// Allocation-free; never panics. With [`StepBudget::UNLIMITED`] it is
    /// [`update`](Self::update).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })) // 4 chunks per screen
    ///     .build(|_| label("hi"));
    /// let mut updates = 1;
    /// while ui.update_budgeted(StepBudget::chunks(2)) == Wake::Now {
    ///     updates += 1; // e.g. yield to the scheduler here
    /// }
    /// assert_eq!(updates, 2);
    /// ```
    #[doc(alias = "step_budgeted")]
    pub fn update_budgeted(&mut self, budget: StepBudget) -> Wake {
        let now = self.clock.now();
        self.core.update_budgeted(&mut self.engine, now, budget)
    }

    /// The current time of the `Ui`'s clock (or platform): the time base of the deadlines
    /// [`update`](Self::update) returns. Never panics itself (it calls the clock's `now`).
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let clock = MockClock::new();
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(clock.clone()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// clock.advance(Duration::ms(250));
    /// assert_eq!(ui.now(), Instant::from_millis(250));
    /// ```
    #[must_use]
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// Tells the next update that an input device changed (e.g. from a touch interrupt
    /// through [`waker`](Self::waker)).
    pub fn notify_input(&self) {
        self.core.notify_input();
    }

    /// The waker, set by channel sends and input notifications (`'static`, usable from
    /// interrupts and other tasks).
    #[must_use]
    pub fn waker(&self) -> &'static UiWaker {
        self.core.waker()
    }

    /// What the UI's memory is used for, by part: the engine (tree, render scratch and
    /// caches, glyph and image caches, layer buffer, draw buffers), the reactive runtime, the
    /// engine command queues and the waker pool ([`MemoryReport`](crate::MemoryReport): units,
    /// shared parts and what is not included).
    ///
    /// For budgeting a device (which part to shrink: `EngineConfig` cache budgets, a static
    /// [`layer_buf`](UiBuilder::layer_buf), smaller draw buffers), a debug console, and tests
    /// that pin the memory of a scene. Allocates nothing; never panics; O(nodes), so call it
    /// on demand, not every frame.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// twine_view::draw_buffers!(static BUFS: 2 x 8 rows x 64 px @ Rgb565);
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::partial_double_from(BUFS.take().unwrap()))
    ///     .build(|_| column((label("a"), label("b"))));
    /// let m = ui.memory_report();
    /// assert_eq!(m.engine.draw_buffers_static, 2 * 64 * 2 * 8);
    /// assert_eq!(m.engine.draw_buffers_heap, 0);
    /// assert_eq!(m.engine.nodes, ui.engine().tree().len());
    /// ```
    #[must_use]
    pub fn memory_report(&self) -> crate::MemoryReport {
        self.core.memory_report(&self.engine)
    }

    /// Messages each channel delivers per update (see [`UiBuilder::messages_per_channel`]).
    #[must_use]
    pub fn messages_per_channel(&self) -> usize {
        self.core.messages_per_channel()
    }

    /// Changes how many messages each channel delivers per update (see
    /// [`UiBuilder::messages_per_channel`]; `0` counts as `1`). Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).build(|_| label("hi"));
    /// ui.set_messages_per_channel(0);
    /// assert_eq!(ui.messages_per_channel(), 1);
    /// ```
    pub fn set_messages_per_channel(&mut self, n: usize) {
        self.core.set_messages_per_channel(n);
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The engine, mutably (full LVGL-style control).
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// The faults raised since the last call, clearing them: the engine's (display flush
    /// errors, capacity, build failures, …) and the reactive runtime's (effect loop cuts, the
    /// depth guard, dropped channel messages). [`Ui::update`] forwards the runtime's faults to
    /// the engine, so the fault hook sees them too.
    ///
    /// Twine only reports: what a fault means for the product and how to react is the
    /// application's decision.
    ///
    /// ```no_run
    /// # use twine_view::prelude::*;
    /// # fn check(ui: &mut Ui) {
    /// let faults = ui.take_faults();
    /// if faults.contains(FaultKind::FlushError)
    ///     && ui.display_health().is_some_and(|h| h.state == DisplayState::Failed)
    /// {
    ///     // e.g. re-initialise the panel, then redraw everything:
    ///     ui.recover_display();
    /// }
    /// # }
    /// ```
    pub fn take_faults(&mut self) -> Faults {
        self.core.take_faults(&mut self.engine)
    }

    /// Occurrences of every fault kind since start-up (saturating, never reset).
    #[must_use]
    pub fn fault_counts(&self) -> FaultCounts {
        self.engine.fault_counts()
    }

    /// The last record of `kind` (display, node, time, occurrences, detail code).
    #[must_use]
    pub fn last_fault(&self, kind: twine_core::fault::FaultKind) -> Option<&FaultRecord> {
        self.engine.last_fault(kind)
    }

    /// Sets the function called for every fault as it is raised (`None` removes it); see
    /// [`FaultHook`] for what it may do.
    pub fn set_fault_hook(&mut self, hook: Option<FaultHook>) {
        self.engine.set_fault_hook(hook);
    }

    /// The flush health of the display (see [`Engine::display_health`] and
    /// [`FlushPolicy`](twine_engine::FlushPolicy)).
    #[must_use]
    pub fn display_health(&self) -> Option<twine_engine::DisplayHealth> {
        self.engine.display_health(self.core.display())
    }

    /// The health of input device `id` as the engine last read it (see
    /// [`Engine::input_health`]; devices added with [`UiBuilder::input`] are numbered in the
    /// order they were added, see [`Engine::inputs`]). A device entering
    /// [`Degraded`](twine_hal::DeviceHealth::Degraded) or
    /// [`Failed`](twine_hal::DeviceHealth::Failed) also raises
    /// [`FaultKind::InputDevice`].
    #[must_use]
    pub fn input_health(&self, id: InputId) -> Option<twine_hal::DeviceHealth> {
        self.engine.input_health(id)
    }

    /// Recovers the display after flush failures: clears its error count, resumes a display
    /// halted by [`FlushPolicy::Halt`](twine_engine::FlushPolicy::Halt) and redraws it (see
    /// [`Engine::recover_display`]).
    pub fn recover_display(&mut self) {
        self.primary().recover();
    }

    /// The motion preference (see [`set_motion`](Self::set_motion)).
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).build(|_| label("hi"));
    /// assert_eq!(ui.motion(), Motion::Full); // the default
    /// ```
    #[must_use]
    pub fn motion(&self) -> Motion {
        self.engine.motion()
    }

    /// Sets the motion preference at run time (e.g. when the user toggles "reduce motion" in
    /// the settings): [`Engine::set_motion`], and the value [`use_motion`](crate::use_motion)
    /// reads (tracked) changes at once. Idempotent; never allocates or panics.
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).clock(MockClock::new()).build(|_| label("hi"));
    /// ui.set_motion(Motion::Reduced); // e.g. from a settings switch
    /// assert_eq!(ui.motion(), Motion::Reduced);
    /// ```
    pub fn set_motion(&mut self, motion: Motion) {
        self.core.set_motion(&mut self.engine, motion);
    }

    /// Switches the theme of the first display (every node is re-styled; the display is
    /// redrawn once): a theme value or a shared one ([`IntoTheme`]). Another display:
    /// [`display_mut`](Self::display_mut). Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// ui.set_theme(DefaultTheme::dark()); // e.g. from a settings screen
    /// ui.update();
    /// ```
    pub fn set_theme(&mut self, t: impl IntoTheme) {
        self.primary().set_theme(t);
    }

    /// Adds a further display to the `Ui` and builds `app` on it (the run-time counterpart of
    /// [`UiBuilder::display`]); returns its id. `display` brings its own draw buffers, inputs,
    /// theme, rotation reserve and command channel. The application runs in a child scope of
    /// the [root scope](Self::root_scope) (contexts the first display's application provided
    /// there are visible to it), with its own theme state and engine-command queue; it is
    /// updated by this `Ui`'s [`update`](Self::update) and woken by its [`waker`](Self::waker),
    /// so the run loop that drives the `Ui` drives it too. Each display keeps its own dirty
    /// areas (an update redraws only what changed on each), inputs, theme mode, rotation and
    /// power state ([`display_mut`](Self::display_mut)). Wakes the `Ui`.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = |w, h| MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel(96, 48))
    ///     .runtime(Runtime::take().unwrap())
    ///     .clock(MockClock::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::default()))
    ///     .build(|_| label("main"));
    /// // e.g. a service display plugged in at run time:
    /// let service = ui
    ///     .mount_on(
    ///         DisplayBuilder::new(panel(64, 32)).buffers(BufferMode::alloc(BufferSpec::default())),
    ///         |_| label("service"),
    ///     )
    ///     .unwrap();
    /// assert_eq!(ui.displays().collect::<Vec<_>>(), [ui.display(), service]);
    /// ui.update();
    /// ```
    ///
    /// # Errors
    ///
    /// [`UiError::Engine`] when the engine refuses the display, its buffers or inputs (see
    /// [`Engine::add_display`]); [`UiError::Build`] when a widget of `app` cannot be created
    /// (see [`UiCore::mount_on`]: the application is taken down again; the display, already
    /// added to the engine, stays blank). The `Ui`'s other displays are unaffected.
    pub fn mount_on<S: DisplaySetup, B: HasBuffers, V: View>(
        &mut self,
        display: DisplayBuilder<S, B>,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<DisplayId, UiError> {
        let id = mount_display(&mut self.core, &mut self.engine, display, app)?;
        self.core.waker().wake();
        Ok(id)
    }

    /// The displays of the `Ui`: the first one ([`display`](Self::display)), then those added
    /// with [`UiBuilder::display`] and [`mount_on`](Self::mount_on), in that order.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// assert_eq!(ui.displays().collect::<Vec<_>>(), [ui.display()]);
    /// ```
    pub fn displays(&self) -> impl Iterator<Item = DisplayId> + '_ {
        self.core.displays()
    }

    /// Run-time control of `display` (rotation, brightness, sleep, theme, theme mode,
    /// health; see [`DisplayMut`]); `None` for a display without an application of this
    /// `Ui`. The `Ui`'s own methods ([`set_rotation`](Self::set_rotation), …) act on the
    /// first display.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).theme(DefaultTheme::light()).build(|_| label("hi"));
    /// let d = ui.display();
    /// ui.display_mut(d).unwrap().set_theme_mode(ThemeMode::Dark);
    /// assert_eq!(ui.display_mut(d).unwrap().theme_mode(), ThemeMode::Dark);
    /// ```
    pub fn display_mut(&mut self, display: DisplayId) -> Option<DisplayMut<'_>> {
        self.core.display_scope(display)?;
        Some(DisplayMut {
            engine: &mut self.engine,
            core: &self.core,
            display,
        })
    }

    /// Run-time control of the first display.
    fn primary(&mut self) -> DisplayMut<'_> {
        DisplayMut {
            engine: &mut self.engine,
            core: &self.core,
            display: self.core.display(),
        }
    }

    /// The root scope: the scope of the first display's application (the applications of
    /// further displays run in child scopes, [`DisplayMut::scope`]).
    #[must_use]
    pub fn root_scope(&self) -> Scope {
        self.core.root_scope()
    }

    /// The reactive runtime the `Ui` runs in (the token given to [`UiBuilder::runtime`]):
    /// for code outside the view tree that batches writes, reads the runtime's statistics
    /// and memory, or takes its faults. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let rt = ui.runtime();
    /// assert!(rt.memory().scopes >= 1); // the view's scopes live in this runtime
    /// ```
    #[must_use]
    pub fn runtime(&self) -> Runtime {
        self.core.runtime()
    }

    /// The first display (the one given to [`Ui::builder`]; see [`displays`](Self::displays)
    /// for all).
    #[must_use]
    pub fn display(&self) -> DisplayId {
        self.core.display()
    }

    /// The description of the first display: its current logical size and rotation. Never
    /// panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let info = ui.display_info();
    /// assert_eq!((info.width, info.height, info.rotation), (64, 32, Rotation::Deg0));
    /// ```
    #[must_use]
    pub fn display_info(&self) -> twine_hal::DisplayInfo {
        display_info(&self.engine, self.core.display())
    }

    /// Rotates the display at the next update ([`Engine::set_rotation`]): the driver rotates
    /// in hardware, or the engine in software (needs [`UiBuilder::reserve_rotation`]); the
    /// draw format is checked again, the screens are laid out for the new size, the display is
    /// redrawn and touch input is fitted to the new rotation — all between two frames, without
    /// allocating. A rotation that does not fit raises
    /// [`FaultKind::DisplayControl`] and the display keeps its rotation. Wakes the `Ui`.
    /// Idempotent.
    ///
    /// # Errors
    /// [`UiError::Engine`] with [`EngineError::InvalidConfig`] for a framebuffer display.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_rotation_control();
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// ui.set_rotation(Rotation::Deg90).unwrap();
    /// ui.update();
    /// assert_eq!((ui.display_info().width, ui.display_info().height), (32, 64));
    /// ```
    pub fn set_rotation(&mut self, rotation: Rotation) -> Result<(), UiError> {
        self.primary().set_rotation(rotation)
    }

    /// Sets the panel brightness at the next update ([`Engine::set_display_brightness`]; a
    /// driver without brightness control raises [`FaultKind::DisplayControl`] — dim an LCD
    /// with its backlight instead). Wakes the `Ui`. Idempotent; never panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// ui.set_brightness(Fraction::pct(20));
    /// ui.update();
    /// assert_eq!(ui.engine().display_brightness(ui.display()), Some(Fraction::pct(20)));
    /// ```
    pub fn set_brightness(&mut self, level: Fraction) {
        self.primary().set_brightness(level);
    }

    /// Puts the display to sleep (`true`) or wakes it (`false`) at the next update
    /// ([`Engine::set_display_sleep`]): asleep, the panel sleeps and nothing is drawn (the UI
    /// keeps running; what changes is drawn after the wake). Wakes the `Ui`. Idempotent; never
    /// panics.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// ui.set_display_sleep(true);
    /// ui.update();
    /// assert!(ui.display_asleep());
    /// ```
    pub fn set_display_sleep(&mut self, sleep: bool) {
        self.primary().set_sleep(sleep);
    }

    /// Whether the first display is asleep ([`set_display_sleep`](Self::set_display_sleep),
    /// [`DisplayCmd::Sleep`]): the sleep request was applied by an update. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// assert!(!ui.display_asleep());
    /// ```
    #[must_use]
    pub fn display_asleep(&self) -> bool {
        self.engine.display_asleep(self.core.display())
    }

    /// The time since the last user input ([`Engine::inactive_for`] at the clock's current
    /// time): touch, keys, encoders, buttons, or [`trigger_activity`](Self::trigger_activity).
    /// With `EngineConfig::idle_timeout` set, an update with nothing to do returns
    /// [`Wake::IdleFor`] once this exceeds the timeout.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let clock = MockClock::new();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(clock.clone()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// ui.update();
    /// clock.advance(Duration::secs(10));
    /// assert_eq!(ui.inactive_for(), Duration::secs(10));
    /// ```
    #[doc(alias = "lv_display_get_inactive_time")]
    #[must_use]
    pub fn inactive_for(&self) -> Duration {
        self.engine.inactive_for(self.clock.now())
    }

    /// Records user input that did not come through an input device (e.g. a hardware button
    /// the application handles itself): restarts [`inactive_for`](Self::inactive_for) and the
    /// idle timeout. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let clock = MockClock::new();
    /// let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(clock.clone()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// ui.update();
    /// clock.advance(Duration::secs(5));
    /// ui.trigger_activity(); // the application read a hardware button
    /// assert_eq!(ui.inactive_for(), Duration::ZERO);
    /// ```
    #[doc(alias = "lv_display_trigger_activity")]
    pub fn trigger_activity(&mut self) {
        let now = self.clock.now();
        self.engine.trigger_activity(now);
    }
}

/// The description of `display` (a default one if the engine lost it, which cannot happen for
/// the `Ui`'s own display).
pub(crate) fn display_info(engine: &Engine, display: DisplayId) -> twine_hal::DisplayInfo {
    engine
        .display_info(display)
        .unwrap_or(twine_hal::DisplayInfo::new(0, 0, twine_core::ColorFormat::Rgb565))
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.core.shut_down(&mut self.engine);
    }
}
