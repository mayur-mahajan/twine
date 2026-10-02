//! Scope hooks ([`ScopeExt`]): node references, tweens, animations, timers, modals, themes and
//! the motion preference.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
use core::cell::{Cell, RefCell};

use twine_anim::{Anim, AnimId, AnimSpec, EASING_ONE, Interpolate, Motion, TimerId};
use twine_core::Duration;
use twine_engine::{
    DisplayId, Engine, EventCode, EventFilter, EventResult, NodeId, ThemeHook, ThemeMode, Widget,
};
use twine_reactive::{
    ReadSignal, Scope, Signal, StoredValue, batch, defer_current_effect, dispose_current_effect, untrack,
};
use twine_style::design::{Element, ElementType};

use crate::access::{EngineAccess, no_engine};
use crate::engine_queue::{EngineCmd, EngineQueue, defer};
use crate::nav::{ModalHandle, show_modal};
use crate::node_ref::NodeRef;
use crate::view::View;

/// What every scope of a `Ui` can reach of it, provided as context in its root scope: the
/// display it runs on, its engine command queue and its theme state (one context: one lookup
/// type, less code on firmware).
#[derive(Clone)]
pub(crate) struct UiContext {
    pub(crate) display: DisplayId,
    pub(crate) queue: Rc<EngineQueue>,
    /// The display's theme mode and design epoch, as the `Ui` last saw them.
    pub(crate) theme: Signal<ThemeState>,
    /// The engine's motion preference, as the `Ui` last saw it.
    pub(crate) motion: Signal<Motion>,
}

/// The theme of a `Ui`'s display as its reactive layer sees it: the mode, the theme's modes
/// and the engine's design epoch (changes with any theme, mode or element table change).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThemeState {
    mode: ThemeMode,
    modes: &'static [ThemeMode],
    epoch: u32,
}

impl ThemeState {
    /// The current state of `display`.
    pub(crate) fn of(e: &Engine, display: DisplayId) -> Self {
        Self {
            mode: e.theme_mode(display),
            modes: e.theme_modes(display),
            epoch: e.design_epoch(display),
        }
    }
}

/// Keeps `theme` in step with the engine: a theme change, a mode switch or a table swap
/// re-styles every node of the display, so the `Ui`'s root node `node` receives
/// `StyleChanged`; its handler compares the engine's state with the last one seen (a plain
/// comparison, no reactive access) and writes the signal only when it changed. No per-update
/// cost; catches changes made through the engine directly too.
pub(crate) fn track_theme(e: &mut Engine, node: NodeId, display: DisplayId, theme: Signal<ThemeState>) {
    let mut seen = ThemeState::of(e, display);
    e.add_event_handler(node, EventFilter::Code(EventCode::StyleChanged), move |ecx, _| {
        let now = ThemeState::of(ecx.engine(), display);
        if now != seen {
            seen = now;
            theme.set(now);
        }
        EventResult::Continue
    });
}

/// The root node of the view that owns a scope — the `Ui`'s app view, a navigator screen's
/// view or a modal's view — provided as context. A modal inherits its text style (font,
/// color, base direction) from it, although its nodes live on the top layer.
#[derive(Clone, Debug, Default)]
pub(crate) struct StyleAnchor(Rc<Cell<Option<NodeId>>>);

impl StyleAnchor {
    /// An anchor at `node`.
    pub(crate) fn new(node: NodeId) -> Self {
        Self(Rc::new(Cell::new(Some(node))))
    }

    /// The anchor node (`None` before the view is built).
    pub(crate) fn get(&self) -> Option<NodeId> {
        self.0.get()
    }

    /// Sets the anchor node once the view is built.
    pub(crate) fn set(&self, node: NodeId) {
        self.0.set(Some(node));
    }
}

/// The display of the `Ui` that owns `cx` (or the engine's default display).
pub(crate) fn display_of(cx: Scope, e: &Engine) -> Option<DisplayId> {
    cx.use_context::<UiContext>()
        .map(|c| c.display)
        .or_else(|| e.default_display())
}

