//! Paged containers: [`tabview`] + [`tab`], [`tileview`] + [`tile`].

use alloc::boxed::Box;

use twine_engine::{NodeId, fmt_node_id};
use twine_style::Dir;
use twine_widgets::label::Label;
use twine_widgets_ext::tabview::Tabview;
use twine_widgets_ext::tileview::{self, Tile, Tileview};

use crate::build::{BuildCx, WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, event_value, on_value_changed};
use crate::prop::IntoProp;
use crate::text::{IntoText, TextProp, bind_label_text};
use crate::view::{View, ViewSeq};

/// Whether `node` has been laid out (a model change then animates).
fn shown(e: &twine_engine::Engine, node: NodeId) -> bool {
    e.coords(node).width() > 0
}

// ---- Tabview --------------------------------------------------------------------------------

/// Tabs: a button bar and swipeable pages, one per [`tab`] of `tabs`; the index of the active
/// tab in `selected` (a plain value or a signal kept in sync both ways).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     let page = cx.signal(0usize);
///     tabview(page, (
///         tab("Home", label("Welcome")),
///         tab("Settings", label("Nothing yet")),
///     ))
///     .bar_position(Dir::BOTTOM)
/// }
/// # let _ = app;
/// ```
pub fn tabview(selected: impl IntoModel<usize>, tabs: impl ViewSeq) -> WidgetView<Tabview> {
    let model = selected.into_model();
    widget_view(Tabview::new)
        .children(tabs)
        .after_children(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, idx: usize| {
                    let anim = shown(e, n);
                    e.with_widget_mut(n, |t: &mut Tabview, wcx| {
                        let anim = anim && t.animated();
                        t.set_active(wcx, u32::try_from(idx).unwrap_or(u32::MAX), anim);
                    });
                },
                |e, n, ev| {
                    event_value(ev)
                        .and_then(|v| usize::try_from(v).ok())
                        .or_else(|| e.widget::<Tabview>(n).map(|t| t.active() as usize))
                        .unwrap_or_default()
                },
            );
        })
}

impl WidgetView<Tabview> {
    /// Where the bar is (`Dir::TOP` by default, `BOTTOM`, `LEFT`, `RIGHT`).
    #[must_use]
    pub fn bar_position(self, dir: impl IntoProp<Dir>) -> Self {
        self.bind(dir, |t: &mut Tabview, cx, d| t.set_tab_bar_position(cx, d))
    }

    /// The bar's height (top / bottom) or width (left / right).
    #[must_use]
    pub fn bar_size(self, size: impl IntoProp<i32>) -> Self {
        self.bind(size, |t: &mut Tabview, cx, s| t.set_tab_bar_size(cx, s))
    }

    /// Whether switching tabs (by clicking or from the model) slides the pages (default on);
    /// off switches instantly.
    #[must_use]
    pub fn animated(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |t: &mut Tabview, cx, on| t.set_animated(cx, on))
    }

    /// Called with the index of the tab the user switched to.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(usize) + 'static) -> Self {
        self.op(move |cx, node| {
            on_value_changed(
                cx,
                node,
                |_, _, ev| event_value(ev).and_then(|v| usize::try_from(v).ok()),
                f,
            );
        })
    }
}

/// A tab of a [`tabview`]: a button titled `title` in the bar and a page holding `content`.
/// Outside a tabview it logs `warn!` and builds a plain container.
pub fn tab(title: impl IntoText, content: impl ViewSeq) -> TabView {
    TabView {
        title: title.into_text(),
        content: Box::new(move |cx: &mut BuildCx<'_>| content.build_seq(cx)),
    }
}

/// The view of a [`tab`].
pub struct TabView {
    title: TextProp,
    content: Box<dyn FnOnce(&mut BuildCx<'_>)>,
}

impl core::fmt::Debug for TabView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabView").finish_non_exhaustive()
    }
}

