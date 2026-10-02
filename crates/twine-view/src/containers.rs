//! Layout containers: [`column()`], [`row`], [`flex`], [`grid`], [`container`], [`card`],
//! [`stack`], [`spacer`], [`scroll_view`], and the [`Layout`] of
//! [`StyleExt::layout`](crate::StyleExt::layout).

use twine_core::Opa;
use twine_engine::{Engine, NodeId, Obj, ObjFlags, Widget};
use twine_widgets::container::Card;

use core::cell::Cell;

use twine_style::{
    Align, Axis, CrossAlign, FlexDirection, FlexFlow, GridAlign, GridTracks, LayoutKind, Length, MainAlign,
    PropId, Radius, ScrollbarMode, Selector, StyleProp,
};

use crate::build::{BuildCx, BuildOp, WidgetView, widget_view};
use crate::modifiers::ViewExt;
use crate::prop::IntoProp;
use crate::style_ext::StyleExt;
use crate::view::{View, ViewSeq};

/// Makes a container transparent like LVGL's `lv_obj` with the `transp` style: no
/// background, border, padding, radius or shadow (the theme's card look is overridden).
fn transparent(e: &mut Engine, n: NodeId) {
    for p in [
        StyleProp::BgOpacity(Opa::TRANSP.into()),
        StyleProp::BorderWidth(Length::Px(0).into()),
        StyleProp::PaddingTop(Length::Px(0).into()),
        StyleProp::PaddingBottom(Length::Px(0).into()),
        StyleProp::PaddingLeft(Length::Px(0).into()),
        StyleProp::PaddingRight(Length::Px(0).into()),
        StyleProp::Radius(Radius::Px(0).into()),
        StyleProp::ShadowWidth(0),
    ] {
        e.set_local_prop(n, Selector::MAIN, p);
    }
}

/// How a view arranges its children, set with [`StyleExt::layout`](crate::StyleExt::layout) on
/// any view (typically a [`container`] or a [`card`]).
///
/// Each variant expands to exactly the properties the matching container sets, with no
/// extra work: [`row()`](Self::row)/[`column()`](Self::column)/[`flex`](Self::flex) set the
/// `layout` and `flex_flow` style properties, [`grid`](Self::grid) sets the `layout` property
/// and the `grid_column_tracks` / `grid_row_tracks` properties (like the [`grid`](crate::grid)
/// container), [`none`](Self::none) sets `layout` only. All are local style properties, so a
/// layout may also depend on the state (`.on_state(State::CHECKED, |s| s.layout(..))`). A
/// layout may be dynamic (a signal or a closure returning a `Layout`), e.g. a row on wide
/// displays and a column on narrow ones.
///
/// Switching away from a grid keeps its track properties (unused until the node is a grid
/// again).
///
/// ```
/// use twine_view::prelude::*;
/// let _a = container((label("a"), label("b"))).layout(Layout::column());
/// let _b = container((label("a"), label("b"))).layout(Layout::flex(FlexFlow::ROW.wrap(true)));
/// let _c = container((label("a").grid_col(0), label("b").grid_col(1)))
///     .layout(Layout::grid(grid_tracks![fr(1), fr(1)], grid_tracks![content]));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(alias = "LayoutKind")]
#[doc(alias = "lv_obj_set_layout")]
pub enum Layout {
    /// No layout: children are placed by their own position and alignment
    /// ([`LayoutKind::None`]).
    None,
    /// A flex layout with this flow ([`LayoutKind::Flex`] + `flex_flow`).
    Flex(FlexFlow),
    /// A grid layout with these tracks ([`LayoutKind::Grid`] + the node's tracks).
    Grid {
        /// The column tracks (a `'static` list, or a list built at run time, held by the node's
        /// local style).
        columns: GridTracks,
        /// The row tracks.
        rows: GridTracks,
    },
}

impl Layout {
    /// No layout.
    /// ```
    /// use twine_view::prelude::*;
    /// // Children keep their own positions (`.pos(x, y)`).
    /// let _v = container((label("a").pos(4, 4), label("b").pos(40, 20))).layout(Layout::none());
    /// ```
    #[must_use]
    pub const fn none() -> Self {
        Self::None
    }

    /// A flex row without wrapping (like [`row`](crate::row)).
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = container((label("a"), label("b"))).layout(Layout::row());
    /// ```
    #[must_use]
    pub const fn row() -> Self {
        Self::Flex(FlexFlow::ROW)
    }

