//! Scope hooks ([`ScopeExt`]): node references, tweens, animations, timers, modals, themes.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
use core::cell::{Cell, RefCell};

use twine_anim::{Anim, AnimId, Easing, Interpolate, TimerId};
use twine_core::Duration;
use twine_engine::{DisplayId, Engine, NodeId, ThemeHook, Widget};
use twine_reactive::{
    ReadSignal, Scope, Signal, StoredValue, batch, defer_current_effect, dispose_current_effect, untrack,
};

use crate::access::{EngineAccess, no_engine};
use crate::nav::{ModalHandle, show_modal};
use crate::node_ref::NodeRef;
use crate::view::View;

/// The display a `Ui` runs on, provided as context in its root scope.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UiDisplay(pub(crate) DisplayId);

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
    cx.use_context::<UiDisplay>()
        .map(|d| d.0)
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

/// A copy of the parameters of `a` (without its callbacks).
fn copy_anim(a: &Anim) -> Anim {
    let mut b = Anim::new(a.start, a.end);
    b.duration = a.duration;
    b.delay = a.delay;
    b.easing = a.easing;
    b.repeat = a.repeat;
    b.repeat_delay = a.repeat_delay;
    b.playback = a.playback;
    b.playback_delay = a.playback_delay;
    b.early_apply = a.early_apply;
    b
}

/// The state of a [`tween`](ScopeExt::tween).
struct Tween<T> {
    anim: Option<AnimId>,
    from: T,
    to: T,
}

/// Controls a free-running animation created with [`ScopeExt::animation`]. `Copy`, like a
/// signal; its state is owned by the scope that created the animation (once that scope is
/// disposed every method does nothing and `is_playing` is `false`). Every method works where
/// the engine is available — inside handlers, effects, timers and while building (see
/// [`ScopeExt`] § Where the engine is available); elsewhere it does nothing (`is_playing` is
/// `false`) and, in debug builds, logs `warn!` once per call site.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AnimController {
    state: StoredValue<CtlState>,
    out: Signal<i32>,
}

struct CtlState {
    id: Option<AnimId>,
    params: Anim,
}

impl core::fmt::Debug for AnimController {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimController").field("id", &self.id()).finish()
    }
}

impl AnimController {
    #[cfg_attr(debug_assertions, track_caller)]
    fn engine(what: &'static str, f: impl FnOnce(&mut Engine)) {
        if EngineAccess::with(f).is_none() {
            no_engine(what);
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
        let Some(params) = ctl.state.try_with(|s| copy_anim(&s.params)) else {
            return; // disposed with its scope
        };
        let out = ctl.out;
        let id = e.anim_start_fn(params, move |e, v| {
            if out.is_alive() {
                EngineAccess::provide(e, || out.set_if_changed(v));
            }
        });
        if let Some(old) = ctl.state.with_mut(|s| s.id.replace(id)) {
            e.anim_stop(old);
        }
    }

    /// Pauses at the current value.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn pause(&self) {
        let id = self.id();
        Self::engine("AnimController::pause", |e| {
            if let Some(id) = id {
                e.anim_pause(id);
            }
        });
    }

    /// Continues after [`pause`](Self::pause).
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn resume(&self) {
        let id = self.id();
        Self::engine("AnimController::resume", |e| {
            if let Some(id) = id {
                e.anim_resume(id);
            }
        });
    }

    /// Starts again from the beginning (also after the animation ended or was stopped).
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn restart(&self) {
        let ctl = *self;
        Self::engine("AnimController::restart", move |e| match ctl.id() {
            Some(id) if e.anim_exists(id) => {
                e.anim_restart(id);
            }
            _ => Self::start(e, ctl),
        });
    }

    /// Stops the animation (the value stays where it is).
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn stop(&self) {
        let ctl = *self;
        Self::engine("AnimController::stop", move |e| {
            if let Some(id) = ctl.take_id() {
                e.anim_stop(id);
            }
        });
    }

