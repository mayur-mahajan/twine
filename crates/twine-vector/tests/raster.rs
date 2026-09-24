//! Rasterizer: fills, fill rules, sub-pixel coverage, chunked rendering, allocations.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use twine_core::{Color, ColorFormat, Fx, Opa, Rect};
use twine_render::{Painter, RenderConfig};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_vector::{FillRule, FxPoint, Paint, PainterVectorExt, Path, VectorCaches, VectorDsc};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

const BLUE: Color = Color::hex(0x1E88E5);
const RED: Color = Color::hex(0xE53935);

fn fill(color: Color, rule: FillRule) -> VectorDsc {
    VectorDsc {
        fill: Some((Paint::Solid(color), rule)),
        ..VectorDsc::default()
    }
}

fn pt(x: i32, y: i32) -> FxPoint {
    FxPoint::from_int(x, y)
}

/// A five-pointed star drawn as one self-intersecting pentagram.
fn star(cx: i32, cy: i32, r: i32) -> Path {
    let mut p = Path::new();
    let outer: [(i32, i32); 5] = [(0, -100), (59, 81), (-95, -31), (95, -31), (-59, 81)];
    for (i, (x, y)) in outer.into_iter().enumerate() {
        let q = pt(cx + x * r / 100, cy + y * r / 100);
        if i == 0 {
            p.move_to(q);
        } else {
            p.line_to(q);
        }
    }
    p.close();
    p
}

#[test]
fn triangle_fill_snapshot() {
    let mut h = RenderHarness::new(80, 60, ColorFormat::Rgb565);
    let mut p = Path::new();
    p.move_to(FxPoint::new(Fx::from_ratio(81, 2), Fx::from_int(5)))
        .line_to(pt(74, 55))
        .line_to(FxPoint::new(Fx::from_int(6), Fx::from_ratio(101, 2)))
        .close();
    h.paint(|pa| pa.vector(&p, &fill(BLUE, FillRule::NonZero)));
    assert_render_snapshot!(h, "triangle_fill");
}

#[test]
fn star_nonzero_vs_evenodd_snapshots() {
    let s = star(40, 42, 36);
    let mut h = RenderHarness::new(80, 80, ColorFormat::Rgb565);
    h.paint(|pa| pa.vector(&s, &fill(RED, FillRule::NonZero)));
    // Center filled with non-zero.
    let center = &h.rgb888()[(42 * 80 + 40) * 3..(42 * 80 + 40) * 3 + 3];
    assert_eq!(center, &[231, 56, 49]);
    assert_render_snapshot!(h, "star_nonzero");
    let mut h = RenderHarness::new(80, 80, ColorFormat::Rgb565);
    h.paint(|pa| pa.vector(&s, &fill(RED, FillRule::EvenOdd)));
    // Hole in the center with even-odd.
    let center = &h.rgb888()[(42 * 80 + 40) * 3..(42 * 80 + 40) * 3 + 3];
    assert_eq!(center, &[255, 255, 255]);
    assert_render_snapshot!(h, "star_evenodd");
}

fn l8_harness(w: u16, h: u16) -> RenderHarness {
    let mut hh = RenderHarness::new(w, h, ColorFormat::L8);
    hh.clear(Color::BLACK);
    hh
}

#[test]
fn half_pixel_edge_coverage_128() {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut h = l8_harness(30, 10);
        let mut p = Path::new();
        p.rounded_rect(
            twine_vector::FxRect::new(
                Fx::from_ratio(21, 2),
                Fx::from_int(2),
                Fx::from_int(20),
                Fx::from_int(8),
            ),
            Fx::ZERO,
            Fx::ZERO,
        );
        h.paint(|pa| pa.vector(&p, &fill(Color::WHITE, rule)));
        let row = &h.data()[5 * 30..6 * 30];
        assert_eq!(row[9], 0, "{rule:?}");
        assert!(row[10].abs_diff(128) <= 2, "{rule:?}: {}", row[10]);
        assert_eq!(row[11], 255, "{rule:?}");
        assert_eq!(row[19], 255, "{rule:?}");
        assert_eq!(row[20], 0, "{rule:?}");
        // Rows outside the rect stay untouched.
        assert!(h.data()[..2 * 30].iter().all(|&v| v == 0));
    }
}

