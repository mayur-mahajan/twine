//! [`virtual_list`]: a scrollable keyed list that only builds the visible rows.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_core::Opa;
use twine_engine::{Engine, EventCode, EventFilter, EventResult, NodeId, Obj, ObjFlags};
use twine_reactive::{ReadSignal, Scope, Signal};
use twine_style::{Axis, Length, ScrollbarMode, Selector, StyleProp};

use twine_core::alloc_guard::{self, AllocSite};

use super::for_each::{delete_row, sync_item};
use super::items::{IntoItems, Items};
use crate::access::EngineAccess;
use crate::build::{BuildCx, BuildOp, WidgetView, widget_view};
use crate::modifiers::ViewExt;
use crate::view::{AnyView, IntoAnyView, View};

/// A vertically scrolling keyed list of rows `row_height` pixels high, where only the visible
/// rows (plus one) exist — the same API as [`for_each`](crate::for_each): `items` is any
/// [`IntoItems`] (a signal or memo of a `Vec` is read in place, never cloned), `key`
/// identifies an item, and `view` builds a row from a [`ReadSignal`] of its item.
///
/// Scrolling disposes the rows that leave the viewport and builds the ones that enter it, so
/// memory stays constant however long the list is. When the items change, a visible row whose
/// key is still visible is kept (moved to its new position if it moved) and its item signal is
/// updated in place when the item changed — so its bindings run, nothing is rebuilt; rows whose
/// keys left the viewport are disposed, rows of newly visible keys are built.
///
/// Size the list with modifiers (default: the parent's full content area).
///
/// **Bound.** At most [`Limits::max_list_items`](twine_core::Limits::max_list_items) rows
/// exist: visible items beyond it get no row, and a
/// [`CapacityFault::ListItems`](twine_core::fault::CapacityFault::ListItems) fault is raised.
///
/// **Cost.** Per visible row: a scope and a signal holding a copy of its item. A scroll that
/// does not change the visible range reads nothing; one that does compares the visible keys.
/// A change of the items compares the keys and values of the visible rows only. The sync's
/// scratch is kept between syncs (reserved for the bound with
/// [`Limits::preallocate`](twine_core::Limits::preallocate)), so scrolling and changes
/// allocate nothing beyond the rows they build.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn log(cx: Scope) -> impl View {
///     let lines = cx.signal((0..10_000u32).map(|i| (i, format!("Line {i}"))).collect::<Vec<_>>());
///     virtual_list(lines, |(id, _)| *id, 30, |_cx, line| label(line.map(|(_, t)| t.clone())))
///         .size(200, 240)
/// }
/// # let _ = log;
/// ```
pub fn virtual_list<T, K, V, M>(
    items: impl IntoItems<T, M>,
    key: impl Fn(&T) -> K + 'static,
    row_height: i32,
    view: impl Fn(Scope, ReadSignal<T>) -> V + 'static,
) -> VirtualList
where
    T: Clone + PartialEq + 'static,
    K: PartialEq + 'static,
    V: View,
{
    let list = Rc::new(List {
        items: items.into_items(),
        key: Box::new(key),
        view: Box::new(move |s, t| view(s, t).into_any()),
        row_height: row_height.max(1),
        state: RefCell::new(ListState {
            rows: Vec::new(),
            count: 0,
            range: (0, 0),
            content: None,
            bound: usize::MAX,
            scratch: Scratch {
                shown: Vec::new(),
                gone: Vec::new(),
                added: Vec::new(),
            },
        }),
    });
    let inner = widget_view(|| Obj).op(move |cx, node| {
        let e = cx.engine();
        let limits = *e.limits();
        {
            let mut s = list.state.borrow_mut();
            s.bound = usize::from(limits.max_list_items);
            if limits.preallocate {
                s.reserve();
            }
        }
        for p in [
            StyleProp::BgOpacity(Opa::TRANSP.into()),
            StyleProp::BorderWidth(Length::Px(0).into()),
            StyleProp::PaddingTop(Length::Px(0).into()),
            StyleProp::PaddingBottom(Length::Px(0).into()),
            StyleProp::PaddingLeft(Length::Px(0).into()),
            StyleProp::PaddingRight(Length::Px(0).into()),
            StyleProp::Width(Length::pct(100).into()),
            StyleProp::Height(Length::pct(100).into()),
        ] {
            e.set_local_prop(node, Selector::MAIN, p);
        }
        e.set_scroll_dir(node, Axis::Vertical);
        e.set_scrollbar_mode(node, ScrollbarMode::Auto);
        // The content: an invisible spacer as tall as all rows.
        let Ok(content) = e.create(node, Box::new(Obj)) else {
            return;
        };
        e.set_flag(content, ObjFlags::all(), false);
        for p in [
            StyleProp::BgOpacity(Opa::TRANSP.into()),
            StyleProp::BorderWidth(Length::Px(0).into()),
            StyleProp::Width(Length::Px(1).into()),
            StyleProp::Height(Length::Px(0).into()),
        ] {
            e.set_local_prop(content, Selector::MAIN, p);
        }
        list.state.borrow_mut().content = Some(content);
        let scope = cx.scope();
        let rt = scope.runtime();
        let l = list.clone();
        cx.on_delete(node, move || {
            // Taken out first: disposing runs user cleanups.
            let rows = core::mem::take(&mut l.state.borrow_mut().rows);
            for row in rows {
                row.scope.dispose();
            }
        });
        for code in [EventCode::Scroll, EventCode::SizeChanged] {
            let l = list.clone();
            cx.engine()
                .add_event_handler(node, EventFilter::Code(code), move |ecx, ev| {
                    if ev.target == node {
                        l.sync(ecx.engine_mut(), node, scope, false);
                    }
                    EventResult::Continue
                });
        }
        let l = list.clone();
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                // The items are read (and tracked) by `sync`.
                let found = EngineAccess::with(rt, |e| {
                    let alive = e.tree().contains(node);
                    if alive {
                        l.sync(e, node, scope, true);
                    }
                    alive
                });
                match found {
                    None => rt.defer_current_effect(),
                    Some(false) => rt.dispose_current_effect(),
                    Some(true) => {}
                }
            });
        });
    });
    VirtualList(inner)
}

