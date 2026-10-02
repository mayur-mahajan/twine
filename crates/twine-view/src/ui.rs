//! The runtime: [`Ui`], [`UiBuilder`] and [`UiCore`].

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use twine_anim::Motion;
use twine_core::fault::{FaultCounts, FaultKind, Faults};
use twine_core::{Instant, Rotation};
use twine_engine::{
    BufferMode, DisplayId, Engine, EngineConfig, EngineError, FaultHook, FaultRecord, InputId, InputKind,
    ThemeHook, Wake,
};
use twine_hal::{Clock, DisplayDriver, FramebufferDisplay, InputDevice};
use twine_reactive::{
    Scope, Signal, UiWaker, WakerLease, any_channel_pending, batch, create_root, drain_channels,
    flush_effects_with, has_pending_effects,
};

use crate::access::{EffectCx, EngineAccess};
use crate::build::BuildCx;
use crate::engine_queue::{DEFAULT_ENGINE_QUEUE_CAPACITY, EngineQueue};
use crate::error::{BuildError, BuildFailure, UiError};
use crate::hooks::{StyleAnchor, ThemeState, UiContext, track_theme};
use crate::view::View;

/// Messages delivered per channel per update (the rest waits for the next update, which is
/// requested at once).
const MAX_MESSAGES_PER_CHANNEL: usize = 16;

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
/// let mut core = UiCore::mount(&mut engine, d, |_cx| label("hi")).unwrap();
/// let _wake = core.update(&mut engine, twine_core::Instant::from_millis(0));
/// ```
///
/// **Waker.** Every `UiCore` has its own [`UiWaker`], set on its root scope
/// ([`Scope::set_ui_waker`]): channel handlers of this application wake this `UiCore` only.
/// [`mount`](Self::mount) leases one from a process-wide pool ([`WakerLease`]: `static`
/// slots first, no heap for the first few UIs, and returned on drop, so remounting does not
/// grow the heap); [`mount_with_waker`](Self::mount_with_waker) uses a `static` the
/// application declares (no pool, and interrupt handlers can name it before the UI exists).
/// Either way the update cycle reads it through one `&'static` reference.
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
pub struct UiCore {
    root: Scope,
    display: DisplayId,
    waker: &'static UiWaker,
    /// Engine commands issued without the engine (also provided on `root`).
    queue: Rc<EngineQueue>,
    /// Returns a pooled waker when the `UiCore` is dropped (`None` for an application's
    /// `static`). Declared after `root`'s disposal in `Drop`.
    lease: Option<WakerLease>,
    /// The engine's motion preference as the `Ui`'s reactive layer sees it (also provided on
    /// `root`), and the value last written to it.
    motion: Signal<Motion>,
    motion_seen: Motion,
}

impl core::fmt::Debug for UiCore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiCore")
            .field("root", &self.root)
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

/// Moves the faults the reactive runtime recorded since the last call into the engine's fault
/// stream (one record per kind, with the number of occurrences), so the application sees every
/// fault in one place and through one hook.
fn forward_reactive_faults(engine: &mut Engine) {
    let counts = twine_reactive::take_faults();
    for kind in counts.kinds().iter() {
        engine.raise_fault(FaultRecord::new(kind).occurrences(counts.get(kind)));
    }
}

