//! [`ViewExt`]: the modifiers every widget view has.

use alloc::boxed::Box;

use twine_core::Point;
use twine_engine::{
    Engine, EventCode, EventCx, EventFilter, EventResult, GroupId, NodeId, ObjFlags, State, Widget,
    fmt_node_id,
};
use twine_style::{Anchor, Axis, Part, ScrollSnap, ScrollbarMode, Selector, Side};

use twine_reactive::Runtime;

use crate::access::EngineAccess;
use crate::bind::{bind_effect, bind_node};
use crate::build::{BuildCx, BuildOp};
use crate::node_ref::NodeRef;
use crate::prop::{IntoProp, Prop};
use crate::style_ext::{StyleScope, scoped};
use crate::view::View;

// The build steps of the generic `ViewExt` helpers, as free functions: a closure written
// inside a trait method has a distinct type (and machine code) per implementing view type;
// these depend only on their own parameters, so all views share one copy.

/// The build step of [`ViewExt::flag`].
fn flag_op(flag: ObjFlags, p: Prop<bool>) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| bind_node(cx, node, p, move |e, n, on| e.set_flag(n, flag, on))
}

/// The build step of [`ViewExt::state`].
fn state_op(state: State, p: Prop<bool>) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| bind_node(cx, node, p, move |e, n, on| e.set_state(n, state, on))
}

/// The current value of `p` (a dynamic one is evaluated: its reads are tracked by the running
/// binding).
fn current<T: Copy>(p: &Prop<T>) -> T {
    match p {
        Prop::Static(v) => *v,
        Prop::Dynamic(f) => f(),
    }
}