impl View for TabView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let tv = cx.parent();
        let initial = match &self.title {
            TextProp::Static(s) => s,
            _ => "",
        };
        let page = if cx.engine().widget::<Tabview>(tv).is_some() {
            cx.engine()
                .with_widget_mut(tv, |t: &mut Tabview, wcx| t.add_tab(wcx, initial))
                .flatten()
        } else {
            twine_core::warn!(target: "twine::view", "tab outside a tabview (parent {})", fmt_node_id(tv));
            None
        };
        let Some(page) = page else {
            let c = cx.create(twine_engine::Obj);
            cx.with_parent(c, self.content);
            return c;
        };
        if !matches!(self.title, TextProp::Static(_)) {
            let e = cx.engine();
            let label = e.widget::<Tabview>(tv).and_then(|t| {
                let n = t.tab_count(e).checked_sub(1)?;
                let b = t.tab_button(e, n)?;
                e.tree().children(b).find(|&c| e.widget::<Label>(c).is_some())
            });
            if let Some(l) = label {
                bind_label_text(cx, l, self.title);
            }
        }
        cx.with_parent(page, self.content);
        page
    }
}

// ---- Tileview -------------------------------------------------------------------------------

/// A 2D grid of full-size [`tile`]s swiped in the directions each tile allows; the
/// `(column, row)` of the tile in view in `active` (a plain value or a signal kept in sync both
/// ways).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     let at = cx.signal((0u8, 0u8));
///     tileview(at, (
///         tile(0, 0, Dir::RIGHT, label("Swipe left")),
///         tile(1, 0, Dir::LEFT, label("Swipe right")),
///     ))
/// }
/// # let _ = app;
/// ```
pub fn tileview(active: impl IntoModel<(u8, u8)>, tiles: impl ViewSeq) -> WidgetView<Tileview> {
    let model = active.into_model();
    widget_view(Tileview::new)
        .children(tiles)
        .after_children(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, (col, row): (u8, u8)| {
                    let anim = shown(e, n);
                    let current = e
                        .widget::<Tileview>(n)
                        .and_then(Tileview::tile_active)
                        .and_then(|t| e.widget::<Tile>(t))
                        .map(|t| (t.col(), t.row()));
                    if current != Some((col, row)) || !anim {
                        e.with_widget_mut(n, |t: &mut Tileview, wcx| {
                            t.set_tile_by_index(wcx, col, row, anim);
                        });
                    }
                },
                |e, n, _| {
                    e.widget::<Tileview>(n)
                        .and_then(Tileview::tile_active)
                        .and_then(|t| e.widget::<Tile>(t))
                        .map_or((0, 0), |t| (t.col(), t.row()))
                },
            );
        })
}

/// A tile of a [`tileview()`] at `(col, row)` that the user may leave in `dirs`, holding
/// `content`. Outside a tileview it logs `warn!` and builds a plain container.
pub fn tile(col: u8, row: u8, dirs: Dir, content: impl ViewSeq) -> TileView {
    TileView {
        col,
        row,
        dirs,
        content: Box::new(move |cx: &mut BuildCx<'_>| content.build_seq(cx)),
    }
}

/// The view of a [`tile`].
pub struct TileView {
    col: u8,
    row: u8,
    dirs: Dir,
    content: Box<dyn FnOnce(&mut BuildCx<'_>)>,
}

impl core::fmt::Debug for TileView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TileView")
            .field("col", &self.col)
            .field("row", &self.row)
            .finish_non_exhaustive()
    }
}

impl View for TileView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let tv = cx.parent();
        let t = if cx.engine().widget::<Tileview>(tv).is_some() {
            tileview::add_tile(cx.engine(), tv, self.col, self.row, self.dirs).ok()
        } else {
            twine_core::warn!(target: "twine::view", "tile outside a tileview (parent {})", fmt_node_id(tv));
            None
        };
        let node = t.unwrap_or_else(|| cx.create(twine_engine::Obj));
        cx.with_parent(node, self.content);
        node
    }
}