impl UiCore {
    /// Builds `app` on the active screen of `display` (inside one batch, with the engine lent
    /// to [`EngineAccess`]), lays it out, and sets a waker leased from the pool
    /// ([`UiWaker::lease`]) on the root scope, so the application's channels wake this UI.
    ///
    /// # Errors
    ///
    /// [`BuildError`] when a widget could not be created while `app` was built (each failure
    /// was also raised as a [`FaultKind::BuildFailed`] fault; see
    /// [`BuildCx::create`](crate::BuildCx::create)). The application is then taken down
    /// again: its scope is disposed and the nodes it created are deleted, so the engine is
    /// left as before (apart from the fault stream).
    pub fn mount<V: View>(
        engine: &mut Engine,
        display: DisplayId,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, BuildError> {
        Self::mount_inner(engine, display, None, DEFAULT_ENGINE_QUEUE_CAPACITY, app)
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
    /// let core = UiCore::mount_with_waker(&mut engine, d, &WAKER, |_cx| label("hi")).unwrap();
    /// assert!(core::ptr::eq(core.waker(), &WAKER));
    /// ```
    ///
    /// # Errors
    ///
    /// As [`mount`](Self::mount).
    pub fn mount_with_waker<V: View>(
        engine: &mut Engine,
        display: DisplayId,
        waker: &'static UiWaker,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, BuildError> {
        Self::mount_inner(engine, display, Some(waker), DEFAULT_ENGINE_QUEUE_CAPACITY, app)
    }

    fn mount_inner<V: View>(
        engine: &mut Engine,
        display: DisplayId,
        app_waker: Option<&'static UiWaker>,
        queue_capacity: usize,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<UiCore, BuildError> {
        let failed_before = engine.fault_counts().get(FaultKind::BuildFailed);
        let root = create_root();
        let queue = EngineQueue::new(queue_capacity);
        let theme = root.signal(ThemeState::of(engine, display));
        let motion = root.signal(engine.motion());
        root.provide(UiContext {
            display,
            queue: queue.clone(),
            theme,
            motion,
        });
        let anchor = StyleAnchor::default();
        root.provide(anchor.clone());
        let parent = engine.active_screen(display);
        let mut built = None;
        EngineAccess::provide(engine, || {
            batch(|| {
                let view = app(root);
                EngineAccess::with(|e| {
                    let parent = if let Some(p) = parent {
                        p
                    } else {
                        twine_core::warn!(target: "twine::view", "Ui: display {} has no screen", display);
                        match e.create_root(Box::new(twine_engine::Obj)) {
                            Ok(r) => r,
                            Err(err) => {
                                let cause = BuildFailure::of(&err);
                                e.raise_fault(FaultRecord::new(FaultKind::BuildFailed).code(cause.code()));
                                return;
                            }
                        }
                    };
                    let mut cx = BuildCx::new(e, parent, root);
                    let node = view.build(&mut cx);
                    anchor.set(node);
                    built = Some(node);
                });
            });
        });
        forward_reactive_faults(engine);
        let failures = engine
            .fault_counts()
            .get(FaultKind::BuildFailed)
            .saturating_sub(failed_before);
        if failures > 0 {
            let last = engine.last_fault(FaultKind::BuildFailed);
            let err = BuildError::new(
                failures,
                last.and_then(|r| BuildFailure::from_code(r.code))
                    .unwrap_or(BuildFailure::Engine),
                last.and_then(|r| r.node),
            );
            twine_core::error!(
                target: "twine::view",
                "ui build failed on display {}: {} widget(s) not created",
                display,
                failures
            );
            EngineAccess::provide(engine, || root.dispose());
            if let Some(n) = built.filter(|&n| engine.tree().contains(n)) {
                let _ = engine.delete(n);
            }
            return Err(err);
        }
        // The waker is taken only once the build succeeded (a failed mount holds none).
        let (waker, lease) = if let Some(w) = app_waker {
            (w, None)
        } else {
            let lease = UiWaker::lease();
            (lease.get(), Some(lease))
        };
        root.set_ui_waker(waker);
        queue.set_waker(waker);
        if let Some(n) = built.filter(|&n| engine.tree().contains(n)) {
            track_theme(engine, n, display, theme);
        }
        engine.update_layout();
        twine_core::info!(
            target: "twine::view",
            "ui built on display {}: {} nodes, {} reactive nodes",
            display,
            engine.tree().len(),
            twine_reactive::runtime_stats().nodes
        );
        Ok(UiCore {
            root,
            display,
            waker,
            queue,
            lease,
            motion,
            motion_seen: engine.motion(),
        })
    }

    /// One update at `now`, in the documented order: 1 time, then the engine commands queued
    /// since the last update (see [`UiCore`] § Engine commands; effects deferred for lack of
    /// an engine run first, as they were requested earlier: e.g. an animation created, then
    /// paused, between updates), 2 channel messages, 3 inputs, 4 timers, 5 animations (2–5
    /// inside one batch, with the engine lent to the application code they run), 6 the effect
    /// flush (bindings), 7 layout, 8 refresh; returns 9 when to run again (the engine's
    /// deadline, or at once while effects, channel messages, engine commands or the waker are
    /// pending).
    pub fn update(&mut self, engine: &mut Engine, now: Instant) -> Wake {
        let t0 = engine.config().hires_timer.map(|f| f());
        engine.begin_step(now);
        if self.waker.take() {
            engine.notify_input_all();
        }
        let messages = batch(|| {
            if self.queue.is_pending() {
                self.apply_queue(engine);
            }
            self.sync_motion(engine);
            let messages = EngineAccess::provide(engine, || drain_channels(MAX_MESSAGES_PER_CHANNEL));
            engine.read_inputs(now);
            engine.run_timers(now);
            engine.run_anims(now);
            EffectCx::scoped(engine, |ecx| flush_effects_with(ecx));
            messages
        });
        forward_reactive_faults(engine);
        let t1 = engine.config().hires_timer.map(|f| f());
        let wake = engine.finish_step(now);
        let busy =
            has_pending_effects() || any_channel_pending() || self.waker.is_set() || self.queue.is_pending();
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
    /// let mut core = UiCore::mount(&mut engine, d, |_cx| label("hi")).unwrap();
    /// core.set_motion(&mut engine, Motion::Reduced);
    /// assert_eq!(engine.motion(), Motion::Reduced);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn set_motion(&mut self, engine: &mut Engine, motion: Motion) {
        engine.set_motion(motion);
        batch(|| self.sync_motion(engine));
    }

    /// Applies the queued engine commands, after the effects deferred before them.
    #[cold]
    #[inline(never)]
    fn apply_queue(&self, engine: &mut Engine) {
        EffectCx::scoped(engine, |ecx| flush_effects_with(ecx));
        self.queue.apply(engine, self.display);
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
    /// let mut core = UiCore::mount(&mut engine, d, |_cx| label("hi")).unwrap();
    /// core.set_engine_queue_capacity(64);
    /// # core.dispose(&mut engine);
    /// ```
    pub fn set_engine_queue_capacity(&mut self, capacity: usize) {
        self.queue.set_capacity(capacity);
    }

    /// The root scope of the application.
    #[must_use]
    pub fn root_scope(&self) -> Scope {
        self.root
    }

    /// The display the application runs on.
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
    /// let core = UiCore::mount(&mut engine, d, |_cx| label("hi")).unwrap();
    /// engine.raise_fault(FaultRecord::new(FaultKind::Capacity));
    /// assert!(core.take_faults(&mut engine).contains(FaultKind::Capacity));
    /// assert!(core.take_faults(&mut engine).is_empty()); // cleared
    /// # core.dispose(&mut engine);
    /// ```
    pub fn take_faults(&self, engine: &mut Engine) -> Faults {
        forward_reactive_faults(engine);
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
    /// let core = UiCore::mount(&mut engine, d, |_cx| label("hi")).unwrap();
    /// let root = core.root_scope();
    /// core.dispose(&mut engine);
    /// assert!(!root.is_alive());
    /// ```
    pub fn dispose(mut self, engine: &mut Engine) {
        self.shut_down(engine);
    }

    /// [`dispose`](Self::dispose) in place (for the owners of an engine: `Ui`, `AsyncUi`).
    pub(crate) fn shut_down(&mut self, engine: &mut Engine) {
        if self.queue.is_pending() {
            batch(|| self.apply_queue(engine));
        }
        let root = self.root;
        if root.is_alive() {
            EngineAccess::provide(engine, || root.dispose());
        }
    }
}

impl Drop for UiCore {
    fn drop(&mut self) {
        if self.root.is_alive() {
            self.root.dispose();
        }
        if self.queue.len() > 0 {
            twine_core::warn!(
                target: "twine::view",
                "UiCore dropped without UiCore::dispose: {} engine command(s) (animations, timers, \
                 modals of its scopes) not applied",
                self.queue.len()
            );
        }
        // The root (and with it every channel routed to the waker) is gone: the pooled waker
        // can go back, reset, for the next UI.
        drop(self.lease.take());
    }
}

/// How the display is added to the engine.
pub trait DisplaySetup: 'static {
    /// Adds the display (`buffers`: `None` for the default) and returns its id.
    fn add(self, engine: &mut Engine, buffers: Option<BufferMode>) -> Result<DisplayId, EngineError>;
}

/// A [`DisplayDriver`] flushed from partial draw buffers ([`Ui::builder`]).
#[derive(Debug)]
pub struct Partial<D>(D);

/// A [`FramebufferDisplay`] ([`Ui::builder_fb`]).
#[derive(Debug)]
pub struct Framebuffer<D>(D);

/// Rows of the default partial draw buffer.
const DEFAULT_BUFFER_ROWS: u32 = 40;

impl<D: DisplayDriver + 'static> DisplaySetup for Partial<D> {
    fn add(self, engine: &mut Engine, buffers: Option<BufferMode>) -> Result<DisplayId, EngineError> {
        let buffers = buffers.unwrap_or_else(|| {
            let info = self.0.info();
            let (w, h) = if info.rotation == Rotation::Deg90 || info.rotation == Rotation::Deg270 {
                (info.height, info.width)
            } else {
                (info.width, info.height)
            };
            let w = u32::from(w.max(h));
            let rows = DEFAULT_BUFFER_ROWS.min(u32::from(h.max(1)));
            let len = info.format.stride(w) as usize * rows as usize;
            // One 4-byte aligned buffer, allocated once (like a `'static` MCU buffer).
            let v: &'static mut [u8] = Box::leak(alloc::vec![0u8; len + 3].into_boxed_slice());
            let off = v.as_ptr().align_offset(4).min(3);
            BufferMode::partial_single(&mut v[off..off + len])
        });
        engine.add_display(self.0, buffers)
    }
}

impl<D: FramebufferDisplay + 'static> DisplaySetup for Framebuffer<D> {
    fn add(self, engine: &mut Engine, buffers: Option<BufferMode>) -> Result<DisplayId, EngineError> {
        engine.add_framebuffer_display(self.0, buffers.unwrap_or(BufferMode::full()))
    }
}

/// Adds an input device to the engine.
type InputAdder = Box<dyn FnOnce(&mut Engine, DisplayId) -> Result<InputId, EngineError>>;

/// Configures and builds a [`Ui`] (see [`Ui::builder`]).
#[must_use]
pub struct UiBuilder<S: DisplaySetup> {
    pub(crate) display: S,
    config: EngineConfig,
    buffers: Option<BufferMode>,
    inputs: Vec<InputAdder>,
    clock: Option<Box<dyn Clock>>,
    theme: Option<Rc<dyn ThemeHook>>,
    fault_hook: Option<FaultHook>,
    waker: Option<&'static UiWaker>,
    queue_capacity: usize,
    motion: Motion,
}

impl<S: DisplaySetup> core::fmt::Debug for UiBuilder<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiBuilder")
            .field("inputs", &self.inputs.len())
            .field("clock", &self.clock.is_some())
            .field("theme", &self.theme.as_ref().map(|t| t.name()))
            .finish_non_exhaustive()
    }
}