/// Runs `f` with the engine once one is available: at once when the engine is lent (while
/// the `Ui` builds or runs), else at the next effect flush of the `Ui`. Nothing happens if
/// `cx` is disposed before.
pub(crate) fn with_engine_once(cx: Scope, f: impl FnOnce(&mut Engine) + 'static) {
    let mut f = Some(f);
    cx.effect_with_cx(move |_: &mut dyn Any| {
        let Some(g) = f.take() else { return };
        let mut g = Some(g);
        let ran = EngineAccess::with(|e| {
            if let Some(g) = g.take() {
                g(e);
            }
        });
        if ran.is_some() {
            dispose_current_effect();
        } else {
            f = g.take();
            defer_current_effect();
        }
    });
}

/// Applies an animation's eased progress (`0..=1024`, may overshoot) to its value.
type ApplyFn = Rc<dyn Fn(&mut Engine, i32)>;

/// The state of a [`tween`](ScopeExt::tween).
struct Tween<T> {
    anim: Option<AnimId>,
    /// The motion preference the running animation was started under.
    motion: Motion,
    from: T,
    to: T,
}

/// Controls a free-running animation created with [`ScopeExt::animation`]. `Copy`, like a
/// signal; its state is owned by the scope that created the animation (once that scope is
/// disposed every method does nothing and `is_playing` is `false`).
///
/// The methods act at once where the engine is available — inside handlers, effects, timers
/// and while building (see [`ScopeExt`] § Where the engine is available). Called elsewhere
/// (the main loop, between updates) they are queued and take effect at the start of the next
/// `Ui::update`, in call order. [`is_playing`](Self::is_playing) needs the engine to answer:
/// without it, it returns `false` and, in debug builds, logs `warn!` once per call site.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AnimController {
    state: StoredValue<CtlState>,
}

struct CtlState {
    id: Option<AnimId>,
    spec: AnimSpec,
    /// The motion preference the running animation was started under.
    motion: Motion,
    /// Writes a progress value to the output signal (type-erased: the controller is not
    /// generic over the animated type).
    apply: ApplyFn,
    /// The scope that created the animation (finds its `Ui`'s engine command queue).
    cx: Scope,
}

/// An [`AnimController`] method, queued while the engine is not lent.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AnimOp {
    Pause,
    Resume,
    Restart,
    Stop,
    SetPlaying(bool),
}

impl core::fmt::Debug for AnimController {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimController").field("id", &self.id()).finish()
    }
}

impl AnimController {
    /// Runs `op` now with the engine, or queues it for the next update of the `Ui`.
    fn control(self, op: AnimOp) {
        if EngineAccess::with(|e| self.apply(e, op)).is_none() {
            if let Some(cx) = self.state.try_with(|s| s.cx) {
                defer(cx, EngineCmd::anim(self, op));
            }
        }
    }

    /// The running animation (`None` when stopped or disposed).
    fn id(self) -> Option<AnimId> {
        self.state.try_with(|s| s.id).flatten()
    }

    /// Takes the running animation's id.
    fn take_id(self) -> Option<AnimId> {
        self.state.try_with_mut(|s| s.id.take()).flatten()
    }

    fn start(e: &mut Engine, ctl: Self) {
        let Some((spec, apply)) = ctl.state.try_with(|s| (s.spec, s.apply.clone())) else {
            return; // disposed with its scope
        };
        let motion = e.motion();
        let id = e.anim_start_fn(Anim::with_spec(0, EASING_ONE, spec), move |e, v| apply(e, v));
        if let Some(old) = ctl.state.with_mut(|s| {
            s.motion = motion;
            s.id.replace(id)
        }) {
            e.anim_stop(old);
        }
    }

    /// Whether the running animation was started under another motion preference than the
    /// engine's current one (a restart then starts it anew, with the full spec adjusted to
    /// the current preference).
    fn motion_changed(self, e: &Engine) -> bool {
        self.state.try_with(|s| s.motion != e.motion()).unwrap_or(false)
    }

