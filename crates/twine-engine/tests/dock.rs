//! Keyboard avoidance in the engine (R4.S05): `ObjFlags::DOCK_BOTTOM` reserves the bottom of
//! the parent for the docked node and lays the other children out above it; it neither
//! scrolls with the parent nor counts for its scroll extents; and
//! `Engine::scroll_to_view_after_layout` scrolls once the layout is final, without a layout
//! of its own and without allocating after the first request.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::white_screen;
use twine_core::Rect;
use twine_engine::{Engine, EventCode, EventFilter, EventResult, NodeId, Obj, ObjFlags};
use twine_style::{Length, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A 200 × 100 screen with a page filling it (no padding, no border) and a scrollable list
/// filling the page, holding ten 30 px rows; and a docked 40 px node in the page (not yet
/// docked: the caller sets the flag).
struct Scene {
    h: EngineHarness,
    page: NodeId,
    list: NodeId,
    rows: Vec<NodeId>,
    dock: NodeId,
}

fn plain(e: &mut Engine, parent: NodeId) -> NodeId {
    let n = e.create(parent, Box::new(Obj)).unwrap();
    for p in [
        StyleProp::PaddingTop(Length::Px(0).into()),
        StyleProp::PaddingBottom(Length::Px(0).into()),
        StyleProp::PaddingLeft(Length::Px(0).into()),
        StyleProp::PaddingRight(Length::Px(0).into()),
        StyleProp::BorderWidth(Length::Px(0).into()),
    ] {
        e.set_local_prop(n, Selector::MAIN, p);
    }
    n
}

fn scene() -> Scene {
    let mut h = EngineHarness::new(200, 100).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    let s = h.screen();
    let e = h.engine_mut();
    let page = plain(e, s);
    e.set_size(page, Length::pct(100), Length::pct(100));
    e.set_flag(page, ObjFlags::SCROLLABLE, false);
    let list = plain(e, page);
    e.set_size(list, Length::pct(100), Length::pct(100));
    let rows = (0..10)
        .map(|i| {
            let r = plain(e, list);
            e.set_size(r, Length::pct(100), Length::Px(30));
            e.set_pos(r, 0, i * 30);
            r
        })
        .collect();
    let dock = plain(e, page);
    e.set_size(dock, Length::pct(10), Length::Px(40));
    h.run_until_idle();
    Scene {
        h,
        page,
        list,
        rows,
        dock,
    }
}

#[test]
fn docking_reserves_the_bottom_and_undocking_or_hiding_gives_it_back() {
    let mut s = scene();
    let e = s.h.engine();
    assert_eq!(e.coords(s.list), Rect::new(0, 0, 200, 100));
    s.h.engine_mut().set_flag(s.dock, ObjFlags::DOCK_BOTTOM, true);
    assert!(s.h.engine().layout_pending(), "docking must lay the parent out");
    s.h.update();
    let e = s.h.engine();
    // Full width (its own 10 % width is ignored), own height, at the bottom.
    assert_eq!(e.coords(s.dock), Rect::new(0, 60, 200, 100));
    assert_eq!(e.coords(s.list), Rect::new(0, 0, 200, 60));

    s.h.engine_mut().set_flag(s.dock, ObjFlags::HIDDEN, true);
    s.h.update();
    assert_eq!(s.h.engine().coords(s.list), Rect::new(0, 0, 200, 100));
    s.h.engine_mut().set_flag(s.dock, ObjFlags::HIDDEN, false);
    s.h.update();
    assert_eq!(s.h.engine().coords(s.list), Rect::new(0, 0, 200, 60));

    s.h.engine_mut().set_flag(s.dock, ObjFlags::DOCK_BOTTOM, false);
    s.h.update();
    let e = s.h.engine();
    assert_eq!(e.coords(s.list), Rect::new(0, 0, 200, 100));
    assert_eq!(e.coords(s.dock), Rect::new(0, 0, 20, 40));
}

#[test]
fn a_docked_node_neither_scrolls_with_its_parent_nor_counts_for_its_extents() {
    let mut s = scene();
    // Dock inside the scrolling list itself.
    let e = s.h.engine_mut();
    let d = e.create(s.list, Box::new(Obj)).unwrap();
    e.set_size(d, Length::pct(100), Length::Px(40));
    e.set_flag(d, ObjFlags::DOCK_BOTTOM, true);
    s.h.update();
    let e = s.h.engine();
    let before = e.coords(d);
    assert_eq!(before, Rect::new(0, 60, 200, 100));
    // Ten rows of 30 px in a 100 px list: 200 px to scroll, the dock adds nothing.
    assert_eq!(e.scroll_bottom(s.list), 200);
    s.h.engine_mut().scroll_to_y(s.list, 50, false);
    s.h.update();
    let e = s.h.engine();
    assert_eq!(e.coords(d), before);
    assert_eq!(e.coords(s.rows[0]).y0, -50);
}

#[test]
fn scroll_to_view_after_layout_waits_for_the_layout_and_lays_nothing_out() {
    let mut s = scene();
    let last = *s.rows.last().unwrap();
    let passes = s.h.engine().layout_passes();
    s.h.engine_mut().scroll_to_view_after_layout(last, false);
    s.h.engine_mut().scroll_to_view_after_layout(last, false); // merged
    assert_eq!(s.h.engine().scroll_offset(s.list).y, 0, "not before the layout");
    s.h.update();
    let e = s.h.engine();
    assert_eq!(e.coords(last), Rect::new(0, 70, 200, 100));
    assert_eq!(e.scroll_offset(s.list).y, 200);
    assert_eq!(e.layout_passes(), passes, "a scroll is no layout pass");

    // A deleted node is skipped.
    let first = s.rows[0];
    s.h.engine_mut().scroll_to_view_after_layout(first, false);
    s.h.engine_mut().delete(first).unwrap();
    s.h.update();
    assert_eq!(s.h.engine().scroll_offset(s.list).y, 200); // not scrolled back to it
}

#[test]
fn a_request_from_size_changed_runs_after_the_same_pass() {
    let mut s = scene();
    let target = s.rows[9];
    // Like a docked keyboard: when it gets its size, the target is scrolled into view.
    let fired = Rc::new(Cell::new(0));
    let f = fired.clone();
    s.h.engine_mut()
        .add_event_handler(s.dock, EventFilter::Code(EventCode::SizeChanged), move |cx, _| {
            f.set(f.get() + 1);
            cx.engine_mut().scroll_to_view_after_layout(target, false);
            EventResult::Continue
        });
    s.h.engine_mut()
        .set_size(s.dock, Length::pct(100), Length::Px(50));
    s.h.engine_mut().set_flag(s.dock, ObjFlags::DOCK_BOTTOM, true);
    let passes = s.h.engine().layout_passes();
    s.h.update();
    let e = s.h.engine();
    assert_eq!(fired.get(), 1);
    assert_eq!(
        e.layout_passes(),
        passes + 1,
        "one pass for the dock and the scroll"
    );
    // The list is 50 px tall now; the last row ends at its bottom.
    assert_eq!(e.coords(s.list), Rect::new(0, 0, 200, 50));
    assert_eq!(e.coords(target), Rect::new(0, 20, 200, 50));
}

#[test]
fn scroll_requests_allocate_nothing_after_the_first() {
    let mut s = scene();
    let rows = s.rows.clone();
    s.h.engine_mut().scroll_to_view_after_layout(rows[9], false);
    s.h.update();
    let ((), stats) = count_allocs(|| {
        for r in [rows[0], rows[5], rows[9]] {
            s.h.engine_mut().scroll_to_view_after_layout(r, false);
            s.h.engine_mut().update_layout();
        }
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
    assert_eq!(s.h.engine().scroll_offset(s.list).y, 200);
    let _ = s.page;
}