impl<S: DisplaySetup> UiBuilder<S> {
    pub(crate) fn new(display: S) -> Self {
        Self {
            display,
            config: EngineConfig::default(),
            buffers: None,
            inputs: Vec::new(),
            clock: None,
            theme: None,
            fault_hook: None,
            waker: None,
            queue_capacity: DEFAULT_ENGINE_QUEUE_CAPACITY,
            motion: Motion::Full,
        }
    }

    /// The draw buffers (default: one 40-row partial buffer allocated on the heap for
    /// [`Ui::builder`], `Full` for [`Ui::builder_fb`]).
    pub fn buffers(mut self, m: BufferMode) -> Self {
        self.buffers = Some(m);
        self
    }

    /// Adds an input device (any number). Keypads and encoders are attached to a default
    /// focus group.
    pub fn input(mut self, d: impl InputDevice + 'static) -> Self {
        self.inputs.push(Box::new(move |e, disp| e.add_input(d, disp)));
        self
    }

    /// The clock (required).
    pub fn clock(mut self, c: impl Clock + 'static) -> Self {
        self.clock = Some(Box::new(c));
        self
    }

    /// The theme of the display.
    pub fn theme(mut self, t: impl ThemeHook + 'static) -> Self {
        self.theme = Some(Rc::new(t));
        self
    }

