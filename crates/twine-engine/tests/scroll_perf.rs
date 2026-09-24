//! Performance guarantees of scrolling (P1, P2, P4): only the scrolled list is redrawn, no
//! layout runs, a steady drag frame allocates nothing, the engine idles once the scroll has
//! settled, and a long list draws only its visible rows.

mod common;

use common::white_screen;
use twine_core::{Duration, Point, Rect};
use twine_engine::NodeId;
use twine_hal::BufferSpec;
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::scenes::scroll_list;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const LIST: Rect = Rect::from_xywh(20, 40, 200, 200);

/// A 240×320 screen with a 200×200 list of `rows` rows of 40 px at (20, 40).
fn scene(rows: usize) -> (EngineHarness, NodeId) {
    scene_with(rows, BufferSpec::PartialDouble { rows: 40 })
}

fn scene_with(rows: usize, buffers: BufferSpec) -> (EngineHarness, NodeId) {
    let mut ids = None;
    let mut h = EngineHarness::new(240, 320)
        .no_theme()
        .buffers(buffers)
        .mount_engine(|e| {
            let s = white_screen(e);
            ids = Some(scroll_list(e, s, LIST, rows, 40).0);
        });
    h.run_until_idle();
    (h, ids.unwrap())
}

/// Presses in the list and starts scrolling (two reads).
fn start_drag(h: &mut EngineHarness) -> i32 {
    h.press(Point::new(100, 200));
    let mut y = 200;
    for _ in 0..2 {
        y -= 12;
        h.clock().advance(h.engine().config().read_period);
        h.move_to(Point::new(100, y));
    }
    y
}

/// One drag frame: the pointer moves 4 px up one read period later.
fn drag_frame(h: &mut EngineHarness, y: &mut i32) {
    *y -= 4;
    h.clock().advance(h.engine().config().read_period);
    h.move_to(Point::new(100, *y));
}

#[test]
fn scrolling_list_invalidates_only_list_area() {
    let (mut h, list) = scene(100);
    let mut y = start_drag(&mut h);
    for _ in 0..10 {
        drag_frame(&mut h, &mut y);
        assert!(!h.flushes().is_empty(), "each drag frame redraws");
        for f in h.flushes() {
            assert!(LIST.contains_rect(&f.area), "{:?} outside the list", f.area);
        }
        assert!(h.last_frame().dirty_px <= LIST.area() as u32);
    }
    assert!(h.engine().scroll_offset(list).y > 40);
}

#[test]
fn scrolling_does_not_run_layout() {
    let (mut h, _) = scene(100);
    let mut y = start_drag(&mut h);
    let stats = h.engine().layout_stats();
    for _ in 0..10 {
        drag_frame(&mut h, &mut y);
        assert!(!h.engine().layout_pending());
    }
    h.release();
    h.run_until_idle();
    assert_eq!(h.engine().layout_stats(), stats, "no layout pass ran");
}

#[test]
fn scroll_frame_zero_alloc() {
    let (mut h, list) = scene(100);
    let mut y = start_drag(&mut h);
    for _ in 0..5 {
        drag_frame(&mut h, &mut y); // warm-up
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..60 {
            drag_frame(&mut h, &mut y);
        }
    });
    assert!(h.engine().is_scrolling(list));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn idle_after_scroll_settles() {
    let (mut h, list) = scene(100);
    let mut y = start_drag(&mut h);
    for _ in 0..3 {
        drag_frame(&mut h, &mut y);
    }
    h.release();
    let t = h.run_until_idle();
    assert!(t < Duration::secs(2), "settled in {t:?}");
    assert!(!h.engine().is_scrolling(list));
    assert_eq!(h.engine().input_deadline(), None);
    h.assert_idle();
}

#[test]
fn large_list_scroll_cost() {
    // One chunk per frame (buffers of the full height): nodes are counted once per chunk.
    let (mut h, list) = scene_with(1000, BufferSpec::PartialDouble { rows: 320 });
    let mut y = start_drag(&mut h);
    drag_frame(&mut h, &mut y);
    let s = h.last_frame();
    assert_eq!(s.chunks, 1);
    // A 200 px list of 40 px rows shows at most 6 rows (partially) at a time; drawing starts
    // at the list (the topmost node covering the dirty area) and skips hidden rows.
    let visible_rows = 6;
    println!(
        "large_list_scroll_cost: render_us={} nodes_drawn={} dirty_px={}",
        s.render_us, s.nodes_drawn, s.dirty_px
    );
    assert!(h.engine().scroll_offset(list).y > 0);
    assert!(
        s.nodes_drawn <= visible_rows + 10,
        "nodes drawn: {} (visible rows {visible_rows})",
        s.nodes_drawn
    );
}
