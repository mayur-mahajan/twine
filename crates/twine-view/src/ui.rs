//! The runtime: [`Ui`], [`UiBuilder`] and [`UiCore`].

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::task::{RawWaker, RawWakerVTable, Waker};

use twine_core::{Instant, Rotation};
use twine_engine::{
    BufferMode, DisplayId, Engine, EngineConfig, EngineError, InputId, InputKind, ThemeHook, Wake,
};
use twine_hal::{Clock, DisplayDriver, FramebufferDisplay, InputDevice};
use twine_reactive::{
    Scope, UiWaker, any_channel_pending, batch, create_root, drain_channels, flush_effects_with,
    has_pending_effects, register_waker,
};

use crate::access::{EffectCx, EngineAccess};
use crate::build::BuildCx;
use crate::hooks::UiDisplay;
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
/// let mut core = UiCore::mount(&mut engine, d, |_cx| label("hi"));
/// let _wake = core.update(&mut engine, twine_core::Instant::from_millis(0));
/// ```
pub struct UiCore {
    root: Scope,
    display: DisplayId,
    waker: &'static UiWaker,
}

impl core::fmt::Debug for UiCore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiCore")
            .field("root", &self.root)
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

static WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(waker_clone, waker_wake, waker_wake, waker_drop);

#[allow(unsafe_code)]
unsafe fn waker_clone(p: *const ()) -> RawWaker {
    RawWaker::new(p, &WAKER_VTABLE)
}

#[allow(unsafe_code)]
unsafe fn waker_wake(p: *const ()) {
    // SAFETY: every `RawWaker` with this vtable is created by `ui_waker` (or cloned from one)
    // from a `&'static UiWaker`, so `p` points to a live `UiWaker` forever. `UiWaker` is
    // `Sync` (an atomic flag and a critical-section mutex), so waking it from any thread or
    // interrupt is sound.
    let w = unsafe { &*p.cast::<UiWaker>() };
    w.wake();
}

#[allow(unsafe_code)]
unsafe fn waker_drop(_p: *const ()) {}

/// A task [`Waker`] that wakes `w` (sets its flag and wakes the task registered in it).
fn ui_waker(w: &'static UiWaker) -> Waker {
    let raw = RawWaker::new(core::ptr::from_ref(w).cast::<()>(), &WAKER_VTABLE);
    // SAFETY: the vtable functions uphold the `RawWaker` contract: the data pointer is a
    // `&'static UiWaker` (valid for the program's lifetime, so clones and drops need no
    // bookkeeping), and `wake`/`wake_by_ref` only call `UiWaker::wake`, which is thread- and
    // interrupt-safe (`UiWaker: Sync`).
    #[allow(unsafe_code)]
    unsafe {
        Waker::from_raw(raw)
    }
}

impl UiCore {
    /// Builds `app` on the active screen of `display` (inside one batch, with the engine lent
    /// to [`EngineAccess`]), lays it out, and registers the `Ui`'s waker with the reactive
    /// channels.
    pub fn mount<V: View>(engine: &mut Engine, display: DisplayId, app: impl FnOnce(Scope) -> V) -> UiCore {
        let root = create_root();
        root.provide(UiDisplay(display));
        let parent = engine.active_screen(display);
        EngineAccess::provide(engine, || {
            batch(|| {
                let view = app(root);
                EngineAccess::with(|e| {
                    let parent = if let Some(p) = parent {
                        p
                    } else {
                        twine_core::warn!(target: "twine::view", "Ui: display {} has no screen", display);
                        let Ok(r) = e.create_root(Box::new(twine_engine::Obj)) else {
                            return;
                        };
                        r
                    };
                    let mut cx = BuildCx::new(e, parent, root);
                    view.build(&mut cx);
                });
            });
        });
        let waker: &'static UiWaker = Box::leak(Box::new(UiWaker::new()));
        register_waker(&ui_waker(waker));
        engine.update_layout();
        twine_core::info!(
            target: "twine::view",
            "ui built on display {}: {} nodes, {} reactive nodes",
            display,
            engine.tree().len(),
            twine_reactive::debug_stats().nodes
        );
        UiCore { root, display, waker }
    }

    /// One update at `now`, in the documented order: 1 time, 2 channel messages, 3 inputs,
    /// 4 timers, 5 animations (2–5 inside one batch, with the engine lent to the application
    /// code they run), 6 the effect flush (bindings), 7 layout, 8 refresh; returns 9 when to
    /// run again (the engine's deadline, or at once while effects, channel messages or the
    /// waker are pending).
    pub fn update(&mut self, engine: &mut Engine, now: Instant) -> Wake {
        let t0 = engine.config().hires_timer.map(|f| f());
        engine.begin_step(now);
        if self.waker.take() {
            engine.notify_input_all();
        }
        let messages = batch(|| {
            let messages = EngineAccess::provide(engine, || drain_channels(MAX_MESSAGES_PER_CHANNEL));
            engine.read_inputs(now);
            engine.run_timers(now);
            engine.run_anims(now);
            EffectCx::scoped(engine, |ecx| flush_effects_with(ecx));
            messages
        });
        let t1 = engine.config().hires_timer.map(|f| f());
        let wake = engine.finish_step(now);
        let busy = has_pending_effects() || any_channel_pending() || self.waker.is_set();
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
    /// task [`Waker`] in it to be woken.
    #[must_use]
    pub fn waker(&self) -> &'static UiWaker {
        self.waker
    }

    /// Tells the next update that an input device changed (callable from anywhere through
    /// the [`waker`](Self::waker)).
    pub fn notify_input(&self) {
        self.waker.wake();
    }

    /// Disposes the application's scope with the engine lent to its cleanups.
    pub fn dispose(self, engine: &mut Engine) {
        let root = self.root;
        EngineAccess::provide(engine, || root.dispose());
    }
}

impl Drop for UiCore {
    fn drop(&mut self) {
        if self.root.is_alive() {
            self.root.dispose();
        }
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
    /// [`EngineError::InvalidConfig`] without a clock; the engine's errors for the display,
    /// buffers and inputs.
    pub fn try_build<V: View>(self, app: impl FnOnce(Scope) -> V) -> Result<Ui, EngineError> {
        let (engine, core, clock) = self.build_parts(app)?;
        Ok(Ui { engine, core, clock })
    }

    /// The engine (display, theme, inputs added), the mounted application and the clock:
    /// everything of a [`Ui`] (shared with the async runtime).
    pub(crate) fn build_parts<V: View>(
        self,
        app: impl FnOnce(Scope) -> V,
    ) -> Result<(Engine, UiCore, Box<dyn Clock>), EngineError> {
        let Some(clock) = self.clock else {
            twine_core::error!(target: "twine::view", "Ui: no clock (UiBuilder::clock)");
            return Err(EngineError::InvalidConfig("Ui needs a clock"));
        };
        let mut engine = Engine::new(self.config)?;
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
        let core = UiCore::mount(&mut engine, display, app);
        Ok((engine, core, clock))
    }

    /// [`try_build`](Self::try_build), panicking on a configuration error.
    ///
    /// # Panics
    /// Without a clock or when the engine rejects the display, buffers or inputs.
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
        let root = self.core.root;
        if root.is_alive() {
            EngineAccess::provide(&mut self.engine, || root.dispose());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_waker_sets_the_flag_through_clones() {
        static W: UiWaker = UiWaker::new();
        let w: &'static UiWaker = &W;
        let waker = ui_waker(w);
        let clone = waker.clone();
        drop(waker);
        clone.wake_by_ref();
        assert!(w.take());
        clone.wake();
        assert!(w.take());
        assert!(!w.is_set());
    }
}