struct Scene {
    circle: Path,
    star: Path,
    big: Path,
}

fn make_scene() -> Scene {
    let mut circle = Path::new();
    circle.circle(
        FxPoint::new(Fx::from_ratio(301, 4), Fx::from_ratio(123, 2)),
        Fx::from_ratio(97, 2),
    );
    // Partly outside the buffer on every side.
    let mut big = Path::new();
    big.move_to(pt(-30, 40))
        .line_to(pt(150, -20))
        .line_to(pt(140, 150))
        .line_to(pt(-10, 130))
        .close();
    Scene {
        circle,
        star: star(60, 60, 55),
        big,
    }
}

fn draw_scene(sc: &Scene, pa: &mut Painter<'_>) {
    pa.vector(&sc.circle, &fill(BLUE, FillRule::NonZero));
    pa.vector(
        &sc.star,
        &VectorDsc {
            fill: Some((Paint::Solid(RED), FillRule::EvenOdd)),
            opa: Opa::P70,
            ..VectorDsc::default()
        },
    );
    pa.vector(
        &sc.big,
        &VectorDsc {
            fill: Some((Paint::Solid(Color::BLACK), FillRule::NonZero)),
            opa: Opa::P20,
            ..VectorDsc::default()
        },
    );
}

#[test]
fn clip_chunk_identical_to_full() {
    let sc = make_scene();
    let scene = |pa: &mut Painter<'_>| draw_scene(&sc, pa);
    let mut full = RenderHarness::new(120, 120, ColorFormat::Rgb565);
    full.paint(scene);
    for rows in [30, 7, 1] {
        let mut chunked = RenderHarness::new(120, 120, ColorFormat::Rgb565);
        chunked.paint_chunked(rows, scene);
        assert!(full.data() == chunked.data(), "chunk rows {rows} differ");
    }
    // Clip rectangles (horizontal and vertical splits) also match.
    let mut clipped = RenderHarness::new(120, 120, ColorFormat::Rgb565);
    for r in [
        Rect::new(0, 0, 50, 70),
        Rect::new(50, 0, 120, 70),
        Rect::new(0, 70, 120, 120),
    ] {
        clipped.paint(|pa| pa.with_clip(r, scene));
    }
    assert!(full.data() == clipped.data(), "clip split differs");
    assert_render_snapshot!(full, "raster_chunk_scene");
}

#[test]
fn no_alloc_steady_state() {
    let sc = make_scene();
    let scene = |pa: &mut Painter<'_>| draw_scene(&sc, pa);
    let mut h = RenderHarness::new(200, 150, ColorFormat::Rgb565).with_config(RenderConfig::default());
    h.paint(scene);
    let ((), stats) = count_allocs(|| h.paint(scene));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    // Explicit caches, pre-sized: no allocation even on the first draw.
    let mut caches = VectorCaches::with_capacity(200, 256);
    let s = star(60, 60, 50);
    let ((), stats) =
        count_allocs(|| h.paint(|pa| pa.vector_with(&mut caches, &s, &fill(RED, FillRule::EvenOdd))));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert!(caches.bytes_reserved() > 0);
}

#[test]
fn off_screen_and_degenerate_paths_draw_nothing() {
    let mut h = l8_harness(20, 20);
    let mut p = Path::new();
    p.circle(pt(-50, -50), Fx::from_int(10));
    p.move_to(pt(5, 5)).line_to(pt(15, 5)).close(); // zero area
    h.paint(|pa| {
        pa.vector(&p, &fill(Color::WHITE, FillRule::NonZero));
        pa.vector(&Path::new(), &fill(Color::WHITE, FillRule::NonZero));
        assert!(pa.touched_area().is_none());
    });
}