    /// Applies `op` (a controller of a disposed scope does nothing).
    pub(crate) fn apply(self, e: &mut Engine, op: AnimOp) {
        match (op, self.id()) {
            (AnimOp::Pause | AnimOp::SetPlaying(false), Some(id)) => {
                e.anim_pause(id);
            }
            (AnimOp::Resume, Some(id)) => {
                e.anim_resume(id);
            }
            (AnimOp::Restart, Some(id)) if e.anim_exists(id) && !self.motion_changed(e) => {
                e.anim_restart(id);
            }
            (AnimOp::SetPlaying(true), Some(id)) if e.anim_exists(id) => {
                e.anim_resume(id);
            }
            (AnimOp::Restart | AnimOp::SetPlaying(true), _) => Self::start(e, self),
            (AnimOp::Stop, _) => {
                if let Some(id) = self.take_id() {
                    e.anim_stop(id);
                }
            }
            (AnimOp::Pause | AnimOp::Resume | AnimOp::SetPlaying(false), None) => {}
        }
    }

    /// Pauses at the current value.
    pub fn pause(&self) {
        self.control(AnimOp::Pause);
    }

    /// Continues after [`pause`](Self::pause).
    pub fn resume(&self) {
        self.control(AnimOp::Resume);
    }

    /// Starts again from the beginning (also after the animation ended or was stopped).
    pub fn restart(&self) {
        self.control(AnimOp::Restart);
    }

    /// Stops the animation (the value stays where it is).
    pub fn stop(&self) {
        self.control(AnimOp::Stop);
    }

    /// Plays (`true`, resuming or restarting) or pauses (`false`).
    pub fn set_playing(&self, on: bool) {
        self.control(AnimOp::SetPlaying(on));
    }

    /// Whether the animation exists and is not paused (`false` without the engine).
    #[must_use]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn is_playing(&self) -> bool {
        let id = self.id();
        let playing = EngineAccess::with(|e| id.is_some_and(|id| e.anim_exists(id) && !e.anim_is_paused(id)));
        if let Some(p) = playing {
            p
        } else {
            // Not in a closure: `#[track_caller]` does not pass through closures.
            no_engine("AnimController::is_playing");
            false
        }
    }
}

/// The theme of the `Ui`'s display (see [`use_theme`]): switch the theme or its mode, read
/// the mode and the values of [design elements](twine_style::design).
///
/// Styles that use design elements (`.bg(design::SURFACE)`) follow the theme by themselves:
/// [`set_mode`](Self::set_mode) needs no binding or rebuild. [`mode`](Self::mode) and
/// [`get`](Self::get) are for the rest of the application (e.g. a chart drawn in code, an
/// icon chosen per mode); both are tracked, so an effect or a closure property that calls
/// them re-runs when the theme changes.
#[derive(Clone, Copy, Debug)]
pub struct ThemeHandle {
    cx: Scope,
}

impl ThemeHandle {
    /// Installs `theme` (every node is re-styled; the display is redrawn once). At once inside
    /// handlers, effects and timers run by the `Ui`; called elsewhere (between updates) it is
    /// queued and takes effect at the start of the next `Ui::update`.
    /// Never panics. Cost: O(nodes of the display) (see
    /// [`Engine::set_theme`](twine_engine::Engine::set_theme)); for light/dark switching prefer
    /// [`set_mode`](Self::set_mode).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let theme = use_theme(cx);
    ///     button(label("Simple")).on_click(move || theme.set(SimpleTheme::builder().build()))
    /// }
    /// # let _ = app;
    /// ```
    pub fn set(&self, theme: impl ThemeHook + 'static) {
        self.set_rc(Rc::new(theme));
    }

