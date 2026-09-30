//! Layout containers: [`column()`], [`row`], [`flex`], [`grid`], [`container`], [`stack`],
//! [`spacer`], [`scroll_view`].

use twine_core::Opa;
use twine_engine::{Engine, NodeId, Obj, ObjFlags};

use alloc::vec::Vec;
use core::cell::Cell;

use twine_style::{
    Align, Axis, CrossAlign, FlexDirection, FlexFlow, GridAlign, GridTrack, LayoutKind, Length, MainAlign,
    Radius, ScrollbarMode, Selector, StyleProp,
};

use crate::bind::bind_node;
use crate::build::{BuildCx, BuildOp, WidgetView, widget_view};
use crate::modifiers::ViewExt;
use crate::prop::IntoProp;
use crate::view::{View, ViewSeq};

/// Makes a container transparent like LVGL's `lv_obj` with the `transp` style: no
/// background, border, padding, radius or shadow (the theme's card look is overridden).
fn transparent(e: &mut Engine, n: NodeId) {
    for p in [
        StyleProp::BgOpacity(Opa::TRANSP),
        StyleProp::BorderWidth(Length::Px(0)),
        StyleProp::PaddingTop(Length::Px(0)),
        StyleProp::PaddingBottom(Length::Px(0)),
        StyleProp::PaddingLeft(Length::Px(0)),
        StyleProp::PaddingRight(Length::Px(0)),
        StyleProp::Radius(Radius::Px(0)),
        StyleProp::ShadowWidth(0),
    ] {
        e.set_local_prop(n, Selector::MAIN, p);
    }
}

macro_rules! forward_view_ext {
    ($t:ident) => {
        impl View for $t {
            fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
                self.0.build(cx)
            }
        }

        impl ViewExt for $t {
            type Widget = Obj;
            fn push_op(self, op: BuildOp) -> Self {
                $t(self.0.push_op(op))
            }
        }
    };
}

/// A flex container ([`column()`], [`row`], [`flex`], [`scroll_view`]).
#[derive(Debug)]
#[must_use]
pub struct Flex(WidgetView<Obj>);

forward_view_ext!(Flex);

impl Flex {
    /// Items wrap into several tracks when they do not fit (`false` by default).
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = flex(FlexDirection::Row, (label("a"), label("b"))).wrap(true).reverse(false);
    /// ```
    pub fn wrap(mut self, on: bool) -> Self {
        let flow = self.0.shared::<Cell<FlexFlow>>();
        flow.set(flow.get().wrap(on));
        self
    }

    /// Items are placed from the last child to the first (`false` by default).
    pub fn reverse(mut self, on: bool) -> Self {
        let flow = self.0.shared::<Cell<FlexFlow>>();
        flow.set(flow.get().reverse(on));
        self
    }

    /// Placement of the items on the main axis, like CSS `justify-content`.
    pub fn justify(self, a: impl IntoProp<MainAlign>) -> Self {
        self.style_prop(a, StyleProp::FlexMainAlign)
    }

    /// Placement of the items across the main axis, like CSS `align-items`: inside their
    /// track and, with it, the track(s) in the container (LVGL's cross and track placement;
    /// use [`align_content`](Self::align_content) afterwards to place the tracks differently).
    ///
    /// Distributing space is a main-axis or track placement, so it does not compile here:
    ///
    /// ```compile_fail
    /// use twine_view::prelude::*;
    /// let _v = column(label("a")).align_items(MainAlign::SpaceBetween);
    /// ```
    pub fn align_items(self, a: impl IntoProp<CrossAlign>) -> Self {
        self.style_props(a, |a| {
            [
                StyleProp::FlexCrossAlign(a),
                StyleProp::FlexTrackAlign(a.to_main()),
            ]
        })
    }

    /// Placement of the tracks of a wrapping container, like CSS `align-content` (LVGL track
    /// placement).
    pub fn align_content(self, a: impl IntoProp<MainAlign>) -> Self {
        self.style_prop(a, StyleProp::FlexTrackAlign)
    }
}

/// A grid container ([`grid`]).
#[derive(Debug)]
#[must_use]
pub struct Grid(WidgetView<Obj>);

forward_view_ext!(Grid);

impl Grid {
    /// How the columns are placed in the container.
    pub fn column_align(self, a: impl IntoProp<GridAlign>) -> Self {
        self.style_prop(a, StyleProp::GridColumnAlign)
    }

    /// How the rows are placed in the container.
    pub fn row_align(self, a: impl IntoProp<GridAlign>) -> Self {
        self.style_prop(a, StyleProp::GridRowAlign)
    }
}

/// A plain container ([`container`], [`stack`], [`spacer`]).
#[derive(Debug)]
#[must_use]
pub struct Container(WidgetView<Obj>);

forward_view_ext!(Container);

