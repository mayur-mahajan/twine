//! Animated screen loads: [`ScreenAnim`], [`ScreenLoad`] and [`Engine::load_screen_anim`]
//! (LVGL `lv_screen_load_anim`, LVGL 9.6 `lv_display.c`).
//!
//! Like LVGL, a screen load animation moves the screens (their position, the whole subtree
//! with them) or fades their opacity, never re-laying them out. While it runs both screens
//! are drawn — the new one above the old one, except for the `Out*` and `FadeOut` animations
//! where the old one leaves above the new one — and pointer input does not reach either
//! screen.

use twine_anim::{Anim, AnimId};
use twine_core::{Duration, Opa};
use twine_style::{PropId, Selector, StyleProp};

use crate::anim::{Deferred, Op};
use crate::{DisplayId, Engine, EventCode, EventParam, InvalidateReason, NodeId, fmt_node_id};

/// How a new screen replaces the active one (LVGL `lv_screen_load_anim_t`); every variant but
/// `None` carries the animation time.
///
/// | Variant | New screen | Old screen | Drawn on top |
/// |---------|------------|------------|--------------|
/// | `OverLeft/Right/Top/Bottom` | slides in from the right / left / bottom / top | stays | new |
/// | `MoveLeft/Right/Top/Bottom` | slides in | slides out the other side | new |
/// | `FadeIn` | fades in (opacity 0 → 255) | stays | new |
/// | `FadeOut` | stays | fades out | old |
/// | `OutLeft/Right/Top/Bottom` | stays | slides out to the left / right / top / bottom | old |
///
/// ```
/// use twine_core::Duration;
/// use twine_engine::ScreenAnim;
///
/// let load = ScreenAnim::MoveLeft(Duration::ms(300)).delay(Duration::ms(50)).auto_delete(true);
/// assert_eq!(load.anim.duration(), Duration::ms(300));
/// assert!(load.auto_delete);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ScreenAnim {
    /// Switch at once (after the delay).
    None,
    /// The new screen slides in from the right, over the old one.
    OverLeft(Duration),
    /// The new screen slides in from the left, over the old one.
    OverRight(Duration),
    /// The new screen slides in from the bottom, over the old one.
    OverTop(Duration),
    /// The new screen slides in from the top, over the old one.
    OverBottom(Duration),
    /// Both screens move to the left.
    MoveLeft(Duration),
    /// Both screens move to the right.
    MoveRight(Duration),
    /// Both screens move up.
    MoveTop(Duration),
    /// Both screens move down.
    MoveBottom(Duration),
    /// The new screen fades in over the old one.
    FadeIn(Duration),
    /// The old screen fades out, revealing the new one.
    FadeOut(Duration),
    /// The old screen slides out to the left, revealing the new one.
    OutLeft(Duration),
    /// The old screen slides out to the right, revealing the new one.
    OutRight(Duration),
    /// The old screen slides out to the top, revealing the new one.
    OutTop(Duration),
    /// The old screen slides out to the bottom, revealing the new one.
    OutBottom(Duration),
}

impl ScreenAnim {
    /// Every animated variant (with `d`), in LVGL enum order, then `None`.
    #[must_use]
    pub const fn all(d: Duration) -> [ScreenAnim; 15] {
        use ScreenAnim as S;
        [
            S::OverLeft(d),
            S::OverRight(d),
            S::OverTop(d),
            S::OverBottom(d),
            S::MoveLeft(d),
            S::MoveRight(d),
            S::MoveTop(d),
            S::MoveBottom(d),
            S::FadeIn(d),
            S::FadeOut(d),
            S::OutLeft(d),
            S::OutRight(d),
            S::OutTop(d),
            S::OutBottom(d),
            S::None,
        ]
    }

    /// The animation time (zero for `None`).
    #[must_use]
    pub const fn duration(self) -> Duration {
        use ScreenAnim as S;
        match self {
            S::None => Duration::ZERO,
            S::OverLeft(d)
            | S::OverRight(d)
            | S::OverTop(d)
            | S::OverBottom(d)
            | S::MoveLeft(d)
            | S::MoveRight(d)
            | S::MoveTop(d)
            | S::MoveBottom(d)
            | S::FadeIn(d)
            | S::FadeOut(d)
            | S::OutLeft(d)
            | S::OutRight(d)
            | S::OutTop(d)
            | S::OutBottom(d) => d,
        }
    }