/// A virtual list's items, row builder and rows.
struct List<T: 'static, K: 'static> {
    items: Items<T>,
    key: Box<dyn Fn(&T) -> K>,
    view: Box<dyn Fn(Scope, ReadSignal<T>) -> AnyView>,
    row_height: i32,
    state: RefCell<ListState<T, K>>,
}

/// The rows that exist and the list's extent.
struct ListState<T: 'static, K> {
    rows: Vec<VRow<T, K>>,
    /// The number of items as of the last sync.
    count: usize,
    /// The item indices `first..last` the rows cover.
    range: (usize, usize),
    /// The spacer child giving the list its scroll height.
    content: Option<NodeId>,
    /// The most rows that exist at once ([`Limits::max_list_items`](twine_core::Limits::max_list_items)).
    bound: usize,
    /// Kept between syncs (cleared, not freed), so a sync allocates nothing beyond the rows
    /// it builds once the scratch has reached its size (or was reserved for `bound`).
    scratch: Scratch<T, K>,
}

/// What a sync works with.
struct Scratch<T: 'static, K> {
    /// Whether each visible index has a row.
    shown: Vec<bool>,
    /// Rows to dispose and delete.
    gone: Vec<VRow<T, K>>,
    /// Rows to build: `(index, key, item)`.
    added: Vec<(usize, K, T)>,
}

impl<T: 'static, K> ListState<T, K> {
    /// Reserves the rows and the scratch for `bound` rows ([`Limits::preallocate`](twine_core::Limits::preallocate)).
    fn reserve(&mut self) {
        let n = self.bound;
        self.rows.reserve_exact(n);
        self.scratch.shown.reserve_exact(n);
        self.scratch.gone.reserve_exact(n);
        self.scratch.added.reserve_exact(n);
    }
}