    /// The theme of the display, already shared.
    pub fn theme_rc(mut self, t: Rc<dyn ThemeHook>) -> Self {
        self.theme = Some(t);
        self
    }

    /// The engine configuration.
    pub fn config(mut self, c: EngineConfig) -> Self {
        self.config = c;
        self
    }

    /// The function called for every fault (see [`Ui::set_fault_hook`]). Set here, it also sees
    /// the faults raised while the application is built.
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
    /// let mut ui = Ui::builder(panel).clock(MockClock::new()).fault_hook(on_fault).build(|_| label("hi"));
    /// ui.engine_mut().raise_fault(FaultRecord::new(FaultKind::Capacity));
    /// assert_eq!(FAULTS.load(Ordering::Relaxed), 1);
    /// ```
    pub fn fault_hook(mut self, hook: FaultHook) -> Self {
        self.fault_hook = Some(hook);
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
    /// let ui = Ui::builder(panel).clock(MockClock::new()).waker(&WAKER).build(|_| label("hi"));
    /// assert!(core::ptr::eq(ui.waker(), &WAKER));
    /// ```
    pub fn waker(mut self, waker: &'static UiWaker) -> Self {
        self.waker = Some(waker);
        self
    }

    /// How many engine commands the `Ui` can queue while the engine is not lent (default
    /// [`DEFAULT_ENGINE_QUEUE_CAPACITY`]): the cleanups of scopes disposed and the
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
    /// let ui = Ui::builder(panel).clock(MockClock::new()).engine_queue_capacity(64).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn engine_queue_capacity(mut self, capacity: usize) -> Self {
        self.queue_capacity = capacity;
        self
    }

