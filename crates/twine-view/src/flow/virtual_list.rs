//! [`virtual_list`]: a scrollable list that only builds the visible rows.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_core::Opa;
use twine_engine::{Engine, EventCode, EventFilter, EventResult, NodeId, Obj, ObjFlags};
use twine_reactive::{Scope, defer_current_effect, dispose_current_effect, untrack};
use twine_style::{Dir, Length, ScrollbarMode, Selector, StyleProp};

use super::dispose_with;
use crate::access::EngineAccess;
use crate::build::{BuildCx, BuildOp, WidgetView, widget_view};
use crate::modifiers::ViewExt;
use crate::view::{AnyView, IntoAnyView, View};

/// A vertically scrolling list of `count()` rows of `row_height` pixels, where only the
/// visible rows (plus one) exist. Scrolling disposes the rows that leave the viewport and
/// builds the ones that enter it, so memory stays constant however long the list is. The list
/// is rebuilt when `count()` changes.
///
/// Size the list with modifiers (default: the parent's full content area).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(_cx: Scope) -> impl View {
///     virtual_list(|| 10_000, 30, |_cx, i| label(format!("Row {i}"))).size(200, 240)
/// }
/// # let _ = app;
/// ```
pub fn virtual_list<V: View>(
    count: impl Fn() -> usize + 'static,
    row_height: i32,
    view: impl Fn(Scope, usize) -> V + 'static,
) -> VirtualList {
    let state = Rc::new(RefCell::new(ListState {
        rows: Vec::new(),
        count: 0,
        row_height: row_height.max(1),
        content: None,
    }));
    let view: Rc<dyn Fn(Scope, usize) -> AnyView> = Rc::new(move |s, i| view(s, i).into_any());
    let count: Rc<dyn Fn() -> usize> = Rc::new(count);
    let inner = widget_view(|| Obj).op(move |cx, node| {
        let e = cx.engine();
        for p in [
            StyleProp::BgOpa(Opa::TRANSP),
            StyleProp::BorderWidth(0),
            StyleProp::PadTop(0),
            StyleProp::PadBottom(0),
            StyleProp::PadLeft(0),
            StyleProp::PadRight(0),
            StyleProp::Width(Length::pct(100)),
            StyleProp::Height(Length::pct(100)),
        ] {
            e.set_local_prop(node, Selector::MAIN, p);
        }
        e.set_scroll_dir(node, Dir::VER);
        e.set_scrollbar_mode(node, ScrollbarMode::Auto);
        // The content: an invisible spacer as tall as all rows.
        let Ok(content) = e.create(node, Box::new(Obj)) else {
            return;
        };
        e.set_flag(content, ObjFlags::all(), false);
        for p in [
            StyleProp::BgOpa(Opa::TRANSP),
            StyleProp::BorderWidth(0),
            StyleProp::Width(Length::Px(1)),
            StyleProp::Height(Length::Px(0)),
        ] {
            e.set_local_prop(content, Selector::MAIN, p);
        }
        state.borrow_mut().content = Some(content);
        let scope = cx.scope();
        let rows = state.clone();
        cx.on_delete(node, move || {
            for (_, _, s) in rows.borrow_mut().rows.drain(..) {
                s.dispose();
            }
        });
        for code in [EventCode::Scroll, EventCode::SizeChanged] {
            let (st, v) = (state.clone(), view.clone());
            cx.engine()
                .add_event_handler(node, EventFilter::Code(code), move |ecx, ev| {
                    if ev.target == node {
                        update_range(ecx.engine_mut(), node, scope, &st, &*v);
                    }
                    EventResult::Continue
                });
        }
        let (st, v) = (state.clone(), view.clone());
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                let n = count();
                match EngineAccess::with(|e| e.tree().contains(node)) {
                    None => return defer_current_effect(),
                    Some(false) => return dispose_current_effect(),
                    Some(true) => {}
                }
                untrack(|| {
                    EngineAccess::with(|e| {
                        {
                            let mut s = st.borrow_mut();
                            for (_, _, sc) in s.rows.drain(..) {
                                dispose_with(e, sc);
                            }
                        }
                        // Delete every row (keep the content spacer).
                        let content = st.borrow().content;
                        let kids: Vec<NodeId> =
                            e.tree().children(node).filter(|k| Some(*k) != content).collect();
                        for k in kids {
                            let _ = e.delete(k);
                        }
                        let rh = {
                            let mut s = st.borrow_mut();
                            s.count = n;
                            s.row_height
                        };
                        if let Some(c) = content {
                            let h = i32::try_from(n)
                                .unwrap_or(i32::MAX / rh.max(1))
                                .saturating_mul(rh);
                            e.set_local_prop(c, Selector::MAIN, StyleProp::Height(Length::Px(h)));
                        }
                    });
                    EngineAccess::with(|e| update_range(e, node, scope, &st, &*v));
                });
            });
        });
    });
    VirtualList(inner)
}

/// The rows that exist and the list's parameters.
struct ListState {
    /// `(index, node, scope)` of the built rows.
    rows: Vec<(usize, NodeId, Scope)>,
    count: usize,
    row_height: i32,
    /// The spacer child giving the list its scroll height.
    content: Option<NodeId>,
}

/// Builds the rows entering the visible range and disposes those leaving it.
fn update_range(
    e: &mut Engine,
    node: NodeId,
    scope: Scope,
    st: &RefCell<ListState>,
    view: &dyn Fn(Scope, usize) -> AnyView,
) {
    let area = e.content_area(node);
    let scroll_y = e.scroll_offset(node).y.max(0);
    let (count, rh) = {
        let s = st.borrow();
        (s.count, s.row_height)
    };
    let (first, last) = if count == 0 || area.height() <= 0 {
        (0, 0)
    } else {
        let first = (scroll_y / rh) as usize;
        let last = ((scroll_y + area.height()) / rh) as usize + 1;
        (first.min(count), last.min(count))
    };
    // Dispose the rows outside `first..last`.
    let gone: Vec<(usize, NodeId, Scope)> = {
        let mut s = st.borrow_mut();
        let (keep, gone): (Vec<_>, Vec<_>) = s.rows.drain(..).partition(|(i, ..)| (first..last).contains(i));
        s.rows = keep;
        gone
    };
    for (_, n, sc) in gone {
        dispose_with(e, sc);
        if e.tree().contains(n) {
            let _ = e.delete(n);
        }
    }
    // Build the missing rows.
    for i in first..last {
        if st.borrow().rows.iter().any(|(j, ..)| *j == i) {
            continue;
        }
        let child = scope.child();
        let v = EngineAccess::provide(e, || view(child, i));
        let row = {
            let mut bcx = BuildCx::new(e, node, child);
            v.build(&mut bcx)
        };
        let y = i32::try_from(i).unwrap_or(i32::MAX / rh).saturating_mul(rh);
        e.set_pos(row, 0, y);
        e.set_size(row, Length::pct(100), rh);
        st.borrow_mut().rows.push((i, row, child));
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
