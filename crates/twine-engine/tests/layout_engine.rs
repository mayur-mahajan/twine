//! The engine's layout pass: dirty scheduling, delta moves, events, precise invalidation,
//! convergence, allocations and layout snapshots.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Mutex;

use common::{boxed, style, white_screen};
use twine_core::{Color, Duration, Opa, Rect};
use twine_engine::{
    Engine, EventCode, EventFilter, EventParam, EventResult, MAX_LAYOUT_ITERATIONS, NodeId, Obj, ObjFlags,
    Wake,
};
use twine_style::{Align, FlexAlign, FlexFlow, GridAlign, GridTrack, LayoutKind, Length, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// Captures `twine::layout` warnings.
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.target() == "twine::layout" && m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            WARNINGS.lock().unwrap().push(r.args().to_string());
        }
    }
    fn flush(&self) {}
}

static LOGGER: Capture = Capture;

/// A plain `Obj` under `parent` with a fixed size at `(x, y)` in the parent's content area.
fn node(e: &mut Engine, parent: NodeId, x: i32, y: i32, w: i32, h: i32, c: Color) -> NodeId {
    let n = e.create(parent, Box::new(Obj)).unwrap();
    e.set_pos(n, x, y);
    e.set_size(n, w, h);
    style(e, n, &[StyleProp::BgColor(c), StyleProp::BgOpa(Opa::COVER)]);
    n
}

fn harness(w: u16, h: u16) -> EngineHarness {
    let mut h = EngineHarness::new(w, h).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    h
}

fn invalidated(h: &EngineHarness) -> Vec<Rect> {
    h.engine().invalidation_log().iter().map(|(r, _)| *r).collect()
}

/// Two fixed-size containers with three leaves each.
fn two_containers(h: &mut EngineHarness) -> ([NodeId; 4], [NodeId; 4]) {
    let s = h.screen();
    let e = h.engine_mut();
    let mut a = [s; 4];
    let mut b = [s; 4];
    a[0] = node(e, s, 0, 0, 100, 100, Color::hex(0xDD_DD_DD));
    b[0] = node(e, s, 120, 0, 100, 100, Color::hex(0xCC_CC_CC));
    for i in 1..4 {
        a[i] = node(e, a[0], 0, (i as i32 - 1) * 30, 50, 20, Color::RED);
        b[i] = node(e, b[0], 0, (i as i32 - 1) * 30, 50, 20, Color::BLUE);
    }
    h.run_until_idle();
    (a, b)
}

#[test]
fn layout_runs_only_for_dirty_subtree() {
    let mut h = harness(240, 120);
    let (a, b) = two_containers(&mut h);
    assert_eq!(h.engine().coords(a[2]), Rect::from_xywh(0, 30, 50, 20));
    assert_eq!(h.engine().coords(b[3]), Rect::from_xywh(120, 60, 50, 20));

    h.engine_mut().set_width(a[1], 70);
    h.update();
    let visited = h.engine().layout_visited();
    assert!(visited.contains(&a[0]), "{visited:?}");
    for n in &visited {
        assert!(a.contains(n), "{n} outside the dirty subtree was laid out");
    }
    for n in b {
        assert!(!visited.contains(&n));
    }
    assert!(!visited.contains(&h.screen()));
    let st = h.engine().layout_stats();
    assert_eq!((st.roots, st.moved, st.iterations), (1, 1, 1));
    assert_eq!(h.engine().coords(a[1]), Rect::from_xywh(0, 0, 70, 20));
}