    /// [`set`](Self::set) with the theme already shared.
    /// For a theme shared with other displays or kept to switch back to. Never panics.
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_engine::ThemeHook;
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let dark: Rc<dyn ThemeHook> = Rc::new(DefaultTheme::dark());
    ///     let theme = use_theme(cx);
    ///     button(label("Dark")).on_click(move || theme.set_rc(dark.clone()))
    /// }
    /// # let _ = app;
    /// ```
    pub fn set_rc(&self, theme: Rc<dyn ThemeHook>) {
        let cx = self.cx;
        let mut theme = Some(theme);
        EngineAccess::with(|e| {
            if let (Some(d), Some(t)) = (display_of(cx, e), theme.take()) {
                e.set_theme(d, t);
            }
        });
        if let Some(t) = theme {
            defer(cx, EngineCmd::set_theme(t));
        }
    }

    /// Switches the theme to `mode` (light, dark, night, high contrast): the display takes the
    /// theme's design element table for `mode` and every node is re-resolved once
    /// ([`Engine::set_theme_mode`]); views are not rebuilt and no binding re-runs, except
    /// those that read [`mode`](Self::mode), [`modes`](Self::modes) or [`get`](Self::get).
    /// Allocates nothing once the theme has built its table for `mode`. Like
    /// [`set`](Self::set), at once where the `Ui` runs code, else queued for the next update.
    /// A mode the theme does not support ([`modes`](Self::modes)) is refused: the display
    /// keeps its current mode and a warning is logged; never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let theme = use_theme(cx);
    ///     // A day / night / high-contrast button: cycles through the theme's modes.
    ///     button(label(move || format!("{:?}", theme.mode())))
    ///         .bg(design::SURFACE_VARIANT)
    ///         .on_click(move || theme.set_mode(theme.mode().next_in(theme.modes())))
    /// }
    /// # let _ = app;
    /// ```
    pub fn set_mode(&self, mode: ThemeMode) {
        let cx = self.cx;
        let applied = EngineAccess::with(|e| {
            if let Some(d) = display_of(cx, e) {
                e.set_theme_mode(d, mode);
            }
        });
        if applied.is_none() {
            defer(cx, EngineCmd::set_theme_mode(mode));
        }
    }

    /// The theme mode of the `Ui`'s display. **Tracked**: an effect, memo or closure property
    /// that calls it re-runs when the mode changes (or the theme is replaced). Outside a `Ui`
    /// (a scope without one) the engine's default display is read untracked, if the engine is
    /// available; else [`ThemeMode::Light`].
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let theme = use_theme(cx);
    ///     // Re-evaluated when the mode changes.
    ///     label(move || if theme.mode() == ThemeMode::Light { "Day" } else { "Night" })
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    pub fn mode(&self) -> ThemeMode {
        if let Some(ui) = self.cx.use_context::<UiContext>() {
            return ui.theme.with(|s| s.mode);
        }
        EngineAccess::with(|e| e.default_display().map(|d| e.theme_mode(d)))
            .flatten()
            .unwrap_or_default()
    }

    /// The modes the theme supports ([`ThemeHook::modes`], e.g. all four for `DefaultTheme`;
    /// empty without a theme). **Tracked** like [`mode`](Self::mode) (it changes when the
    /// theme is replaced), and read like it outside a `Ui` (empty if the engine is not
    /// available). Cycle through them with [`ThemeMode::next_in`] (example at
    /// [`set_mode`](Self::set_mode)).
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let theme = use_theme(cx);
    ///     label(move || format!("{} modes", theme.modes().len()))
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    pub fn modes(&self) -> &'static [ThemeMode] {
        if let Some(ui) = self.cx.use_context::<UiContext>() {
            return ui.theme.with(|s| s.modes);
        }
        EngineAccess::with(|e| e.default_display().map(|d| e.theme_modes(d)))
            .flatten()
            .unwrap_or(&[])
    }

    /// The value the theme gives the [design element](twine_style::design) `element` in its
    /// current mode (`None` if the theme does not define it, or the engine is not available —
    /// it is inside handlers, effects, bindings and while views are built). **Tracked** like
    /// [`mode`](Self::mode).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let theme = use_theme(cx);
    ///     // The primary color as a hex string, updated when the mode changes.
    ///     label(move || match theme.get(design::PRIMARY) {
    ///         Some(c) => c.to_string(),
    ///         None => String::from("no theme"),
    ///     })
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    pub fn get<T: ElementType>(&self, element: Element<T>) -> Option<T> {
        let cx = self.cx;
        if let Some(ui) = cx.use_context::<UiContext>() {
            ui.theme.with(|_| ());
        }
        EngineAccess::with(|e| display_of(cx, e).and_then(|d| e.design_value(d, element))).flatten()
    }
}