/// Wraps a no-argument handler as an engine handler that lends the engine to it.
fn simple_handler<R>(
    rt: Runtime,
    mut f: impl FnMut() -> R + 'static,
) -> impl FnMut(&mut EventCx<'_>, &twine_engine::Event) -> EventResult + 'static {
    move |cx, _ev| {
        EngineAccess::provide(rt, cx.engine_mut(), || {
            f();
        });
        EventResult::Continue
    }
}

macro_rules! flag_mods {
    ($($(#[$m:meta])* $name:ident => $flag:ident;)*) => {
        $(
            $(#[$m])*
            #[must_use]
            fn $name<M>(self, on: impl IntoProp<bool, M>) -> Self {
                self.flag(ObjFlags::$flag, on)
            }
        )*
    };
}

macro_rules! event_mods {
    ($($(#[$m:meta])* $name:ident => $code:ident;)*) => {
        $(
            $(#[$m])*
            #[must_use]
            fn $name<R>(self, f: impl FnMut() -> R + 'static) -> Self {
                self.on_code(EventCode::$code, f)
            }
        )*
    };
}

/// The modifiers of every widget view: flags, per-part / per-state styles, relations,
/// events and identity (the table of the view API). Every view also has the style modifiers
/// of [`StyleExt`](crate::StyleExt): whole styles ([`style`](crate::StyleExt::style), static or
/// heap) and the style-property modifiers (sizes and positions, spacing, colors, borders,
/// shadows, text, visual effects, layout item properties), which set **local style
/// properties** of the `Main` part; [`part`](Self::part) / [`on_state`](Self::on_state) give them a part or a
/// state. Flag modifiers set object flags. Every value may be a constant, a signal, a memo or a
/// closure ([`IntoProp`]); dynamic values become bindings whose re-runs with an unchanged value
/// do nothing (the engine setters are idempotent).
///
/// Implemented by [`WidgetView`](crate::WidgetView) and the containers. The control-flow
/// views ([`when`](crate::when), [`dynamic`](crate::dynamic), [`for_each`](crate::for_each))
/// have no node of their own to style and do not implement it.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let hot = cx.signal(false);
/// let _v = label("21 °C")
///     .text_color(move || if hot.get() { Color::RED } else { Color::BLACK })
///     .padding(4)
///     .bg(Color::hex(0xEEEEEE))
///     .radius(6)
///     .on_state(State::PRESSED, |s| s.bg(Color::hex(0xCCCCCC)))
///     .test_id("temp");
/// cx.dispose();
/// ```
pub trait ViewExt: View + Sized {
    /// The widget type of the view's root node (for [`node_ref`](Self::node_ref)).
    type Widget: Widget;

    /// Appends a build step run right after the node is created.
    #[must_use]
    fn push_op(self, op: BuildOp) -> Self;

    /// Appends a build step `f(cx, node)`.
    #[must_use]
    fn op(self, f: impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static) -> Self {
        self.push_op(Box::new(f))
    }

    /// Binds an object flag.
    /// A constant is set once; a signal, memo or closure becomes a binding. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let advanced = cx.signal(false);
    ///     column((
    ///         label("Basic"),
    ///         label("Advanced").flag(ObjFlags::HIDDEN, move || !advanced.get()),
    ///     ))
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    fn flag<M>(self, flag: ObjFlags, on: impl IntoProp<bool, M>) -> Self {
        self.op(flag_op(flag, on.into_prop()))
    }

    /// Binds a state (added while the value is `true`).
    /// Never panics. See [`on_state`](Self::on_state) for styling the state.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let selected = cx.signal(true);
    ///     button(label("Tab")).state(State::CHECKED, selected)
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    fn state<M>(self, state: State, on: impl IntoProp<bool, M>) -> Self {
        self.op(state_op(state, on.into_prop()))
    }

    /// Calls `f` for every event `code` of the node (the engine is available through
    /// [`EngineAccess`] meanwhile). Its result is ignored.
    #[must_use]
    fn on_code<R>(self, code: EventCode, f: impl FnMut() -> R + 'static) -> Self {
        self.op(move |cx, node| {
            let rt = cx.runtime();
            cx.engine()
                .add_event_handler(node, EventFilter::Code(code), simple_handler(rt, f));
        })
    }

    // ---- Per-part / per-state styles --------------------------------------------------------

    /// Styles one part of the widget (e.g. `Part::Indicator` of a slider or bar, `Part::Knob`):
    /// `f` receives a [`StyleScope`] with every style modifier
    /// ([`StyleExt`](crate::StyleExt)), setting local properties of that part. Constants are set once; signals, memos and closures become one
    /// binding each. Combine with a state through [`StyleScope::on_state`]. A view that uses no
    /// scope pays nothing for this; never panics (a part the widget does not draw is simply
    /// never read).
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let alarm = cx.signal(false);
    /// let _v = bar(cx.signal(70))
    ///     .part(Part::Indicator, |s| {
    ///         s.bg(move || if alarm.get() { Color::RED } else { Color::BLUE })
    ///     });
    /// cx.dispose();
    /// ```
    #[must_use]
    fn part(self, part: Part, f: impl FnOnce(StyleScope<Self>) -> StyleScope<Self>) -> Self {
        scoped(self, Selector::part(part), f)
    }

    /// Styles the `Main` part while all of `state` is active, e.g. a pressed look
    /// (`State::PRESSED`) or a checked one: `f` receives a [`StyleScope`] with every style
    /// modifier ([`StyleExt`](crate::StyleExt)). The look is applied when the node enters the
    /// state and removed when it leaves it. Never panics; a view without scopes pays nothing.
    ///
    /// # Which look wins
    ///
    /// When the looks of several active states set the same property, the one with the higher
    /// state precedence wins — over the theme's styles too, whatever was declared first. The
    /// highest-ranked state in which two selectors differ decides (full rule:
    /// [`State`](twine_style::State#precedence)):
    ///
    /// | Rank (low → high) | States |
    /// |-------------------|--------|
    /// | – | no state (the view's own modifiers, the theme's base look) |
    /// | 0–4 | `ALT`, `CHECKED`, `FOCUSED`, `FOCUS_KEY`, `EDITED` |
    /// | 5–8 | `HOVERED`, `PRESSED`, `SCROLLED`, `DISABLED` |
    /// | 9–12 | the app's states `State::custom::<0>()` … `custom::<3>()` |
    ///
    /// | Node state | Looks | Shown |
    /// |------------|-------|-------|
    /// | `PRESSED \| CHECKED` | `on_state(CHECKED)`, `on_state(PRESSED)` | pressed |
    /// | `PRESSED \| CHECKED` | `on_state(PRESSED)`, `on_state(PRESSED \| CHECKED)` | pressed and checked |
    /// | `DISABLED \| PRESSED` | `on_state(DISABLED)`, `on_state(PRESSED \| CHECKED)` | disabled |
    /// | `DISABLED \| ALARM` | `on_state(DISABLED)`, `on_state(ALARM)` | alarm |
    ///
    /// Application states (up to four, [`State::custom`](twine_style::State::custom)) rank above
    /// the built-in ones, so an app look is not overridden by the theme's pressed or disabled
    /// look; style both with `on_state(DISABLED | ALARM, ..)`.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = button(label("OK"))
    ///     .on_state(State::PRESSED, |s| {
    ///         s.bg(Color::hex(0x1565C0)).transform_scale(Scale::pct(97))
    ///     });
    ///
    /// // An application state, named for debug output and tree dumps.
    /// const ALARM: State = State::custom::<0>();
    /// State::set_custom_name(ALARM, "ALARM");
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let alarm = cx.signal(false);
    /// let _v = button(label("Pump"))
    ///     .state(ALARM, alarm) // in the ALARM state while the signal is true
    ///     .on_state(ALARM, |s| s.bg(Color::RED))
    ///     .on_state(State::DISABLED | ALARM, |s| s.bg(Color::hex(0x8B0000)));
    /// cx.dispose();
    /// ```
    #[must_use]
    fn on_state(self, state: State, f: impl FnOnce(StyleScope<Self>) -> StyleScope<Self>) -> Self {
        scoped(self, Selector::state(state), f)
    }

    /// Styles any part in any states (`Selector::part(Part::Knob).with_state(State::PRESSED)`):
    /// the general form of [`part`](Self::part) and [`on_state`](Self::on_state). Never panics;
    /// costs one build step per modifier used in `f` and nothing otherwise.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let pressed_knob = Selector::part(Part::Knob).with_state(State::PRESSED);
    /// let _v = slider(twine_reactive::Runtime::take().unwrap().create_root().signal(30))
    ///     .styled(pressed_knob, |s| s.bg(Color::RED).padding(6));
    /// ```
    #[must_use]
    fn styled(self, selector: Selector, f: impl FnOnce(StyleScope<Self>) -> StyleScope<Self>) -> Self {
        scoped(self, selector, f)
    }

    // ---- Relations --------------------------------------------------------------------------

    /// Places the node relative to the node of `base` (LVGL `lv_obj_align_to`), following it
    /// when it moves: next to it (`Anchor::BelowLeft`, …) or inside it (an [`Align`](twine_style::Align),
    /// e.g. `Align::Center`). The reference must be filled (by `.node_ref(base)` on another
    /// view) before the layout runs; until then the relation is not set. `anchor`, `dx` and
    /// `dy` may be dynamic.
    /// Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let field: NodeRef<Label> = cx.node_ref();
    ///     container((
    ///         label("Name").node_ref(field).pos(10, 10),
    ///         // A hint 4 px below the label, following it when it moves.
    ///         label("required").align_to(field, Anchor::BelowLeft, 0, 4),
    ///     ))
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    fn align_to<W: Widget, MA, MX, MY>(
        self,
        base: NodeRef<W>,
        anchor: impl IntoProp<Anchor, MA>,
        dx: impl IntoProp<i32, MX>,
        dy: impl IntoProp<i32, MY>,
    ) -> Self {
        let (anchor, dx, dy) = (anchor.into_prop(), dx.into_prop(), dy.into_prop());
        self.op(move |cx, node| {
            let scope = cx.scope();
            cx.provide(|| {
                bind_effect(
                    scope,
                    node,
                    move || (base.get(), current(&anchor), current(&dx), current(&dy)),
                    move |e, n, (b, anchor, dx, dy): (Option<NodeId>, Anchor, i32, i32)| match b {
                        Some(b) => e.align_to(n, b, anchor, dx, dy),
                        None => twine_core::debug!(
                            target: "twine::view",
                            "align_to of {}: reference not filled yet",
                            fmt_node_id(n)
                        ),
                    },
                );
            });
        })
    }

    // ---- Flags --------------------------------------------------------------------------

    flag_mods! {
        /// Not drawn, not hit-tested, ignored by layouts.
        hidden => HIDDEN;
        /// Receives pointer presses.
        clickable => CLICKABLE;
        /// Toggles the `CHECKED` state when clicked.
        checkable => CHECKABLE;
        /// Can be scrolled.
        scrollable => SCROLLABLE;
        /// Scrolling continues with momentum after release.
        scroll_momentum => SCROLL_MOMENTUM;
        /// Elastic overscroll.
        scroll_elastic => SCROLL_ELASTIC;
        /// Scrolls at most one snappable child per gesture.
        scroll_one => SCROLL_ONE;
        /// Scrolls itself into view when focused.
        scroll_on_focus => SCROLL_ON_FOCUS;
        /// Positioned by hand, ignored by the parent's layout and scroll extents.
        floating => FLOATING;
        /// Positioned by hand, ignored by the parent's layout.
        ignore_layout => IGNORE_LAYOUT;
        /// Events also go to the parent.
        event_bubble => EVENT_BUBBLE;
        /// Children are not clipped to the node.
        overflow_visible => OVERFLOW_VISIBLE;
    }

    /// Disabled: the `DISABLED` state (no input, disabled look).
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let busy = cx.signal(false);
    ///     button(label("Save")).disabled(busy)
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    fn disabled<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.state(State::DISABLED, on)
    }

    /// Which directions can be scrolled (all by default; stop scrolling with
    /// [`scrollable(false)`](Self::scrollable)).
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = scroll_view(Axis::Vertical, (label("a"), label("b"))).scroll_dir(Axis::Vertical);
    /// ```
    #[must_use]
    fn scroll_dir<M>(self, axis: impl IntoProp<Axis, M>) -> Self {
        let p = axis.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, |e: &mut Engine, n, a: Axis| e.set_scroll_dir(n, a)))
    }

    /// When scrollbars are shown.
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = scroll_view(Axis::Vertical, (label("a"), label("b"))).scrollbar(ScrollbarMode::Active);
    /// ```
    #[must_use]
    fn scrollbar<M>(self, m: impl IntoProp<ScrollbarMode, M>) -> Self {
        let p = m.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scrollbar_mode))
    }

    /// Horizontal scroll snapping of the children.
    /// ```
    /// use twine_view::prelude::*;
    /// // A horizontal carousel: the child nearest the center snaps to it.
    /// let _v = row((card(label("1")), card(label("2")))).scrollable(true).scroll_snap_x(ScrollSnap::Center);
    /// ```
    #[must_use]
    fn scroll_snap_x<M>(self, s: impl IntoProp<ScrollSnap, M>) -> Self {
        let p = s.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scroll_snap_x))
    }

    /// Vertical scroll snapping of the children.
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = scroll_view(Axis::Vertical, (label("a"), label("b"))).scroll_snap_y(ScrollSnap::Start);
    /// ```
    #[must_use]
    fn scroll_snap_y<M>(self, s: impl IntoProp<ScrollSnap, M>) -> Self {
        let p = s.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scroll_snap_y))
    }

    /// Keypad / encoder focusable: in the default focus group (and focused when clicked).
    /// Needs a default focus group (`Ui` creates one when a keypad or encoder is added); without
    /// one the node is only made click-focusable. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = container(label("Card")).focusable(true).on_click(|| {});
    /// ```
    #[must_use]
    fn focusable<M>(self, on: impl IntoProp<bool, M>) -> Self {
        let p = on.into_prop();
        self.op(move |cx, node| {
            bind_node(cx, node, p, |e, n, on| {
                e.set_flag(n, ObjFlags::CLICK_FOCUSABLE, on);
                let g = e.default_group();
                match (on, e.group_of(n), g) {
                    (true, None, Some(g)) => e.group_add(g, n),
                    (false, Some(_), _) => e.group_remove(n),
                    _ => {}
                }
            });
        })
    }

    // ---- Events -------------------------------------------------------------------------

    event_mods! {
        /// Clicked (released without scrolling).
        on_click => Clicked;
        /// Pressed.
        on_press => Pressed;
        /// Released.
        on_release => Released;
        /// Long pressed.
        on_long_press => LongPressed;
        /// Repeated while long pressed.
        on_long_press_repeat => LongPressedRepeat;
        /// Got the focus.
        on_focus => Focused;
        /// Lost the focus.
        on_defocus => Defocused;
        /// The widget's value changed (widgets with a value also have a typed `.on_change`).
        on_value_changed => ValueChanged;
    }

    /// A swipe gesture on the node, with the side it moved towards.
    #[must_use]
    fn on_gesture(self, mut f: impl FnMut(Side) + 'static) -> Self {
        self.op(move |cx, node| {
            let rt = cx.runtime();
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Gesture), move |ecx, ev| {
                    if let Some(d) = ev.dir() {
                        EngineAccess::provide(rt, ecx.engine_mut(), || f(d));
                    }
                    EventResult::Continue
                });
        })
    }

    /// A key sent to the focused node.
    #[must_use]
    fn on_key(self, mut f: impl FnMut(twine_engine::Key) + 'static) -> Self {
        self.op(move |cx, node| {
            let rt = cx.runtime();
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Key), move |ecx, ev| {
                    if let Some(k) = ev.key() {
                        EngineAccess::provide(rt, ecx.engine_mut(), || f(k));
                    }
                    EventResult::Continue
                });
        })
    }

    /// The node scrolled; `f` receives the new scroll offset.
    #[must_use]
    fn on_scroll(self, mut f: impl FnMut(Point) + 'static) -> Self {
        self.op(move |cx, node| {
            let rt = cx.runtime();
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Scroll), move |ecx, ev| {
                    if ev.target == ecx.node() {
                        let p = ecx.engine().scroll_offset(ev.target);
                        EngineAccess::provide(rt, ecx.engine_mut(), || f(p));
                    }
                    EventResult::Continue
                });
        })
    }

    /// Any event with full access to the [`EventCx`] (the engine is reached through the
    /// context itself here, not through [`EngineAccess`]).
    #[must_use]
    fn on_event(self, code: EventCode, mut f: impl FnMut(&mut EventCx<'_>) + 'static) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .add_event_handler(node, EventFilter::Code(code), move |ecx, _ev| {
                    f(ecx);
                    EventResult::Continue
                });
        })
    }

    // ---- Identity -----------------------------------------------------------------------

    /// A test id for queries in tests and tree dumps.
    #[must_use]
    fn test_id(self, id: &'static str) -> Self {
        self.op(move |cx, node| cx.engine().set_test_id(node, id))
    }

    /// Fills `r` with the node when it is built.
    #[must_use]
    fn node_ref(self, r: NodeRef<Self::Widget>) -> Self {
        self.op(move |_cx, node| r.fill(node))
    }

    /// Adds the node to focus group `g`.
    #[must_use]
    fn group(self, g: GroupId) -> Self {
        self.op(move |cx, node| {
            let e = cx.engine();
            if e.group_of(node) != Some(g) {
                e.group_add(g, node);
            }
        })
    }
}