    /// The initial motion preference (default [`Motion::Full`]): how much the UI animates,
    /// e.g. the platform's "reduce motion" accessibility setting. Applied before the
    /// application is built, so even its first animations honour it; change it at run time
    /// with [`Ui::set_motion`] or [`use_motion`](crate::use_motion). See [`Motion`] for what
    /// each level does to which animation.
    ///
    /// ```no_run
    /// # use twine_view::prelude::*;
    /// # fn app(_cx: Scope) -> impl View { label("hi") }
    /// # fn run(display: impl twine_hal::DisplayDriver + 'static, clock: impl twine_hal::Clock + 'static) {
    /// let ui = Ui::builder(display).clock(clock).motion(Motion::Reduced).build(app);
    /// assert_eq!(ui.motion(), Motion::Reduced);
    /// # }
    /// ```
    pub fn motion(mut self, motion: Motion) -> Self {
        self.motion = motion;
        self
    }

    /// Binds the reactive runtime to the calling execution context (needed once without the
    /// `std` feature, where the runtime is a `static`; a no-op with `std`).
    ///
    /// # Safety
    ///
    /// The caller guarantees that from now on the `Ui` and every reactive handle (signals,
    /// scopes, …) are only used from this execution context: never from an interrupt handler,
    /// another core, or a task that can preempt it. Only [`Channel`](twine_reactive::Channel)
    /// and [`UiWaker`] may be used from other contexts (to send messages and wake the UI). See
    /// [`twine_reactive::bind_to_current_context`].
    #[allow(unsafe_code)]
    pub unsafe fn bind_to_current_context(self) -> Self {
        // SAFETY: forwarded; the caller upholds the single-context contract stated above.
        unsafe {
            twine_reactive::bind_to_current_context();
        }
        self
    }

    /// Builds the `Ui`: creates the engine, adds the display, the theme and the inputs, then
    /// builds `app` (once) on the active screen.
    ///
    /// # Errors
    /// [`UiError::Engine`]: [`EngineError::InvalidConfig`] without a clock; the engine's
    /// errors for the display, buffers and inputs. [`UiError::Build`]: a widget of `app`
    /// could not be created (e.g. more nodes than
    /// [`EngineConfig::max_nodes`](twine_engine::EngineConfig::max_nodes)); see
    /// [`UiCore::mount`]. The fault hook set with [`fault_hook`](Self::fault_hook) has seen
    /// every fault by then.
    pub fn try_build<V: View>(self, app: impl FnOnce(Scope) -> V) -> Result<Ui, UiError> {
        let (engine, core, clock) = self.build_parts(app)?;
        Ok(Ui { engine, core, clock })
    }

