//! The vector demo: every tab renders without warnings, the animated star allocates nothing
//! per frame and redraws only its own area.

use twine::core::{Duration, Rect};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id, by_text, capture_logs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn vector_demo_smoke() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(480, 320).mount(twine_demos::vector::app);
        t.advance(Duration::ms(100));
        for title in ["Gradients", "Strokes", "SVG & motion", "Shapes"] {
            t.find(by_text(title)).click();
            t.advance(Duration::ms(600));
        }
    });
    let bad: Vec<_> = logs
        .iter()
        .filter(|l| l.level <= log::Level::Warn && l.target.starts_with("twine"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn vector_demo_snapshots() {
    let mut t = TestUi::new(480, 320).mount(twine_demos::vector::app);
    t.advance(Duration::ms(100));
    for (title, name) in [
        ("Shapes", "vector_shapes"),
        ("Gradients", "vector_gradients"),
        ("Strokes", "vector_strokes"),
    ] {
        t.find(by_text(title)).click();
        t.advance(Duration::ms(600));
        t.assert_panel_snapshot(name);
    }
}

#[test]
fn motion_tab_is_allocation_free_and_local() {
    let mut t = TestUi::new(480, 320).mount(twine_demos::vector::app);
    t.advance(Duration::ms(100));
    t.find(by_text("SVG & motion")).click();
    for _ in 0..40 {
        t.advance(Duration::ms(16)); // tab animation, SVG parsing, cache warm-up
    }
    let star: Rect = t.find(by_id("motion")).coords();
    let ((), stats) = count_allocs(|| {
        for _ in 0..30 {
            t.advance(Duration::ms(16));
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    for f in t.flushes().iter() {
        assert!(
            star.contains_rect(&f.area),
            "{} outside the star canvas {star}",
            f.area
        );
    }
}