/// The theme of the `Ui` owning `cx`: switch it or its mode, read the mode and design
/// element values ([`ThemeHandle`]).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     button(label("Dark")).on_click(move || use_theme(cx).set(DefaultTheme::dark()))
/// }
/// # let _ = app;
/// ```
#[must_use]
pub fn use_theme(cx: Scope) -> ThemeHandle {
    ThemeHandle { cx }
}

/// The motion preference of the engine the `Ui` runs on (see [`use_motion`]): read it
/// (tracked) and change it at run time. `Copy`.
///
/// The preference is applied by the engine to every animation it starts — tweens and
/// animations of this crate, style transitions, screen loads, scroll animations — so most
/// applications only [`set`](Self::set) it (e.g. from a "reduce motion" switch in the settings)
/// and never read it. [`get`](Self::get) is for code that changes more than timing under
/// reduced motion, e.g. a view that shows a static icon instead of an animated one.
#[derive(Clone, Copy, Debug)]
pub struct MotionHandle {
    cx: Scope,
}

impl MotionHandle {
    /// The motion preference. **Tracked**: an effect, memo or closure property that calls it
    /// re-runs when it changes. Outside a `Ui` the engine's value is read untracked if the
    /// engine is available, else [`Motion::Full`].
    #[must_use]
    pub fn get(&self) -> Motion {
        if let Some(ui) = self.cx.use_context::<UiContext>() {
            return ui.motion.get();
        }
        EngineAccess::with(|e| e.motion()).unwrap_or_default()
    }

    /// Sets the motion preference ([`Engine::set_motion`]): animations started from now on
    /// follow it, and running non-essential animations that would not end by themselves end
    /// when it becomes stricter. [`get`](Self::get) returns the new value at once. Applied now
    /// where the `Ui` runs code (handlers, effects, timers, while building), else queued for
    /// the next `Ui::update`. Idempotent; never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let motion = use_motion(cx);
    ///     let full = move || motion.get() == Motion::Full;
    ///     button(label(move || if full() { "Reduce motion" } else { "Full motion" }))
    ///         .on_click(move || motion.set(if full() { Motion::Reduced } else { Motion::Full }))
    /// }
    /// # let _ = app;
    /// ```
    pub fn set(&self, motion: Motion) {
        let cx = self.cx;
        if let Some(ui) = cx.use_context::<UiContext>() {
            ui.motion.set_if_changed(motion);
        }
        if EngineAccess::with(|e| e.set_motion(motion)).is_none() {
            defer(cx, EngineCmd::set_motion(motion));
        }
    }
}

/// The motion preference of the `Ui` owning `cx`: read (tracked) and change it
/// ([`MotionHandle`]). Set the initial preference with
/// [`UiBuilder::motion`](crate::UiBuilder::motion).
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     let motion = use_motion(cx);
///     // A static icon instead of a spinner when the user asked for less motion.
///     dynamic(move |_| {
///         if motion.get() == Motion::Full { spinner().into_any() } else { label(Symbol::Refresh).into_any() }
///     })
/// }
/// # let _ = app;
/// ```
#[must_use]
pub fn use_motion(cx: Scope) -> MotionHandle {
    MotionHandle { cx }
}