#[test]
fn moving_parent_shifts_children_without_relayout() {
    let mut h = harness(240, 120);
    let (a, _) = two_containers(&mut h);
    h.engine_mut().set_y(a[0], 10);
    h.update();
    // Only the container itself changed its rectangle: the leaves moved along with it.
    assert_eq!(h.engine().layout_stats().moved, 1);
    assert_eq!(h.engine().coords(a[0]), Rect::from_xywh(0, 10, 100, 100));
    assert_eq!(h.engine().coords(a[3]), Rect::from_xywh(0, 70, 50, 20));
    // One invalidation: old ∪ new area of the container (they overlap), clipped to the screen.
    let layout: Vec<Rect> = h
        .engine()
        .invalidation_log()
        .iter()
        .filter(|(_, r)| *r == twine_engine::InvalidateReason::Layout)
        .map(|(r, _)| *r)
        .collect();
    assert_eq!(layout, vec![Rect::from_xywh(0, 0, 100, 110)]);
    // The style change itself invalidated the old area: nothing outside old ∪ new is redrawn.
    let all = invalidated(&h);
    let bounds = all.iter().fold(all[0], |u, r| u.union(r));
    assert_eq!(bounds, Rect::from_xywh(0, 0, 100, 110));
}

#[test]
fn size_change_sends_size_changed_and_invalidates_old_and_new() {
    let mut h = harness(240, 120);
    let (a, _) = two_containers(&mut h);
    let got = Rc::new(Cell::new(None));
    let g = got.clone();
    h.engine_mut()
        .add_event_handler(a[2], EventFilter::Code(EventCode::SizeChanged), move |_, ev| {
            if let EventParam::Area(old) = ev.param {
                g.set(Some(old));
            }
            EventResult::Continue
        });
    let parent_events = Rc::new(Cell::new(0));
    let p = parent_events.clone();
    h.engine_mut()
        .add_event_handler(a[0], EventFilter::Code(EventCode::LayoutChanged), move |_, _| {
            p.set(p.get() + 1);
            EventResult::Continue
        });
    h.engine_mut().set_height(a[2], 26);
    h.engine_mut().set_width(a[2], 60);
    h.update();
    assert_eq!(got.get(), Some(Rect::from_xywh(0, 30, 50, 20)));
    assert_eq!(parent_events.get(), 1);
    assert_eq!(h.engine().coords(a[2]), Rect::from_xywh(0, 30, 60, 26));
    // Style changes invalidate the old area; the layout the union of old and new.
    let log = invalidated(&h);
    assert!(log.contains(&Rect::from_xywh(0, 30, 60, 26)), "{log:?}");
    assert!(
        log.iter()
            .all(|r| Rect::from_xywh(0, 30, 60, 26).contains_rect(r)),
        "{log:?}"
    );
}

#[test]
fn moved_node_invalidates_old_and_new_separately_when_far_apart() {
    let mut h = harness(240, 120);
    let (_, b) = two_containers(&mut h);
    h.engine_mut().set_pos(b[1], 50, 80);
    h.update();
    let layout: Vec<Rect> = h
        .engine()
        .invalidation_log()
        .iter()
        .filter(|(_, r)| *r == twine_engine::InvalidateReason::Layout)
        .map(|(r, _)| *r)
        .collect();
    assert_eq!(
        layout,
        vec![Rect::from_xywh(120, 0, 50, 20), Rect::from_xywh(170, 80, 50, 20)]
    );
    // Nothing else was touched: no other node moved.
    assert_eq!(h.engine().layout_stats().moved, 1);
}

#[test]
fn unchanged_layout_does_not_invalidate() {
    let mut h = harness(240, 120);
    let (a, b) = two_containers(&mut h);
    let events = Rc::new(Cell::new(0));
    for n in a.iter().chain(b.iter()) {
        let ev = events.clone();
        h.engine_mut()
            .add_event_handler(*n, EventFilter::All, move |_, e| {
                if matches!(e.code, EventCode::SizeChanged | EventCode::LayoutChanged) {
                    ev.set(ev.get() + 1);
                }
                EventResult::Continue
            });
    }
    // P1: idle means no layout at all.
    assert!(!h.engine().layout_pending());
    assert_eq!(h.update(), Wake::Idle);
    // P3: a relayout that computes the same rectangles changes nothing.
    h.engine_mut().mark_layout_dirty(a[1]);
    h.engine_mut().mark_layout_dirty(b[0]);
    assert!(h.engine().layout_pending());
    h.engine_mut().update_layout();
    assert!(h.engine().invalidation_log().is_empty());
    assert_eq!(events.get(), 0);
    let st = h.engine().layout_stats();
    assert_eq!((st.moved, st.iterations), (0, 1));
    assert_eq!(h.update(), Wake::Idle);
    // Setting a layout property to its current value is a no-op as well.
    h.engine_mut().set_size(a[1], 50, 20);
    assert!(!h.engine().layout_pending());
    assert!(h.engine().invalidation_log().is_empty());
}