    /// Plays (`true`, resuming or restarting) or pauses (`false`).
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn set_playing(&self, on: bool) {
        let ctl = *self;
        Self::engine("AnimController::set_playing", move |e| match (on, ctl.id()) {
            (true, Some(id)) if e.anim_exists(id) => {
                e.anim_resume(id);
            }
            (true, _) => Self::start(e, ctl),
            (false, Some(id)) => {
                e.anim_pause(id);
            }
            (false, None) => {}
        });
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

/// Switches the theme of the `Ui`'s display at run time (see [`use_theme`]).
#[derive(Clone, Copy, Debug)]
pub struct ThemeHandle {
    cx: Scope,
}

impl ThemeHandle {
    /// Installs `theme` (every node is re-styled; the display is redrawn once). Works inside
    /// handlers, effects and timers run by the `Ui`; elsewhere it does nothing and, in debug
    /// builds, logs `warn!` once per call site (from outside the `Ui`, use `Ui::set_theme`).
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn set(&self, theme: impl ThemeHook + 'static) {
        let cx = self.cx;
        let theme: Rc<dyn ThemeHook> = Rc::new(theme);
        let done = EngineAccess::with(|e| {
            if let Some(d) = display_of(cx, e) {
                e.set_theme(d, theme);
            }
        });
        if done.is_none() {
            no_engine("ThemeHandle::set");
        }
    }
}

/// The theme switcher of the `Ui` owning `cx`.
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
///   signal writes and navigation **defer** their engine work to the next `Ui::update` —
///   correct, nothing is lost;
/// - calls that need the engine *now* — [`NodeRef::with_mut`], the [`AnimController`]
///   methods, [`ThemeHandle::set`] — do nothing (`None` / `false`) and, in debug builds, log
///   `warn!` once per call site. Use the `Ui` methods (`ui.set_theme(..)`,
///   `ui.engine_mut()`) or post a message whose handler makes the call:
///
/// ```
/// use twine_view::prelude::*;
///
/// static PAUSE: Channel<bool, 4> = Channel::new();
///
/// fn app(cx: Scope) -> impl View {
///     let (x, ctl) = cx.animation(Anim::new(0, 100).duration(Duration::ms(1000)));
///     // Runs inside `Ui::update` with the engine lent.
///     cx.on_message(&PAUSE, move |paused| ctl.set_playing(!paused));
///     container(()).width(x)
/// }
/// # let _ = app;
/// // Elsewhere (another task, an ISR, the main loop): post, don't call `ctl` directly.
/// let _ = PAUSE.try_send(true);
/// ```
pub trait ScopeExt: Copy {
    /// An empty [`NodeRef`], filled by `.node_ref(r)` on a view.
    fn node_ref<W: Widget>(self) -> NodeRef<W>;

    /// A signal that animates towards `source()` whenever it changes, over `d` with `easing`
    /// (retargeting mid-flight continues from the current value). Idle once it arrived.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let open = cx.signal(false);
    ///     let h = cx.tween(move || if open.get() { 200 } else { 48 }, Duration::ms(250), Easing::EaseInOut);
    ///     container(label("menu")).height(h).on_click(move || open.update(|o| *o = !*o))
    /// }
    /// # let _ = app;
    /// ```
    fn tween<T: Interpolate + PartialEq + 'static>(
        self,
        source: impl Fn() -> T + 'static,
        d: Duration,
        easing: Easing,
    ) -> ReadSignal<T>;

    /// A free-running animation: its value as a signal and a controller.
    fn animation(self, anim: Anim) -> (ReadSignal<i32>, AnimController);

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
        d: Duration,
        easing: Easing,
    ) -> ReadSignal<T> {
        let init = untrack(&source);
        let out = self.signal(init);
        let st = Rc::new(RefCell::new(Tween {
            anim: None,
            from: init,
            to: init,
        }));
        let s2 = st.clone();
        self.on_cleanup(move || {
            if let Some(id) = s2.borrow_mut().anim.take() {
                EngineAccess::with(|e| e.anim_stop(id));
            }
        });
        self.effect_with_cx(move |_| {
            let target = source();
            if st.borrow().to == target {
                return;
            }
            if d == Duration::ZERO {
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
                // Retarget a running tween by restarting its animation (no allocation).
                let running = st.borrow().anim.filter(|id| e.anim_exists(*id));
                if let Some(id) = running {
                    e.anim_restart(id);
                    return;
                }
                let anim = Anim::new(0, 1024).duration(d).easing(easing);
                let id = e.anim_start_fn(anim, move |e, v| {
                    if !out.is_alive() {
                        return;
                    }
                    let (a, b) = {
                        let s = st2.borrow();
                        (s.from, s.to)
                    };
                    EngineAccess::provide(e, || out.set_if_changed(T::lerp(a, b, v)));
                });
                st.borrow_mut().anim = Some(id);
            });
            if started.is_none() {
                defer_current_effect();
            }
        });
        out.read_only()
    }

    fn animation(self, anim: Anim) -> (ReadSignal<i32>, AnimController) {
        let out = self.signal(anim.start);
        let ctl = AnimController {
            state: self.stored_value(CtlState {
                id: None,
                params: anim,
            }),
            out,
        };
        with_engine_once(self, move |e| AnimController::start(e, ctl));
        // Cleanups run before the scope's stored values are dropped.
        self.on_cleanup(move || {
            if let Some(id) = ctl.take_id() {
                EngineAccess::with(|e| e.anim_stop(id));
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
            EngineAccess::with(|e| e.timer_remove(t));
        }
    });
}
