//! [`ViewExt`]: the modifiers every widget view has.

use alloc::boxed::Box;

use twine_core::{Angle, Color, Insets, Opa, Point, Scale};
use twine_engine::{
    Engine, EventCode, EventCx, EventFilter, EventResult, GroupId, NodeId, ObjFlags, State, Widget,
    fmt_node_id,
};
use twine_image::ImageSource;
use twine_render::{BlendMode, BorderSide, Gradient, ShadowDsc};
use twine_style::{
    Align, Dir, GridAlign, Length, ScrollSnap, ScrollbarMode, Selector, Style, StyleProp, StyleRef,
    TransitionDsc,
};
use twine_text::{Font, TextAlign, TextDecor};

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

macro_rules! style_mods {
    ($($(#[$m:meta])* $name:ident: $ty:ty => $variant:ident;)*) => {
        $(
            $(#[$m])*
            #[must_use]
            fn $name(self, v: impl IntoProp<$ty>) -> Self {
                self.style_prop(v, StyleProp::$variant)
            }
        )*
    };
}

macro_rules! length_mods {
    ($($(#[$m:meta])* $name:ident => $variant:ident;)*) => {
        $(
            $(#[$m])*
            #[must_use]
            fn $name<L: Into<Length> + 'static>(self, v: impl IntoProp<L>) -> Self {
                self.style_prop(v, |l: L| StyleProp::$variant(l.into()))
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

    // ---- Size and position --------------------------------------------------------------

    length_mods! {
        /// Width (`Px`, `Pct` of the parent's content width, `Content`).
        width => Width;
        /// Height.
        height => Height;
        /// Minimum width.
        min_width => MinWidth;
        /// Maximum width.
        max_width => MaxWidth;
        /// Minimum height.
        min_height => MinHeight;
        /// Maximum height.
        max_height => MaxHeight;
        /// X position relative to the alignment point.
        x => X;
        /// Y position relative to the alignment point.
        y => Y;
    }

    /// Width and height.
    #[must_use]
    fn size<L1: Into<Length> + 'static, L2: Into<Length> + 'static>(
        self,
        w: impl IntoProp<L1>,
        h: impl IntoProp<L2>,
    ) -> Self {
        self.width(w).height(h)
    }

    /// X and Y position.
    #[must_use]
    fn pos<L1: Into<Length> + 'static, L2: Into<Length> + 'static>(
        self,
        x: impl IntoProp<L1>,
        y: impl IntoProp<L2>,
    ) -> Self {
        self.x(x).y(y)
    }

    style_mods! {
        /// Alignment in the parent (the position becomes an offset from it).
        align: Align => Align;
    }

    /// Aligns the node to the node of `base` (LVGL `lv_obj_align_to`), following it when it
    /// moves. The reference must be filled (by `.node_ref(base)` on another view) before the
    /// layout runs; until then the relation is not set.
    #[must_use]
    fn align_to<W: Widget>(self, base: NodeRef<W>, align: Align, dx: i32, dy: i32) -> Self {
        self.op(move |cx, node| {
            let scope = cx.scope();
            cx.provide(|| {
                bind_effect(
                    scope,
                    node,
                    move || base.get(),
                    move |e, n, b: Option<NodeId>| match b {
                        Some(b) => e.align_to(n, b, align, dx, dy),
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

    /// Translation after layout (`TranslateX`, `TranslateY`).
    #[must_use]
    fn translate<L1: Into<Length> + 'static, L2: Into<Length> + 'static>(
        self,
        x: impl IntoProp<L1>,
        y: impl IntoProp<L2>,
    ) -> Self {
        self.style_prop(x, |v: L1| StyleProp::TranslateX(v.into()))
            .style_prop(y, |v: L2| StyleProp::TranslateY(v.into()))
    }

    /// Translation after layout as one point (handy to bind to an animated `Point`).
    #[must_use]
    fn offset(self, p: impl IntoProp<Point>) -> Self {
        self.style_props(p, |p: Point| {
            [
                StyleProp::TranslateX(Length::Px(p.x)),
                StyleProp::TranslateY(Length::Px(p.y)),
            ]
        })
    }

    // ---- Spacing ------------------------------------------------------------------------

    /// Padding on all four sides.
    #[must_use]
    fn padding(self, v: impl IntoProp<i32>) -> Self {
        self.style_props(v, |v| {
            [
                StyleProp::PadTop(v),
                StyleProp::PadBottom(v),
                StyleProp::PadLeft(v),
                StyleProp::PadRight(v),
            ]
        })
    }

    /// Left and right padding.
    #[must_use]
    fn padding_hor(self, v: impl IntoProp<i32>) -> Self {
        self.style_props(v, |v| [StyleProp::PadLeft(v), StyleProp::PadRight(v)])
    }

    /// Top and bottom padding.
    #[must_use]
    fn padding_ver(self, v: impl IntoProp<i32>) -> Self {
        self.style_props(v, |v| [StyleProp::PadTop(v), StyleProp::PadBottom(v)])
    }

    /// Padding per side.
    #[must_use]
    fn padding_each(self, v: impl IntoProp<Insets>) -> Self {
        self.style_props(v, |i: Insets| {
            [
                StyleProp::PadTop(i.top),
                StyleProp::PadBottom(i.bottom),
                StyleProp::PadLeft(i.left),
                StyleProp::PadRight(i.right),
            ]
        })
    }

    /// Margin on all four sides.
    #[must_use]
    fn margin(self, v: impl IntoProp<i32>) -> Self {
        self.style_props(v, |v| {
            [
                StyleProp::MarginTop(v),
                StyleProp::MarginBottom(v),
                StyleProp::MarginLeft(v),
                StyleProp::MarginRight(v),
            ]
        })
    }

    /// Margin per side.
    #[must_use]
    fn margin_each(self, v: impl IntoProp<Insets>) -> Self {
        self.style_props(v, |i: Insets| {
            [
                StyleProp::MarginTop(i.top),
                StyleProp::MarginBottom(i.bottom),
                StyleProp::MarginLeft(i.left),
                StyleProp::MarginRight(i.right),
            ]
        })
    }

    /// Gap between rows and columns of a flex or grid container.
    #[must_use]
    fn gap(self, v: impl IntoProp<i32>) -> Self {
        self.style_props(v, |v| [StyleProp::PadRow(v), StyleProp::PadColumn(v)])
    }

    style_mods! {
        /// Gap between rows.
        row_gap: i32 => PadRow;
        /// Gap between columns.
        column_gap: i32 => PadColumn;
    }

    // ---- Background ---------------------------------------------------------------------

    /// Background color, fully opaque.
    #[must_use]
    fn bg(self, c: impl IntoProp<Color>) -> Self {
        self.style_props(c, |c| [StyleProp::BgColor(c), StyleProp::BgOpa(Opa::COVER)])
    }

    style_mods! {
        /// Background color (see [`bg_opa`](Self::bg_opa); the default opacity is transparent).
        bg_color: Color => BgColor;
        /// Background opacity.
        bg_opa: Opa => BgOpa;
        /// Background gradient.
        bg_grad: &'static Gradient => BgGrad;
        /// Background image.
        bg_image: &'static ImageSource => BgImageSrc;
    }

    // ---- Border and outline -------------------------------------------------------------

    /// Border width and color (opaque).
    #[must_use]
    fn border(self, width: impl IntoProp<i32>, color: impl IntoProp<Color>) -> Self {
        self.style_prop(width, StyleProp::BorderWidth)
            .style_props(color, |c| {
                [StyleProp::BorderColor(c), StyleProp::BorderOpa(Opa::COVER)]
            })
    }

    style_mods! {
        /// Border width.
        border_width: i32 => BorderWidth;
        /// Border color.
        border_color: Color => BorderColor;
        /// Border opacity.
        border_opa: Opa => BorderOpa;
        /// Which sides have a border.
        border_side: BorderSide => BorderSide;
        /// Corner radius (`RADIUS_CIRCLE` = `0x7FFF` for "half of the shorter side").
        radius: i32 => Radius;
    }

    /// Outline width, color and padding (distance from the node).
    #[must_use]
    fn outline(
        self,
        width: impl IntoProp<i32>,
        color: impl IntoProp<Color>,
        pad: impl IntoProp<i32>,
    ) -> Self {
        self.style_prop(width, StyleProp::OutlineWidth)
            .style_props(color, |c| {
                [StyleProp::OutlineColor(c), StyleProp::OutlineOpa(Opa::COVER)]
            })
            .style_prop(pad, StyleProp::OutlinePad)
    }

    // ---- Shadow -------------------------------------------------------------------------

    /// All shadow properties at once.
    #[must_use]
    fn shadow(self, s: impl IntoProp<ShadowDsc>) -> Self {
        self.style_props(s, |s: ShadowDsc| {
            [
                StyleProp::ShadowWidth(s.width),
                StyleProp::ShadowOffsetX(s.ofs_x),
                StyleProp::ShadowOffsetY(s.ofs_y),
                StyleProp::ShadowSpread(s.spread),
                StyleProp::ShadowColor(s.color),
                StyleProp::ShadowOpa(s.opa),
            ]
        })
    }

    style_mods! {
        /// Shadow blur width.
        shadow_width: i32 => ShadowWidth;
        /// Shadow spread.
        shadow_spread: i32 => ShadowSpread;
        /// Shadow color.
        shadow_color: Color => ShadowColor;
        /// Shadow opacity.
        shadow_opa: Opa => ShadowOpa;
    }

    /// Shadow offset.
    #[must_use]
    fn shadow_offset(self, x: impl IntoProp<i32>, y: impl IntoProp<i32>) -> Self {
        self.style_prop(x, StyleProp::ShadowOffsetX)
            .style_prop(y, StyleProp::ShadowOffsetY)
    }

    // ---- Text ---------------------------------------------------------------------------

    style_mods! {
        /// Font (inherited by the children).
        font: &'static Font => TextFont;
        /// Text color (inherited).
        text_color: Color => TextColor;
        /// Text opacity (inherited).
        text_opa: Opa => TextOpa;
        /// Text alignment (inherited).
        text_align: TextAlign => TextAlign;
        /// Extra space between letters (inherited).
        letter_space: i32 => TextLetterSpace;
        /// Extra space between lines (inherited).
        line_space: i32 => TextLineSpace;
        /// Underline / strikethrough (inherited).
        text_decor: TextDecor => TextDecor;
    }

    // ---- Visual -------------------------------------------------------------------------

    style_mods! {
        /// Opacity of the whole subtree, rendered through a layer (`OpaLayered`).
        opacity: Opa => OpaLayered;
        /// Rotation of the rendered node.
        transform_rotation: Angle => TransformRotation;
        /// Blend mode.
        blend_mode: BlendMode => BlendMode;
        /// Clips the children to the rounded corners.
        clip_corner: bool => ClipCorner;
    }

    /// Scale of the rendered node (both axes).
    #[must_use]
    fn transform_scale(self, s: impl IntoProp<Scale>) -> Self {
        self.style_props(s, |s| {
            [StyleProp::TransformScaleX(s), StyleProp::TransformScaleY(s)]
        })
    }

    /// Pivot of the transformation, relative to the node.
    #[must_use]
    fn transform_pivot(self, p: impl IntoProp<Point>) -> Self {
        self.style_props(p, |p: Point| {
            [
                StyleProp::TransformPivotX(Length::Px(p.x)),
                StyleProp::TransformPivotY(Length::Px(p.y)),
            ]
        })
    }

    /// Recolors everything the node draws with `color` at `opa` (LVGL `recolor`).
    #[must_use]
    fn recolor(self, color: impl IntoProp<Color>, opa: impl IntoProp<Opa>) -> Self {
        self.style_prop(color, StyleProp::Recolor)
            .style_prop(opa, StyleProp::RecolorOpa)
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

    /// Which directions can be scrolled.
    #[must_use]
    fn scroll_dir(self, d: impl IntoProp<Dir>) -> Self {
        let p = d.into_prop();
        self.op(move |cx, node| bind_node(cx, node, p, Engine::set_scroll_dir))
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

    /// Animates style changes between states with `t`.
    #[must_use]
    fn transition(self, t: impl IntoProp<&'static TransitionDsc>) -> Self {
        self.style_prop(t, StyleProp::Transition)
    }

    // ---- Layout as a child --------------------------------------------------------------

    style_mods! {
        /// Share of the free main-axis space in a flex parent (0 = none).
        flex_grow: u8 => FlexGrow;
    }

    /// Grid cell: column, column span, row, row span.
    #[must_use]
    fn grid_cell(self, col: i32, col_span: i32, row: i32, row_span: i32) -> Self {
        self.style_props(Prop::Static(()), move |()| {
            [
                StyleProp::GridCellColumnPos(col),
                StyleProp::GridCellColumnSpan(col_span),
                StyleProp::GridCellRowPos(row),
                StyleProp::GridCellRowSpan(row_span),
            ]
        })
    }

    /// Alignment inside the grid cell (horizontal, vertical).
    #[must_use]
    fn grid_cell_align(self, x: impl IntoProp<GridAlign>, y: impl IntoProp<GridAlign>) -> Self {
        self.style_prop(x, StyleProp::GridCellXAlign)
            .style_prop(y, StyleProp::GridCellYAlign)
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

    /// A swipe gesture on the node.
    #[must_use]
    fn on_gesture(self, mut f: impl FnMut(Dir) + 'static) -> Self {
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
