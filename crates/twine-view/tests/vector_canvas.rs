//! `vector_canvas`: redraws when a signal read by `draw` changes, is idle otherwise,
//! invalidates only the old and new scene bounds, and animates without allocating.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::{Angle, Color, Duration, Fx, Point, Rect, Transform};
use twine_testing::TestUi;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_vector::{FxPoint, VectorDsc, VectorScene};
use twine_view::prelude::*;

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

/// A five-pointed star of outer radius `r` around (`cx`, `cy`), rotated by `angle`.
fn star(scene: &mut VectorScene, cx: i32, cy: i32, r: i32, angle: Angle, color: Color) {
    let (path, dsc) = scene.path_mut();
    for k in 0..10 {
        let radius = if k % 2 == 0 { r } else { r * 2 / 5 };
        let t = Transform::rotate(Angle(k * 360));
        let (x, y) = t.map(Fx::ZERO, Fx::from_int(-radius));
        let p = FxPoint::new(x + Fx::from_int(cx), y + Fx::from_int(cy));
        if k == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close();
    *dsc = VectorDsc::fill(color);
    dsc.transform = Transform::rotate(angle).around(Point::new(cx, cy));
}

/// A canvas at (0, 0) of 120 × 100 showing a circle at `x` of radius 10.
/// The signals of the circle canvas: its x and an unrelated value `draw` reads.
type CircleSignals = (Signal<i32>, Signal<u8>);

fn circle_ui() -> (TestUi, Signal<i32>, Signal<u8>) {
    let slot: Rc<Cell<Option<CircleSignals>>> = Rc::default();
    let s = slot.clone();
    let mut t = TestUi::new(160, 120).mount(move |cx| {
        let x = cx.signal(30);
        let other = cx.signal(0u8);
        s.set(Some((x, other)));
        vector_canvas(move |scene| {
            // Reads `other` but draws the same scene for every value of it.
            let _ = other.get();
            let (path, dsc) = scene.path_mut();
            path.circle(FxPoint::from_int(x.get(), 50), Fx::from_int(10));
            *dsc = VectorDsc::fill(Color::RED);
        })
        .size(120, 100)
        .align(Align::TopLeft)
    });
    t.run_until_idle();
    let (x, other) = slot.get().unwrap();
    (t, x, other)
}

#[test]
fn vector_canvas_redraws_on_signal() {
    let (mut t, x, _) = circle_ui();
    assert_eq!(t.harness_mut().pixel(30, 50), Color::RED);
    x.set(70);
    t.run_until_idle();
    assert_eq!(t.harness_mut().pixel(70, 50), Color::RED);
    assert_ne!(t.harness_mut().pixel(30, 50), Color::RED);
}

#[test]
fn vector_canvas_idle_when_unchanged() {
    let (mut t, x, other) = circle_ui();
    other.set(1); // `draw` runs again, the scene is equal: nothing to redraw
    t.run_until_idle();
    assert!(t.flushes().is_empty(), "{:?}", t.flushes());
    x.set(30); // same value: the binding does not even run
    t.run_until_idle();
    assert!(t.flushes().is_empty());
    t.assert_idle();
}

#[test]
fn scene_invalidation_covers_bounds_only() {
    let (mut t, x, _) = circle_ui();
    x.set(80);
    t.run_until_idle();
    // Old circle around (30, 50), new around (80, 50), radius 10 (+1 px anti-aliasing).
    let allowed = [Rect::new(19, 39, 42, 62), Rect::new(69, 39, 92, 62)];
    let flushes = t.flushes();
    assert!(!flushes.is_empty());
    for f in flushes.iter() {
        assert!(
            allowed.iter().any(|a| a.contains_rect(&f.area)),
            "{} outside the old and new bounds",
            f.area
        );
    }
}

fn star_ui() -> (TestUi, AnimController) {
    let slot: Rc<Cell<Option<AnimController>>> = Rc::default();
    let s = slot.clone();
    let mut t = TestUi::new(120, 120).mount(move |cx| {
        let (angle, ctrl) = cx.animation(
            Anim::new(0, 3600)
                .duration(Duration::secs(4))
                .repeat(Repeat::Infinite),
        );
        s.set(Some(ctrl));
        vector_canvas(move |scene| star(scene, 60, 60, 40, Angle(angle.get()), Color::hex(0x00F9_A825)))
            .size(120, 120)
            .align(Align::TopLeft)
    });
    t.advance(Duration::ms(16));
    (t, slot.take().unwrap())
}

#[test]
fn rotating_path_anim_snapshot_sequence() {
    let (mut t, _ctrl) = star_ui();
    for i in 0..3 {
        t.advance(Duration::ms(250));
        // The animation runs forever: snapshot the panel as it is (no wait for idle).
        t.assert_panel_snapshot(&format!("vector_canvas_star_{i}"));
    }
}

#[test]
fn rotating_star_allocates_nothing_per_frame() {
    let (mut t, _ctrl) = star_ui();
    for _ in 0..10 {
        t.advance(Duration::ms(16)); // warm-up: buffers grow to their largest size
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..30 {
            t.advance(Duration::ms(16));
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    // Only the star's area is redrawn.
    for f in t.flushes().iter() {
        assert!(Rect::new(18, 18, 102, 102).contains_rect(&f.area), "{}", f.area);
    }
}