/// Hooks tied to a [`Scope`]: everything they create is released when the scope is disposed.
///
/// # Where the engine is available
///
/// The engine is available inside event handlers, effects (bindings), timer and animation
/// callbacks, channel message handlers and while the view is being built — the code the `Ui`
/// runs ([`EngineAccess`](crate::EngineAccess)). Elsewhere (your main loop, another task,
/// after `Ui::update` returned):
///
/// - hooks that create something (`tween`, `animation`, `interval`, `timeout`, `show_modal`),
///   signal writes and navigation **defer** their engine work to the next `Ui::update`;
/// - engine side effects — the cleanups of a disposed scope (stopping its animations,
///   removing its timers, closing its modals), the [`AnimController`] methods,
///   [`ThemeHandle::set`], [`ThemeHandle::set_mode`] and [`MotionHandle::set`] — are
///   **queued** on the `Ui` that owns the scope and applied at the start of its next update
///   (bounded: see [`UiBuilder::engine_queue_capacity`](crate::UiBuilder::engine_queue_capacity);
///   a command that does not fit is dropped and reported as a
///   [`CapacityFault::EngineQueue`](crate::CapacityFault::EngineQueue) fault);
/// - calls that must return what the engine holds — [`NodeRef::with_mut`],
///   [`AnimController::is_playing`] — return `None` / `false` and, in debug builds, log
///   `warn!` once per call site. Use the `Ui` methods (`ui.engine_mut()`) or post a message
///   whose handler makes the call:
///
/// ```
/// use twine_view::prelude::*;
///
/// static TOGGLE: Channel<(), 4> = Channel::new();
///
/// fn app(cx: Scope) -> impl View {
///     let (x, ctl) = cx.animation(0, 100, Duration::ms(1000));
///     // Runs inside `Ui::update` with the engine lent: `is_playing` can answer.
///     cx.on_message(&TOGGLE, move |()| ctl.set_playing(!ctl.is_playing()));
///     container(()).width(x)
/// }
/// # let _ = app;
/// // Elsewhere (another task, an ISR): post rather than asking `ctl` there.
/// let _ = TOGGLE.try_send(());
/// ```
pub trait ScopeExt: Copy {
    /// An empty [`NodeRef`], filled by `.node_ref(r)` on a view.
    fn node_ref<W: Widget>(self) -> NodeRef<W>;