/// A built row.
struct VRow<T: 'static, K> {
    /// The index of its item.
    index: usize,
    key: K,
    node: NodeId,
    scope: Scope,
    item: Signal<T>,
    /// Its index changed: it must be placed again.
    moved: bool,
}

/// Raises the `ListItems` capacity fault of a list that left `n` visible items without a row.
#[cold]
#[inline(never)]
fn cut_fault(e: &mut Engine, node: NodeId, n: usize) {
    twine_core::warn!(
        target: "twine::view",
        "virtual_list {}: {} visible item(s) beyond Limits::max_list_items not shown",
        twine_engine::fmt_node_id(node),
        n
    );
    e.raise_fault(
        twine_engine::FaultRecord::new(twine_core::fault::FaultKind::Capacity)
            .node(node)
            .code(twine_core::fault::CapacityFault::ListItems.code()),
    );
}

/// The y offset of row `i` (`i · row_height`, saturated); `row_height >= 1`.
fn y_of(i: usize, row_height: i32) -> i32 {
    // DIV: `row_height >= 1` (clamped by the constructor); the fallback is never taken.
    let max_rows = i32::MAX.checked_div(row_height).unwrap_or(i32::MAX);
    i32::try_from(i).unwrap_or(max_rows).saturating_mul(row_height)
}

impl<T: Clone + PartialEq + 'static, K: PartialEq + 'static> List<T, K> {
    /// The visible item range `first..last` for `count` items.
    fn visible(&self, e: &Engine, node: NodeId, count: usize) -> (usize, usize) {
        let area = e.content_area(node);
        let scroll_y = e.scroll_offset(node).y.max(0);
        let rh = self.row_height;
        if count == 0 || area.height() <= 0 {
            return (0, 0);
        }
        // DIV: `row_height >= 1` (clamped by the constructor); the fallbacks are never taken.
        let row_at = |y: i32| usize::try_from(y.checked_div(rh).unwrap_or(0)).unwrap_or(0);
        let first = row_at(scroll_y);
        let last = row_at(scroll_y.saturating_add(area.height())).saturating_add(1);
        (first.min(count), last.min(count))
    }

    /// Brings the rows up to date: with `force` (the items changed) always, else (a scroll or
    /// resize) only when the visible range changed.
    fn sync(&self, e: &mut Engine, node: NodeId, scope: Scope, force: bool) {
        let rt = scope.runtime();
        if !force {
            let s = self.state.borrow();
            if self.visible(e, node, s.count) == s.range {
                return;
            }
        }
        let mut changes = None;
        self.items.read(|items| {
            // Untracked: only the read of the list subscribes the effect. Batched: the
            // row signals written here notify their bindings after the sync.
            rt.untrack(|| rt.batch(|| changes = Some(self.diff(e, node, items))));
        });
        if let Some((height, cut)) = changes {
            if cut > 0 {
                cut_fault(e, node, cut);
            }
            self.apply(e, node, scope, height);
        }
    }