    /// A short lowercase name (`"over_left"`, `"fade_in"`, …) for logs and file names.
    #[must_use]
    pub const fn name(self) -> &'static str {
        use ScreenAnim as S;
        match self {
            S::None => "none",
            S::OverLeft(_) => "over_left",
            S::OverRight(_) => "over_right",
            S::OverTop(_) => "over_top",
            S::OverBottom(_) => "over_bottom",
            S::MoveLeft(_) => "move_left",
            S::MoveRight(_) => "move_right",
            S::MoveTop(_) => "move_top",
            S::MoveBottom(_) => "move_bottom",
            S::FadeIn(_) => "fade_in",
            S::FadeOut(_) => "fade_out",
            S::OutLeft(_) => "out_left",
            S::OutRight(_) => "out_right",
            S::OutTop(_) => "out_top",
            S::OutBottom(_) => "out_bottom",
        }
    }

    /// Whether the old screen is drawn above the new one (LVGL `is_out_anim`).
    #[must_use]
    pub const fn old_on_top(self) -> bool {
        matches!(
            self,
            ScreenAnim::FadeOut(_)
                | ScreenAnim::OutLeft(_)
                | ScreenAnim::OutRight(_)
                | ScreenAnim::OutTop(_)
                | ScreenAnim::OutBottom(_)
        )
    }

    /// This animation after a delay.
    #[must_use]
    pub const fn delay(self, d: Duration) -> ScreenLoad {
        ScreenLoad {
            anim: self,
            delay: d,
            auto_delete: false,
        }
    }

    /// This animation, deleting the old screen at the end (`auto_delete`).
    #[must_use]
    pub const fn auto_delete(self, on: bool) -> ScreenLoad {
        ScreenLoad {
            anim: self,
            delay: Duration::ZERO,
            auto_delete: on,
        }
    }

    /// The motion of `(new, old)` screens: property and values (`w`/`h` = display size).
    fn motion(self, w: i32, h: i32) -> (Option<Motion>, Option<Motion>) {
        use ScreenAnim as S;
        let m = |axis, from, to| Some(Motion { axis, from, to });
        match self {
            S::None => (m(Axis::X, 0, 0), None),
            S::OverLeft(_) => (m(Axis::X, w, 0), None),
            S::OverRight(_) => (m(Axis::X, -w, 0), None),
            S::OverTop(_) => (m(Axis::Y, h, 0), None),
            S::OverBottom(_) => (m(Axis::Y, -h, 0), None),
            S::MoveLeft(_) => (m(Axis::X, w, 0), m(Axis::X, 0, -w)),
            S::MoveRight(_) => (m(Axis::X, -w, 0), m(Axis::X, 0, w)),
            S::MoveTop(_) => (m(Axis::Y, h, 0), m(Axis::Y, 0, -h)),
            S::MoveBottom(_) => (m(Axis::Y, -h, 0), m(Axis::Y, 0, h)),
            S::FadeIn(_) => (m(Axis::Opa, 0, 255), None),
            S::FadeOut(_) => (None, m(Axis::Opa, 255, 0)),
            S::OutLeft(_) => (None, m(Axis::X, 0, -w)),
            S::OutRight(_) => (None, m(Axis::X, 0, w)),
            S::OutTop(_) => (None, m(Axis::Y, 0, -h)),
            S::OutBottom(_) => (None, m(Axis::Y, 0, h)),
        }
    }
}

/// A screen load request: the animation, a delay before it starts and whether the old screen
/// is deleted at the end (LVGL `lv_screen_load_anim` arguments).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ScreenLoad {
    /// The animation.
    pub anim: ScreenAnim,
    /// Delay before the animation (and the switch of the active screen) starts.
    pub delay: Duration,
    /// Delete the old screen when the load completes.
    pub auto_delete: bool,
}

