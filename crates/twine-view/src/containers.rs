//! Layout containers: [`column()`], [`row`], [`flex`], [`grid`], [`container`], [`stack`],
//! [`spacer`], [`scroll_view`].

use twine_core::Opa;
use twine_engine::{Engine, NodeId, Obj, ObjFlags};
use twine_style::{
    Align, Dir, FlexAlign, FlexFlow, GridAlign, GridTrack, LayoutKind, Length, ScrollbarMode, Selector,
    StyleProp,
};

use crate::build::{BuildCx, BuildOp, WidgetView, widget_view};
use crate::modifiers::ViewExt;
use crate::prop::IntoProp;
use crate::view::{View, ViewSeq};

/// Makes a container transparent like LVGL's `lv_obj` with the `transp` style: no
/// background, border, padding, radius or shadow (the theme's card look is overridden).
fn transparent(e: &mut Engine, n: NodeId) {
    for p in [
        StyleProp::BgOpa(Opa::TRANSP),
        StyleProp::BorderWidth(0),
        StyleProp::PadTop(0),
        StyleProp::PadBottom(0),
        StyleProp::PadLeft(0),
        StyleProp::PadRight(0),
        StyleProp::Radius(0),
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
    /// Placement of the items on the main axis.
    pub fn justify(self, a: impl IntoProp<FlexAlign>) -> Self {
        self.style_prop(a, StyleProp::FlexMainPlace)
    }

    /// Placement of the items on the cross axis, like CSS `align-items`: inside their track
    /// and, with it, the track(s) in the container (LVGL's cross and track placement; use
    /// [`align_content`](Self::align_content) afterwards to place the tracks differently).
    pub fn align_items(self, a: impl IntoProp<FlexAlign>) -> Self {
        self.style_props(a, |a| {
            [StyleProp::FlexCrossPlace(a), StyleProp::FlexTrackPlace(a)]
        })
    }

    /// Placement of the tracks in the container (LVGL track placement).
    pub fn align_content(self, a: impl IntoProp<FlexAlign>) -> Self {
        self.style_prop(a, StyleProp::FlexTrackPlace)
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

fn flex_view(flow: FlexFlow, children: impl ViewSeq) -> WidgetView<Obj> {
    widget_view(|| Obj)
        .op(move |cx, n| {
            let e = cx.engine();
            transparent(e, n);
            e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(LayoutKind::Flex));
            e.set_local_prop(n, Selector::MAIN, StyleProp::FlexFlow(flow));
        })
        .children(children)
}

/// A flex column (no wrapping), transparent: no background, border or padding (LVGL
/// `lv_obj` with the `transp` style), content-sized unless sized.
///
/// ```
/// use twine_view::prelude::*;
/// let _v = column((label("a"), label("b"))).gap(8).align_items(FlexAlign::Center);
/// ```
pub fn column(children: impl ViewSeq) -> Flex {
    Flex(flex_view(FlexFlow::Column, children))
}

/// A flex row (no wrapping), transparent like [`column()`].
pub fn row(children: impl ViewSeq) -> Flex {
    Flex(flex_view(FlexFlow::Row, children))
}

/// A flex container with any flow (wrapping, reversed), transparent like [`column()`].
pub fn flex(flow: FlexFlow, children: impl ViewSeq) -> Flex {
    Flex(flex_view(flow, children))
}

/// A grid container with `columns` × `rows` tracks (`GridTrack::Px`, `Content`, `Fr`),
/// transparent like [`column()`]. Place children with `.grid_cell(col, span, row, span)`.
///
/// ```
/// use twine_view::prelude::*;
/// static COLS: [GridTrack; 2] = [GridTrack::Fr(1), GridTrack::Fr(1)];
/// static ROWS: [GridTrack; 1] = [GridTrack::Content];
/// let _v = grid(&COLS, &ROWS, (label("a").grid_cell(0, 1, 0, 1), label("b").grid_cell(1, 1, 0, 1)));
/// ```
pub fn grid(columns: &'static [GridTrack], rows: &'static [GridTrack], children: impl ViewSeq) -> Grid {
    Grid(
        widget_view(|| Obj)
            .op(move |cx, n| {
                let e = cx.engine();
                transparent(e, n);
                e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(LayoutKind::Grid));
                e.set_grid_dsc_array(n, columns, rows);
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

/// A scrollable column (`Dir::VER`) or row (`Dir::HOR`) with scrollbars shown while needed,
/// transparent like [`column()`].
pub fn scroll_view(dir: Dir, children: impl ViewSeq) -> Flex {
    let flow = if dir == Dir::HOR {
        FlexFlow::Row
    } else {
        FlexFlow::Column
    };
    Flex(flex_view(flow, children))
        .scrollable(true)
        .scroll_dir(dir)
        .scrollbar(ScrollbarMode::Auto)
}