#[test]
fn content_sized_parent_grows_with_child() {
    let mut h = harness(240, 120);
    let s = h.screen();
    let e = h.engine_mut();
    let outer = e.create(s, Box::new(Obj)).unwrap();
    e.set_pos(outer, 10, 10);
    style(
        e,
        outer,
        &[
            StyleProp::PadLeft(5),
            StyleProp::PadTop(5),
            StyleProp::PadRight(5),
            StyleProp::PadBottom(5),
        ],
    );
    let inner = e.create(outer, Box::new(Obj)).unwrap(); // content-sized too
    let leaf = node(e, inner, 0, 0, 20, 10, Color::RED);
    h.run_until_idle();
    assert_eq!(h.engine().coords(inner), Rect::from_xywh(15, 15, 20, 10));
    assert_eq!(h.engine().coords(outer), Rect::from_xywh(10, 10, 30, 20));
    h.engine_mut().set_width(leaf, 40);
    h.engine_mut().set_y(leaf, 4);
    h.update();
    assert_eq!(h.engine().coords(inner), Rect::from_xywh(15, 15, 40, 14));
    assert_eq!(h.engine().coords(outer), Rect::from_xywh(10, 10, 50, 24));
    assert_eq!(h.engine().coords(leaf), Rect::from_xywh(15, 19, 40, 10));
    // Deleting the leaf shrinks both again.
    h.engine_mut().delete(leaf).unwrap();
    h.update();
    assert_eq!(h.engine().coords(outer), Rect::from_xywh(10, 10, 10, 10));
}

#[test]
fn layout_converges_or_warns() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);
    let mut h = harness(240, 120);
    let s = h.screen();
    let n = node(h.engine_mut(), s, 0, 0, 10, 10, Color::RED);
    h.run_until_idle();
    // Grows itself on every size change: never converges.
    h.engine_mut()
        .add_event_handler(n, EventFilter::Code(EventCode::SizeChanged), |cx, _| {
            let id = cx.node();
            let w = cx.engine().coords(id).width();
            cx.engine_mut().set_width(id, w + 1);
            EventResult::Continue
        });
    h.engine_mut().set_width(n, 20);
    h.engine_mut().update_layout();
    let st = h.engine().layout_stats();
    assert_eq!(st.iterations, MAX_LAYOUT_ITERATIONS);
    assert!(!st.converged);
    assert_eq!(h.engine().coords(n).width(), 20 + 3);
    assert!(
        WARNINGS
            .lock()
            .unwrap()
            .iter()
            .any(|w| w.contains("did not converge")),
        "warning logged"
    );
    // The rest waits for the next update; a handler that settles converges.
    assert!(h.engine().layout_pending());
}

#[test]
fn layout_changes_after_event_handler_are_applied_in_the_same_update() {
    let mut h = harness(240, 120);
    let s = h.screen();
    let e = h.engine_mut();
    let a = node(e, s, 0, 0, 10, 10, Color::RED);
    let b = node(e, s, 0, 20, 10, 10, Color::BLUE);
    h.run_until_idle();
    // `b` follows `a`'s width once (converges in two iterations).
    h.engine_mut()
        .add_event_handler(a, EventFilter::Code(EventCode::SizeChanged), move |cx, _| {
            let w = cx.engine().coords(cx.node()).width();
            cx.engine_mut().set_width(b, w);
            EventResult::Continue
        });
    h.engine_mut().set_width(a, 30);
    h.update();
    let st = h.engine().layout_stats();
    assert_eq!((st.iterations, st.converged), (2, true));
    assert_eq!(h.engine().coords(b).width(), 30);
}