    /// A flex column without wrapping (like [`column`](crate::column)).
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = container((label("a"), label("b"))).layout(Layout::column());
    /// ```
    #[must_use]
    pub const fn column() -> Self {
        Self::Flex(FlexFlow::COLUMN)
    }

    /// A flex layout with any flow, e.g. `FlexFlow::ROW.wrap(true)` (like [`flex`](crate::flex)).
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = container((label("a"), label("b"))).layout(Layout::flex(FlexFlow::ROW.wrap(true)));
    /// ```
    #[must_use]
    pub const fn flex(flow: FlexFlow) -> Self {
        Self::Flex(flow)
    }

    /// A grid layout with `columns` × `rows` tracks, e.g. from
    /// [`grid_tracks!`](twine_style::grid_tracks) or `'static` slices (like [`grid`](crate::grid)).
    /// ```
    /// use twine_view::prelude::*;
    /// static ROWS: [GridTrack; 1] = [GridTrack::Content];
    /// let _v = container((label("a"), label("b")))
    ///     .layout(Layout::grid(grid_tracks![fr(1), px(80)], &ROWS));
    /// ```
    #[must_use]
    pub fn grid(columns: impl Into<GridTracks>, rows: impl Into<GridTracks>) -> Self {
        Self::Grid {
            columns: columns.into(),
            rows: rows.into(),
        }
    }

    /// Applies the layout to `n` as local properties for `sel` (the engine setters are
    /// idempotent).
    pub(crate) fn apply(self, e: &mut Engine, n: NodeId, sel: Selector) {
        match self {
            Self::None => e.set_local_prop(n, sel, StyleProp::Layout(LayoutKind::None)),
            Self::Flex(flow) => {
                e.set_local_prop(n, sel, StyleProp::Layout(LayoutKind::Flex));
                e.set_local_prop(n, sel, StyleProp::FlexFlow(flow));
            }
            Self::Grid { columns, rows } => {
                e.set_local_prop(n, sel, StyleProp::Layout(LayoutKind::Grid));
                e.set_local_grid_tracks(n, sel, PropId::GridColumnTracks, columns);
                e.set_local_grid_tracks(n, sel, PropId::GridRowTracks, rows);
            }
        }
    }
}

macro_rules! forward_view_ext {
    (generic $t:ident) => {
        impl<W: Widget> View for $t<W> {
            fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
                self.0.build(cx)
            }
        }

        impl<W: Widget> ViewExt for $t<W> {
            type Widget = W;
            fn push_op(self, op: BuildOp) -> Self {
                $t(self.0.push_op(op))
            }
        }
    };
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

/// A flex container ([`column()`], [`row`], [`flex`], [`scroll_view`]: a plain object; [`card`]:
/// `Flex<Card>`).
/// `Flex<Card>` (from [`card`]) is the same flex container on a [`Card`](twine_widgets::container::Card)
/// node, so it takes the card look of the theme and every `Flex` method:
///
/// ```
/// use twine_view::prelude::*;
/// let _v: Flex<Card> = card((label("Title"), label("Body"))).wrap(false).gap(8);
/// ```
#[derive(Debug)]
#[must_use]
pub struct Flex<W: Widget = Obj>(WidgetView<W>);

forward_view_ext!(generic Flex);