impl ScreenLoad {
    /// Sets whether the old screen is deleted at the end.
    #[must_use]
    pub const fn auto_delete(mut self, on: bool) -> Self {
        self.auto_delete = on;
        self
    }
}

impl From<ScreenAnim> for ScreenLoad {
    fn from(anim: ScreenAnim) -> Self {
        ScreenLoad {
            anim,
            delay: Duration::ZERO,
            auto_delete: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    X,
    Y,
    Opa,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Motion {
    axis: Axis,
    from: i32,
    to: i32,
}

/// Which screen an animation value is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    New,
    Old,
}

/// Events of a screen load animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreenOp {
    /// The delay is over: the new screen becomes active (LVGL `scr_load_anim_start`).
    Start,
    /// A new value for one of the screens.
    Value(Which, i32),
    /// Done (LVGL `scr_anim_completed`).
    Done,
}

/// A screen load animation in progress on a display.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScreenLoadRun {
    /// The screen being loaded (LVGL `scr_to_load`).
    pub(crate) new: NodeId,
    /// The screen being left.
    pub(crate) old: NodeId,
    anim: ScreenAnim,
    auto_delete: bool,
    started: bool,
    new_anim: AnimId,
    old_anim: Option<AnimId>,
}

fn push_screen(ctx: &mut dyn core::any::Any, d: u8, op: ScreenOp) {
    if let Some(q) = ctx.downcast_mut::<Deferred>() {
        q.ops.push(Op::Screen(d, op));
    }
}

impl Engine {
    /// Loads `screen` with an animation (LVGL `lv_screen_load_anim`):
    ///
    /// 1. A screen load animation still running on the display is finished at once.
    /// 2. `ScreenUnloadStart` is sent to the old screen; after `load.delay` the new screen
    ///    becomes active and receives `ScreenLoadStart`.
    /// 3. During the animation both screens are drawn (see [`ScreenAnim`]) and pointer input
    ///    reaches neither of them.
    /// 4. At the end the new screen receives `ScreenLoaded`, the old one `ScreenUnloaded`;
    ///    both are back at their normal position and opacity, and with `auto_delete` the old
    ///    screen is deleted.
    ///
    /// `ScreenAnim::None` without delay loads instantly ([`load_screen`](Self::load_screen)).
    ///
    /// ```
    /// use twine_core::{Duration, Instant};
    /// use twine_engine::{Engine, EngineConfig, ScreenAnim};
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_engine::BufferMode;
    /// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
    /// # struct Panel(Option<DrawBufferMem>);
    /// # impl DisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
    /// # }
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// # let d = e.add_display(Panel(None), BufferMode::partial_single(buf)).unwrap();
    /// let first = e.active_screen(d).unwrap();
    /// let second = e.create_screen(d).unwrap();
    /// e.load_screen_anim(second, ScreenAnim::MoveLeft(Duration::ms(100)).auto_delete(true));
    /// e.step(Instant::ZERO);
    /// assert_eq!(e.active_screen(d), Some(second));
    /// assert!(e.screen_anim_running(d));
    /// e.step(Instant::from_millis(100));
    /// assert!(!e.screen_anim_running(d));
    /// assert!(!e.tree().contains(first)); // auto-deleted
    /// ```
    pub fn load_screen_anim(&mut self, screen: NodeId, load: impl Into<ScreenLoad>) {
        let load = load.into();
        let Some(d) = self.displays.iter().position(|d| d.screens.contains(&screen)) else {
            twine_core::warn!(target: "twine::engine", "load_screen_anim: {} is not a screen", fmt_node_id(screen));
            return;
        };
        if self.displays[d].active_screen == screen
            || self.displays[d].screen_load.is_some_and(|r| r.new == screen)
        {
            return;
        }
        if self.displays[d].screen_load.is_some() {
            self.finish_screen_anim(d);
        }
        let Some(d) = self.displays.iter().position(|x| x.screens.contains(&screen)) else {
            return;
        };
        let old = self.displays[d].active_screen;
        if old == screen {
            return;
        }
        // Both screens at their normal position and opacity.
        self.reset_screen(d, screen);
        self.reset_screen(d, old);
        if load.anim.duration() == Duration::ZERO && load.delay == Duration::ZERO {
            self.load_screen_now(d, screen);
            if load.auto_delete && self.tree.contains(old) && self.displays[d].active_screen != old {
                let _ = self.delete(old);
            }
            return;
        }
        let (w, h) = {
            let a = self.displays[d].area();
            (a.width(), a.height())
        };
        let (m_new, m_old) = load.anim.motion(w, h);
        let di = d as u8;
        let base = |m: Motion| {
            Anim::new(m.from, m.to)
                .duration(load.anim.duration())
                .delay(load.delay)
        };
        // The new screen's animation always exists: it carries the start/end callbacks (a
        // dummy 0 → 0 animation for `None`, LVGL).
        let mn = m_new.unwrap_or(Motion {
            axis: Axis::X,
            from: 0,
            to: 0,
        });
        let new_anim = base(mn)
            .on_start(move |cx| push_screen(cx.ctx(), di, ScreenOp::Start))
            .on_complete(move |cx| push_screen(cx.ctx(), di, ScreenOp::Done));
        twine_core::info!(
            target: "twine::engine",
            "display {}: loading screen {} ({} {}, delay {})",
            self.displays[d].id,
            fmt_node_id(screen),
            load.anim.name(),
            load.anim.duration(),
            load.delay
        );
        // The old screen's handlers run first (they may load another screen).
        self.send_event(old, EventCode::ScreenUnloadStart, EventParam::None);
        let Some(d) = self.displays.iter().position(|x| x.screens.contains(&screen)) else {
            return;
        };
        if self.displays[d].screen_load.is_some() || self.displays[d].active_screen == screen {
            return;
        }
        let new_anim = self.anim_start_internal(new_anim, move |ctx, v| {
            push_screen(ctx, di, ScreenOp::Value(Which::New, v));
        });
        let old_anim = m_old.map(|m| {
            self.anim_start_internal(base(m), move |ctx, v| {
                push_screen(ctx, di, ScreenOp::Value(Which::Old, v));
            })
        });
        self.input_reset(None, None);
        self.displays[d].screen_load = Some(ScreenLoadRun {
            new: screen,
            old,
            anim: load.anim,
            auto_delete: load.auto_delete,
            started: false,
            new_anim,
            old_anim,
        });
    }