    /// The engine (display, theme, inputs added), the mounted application and the clock:
    /// everything of a [`Ui`] (shared with the async runtime).
    pub(crate) fn build_parts<V: View>(
        self,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<(Engine, UiCore, Box<dyn Clock>), UiError> {
        let Some(clock) = self.clock else {
            twine_core::error!(target: "twine::view", "Ui: no clock (UiBuilder::clock)");
            return Err(EngineError::InvalidConfig("Ui needs a clock").into());
        };
        let mut engine = Engine::new(self.config)?;
        engine.set_fault_hook(self.fault_hook);
        engine.set_motion(self.motion);
        let display = self.display.add(&mut engine, self.buffers)?;
        if let Some(t) = self.theme {
            engine.set_theme(display, t);
        }
        let mut focus_inputs = Vec::new();
        for add in self.inputs {
            let id = add(&mut engine, display)?;
            if matches!(
                engine.input_kind(id),
                Some(InputKind::Keypad | InputKind::Encoder)
            ) {
                focus_inputs.push(id);
            }
        }
        if !focus_inputs.is_empty() {
            let g = engine.create_group()?;
            engine.set_default_group(Some(g));
            for id in focus_inputs {
                engine.set_input_group(id, Some(g));
            }
        }
        let core = UiCore::mount_inner(&mut engine, display, self.waker, self.queue_capacity, app)?;
        Ok((engine, core, clock))
    }

    /// [`try_build`](Self::try_build), panicking on a configuration or build error.
    ///
    /// # Panics
    /// Without a clock, when the engine rejects the display, buffers or inputs, or when a
    /// widget of `app` cannot be created.
    pub fn build<V: View>(self, app: impl FnOnce(Scope) -> V) -> Ui {
        match self.try_build(app) {
            Ok(ui) => ui,
            Err(e) => panic!("twine: cannot build the Ui: {e:?}"),
        }
    }
}

/// The declarative UI runtime: the engine, the application's root scope and the clock.
///
/// Call [`update`](Self::update) whenever the returned [`Wake`] asks for it; an idle UI
/// returns [`Wake::Idle`] and needs no CPU until an input interrupt or a channel message
/// ([`waker`](Self::waker)).
///
/// ```no_run
/// # use twine_view::prelude::*;
/// # fn app(_cx: Scope) -> impl View { label("hi") }
/// fn run(display: impl twine_hal::DisplayDriver + 'static, clock: impl twine_hal::Clock + 'static) -> ! {
///     let mut ui = Ui::builder(display).clock(clock).theme(DefaultTheme::light()).build(app);
///     loop {
///         match ui.update() {
///             Wake::Idle => { /* sleep until an interrupt */ }
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
    pub fn builder<D: DisplayDriver + 'static>(display: D) -> UiBuilder<Partial<D>> {
        UiBuilder::new(Partial(display))
    }

    /// A builder for a memory-mapped framebuffer display (`Full` / `Direct` buffer modes).
    pub fn builder_fb<D: FramebufferDisplay + 'static>(display: D) -> UiBuilder<Framebuffer<D>> {
        UiBuilder::new(Framebuffer(display))
    }

    /// One update (see [`UiCore::update`]) at the clock's current time.
    pub fn update(&mut self) -> Wake {
        let now = self.clock.now();
        self.core.update(&mut self.engine, now)
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
        let _ = self.engine.recover_display(self.core.display());
    }

    /// The motion preference (see [`set_motion`](Self::set_motion)).
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::{MemoryDisplay, MockClock};
    /// use twine_view::prelude::*;
    ///
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let ui = Ui::builder(panel).clock(MockClock::new()).build(|_| label("hi"));
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
    /// let mut ui = Ui::builder(panel).clock(MockClock::new()).build(|_| label("hi"));
    /// ui.set_motion(Motion::Reduced); // e.g. from a settings switch
    /// assert_eq!(ui.motion(), Motion::Reduced);
    /// ```
    pub fn set_motion(&mut self, motion: Motion) {
        self.core.set_motion(&mut self.engine, motion);
    }

    /// Switches the theme (every node is re-styled; the display is redrawn once).
    pub fn set_theme(&mut self, t: impl ThemeHook + 'static) {
        let d = self.core.display();
        self.engine.set_theme(d, Rc::new(t));
    }

    /// The application's root scope.
    #[must_use]
    pub fn root_scope(&self) -> Scope {
        self.core.root_scope()
    }

    /// The display the application runs on.
    #[must_use]
    pub fn display(&self) -> DisplayId {
        self.core.display()
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.core.shut_down(&mut self.engine);
    }
}