    /// A signal that animates towards `source()` whenever it changes, with the timing `spec`
    /// (an [`AnimSpec`], or just a `Duration` for a linear one; its forward play is used:
    /// retargeting mid-flight continues from the current value). Any [`Interpolate`] value:
    /// numbers, colors, points, sizes, angles, scales, opacities. Idle once it arrived.
    ///
    /// Honours the [`Motion`] preference (applied by the engine when each animation starts):
    /// shortened under `Motion::Reduced`, a jump under `Motion::None`, unless the spec is
    /// [`essential`](AnimSpec::essential). One boxed closure when the tween first animates;
    /// retargeting restarts the same animation (no allocation).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let open = cx.signal(false);
    ///     let h = cx.tween(
    ///         move || if open.get() { 200 } else { 48 },
    ///         AnimSpec::new(Duration::ms(250)).ease_in_out(),
    ///     );
    ///     container(label("menu")).height(h).on_click(move || open.update(|o| *o = !*o))
    /// }
    /// # let _ = app;
    /// ```
    fn tween<T: Interpolate + PartialEq + 'static>(
        self,
        source: impl Fn() -> T + 'static,
        spec: impl Into<AnimSpec>,
    ) -> ReadSignal<T>;

    /// A free-running animation of any [`Interpolate`] value from `from` to `to` with the
    /// timing `spec` (duration, easing, delay, repetition, playback): its value as a signal
    /// and a controller. Starts when the scope is built (or at the next update).
    ///
    /// Honours the [`Motion`] preference like every engine animation: a decorative loop
    /// plays once, shortened, under `Motion::Reduced` and jumps to its end under
    /// `Motion::None` — mark the spec [`essential`](AnimSpec::essential) for animations that
    /// convey information (a progress or busy indicator). A [`restart`](AnimController::restart)
    /// after the preference changed starts it anew under the new one.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     // A pulsing dot: color and position animated together.
    ///     let pulse = AnimSpec::new(Duration::ms(800)).ease_in_out().playback(Duration::ms(800)).forever();
    ///     let (c, _) = cx.animation(Color::hex(0x2196F3), Color::hex(0xE91E63), pulse);
    ///     let (p, _) = cx.animation(Point::new(0, 0), Point::new(40, 0), pulse);
    ///     container(()).size(16, 16).bg(c).offset(p)
    /// }
    /// # let _ = app;
    /// ```
    fn animation<T: Interpolate + PartialEq + 'static>(
        self,
        from: T,
        to: T,
        spec: impl Into<AnimSpec>,
    ) -> (ReadSignal<T>, AnimController);

    /// Calls `f` every `period` (inside a batch) while the scope lives.
    fn interval(self, period: Duration, f: impl FnMut() + 'static);

    /// Calls `f` once after `delay`, unless the scope is disposed before.
    fn timeout(self, delay: Duration, f: impl FnOnce() + 'static);

    /// Shows `view` in a modal on the display's top layer: a full-screen semi-transparent
    /// backdrop blocking the input below, the view centered, and a focus group of its own for
    /// keypads and encoders. Closed with [`ModalHandle::close`] (or when this scope is
    /// disposed).
    ///
    /// `view` is called once, with the modal's own scope and its [`ModalHandle`] (`Copy`), so
    /// the modal's buttons can close it:
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     button(label("Open")).on_click(move || {
    ///         let _ = cx.show_modal(|_cx, modal: ModalHandle| {
    ///             button(label("Close")).on_click(move || modal.close())
    ///         });
    ///     })
    /// }
    /// # let _ = app;
    /// ```
    fn show_modal<V: View>(self, view: impl FnOnce(Scope, ModalHandle) -> V + 'static) -> ModalHandle;

    /// The theme switcher of the `Ui` (same as [`use_theme`]).
    fn use_theme(self) -> ThemeHandle;

    /// The motion preference of the `Ui` (same as [`use_motion`]).
    fn use_motion(self) -> MotionHandle;

    /// An empty menu page reference, filled by `menu_page(..).page_ref(r)` (rows can load
    /// pages defined after them).
    fn menu_page_ref(self) -> crate::MenuPageRef;
}

impl ScopeExt for Scope {
    fn node_ref<W: Widget>(self) -> NodeRef<W> {
        NodeRef::new(self)
    }

    fn tween<T: Interpolate + PartialEq + 'static>(
        self,
        source: impl Fn() -> T + 'static,
        spec: impl Into<AnimSpec>,
    ) -> ReadSignal<T> {
        let spec = spec.into();
        let init = untrack(&source);
        let out = self.signal(init);
        let st = Rc::new(RefCell::new(Tween {
            anim: None,
            motion: Motion::Full,
            from: init,
            to: init,
        }));
        let s2 = st.clone();
        self.on_cleanup(move || {
            let id = s2.borrow_mut().anim.take();
            if let Some(id) = id {
                stop_anim(self, id);
            }
        });
        self.effect_with_cx(move |_| {
            let target = source();
            if st.borrow().to == target {
                return;
            }
            if spec.duration == Duration::ZERO && spec.delay == Duration::ZERO {
                st.borrow_mut().to = target;
                out.set_if_changed(target);
                return;
            }
            let from = out.get_untracked();
            let st2 = st.clone();
            let started = EngineAccess::with(|e| {
                {
                    let mut s = st.borrow_mut();
                    s.from = from;
                    s.to = target;
                }
                // Retarget a running tween by restarting its animation (no allocation), unless
                // the motion preference changed since it started (then it starts anew).
                let running = {
                    let s = st.borrow();
                    s.anim.filter(|id| e.anim_exists(*id) && s.motion == e.motion())
                };
                if let Some(id) = running {
                    e.anim_restart(id);
                    return;
                }
                if let Some(old) = st.borrow_mut().anim.take() {
                    e.anim_stop(old);
                }
                let id = e.anim_start_fn(Anim::with_spec(0, EASING_ONE, forward(spec)), move |e, v| {
                    if !out.is_alive() {
                        return;
                    }
                    let (a, b) = {
                        let s = st2.borrow();
                        (s.from, s.to)
                    };
                    EngineAccess::provide(e, || out.set_if_changed(T::lerp(a, b, v)));
                });
                let mut s = st.borrow_mut();
                s.anim = Some(id);
                s.motion = e.motion();
            });
            if started.is_none() {
                defer_current_effect();
            }
        });
        out.read_only()
    }

    fn animation<T: Interpolate + PartialEq + 'static>(
        self,
        from: T,
        to: T,
        spec: impl Into<AnimSpec>,
    ) -> (ReadSignal<T>, AnimController) {
        let out = self.signal(from);
        let apply: ApplyFn = Rc::new(move |e: &mut Engine, v: i32| {
            if out.is_alive() {
                EngineAccess::provide(e, || out.set_if_changed(T::lerp(from, to, v)));
            }
        });
        let ctl = AnimController {
            state: self.stored_value(CtlState {
                id: None,
                spec: spec.into(),
                motion: Motion::Full,
                apply,
                cx: self,
            }),
        };
        with_engine_once(self, move |e| AnimController::start(e, ctl));
        // Cleanups run before the scope's stored values are dropped.
        self.on_cleanup(move || {
            if let Some(id) = ctl.take_id() {
                stop_anim(self, id);
            }
        });
        (out.read_only(), ctl)
    }

    fn interval(self, period: Duration, f: impl FnMut() + 'static) {
        add_timer(self, period, None, Box::new(f));
    }

    fn timeout(self, delay: Duration, f: impl FnOnce() + 'static) {
        let mut f = Some(f);
        add_timer(
            self,
            delay,
            Some(1),
            Box::new(move || {
                if let Some(f) = f.take() {
                    f();
                }
            }),
        );
    }

    fn show_modal<V: View>(self, view: impl FnOnce(Scope, ModalHandle) -> V + 'static) -> ModalHandle {
        show_modal(self, view)
    }

    fn use_theme(self) -> ThemeHandle {
        use_theme(self)
    }

    fn use_motion(self) -> MotionHandle {
        use_motion(self)
    }

    fn menu_page_ref(self) -> crate::MenuPageRef {
        crate::MenuPageRef::new(self)
    }
}

/// An engine timer calling `f` (inside a batch, with the engine lent) while `cx` lives;
/// `repeat`: number of runs (`None`: forever).
fn add_timer(cx: Scope, period: Duration, repeat: Option<u32>, f: Box<dyn FnMut()>) {
    let id: Rc<Cell<Option<TimerId>>> = Rc::default();
    let id2 = id.clone();
    let f = Rc::new(RefCell::new(f));
    with_engine_once(cx, move |e| {
        let t = e.timer_add(period, move |e, tid| {
            if !cx.is_alive() {
                e.timer_remove(tid);
                return;
            }
            let f = f.clone();
            EngineAccess::provide(e, || batch(|| (f.borrow_mut())()));
        });
        if repeat.is_some() {
            e.timer_set_repeat_count(t, repeat);
        }
        id2.set(Some(t));
    });
    cx.on_cleanup(move || {
        if let Some(t) = id.take() {
            if EngineAccess::with(|e| e.timer_remove(t)).is_none() {
                defer(cx, EngineCmd::timer_remove(t));
            }
        }
    });
}

/// The forward play of `spec` (a tween goes once from its current value to its target).
fn forward(spec: AnimSpec) -> AnimSpec {
    AnimSpec {
        repeat: twine_anim::Repeat::ONCE,
        playback: None,
        ..spec
    }
}

/// Stops animation `id` of scope `cx` now, or at the next update of the `Ui` when the engine
/// is not lent (a scope disposed outside `Ui::update`).
fn stop_anim(cx: Scope, id: AnimId) {
    if EngineAccess::with(|e| e.anim_stop(id)).is_none() {
        defer(cx, EngineCmd::anim_stop(id));
    }
}