#[test]
fn flags_and_children_changes_relayout_flex_parent() {
    let mut h = harness(240, 120);
    let s = h.screen();
    let e = h.engine_mut();
    let row = node(e, s, 0, 0, 200, 40, Color::hex(0xEE_EE_EE));
    e.set_layout(row, LayoutKind::Flex);
    e.set_flex_flow(row, FlexFlow::Row);
    let items: Vec<NodeId> = (0..3).map(|_| node(e, row, 0, 0, 20, 20, Color::RED)).collect();
    h.run_until_idle();
    let x = |h: &EngineHarness, i: usize| h.engine().coords(items[i]).x0;
    assert_eq!((x(&h, 0), x(&h, 1), x(&h, 2)), (0, 20, 40));
    h.engine_mut().set_flag(items[0], ObjFlags::HIDDEN, true);
    h.update();
    assert_eq!((x(&h, 1), x(&h, 2)), (0, 20));
    h.engine_mut().set_flag(items[0], ObjFlags::HIDDEN, false);
    h.engine_mut().set_flag(items[1], ObjFlags::IGNORE_LAYOUT, true);
    h.update();
    assert_eq!((x(&h, 0), x(&h, 1), x(&h, 2)), (0, 0, 20));
    h.engine_mut().set_flag(items[1], ObjFlags::IGNORE_LAYOUT, false);
    h.engine_mut().delete(items[0]).unwrap();
    h.update();
    assert_eq!((x(&h, 1), x(&h, 2)), (0, 20));
    let e = h.engine_mut();
    let extra = node(e, row, 0, 0, 20, 20, Color::GREEN);
    e.set_flex_grow(extra, 1);
    h.update();
    assert_eq!(h.engine().coords(extra), Rect::from_xywh(40, 0, 160, 20));
}

#[test]
fn align_to_follows_its_base() {
    let mut h = harness(240, 120);
    let s = h.screen();
    let e = h.engine_mut();
    let base = node(e, s, 20, 20, 40, 20, Color::BLUE);
    let tip = node(e, s, 0, 0, 30, 10, Color::RED);
    e.align_to(tip, base, Align::OutBottomMid, 0, 4);
    h.run_until_idle();
    assert_eq!(h.engine().coords(tip), Rect::from_xywh(25, 44, 30, 10));
    h.engine_mut().set_x(base, 100);
    h.update();
    assert_eq!(h.engine().coords(tip), Rect::from_xywh(105, 44, 30, 10));
    h.engine_mut().clear_align_to(tip);
    h.update();
    assert_eq!(h.engine().coords(tip), Rect::from_xywh(0, 0, 30, 10));
}

#[test]
fn place_is_overwritten_by_layout_of_parent() {
    let mut h = harness(240, 120);
    let s = h.screen();
    let b = boxed(h.engine_mut(), s, Rect::from_xywh(10, 10, 20, 20), Color::RED);
    h.engine_mut().place(b, Rect::from_xywh(50, 50, 20, 20));
    h.update();
    assert_eq!(
        h.engine().coords(b),
        Rect::from_xywh(50, 50, 20, 20),
        "no layout: kept"
    );
    let _ = node(h.engine_mut(), s, 0, 0, 5, 5, Color::BLUE); // the screen is laid out
    h.update();
    assert_eq!(h.engine().coords(b), Rect::from_xywh(10, 10, 20, 20));
}

/// A flex-wrap container with `n` children of mixed sizes.
fn flex_tree(e: &mut Engine, n: usize) -> NodeId {
    let s = white_screen(e);
    let c = e.create(s, Box::new(Obj)).unwrap();
    e.set_size(c, Length::pct(100), Length::pct(100));
    e.set_layout(c, LayoutKind::Flex);
    e.set_flex_flow(c, FlexFlow::RowWrap);
    e.set_flex_align(c, FlexAlign::SpaceEvenly, FlexAlign::Center, FlexAlign::Start);
    style(e, c, &[StyleProp::PadRow(4), StyleProp::PadColumn(4)]);
    for i in 0..n {
        let w = 8 + (i as i32 * 7) % 20;
        let _ = node(
            e,
            c,
            0,
            0,
            w,
            6 + (i as i32 * 3) % 10,
            Color::hex(0x10_20_30 * (i as u32 % 7 + 1)),
        );
    }
    c
}