/// A transparent flex container. The flow is a shared setting so that [`Flex::wrap`] and
/// [`Flex::reverse`] (called after the constructor) are applied by the first build step, once.
fn flex_view(flow: FlexFlow, children: impl ViewSeq) -> Flex {
    let mut v = widget_view(|| Obj);
    let shared = v.shared::<Cell<FlexFlow>>();
    shared.set(flow);
    Flex(
        v.op(move |cx, n| {
            let e = cx.engine();
            transparent(e, n);
            e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(LayoutKind::Flex));
            e.set_local_prop(n, Selector::MAIN, StyleProp::FlexFlow(shared.get()));
        })
        .children(children),
    )
}

/// A flex column (no wrapping), transparent: no background, border or padding (LVGL
/// `lv_obj` with the `transp` style), content-sized unless sized.
///
/// ```
/// use twine_view::prelude::*;
/// let _v = column((label("a"), label("b"))).gap(8).align_items(CrossAlign::Center);
/// ```
pub fn column(children: impl ViewSeq) -> Flex {
    flex_view(FlexFlow::COLUMN, children)
}

/// A flex row (no wrapping), transparent like [`column()`].
pub fn row(children: impl ViewSeq) -> Flex {
    flex_view(FlexFlow::ROW, children)
}

/// A flex container along `direction`, transparent like [`column()`]; add wrapping and reverse
/// order with [`Flex::wrap`] and [`Flex::reverse`].
///
/// ```
/// use twine_view::prelude::*;
/// let _v = flex(FlexDirection::Column, (label("a"), label("b"), label("c"))).wrap(true);
/// ```
#[doc(alias = "FlexFlow")]
pub fn flex(direction: FlexDirection, children: impl ViewSeq) -> Flex {
    flex_view(FlexFlow::new(direction), children)
}

/// A grid container with `columns` × `rows` tracks, transparent like [`column()`]. The tracks
/// are any `Vec<GridTrack>` property: a [`grid_tracks!`](twine_style::grid_tracks) list, a
/// vector, or a signal/closure (the grid is laid out again when they change). Place children
/// with [`grid_col`](crate::ViewExt::grid_col) / [`grid_row`](crate::ViewExt::grid_row) and
/// align them in their cell with [`grid_align`](crate::ViewExt::grid_align).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = grid(
///     grid_tracks![fr(1), px(80), content],
///     grid_tracks![content],
///     (label("a").grid_col(0..2).grid_row(0), label("b").grid_col(2).grid_row(0)),
/// );
/// ```
pub fn grid(
    columns: impl IntoProp<Vec<GridTrack>>,
    rows: impl IntoProp<Vec<GridTrack>>,
    children: impl ViewSeq,
) -> Grid {
    let (columns, rows) = (columns.into_prop(), rows.into_prop());
    Grid(
        widget_view(|| Obj)
            .op(move |cx, n| {
                let e = cx.engine();
                transparent(e, n);
                e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(LayoutKind::Grid));
                bind_node(cx, n, columns, |e: &mut Engine, n, t: Vec<GridTrack>| {
                    e.set_grid_column_tracks(n, t);
                });
                bind_node(cx, n, rows, |e: &mut Engine, n, t: Vec<GridTrack>| {
                    e.set_grid_row_tracks(n, t);
                });
            })
            .children(children),
    )
}

/// A plain object with the theme's container look (a card in the default theme) and no
/// layout: children are positioned by their own position and alignment.
pub fn container(children: impl ViewSeq) -> Container {
    Container(widget_view(|| Obj).children(children))
}

/// A [`container`] whose children are all centered on top of each other.
pub fn stack(children: impl ViewSeq) -> Container {
    Container(widget_view(|| Obj).children(children).after_children(|cx, n| {
        let e = cx.engine();
        let kids: alloc::vec::Vec<NodeId> = e.tree().children(n).collect();
        for c in kids {
            e.set_align(c, Align::Center);
        }
    }))
}

/// An invisible item taking the free space of a flex container (`flex_grow(1)`).
pub fn spacer() -> Container {
    Container(widget_view(|| Obj).op(|cx, n| {
        let e = cx.engine();
        transparent(e, n);
        e.set_flag(n, ObjFlags::CLICKABLE | ObjFlags::SCROLLABLE, false);
        e.set_local_prop(n, Selector::MAIN, StyleProp::FlexGrow(1));
        e.set_local_prop(n, Selector::MAIN, StyleProp::Width(Length::Px(0)));
        e.set_local_prop(n, Selector::MAIN, StyleProp::Height(Length::Px(0)));
    }))
}

/// A scrollable column ([`Axis::Vertical`], also for `Both`) or row ([`Axis::Horizontal`])
/// with scrollbars shown while needed, transparent like [`column()`].
pub fn scroll_view(axis: Axis, children: impl ViewSeq) -> Flex {
    let flow = if axis == Axis::Horizontal {
        FlexFlow::ROW
    } else {
        FlexFlow::COLUMN
    };
    flex_view(flow, children)
        .scrollable(true)
        .scroll_dir(axis)
        .scrollbar(ScrollbarMode::Auto)
}
