//! Criterion benchmark of one drag frame on a scrolling list: a pointer read that scrolls a
//! 200 × 200 list of 100 rows by 4 px, including the scroll (children moved, no layout), the
//! invalidation of the list and its refresh (RGB565, 320 × 240, blocking in-memory panel).
//!
//! Run with `cargo bench -p twine-bench --bench scroll`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use criterion::{Criterion, criterion_group, criterion_main};
use twine_core::{Point, Rect};
use twine_testing::EngineHarness;
use twine_testing::scenes::scroll_list;

fn scroll_drag_frame_100_rows(c: &mut Criterion) {
    let mut list = None;
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = e.active_screen(e.default_display().unwrap()).unwrap();
        list = Some(scroll_list(e, s, Rect::from_xywh(60, 20, 200, 200), 100, 40).0);
    });
    let list = list.unwrap();
    h.engine_mut().scroll_to_y(list, 1000, false);
    h.run_until_idle();
    let period = h.engine().config().read_period;
    // Start the scroll, then move up and down by 4 px per read (the list stays scrolling).
    h.press(Point::new(160, 120));
    for y in [108, 96] {
        h.clock().advance(period);
        h.move_to(Point::new(160, y));
    }
    let mut up = true;
    c.bench_function("scroll_drag_frame_100_rows", |b| {
        b.iter(|| {
            h.clock().advance(period);
            h.move_to(Point::new(160, if up { 92 } else { 96 }));
            up = !up;
        });
    });
    assert!(h.engine().is_scrolling(list));
}

criterion_group!(benches, scroll_drag_frame_100_rows);
criterion_main!(benches);