    /// Matches the rows with the visible items: kept rows get their item updated and their
    /// new index; the rows to delete and to build are left in the scratch. Returns the new
    /// content height (when the number of items changed) and the number of visible items
    /// beyond the bound (not shown).
    fn diff(&self, e: &Engine, node: NodeId, items: &[T]) -> (Option<i32>, usize) {
        let (first, visible_last) = self.visible(e, node, items.len());
        let mut st = self.state.borrow_mut();
        let ListState {
            rows,
            count,
            range,
            bound,
            scratch: Scratch { shown, gone, added },
            ..
        } = &mut *st;
        let last = visible_last.min(first.saturating_add(*bound));
        let cut = visible_last.saturating_sub(last);
        let height = (*count != items.len()).then(|| y_of(items.len(), self.row_height));
        *count = items.len();
        *range = (first, visible_last);
        // Which row shows each visible index (`first <= last <= items.len()`, from `visible`).
        let n = last.saturating_sub(first);
        shown.clear();
        alloc_guard::reserve(shown, n, AllocSite::Reconcile);
        shown.resize(n, false);
        // Whether visible index `i` has no row yet (`false` outside `first..last`).
        let free = |shown: &[bool], i: usize| i.checked_sub(first).and_then(|o| shown.get(o)) == Some(&false);
        let mut r = 0usize;
        while let Some(row) = rows.get(r) {
            let shows = |i: usize| items.get(i).is_some_and(|item| (self.key)(item) == row.key);
            let pos = if free(shown, row.index) && shows(row.index) {
                Some(row.index)
            } else {
                (first..last).find(|&i| free(shown, i) && shows(i))
            };
            match pos {
                Some(i) => {
                    if let Some(flag) = i.checked_sub(first).and_then(|o| shown.get_mut(o)) {
                        *flag = true;
                    }
                    if let (Some(row), Some(item)) = (rows.get_mut(r), items.get(i)) {
                        sync_item(row.item, item);
                        if row.index != i {
                            row.index = i;
                            row.moved = true;
                        }
                    }
                    r = r.saturating_add(1);
                }
                None => alloc_guard::push(gone, rows.swap_remove(r), AllocSite::Reconcile),
            }
        }
        for i in (first..last).filter(|&i| free(shown, i)) {
            if let Some(item) = items.get(i) {
                alloc_guard::push(added, (i, (self.key)(item), item.clone()), AllocSite::Reconcile);
            }
        }
        (height, cut)
    }

    /// Deletes the rows that left, places the moved ones and builds the new ones.
    fn apply(&self, e: &mut Engine, node: NodeId, scope: Scope, height: Option<i32>) {
        let rt = scope.runtime();
        let rh = self.row_height;
        let y_of = |i: usize| y_of(i, rh);
        // Taken out for the time of the apply (user code runs: cleanups, views), given back
        // empty with their capacity.
        let (mut gone, mut added) = {
            let mut s = self.state.borrow_mut();
            (
                core::mem::take(&mut s.scratch.gone),
                core::mem::take(&mut s.scratch.added),
            )
        };
        for row in gone.drain(..) {
            delete_row(e, row.scope, row.node);
        }
        let content = {
            let mut s = self.state.borrow_mut();
            for row in s.rows.iter_mut().filter(|r| r.moved) {
                row.moved = false;
                e.set_pos(row.node, 0, y_of(row.index));
            }
            s.content
        };
        if let (Some(h), Some(content)) = (height, content) {
            e.set_local_prop(content, Selector::MAIN, StyleProp::Height(Length::Px(h).into()));
        }
        for (index, key, item) in added.drain(..) {
            let child = scope.child();
            let item = child.signal(item);
            // The engine is lent to the hooks the row's view uses.
            let v = EngineAccess::provide(rt, e, || (self.view)(child, item.read_only()));
            let row = {
                let mut bcx = BuildCx::new(e, node, child);
                v.build(&mut bcx)
            };
            e.set_pos(row, 0, y_of(index));
            e.set_size(row, Length::pct(100), rh);
            let mut s = self.state.borrow_mut();
            alloc_guard::push(
                &mut s.rows,
                VRow {
                    index,
                    key,
                    node: row,
                    scope: child,
                    item,
                    moved: false,
                },
                AllocSite::Reconcile,
            );
        }
        let mut s = self.state.borrow_mut();
        s.scratch.gone = gone;
        s.scratch.added = added;
    }
}

/// The view of [`virtual_list`] (a scrollable container; size it with modifiers).
#[derive(Debug)]
#[must_use]
pub struct VirtualList(WidgetView<Obj>);

impl View for VirtualList {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        self.0.build(cx)
    }
}

impl ViewExt for VirtualList {
    type Widget = Obj;

    fn push_op(self, op: BuildOp) -> Self {
        VirtualList(self.0.push_op(op))
    }
}