    /// Whether a screen load animation (or its delay) is in progress on `display`.
    #[must_use]
    pub fn screen_anim_running(&self, display: DisplayId) -> bool {
        self.displays
            .get(display.index())
            .is_some_and(|d| d.screen_load.is_some())
    }

    /// The screen shown below or above the active one during a screen load animation (LVGL
    /// `lv_screen_prev`).
    #[must_use]
    pub fn prev_screen(&self, display: DisplayId) -> Option<NodeId> {
        self.displays.get(display.index()).and_then(|d| d.prev_screen)
    }

    /// Puts a screen back at the display origin with its `Opa` local property removed.
    fn reset_screen(&mut self, d: usize, screen: NodeId) {
        if !self.tree.contains(screen) {
            return;
        }
        let area = self.displays[d].area();
        self.place(screen, area);
        self.remove_local_prop(screen, PropId::Opa, Selector::MAIN);
    }

    /// Applies an event of the screen load animation of display `d`.
    pub(crate) fn screen_anim_op(&mut self, d: usize, op: ScreenOp) {
        let Some(run) = self.displays.get(d).and_then(|x| x.screen_load) else {
            return;
        };
        match op {
            ScreenOp::Start => self.screen_anim_start(d),
            ScreenOp::Value(which, v) => {
                let (node, motion) = {
                    let (w, h) = (self.displays[d].area().width(), self.displays[d].area().height());
                    let (mn, mo) = run.anim.motion(w, h);
                    match which {
                        Which::New => (run.new, mn),
                        Which::Old => (run.old, mo),
                    }
                };
                let Some(m) = motion else { return };
                if !self.tree.contains(node) {
                    return;
                }
                let area = self.displays[d].area();
                match m.axis {
                    Axis::X => self.place(node, area.translate(v, 0)),
                    Axis::Y => self.place(node, area.translate(0, v)),
                    Axis::Opa => {
                        self.set_local_prop(node, Selector::MAIN, StyleProp::Opa(Opa(v.clamp(0, 255) as u8)));
                    }
                }
            }
            ScreenOp::Done => self.screen_anim_done(d),
        }
    }