#[test]
fn layout_allocates_nothing_in_steady_state() {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        flex_tree(e, 100);
    });
    h.run_until_idle();
    let s = h.screen();
    let c = h.engine().tree().children(s).next().unwrap();
    let first = h.engine().tree().children(c).next().unwrap();
    let mut w = 8;
    // Warm-up: grows the scratch buffers and the dirty area lists.
    for _ in 0..3 {
        w = if w == 8 { 30 } else { 8 };
        h.engine_mut().set_width(first, w);
        h.advance(Duration::ms(16));
    }
    let ((), stats) = count_allocs(|| {
        w = if w == 8 { 30 } else { 8 };
        h.engine_mut().set_width(first, w);
        h.advance(Duration::ms(16));
    });
    assert!(
        h.engine().layout_stats().moved > 1,
        "{:?}",
        h.engine().layout_stats()
    );
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}

#[test]
fn layout_flex_row_wrap() {
    let mut h = EngineHarness::new(160, 120).no_theme().mount_engine(|e| {
        let c = flex_tree(e, 24);
        style(
            e,
            c,
            &[
                StyleProp::PadLeft(6),
                StyleProp::PadTop(6),
                StyleProp::PadRight(6),
                StyleProp::PadBottom(6),
            ],
        );
        let g = e.tree().children(c).nth(5).unwrap();
        e.set_flex_grow(g, 1);
    });
    h.run_until_idle();
    h.assert_snapshot("layout_flex_row_wrap");
}

static COLS: [GridTrack; 3] = [GridTrack::Px(40), GridTrack::Content, GridTrack::Fr(1)];
static ROWS: [GridTrack; 3] = [GridTrack::Fr(1), GridTrack::Px(30), GridTrack::Fr(2)];

#[test]
fn layout_grid_3x3() {
    let mut h = EngineHarness::new(160, 120).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create(s, Box::new(Obj)).unwrap();
        e.set_size(g, 150, 110);
        e.align(g, Align::Center, 0, 0);
        e.set_layout(g, LayoutKind::Grid);
        e.set_grid_dsc_array(g, &COLS, &ROWS);
        style(
            e,
            g,
            &[
                StyleProp::PadRow(4),
                StyleProp::PadColumn(4),
                StyleProp::BgColor(Color::hex(0xE0_E0_E0)),
                StyleProp::BgOpa(Opa::COVER),
            ],
        );
        let aligns = [GridAlign::Stretch, GridAlign::Center, GridAlign::End];
        for i in 0..8 {
            let (col, row) = (i % 3, i / 3);
            let n = node(e, g, 0, 0, 24, 16, Color::hex(0x30_60_90 + i as u32 * 0x15_0B_00));
            let span = if i == 7 { 2 } else { 1 };
            e.set_grid_cell(n, aligns[col as usize], col, span, aligns[row as usize], row, 1);
        }
    });
    h.run_until_idle();
    h.assert_snapshot("layout_grid_3x3");
}

#[test]
fn layout_align_to() {
    let mut h = EngineHarness::new(160, 120).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let base = node(e, s, 0, 0, 60, 30, Color::hex(0x3F_51_B5));
        e.set_align(base, Align::Center);
        let aligns = [
            (Align::OutTopLeft, Color::RED),
            (Align::OutTopRight, Color::GREEN),
            (Align::OutBottomMid, Color::BLUE),
            (Align::OutLeftMid, Color::hex(0xFF_98_00)),
            (Align::OutRightBottom, Color::hex(0x9C_27_B0)),
            (Align::Center, Color::WHITE),
        ];
        for (a, c) in aligns {
            let n = node(e, s, 0, 0, 16, 10, c);
            e.align_to(n, base, a, 0, 0);
        }
    });
    h.run_until_idle();
    h.assert_snapshot("layout_align_to");
}