impl<W: Widget> Flex<W> {
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
    pub fn justify<M>(self, a: impl IntoProp<MainAlign, M>) -> Self {
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
    pub fn align_items<M>(self, a: impl IntoProp<CrossAlign, M>) -> Self {
        self.style_props(a, |a| {
            [
                StyleProp::FlexCrossAlign(a),
                StyleProp::FlexTrackAlign(a.to_main()),
            ]
        })
    }

    /// Placement of the tracks of a wrapping container, like CSS `align-content` (LVGL track
    /// placement).
    pub fn align_content<M>(self, a: impl IntoProp<MainAlign, M>) -> Self {
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
    pub fn column_align<M>(self, a: impl IntoProp<GridAlign, M>) -> Self {
        self.style_prop(a, StyleProp::GridColumnAlign)
    }

    /// How the rows are placed in the container.
    pub fn row_align<M>(self, a: impl IntoProp<GridAlign, M>) -> Self {
        self.style_prop(a, StyleProp::GridRowAlign)
    }
}

/// A plain container ([`container`], [`stack`], [`spacer`]).
#[derive(Debug)]
#[must_use]
pub struct Container(WidgetView<Obj>);

forward_view_ext!(Container);

/// A flex container of widget `W` (made transparent when `transparent`). The flow is a
/// shared setting so that [`Flex::wrap`] and [`Flex::reverse`] (called after the constructor)
/// are applied by the first build step, once.
fn flex_view<W: Widget>(
    widget: impl FnOnce() -> W + 'static,
    flow: FlexFlow,
    transparent_look: bool,
    children: impl ViewSeq,
) -> Flex<W> {
    let mut v = widget_view(widget);
    let shared = v.shared::<Cell<FlexFlow>>();
    shared.set(flow);
    Flex(
        v.op(move |cx, n| {
            let e = cx.engine();
            if transparent_look {
                transparent(e, n);
            }
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
    flex_view(|| Obj, FlexFlow::COLUMN, true, children)
}

/// A flex row (no wrapping), transparent like [`column()`].
pub fn row(children: impl ViewSeq) -> Flex {
    flex_view(|| Obj, FlexFlow::ROW, true, children)
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
    flex_view(|| Obj, FlexFlow::new(direction), true, children)
}

/// A grid container with `columns` × `rows` tracks, transparent like [`column()`]. The tracks
/// are any track-list property: a [`grid_tracks!`](twine_style::grid_tracks) list, a vector, a
/// `'static` slice, or a signal/closure (the grid is laid out again when they change). They are
/// the local `grid_column_tracks` / `grid_row_tracks` style properties, so state styles and
/// themes can override them. Place children
/// with [`grid_col`](crate::StyleExt::grid_col) / [`grid_row`](crate::StyleExt::grid_row) and
/// align them in their cell with [`grid_align`](crate::StyleExt::grid_align).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = grid(
///     grid_tracks![fr(1), px(80), content],
///     grid_tracks![content],
///     (label("a").grid_col(0..2).grid_row(0), label("b").grid_col(2).grid_row(0)),
/// );
/// ```
pub fn grid<M1, M2>(
    columns: impl IntoProp<GridTracks, M1>,
    rows: impl IntoProp<GridTracks, M2>,
    children: impl ViewSeq,
) -> Grid {
    Grid(
        widget_view(|| Obj)
            .op(|cx, n| {
                let e = cx.engine();
                transparent(e, n);
                e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(LayoutKind::Grid));
            })
            .grid_column_tracks(columns)
            .grid_row_tracks(rows)
            .children(children),
    )
}

/// A plain object with the theme's container look (a card in the default theme) and no
/// layout: children are positioned by their own position and alignment. Give it one with
/// [`.layout(..)`](crate::StyleExt::layout), e.g. `container(..).layout(Layout::row())`; for a
/// card with a column of children use [`card`].
pub fn container(children: impl ViewSeq) -> Container {
    Container(widget_view(|| Obj).children(children))
}

/// A card: a [`Card`] node — the theme's card look (background, border, radius, padding and
/// gap from the theme), applied by the theme to the card class explicitly, so a card looks
/// like one whatever the theme does for plain containers — with its children in a column. It
/// is a [`Flex`], so [`Flex::wrap`], [`Flex::justify`], [`Flex::align_items`], … and every
/// modifier (`.padding(..)`, `.gap(..)`, `.layout(..)`) refine it. Applications restyle all
/// cards through their theme's `.class(&CARD_CLASS, ..)`
/// ([`CARD_CLASS`](twine_widgets::container::CARD_CLASS)).
///
/// Laid out like `container(children).layout(Layout::column())`.
///
/// ```
/// use twine_view::prelude::*;
/// let _v = card((label("Title"), label("Body"))).width(148).padding(8).gap(6);
/// ```
pub fn card(children: impl ViewSeq) -> Flex<Card> {
    flex_view(|| Card, FlexFlow::COLUMN, false, children)
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
        e.set_local_prop(n, Selector::MAIN, StyleProp::Width(Length::Px(0).into()));
        e.set_local_prop(n, Selector::MAIN, StyleProp::Height(Length::Px(0).into()));
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
    flex_view(|| Obj, flow, true, children)
        .scrollable(true)
        .scroll_dir(axis)
        .scrollbar(ScrollbarMode::Auto)
}