    /// LVGL `scr_load_anim_start`: the new screen becomes active, the old one stays visible.
    fn screen_anim_start(&mut self, d: usize) {
        let Some(run) = self.displays[d].screen_load.as_mut() else {
            return;
        };
        if run.started {
            return;
        }
        run.started = true;
        let run = *run;
        let disp = &mut self.displays[d];
        disp.prev_screen = Some(run.old).filter(|o| disp.screens.contains(o));
        disp.active_screen = run.new;
        disp.draw_prev_over_act = run.anim.old_on_top();
        let (id, area) = (disp.id, disp.area());
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        self.send_event(run.new, EventCode::ScreenLoadStart, EventParam::None);
    }

    /// LVGL `scr_anim_completed`.
    fn screen_anim_done(&mut self, d: usize) {
        let Some(run) = self.displays[d].screen_load else {
            return;
        };
        if !run.started {
            self.screen_anim_start(d);
        }
        let disp = &mut self.displays[d];
        disp.screen_load = None;
        disp.draw_prev_over_act = false;
        let prev = disp.prev_screen.take();
        let (id, area) = (disp.id, disp.area());
        // Both screens back to normal (the old one is not shown any more).
        self.reset_screen(d, run.new);
        if let Some(p) = prev {
            self.reset_screen(d, p);
        }
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        twine_core::info!(target: "twine::engine", "display {}: screen {} loaded", id, fmt_node_id(run.new));
        if self.tree.contains(run.new) {
            self.send_event(run.new, EventCode::ScreenLoaded, EventParam::None);
        }
        if let Some(p) = prev.filter(|p| self.tree.contains(*p)) {
            self.send_event(p, EventCode::ScreenUnloaded, EventParam::None);
            if run.auto_delete && self.tree.contains(p) && self.active_screen(id) != Some(p) {
                let _ = self.delete(p);
            }
        }
    }

    /// Finishes the screen load animation of display `d` at once (its end state and events).
    fn finish_screen_anim(&mut self, d: usize) {
        let Some(run) = self.displays[d].screen_load else {
            return;
        };
        self.anim.timeline.remove(run.new_anim);
        if let Some(a) = run.old_anim {
            self.anim.timeline.remove(a);
        }
        self.screen_anim_done(d);
    }

    /// Cleans up the screen load state of displays whose animated screens are among the
    /// `deleted` nodes: deleting the screen being loaded (possible during the delay) cancels
    /// the load; deleting the old screen stops its animation.
    pub(crate) fn screen_anims_forget(&mut self, deleted: &[NodeId]) {
        for d in 0..self.displays.len() {
            let Some(run) = self.displays[d].screen_load else {
                continue;
            };
            if deleted.contains(&run.new) {
                self.anim.timeline.remove(run.new_anim);
                if let Some(a) = run.old_anim {
                    self.anim.timeline.remove(a);
                }
                let disp = &mut self.displays[d];
                disp.screen_load = None;
                disp.draw_prev_over_act = false;
                disp.prev_screen = None;
                if self.tree.contains(run.old) {
                    self.reset_screen(d, run.old);
                }
            } else if deleted.contains(&run.old) {
                if let Some(a) = run.old_anim {
                    self.anim.timeline.remove(a);
                }
                if let Some(r) = self.displays[d].screen_load.as_mut() {
                    r.old_anim = None;
                }
            }
        }
    }

    /// Whether pointer input must skip the screens of display `d` (a screen load animation
    /// runs).
    pub(crate) fn screens_input_blocked(&self, d: usize) -> bool {
        self.displays.get(d).is_some_and(|x| x.screen_load.is_some())
    }
}
