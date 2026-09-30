//! [`ViewExt`]: the modifiers every widget view has.

use alloc::boxed::Box;

use twine_core::Point;
use twine_engine::{
    Engine, EventCode, EventCx, EventFilter, EventResult, GroupId, NodeId, ObjFlags, State, Widget,
    fmt_node_id,
};
use twine_style::{Anchor, Axis, ScrollSnap, ScrollbarMode, Selector, Side, Style, StyleProp, StyleRef};

use crate::access::EngineAccess;
use crate::bind::{bind_effect, bind_node};
use crate::build::{BuildCx, BuildOp};
use crate::node_ref::NodeRef;
use crate::prop::{IntoProp, Prop};
use crate::view::View;

/// Sets `props` as local `Main` properties of `node` (idempotent in the engine).
fn set_main(e: &mut Engine, node: NodeId, props: &[StyleProp]) {
    for p in props {
        e.set_local_prop(node, Selector::MAIN, *p);
    }
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
    mut f: impl FnMut() -> R + 'static,
) -> impl FnMut(&mut EventCx<'_>, &twine_engine::Event) -> EventResult + 'static {
    move |cx, _ev| {
        EngineAccess::provide(cx.engine_mut(), || {
            f();
        });
        EventResult::Continue
    }
}

/// One style-property modifier of [`ViewExt`] (see `prop_modifiers!`): `len` properties take any
/// `Into<Length>` (pixels, `Length::pct`, `Length::dp`, …), `radius` any `Into<Radius>`
/// (pixels, `Radius::Circle`, …), `dur` any `Into<DurationMs>` (a `Duration`), the others
/// their payload type.
macro_rules! prop_modifier {
    (len $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        prop_modifier! { into $name $key [$($ty)+] [$(#[$m])*] }
    };
    (radius $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        prop_modifier! { into $name $key [$($ty)+] [$(#[$m])*] }
    };
    (dur $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        prop_modifier! { into $name $key [$($ty)+] [$(#[$m])*] }
    };
    (into $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        $(#[$m])*
        #[must_use]
        fn $key<L: ::core::convert::Into<::twine_style::__prop_ty!($($ty)+)> + 'static>(
            self,
            v: impl $crate::prop::IntoProp<L>,
        ) -> Self {
            self.style_prop(v, |l: L| ::twine_style::StyleProp::$name(::core::convert::Into::into(l)))
        }
    };
    ($kind:ident $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        $(#[$m])*
        #[must_use]
        fn $key(self, v: impl $crate::prop::IntoProp<::twine_style::__prop_ty!($($ty)+)>) -> Self {
            self.style_prop(v, ::twine_style::StyleProp::$name)
        }
    };
}

/// Generates one modifier per style property from `twine_style::__prop_table!`, named like
/// the `style!` key and the `StyleBuf` builder method.
macro_rules! prop_modifiers {
    (
        []
        $(
            $(#[doc = $doc:literal])*
            $name:ident ( $key:ident ) : $kind:ident [$($ty:tt)+] [ $($flag:ident)* ] $default:ident
                [ $($alias:literal)* ];
        )*
    ) => {
        $(
            prop_modifier! {
                $kind $name $key [$($ty)+]
                [
                    $(#[doc = $doc])*
                    ///
                    #[doc = ::core::concat!(
                        "Sets the local `Main` style property [`StyleProp::", ::core::stringify!($name),
                        "`](::twine_style::StyleProp::", ::core::stringify!($name), ")."
                    )]
                    $(#[doc(alias = $alias)])*
                ]
            }
        )*
    };
}

/// A shorthand parameter: `impl IntoProp<G>` for `len<G>` parameters, else
/// `impl IntoProp<payload type>`.
macro_rules! shorthand_param {
    (len <$g:ident> [$($ty:tt)+]) => { impl $crate::prop::IntoProp<$g> };
    (span [$($ty:tt)+]) => { impl $crate::prop::IntoGridSpan };
    ($kind:ident [$($ty:tt)+]) => { impl $crate::prop::IntoProp<::twine_style::__prop_ty!($($ty)+)> };
}

/// The property of a shorthand parameter: `span` parameters go through
/// [`IntoGridSpan`](crate::prop::IntoGridSpan), the others are properties already.
macro_rules! shorthand_arg {
    (span $p:ident) => {
        $crate::prop::IntoGridSpan::into_grid_span($p)
    };
    ($kind:ident $p:ident) => {
        $p
    };
}

/// Converts a `len` shorthand parameter to a `Length` (other parameters already have their type).
macro_rules! shorthand_let {
    (len $p:ident) => {
        let $p: ::twine_style::Length = ::core::convert::Into::into($p);
    };
    ($kind:ident $p:ident) => {};
}

/// Generates the shorthand modifiers (`padding`, `size`, `border`, …) from
/// `twine_style::__shorthand_table!`: each parameter is one binding that sets its properties.
macro_rules! shorthand_modifiers {
    (
        []
        $(
            $(#[doc = $doc:literal])*
            $name:ident (
                $(
                    $p:ident : $pk:ident $(<$g:ident>)? [$($pty:tt)+]
                        => $( $var:ident $( ( $($sel:tt)+ ) )? $( = $c:ident )? ),+
                );+
            ) [ $($alias:literal)* ] { $(#[doc = $ex:literal])* };
        )*
    ) => {
        $(
            $(#[doc = $doc])*
            $(#[doc(alias = $alias)])*
            #[must_use]
            fn $name<$($($g: ::core::convert::Into<::twine_style::Length> + 'static,)?)+>(
                self,
                $( $p: shorthand_param!($pk $(<$g>)? [$($pty)+]) ),+
            ) -> Self {
                self $(
                    .style_props(shorthand_arg!($pk $p), |$p| {
                        shorthand_let!($pk $p);
                        [$( ::twine_style::__shorthand_prop!([$var] [$($($sel)+)?] [$($c)?] val $p) ),+]
                    })
                )+
            }
        )*
    };
}

macro_rules! flag_mods {
    ($($(#[$m:meta])* $name:ident => $flag:ident;)*) => {
        $(
            $(#[$m])*
            #[must_use]
            fn $name(self, on: impl IntoProp<bool>) -> Self {
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

/// The modifiers of every widget view: sizes and positions, spacing, colors, borders,
/// shadows, text, visual effects, flags, styles, layout item properties, events and identity
/// (the table of the view API). Style modifiers set **local style properties** of the `Main`
/// part; flag modifiers set object flags. Every value may be a constant, a signal, a memo or a
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
/// let cx = twine_reactive::create_root();
/// let hot = cx.signal(false);
/// let _v = label("21 °C")
///     .text_color(move || if hot.get() { Color::RED } else { Color::BLACK })
///     .padding(4)
///     .bg(Color::hex(0xEEEEEE))
///     .radius(6)
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

    /// Binds a local `Main` style property built by `make` from the value.
    #[must_use]
    fn style_prop<T: 'static>(self, v: impl IntoProp<T>, make: impl Fn(T) -> StyleProp + 'static) -> Self {
        let p = v.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, move |e, n, v| set_main(e, n, &[make(v)])))
    }

    /// Binds several local `Main` style properties built by `make` from one value.
    #[must_use]
    fn style_props<T: 'static, const N: usize>(
        self,
        v: impl IntoProp<T>,
        make: impl Fn(T) -> [StyleProp; N] + 'static,
    ) -> Self {
        let p = v.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, move |e, n, v| set_main(e, n, &make(v))))
    }

    /// Binds an object flag.
    #[must_use]
    fn flag(self, flag: ObjFlags, on: impl IntoProp<bool>) -> Self {
        let p = on.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, move |e, n, on| e.set_flag(n, flag, on)))
    }

    /// Binds a state (added while the value is `true`).
    #[must_use]
    fn state(self, state: State, on: impl IntoProp<bool>) -> Self {
        let p = on.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, move |e, n, on| e.set_state(n, state, on)))
    }

    /// Calls `f` for every event `code` of the node (the engine is available through
    /// [`EngineAccess`] meanwhile). Its result is ignored.
    #[must_use]
    fn on_code<R>(self, code: EventCode, f: impl FnMut() -> R + 'static) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .add_event_handler(node, EventFilter::Code(code), simple_handler(f));
        })
    }

    // ---- Style properties -----------------------------------------------------------------
    //
    // One modifier per style property and one per shorthand, generated from the property
    // table of `twine-style`: the names are the `style!` keys and the `StyleBuf` builder
    // methods (see `twine_style::PROPERTIES.md`).

    ::twine_style::__prop_table!(prop_modifiers);

    ::twine_style::__shorthand_table!(shorthand_modifiers);

    // ---- Relations --------------------------------------------------------------------------

    /// Places the node relative to the node of `base` (LVGL `lv_obj_align_to`), following it
    /// when it moves: next to it (`Anchor::BelowLeft`, …) or inside it (an [`Align`](twine_style::Align),
    /// e.g. `Align::Center`). The reference must be filled (by `.node_ref(base)` on another
    /// view) before the layout runs; until then the relation is not set. `anchor`, `dx` and
    /// `dy` may be dynamic.
    #[must_use]
    fn align_to<W: Widget>(
        self,
        base: NodeRef<W>,
        anchor: impl IntoProp<Anchor>,
        dx: impl IntoProp<i32>,
        dy: impl IntoProp<i32>,
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
    #[must_use]
    fn disabled(self, on: impl IntoProp<bool>) -> Self {
        self.state(State::DISABLED, on)
    }

    /// Which directions can be scrolled (all by default; stop scrolling with
    /// [`scrollable(false)`](Self::scrollable)).
    #[must_use]
    fn scroll_dir(self, axis: impl IntoProp<Axis>) -> Self {
        let p = axis.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, |e: &mut Engine, n, a: Axis| e.set_scroll_dir(n, a)))
    }

    /// When scrollbars are shown.
    #[must_use]
    fn scrollbar(self, m: impl IntoProp<ScrollbarMode>) -> Self {
        let p = m.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scrollbar_mode))
    }

    /// Horizontal scroll snapping of the children.
    #[must_use]
    fn scroll_snap_x(self, s: impl IntoProp<ScrollSnap>) -> Self {
        let p = s.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scroll_snap_x))
    }

    /// Vertical scroll snapping of the children.
    #[must_use]
    fn scroll_snap_y(self, s: impl IntoProp<ScrollSnap>) -> Self {
        let p = s.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scroll_snap_y))
    }

    /// Keypad / encoder focusable: in the default focus group (and focused when clicked).
    #[must_use]
    fn focusable(self, on: impl IntoProp<bool>) -> Self {
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

    // ---- Styles -------------------------------------------------------------------------

    /// Adds a style for the `Main` part in the default state.
    #[must_use]
    fn style(self, s: &'static Style) -> Self {
        self.style_for(Selector::MAIN, s)
    }

    /// Adds a style for a part and state.
    #[must_use]
    fn style_for(self, sel: Selector, s: &'static Style) -> Self {
        self.style_ref(sel, StyleRef::Static(s))
    }

    /// Adds a style (static or shared heap style) for a part and state.
    #[must_use]
    fn style_ref(self, sel: Selector, s: StyleRef) -> Self {
        self.op(move |cx, node| cx.engine().add_style(node, s, sel))
    }

    /// Adds a style with theme priority (below every style added with
    /// [`style`](Self::style), like the theme's own class styles).
    #[must_use]
    fn class_style(self, sel: Selector, s: &'static Style) -> Self {
        self.op(move |cx, node| cx.engine().add_theme_style(node, StyleRef::Static(s), sel))
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
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Gesture), move |ecx, ev| {
                    if let Some(d) = ev.dir() {
                        EngineAccess::provide(ecx.engine_mut(), || f(d));
                    }
                    EventResult::Continue
                });
        })
    }

    /// A key sent to the focused node.
    #[must_use]
    fn on_key(self, mut f: impl FnMut(twine_engine::Key) + 'static) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Key), move |ecx, ev| {
                    if let Some(k) = ev.key() {
                        EngineAccess::provide(ecx.engine_mut(), || f(k));
                    }
                    EventResult::Continue
                });
        })
    }

    /// The node scrolled; `f` receives the new scroll offset.
    #[must_use]
    fn on_scroll(self, mut f: impl FnMut(Point) + 'static) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Scroll), move |ecx, ev| {
                    if ev.target == ecx.node() {
                        let p = ecx.engine().scroll_offset(ev.target);
                        EngineAccess::provide(ecx.engine_mut(), || f(p));
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
