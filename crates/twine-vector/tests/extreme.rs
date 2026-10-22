//! Regression tests of R5.S13 (panic-site cleanup of the vector crate): accumulations and
//! geometry at the ends of their ranges no longer overflow.

use twine_core::{Color, ColorFormat, Fx, Rect};
use twine_render::{DrawBuf, Painter, RenderCaches};
use twine_vector::{FxPoint, LineCap, LineJoin, Paint, PainterVectorExt, Path, Stroke, VectorDsc};

/// Draws `f` into a black 16 × 16 `L8` buffer and returns its pixels.
fn draw(f: impl FnOnce(&mut Painter<'_>)) -> Vec<u8> {
    let mut caches = RenderCaches::default();
    let mut px = vec![0u8; 16 * 16];
    let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, 16, 16)).unwrap();
    f(&mut Painter::new(buf, &mut caches));
    px
}

#[test]
fn regression_many_overlapping_edges_do_not_overflow() {
    // 40 000 copies of one square add `40 000 · 2^16` to the same accumulator cells: more
    // than an `i32` holds. The cells now saturate; the coverage is still full inside and
    // empty outside.
    let mut path = Path::new();
    for _ in 0..40_000 {
        path.rect(Rect::from_xywh(4, 4, 8, 8), 0);
    }
    let px = draw(|p| p.vector(&path, &VectorDsc::fill(Color::WHITE)));
    for y in 0..16 {
        for x in 0..16 {
            let inside = (4..12).contains(&x) && (4..12).contains(&y);
            assert_eq!(px[y * 16 + x], if inside { 255 } else { 0 }, "({x}, {y})");
        }
    }
}

#[test]
fn regression_extreme_strokes_do_not_overflow() {
    // The widest stroke with the largest miter limit around coordinates at the ends of the
    // 16.16 range.
    let mut path = Path::new();
    path.move_to(FxPoint::new(Fx::MIN, Fx::MIN))
        .line_to(FxPoint::new(Fx::MAX, Fx::from_int(8)))
        .line_to(FxPoint::new(Fx::MIN, Fx::MAX))
        .cubic_to(
            FxPoint::new(Fx::MAX, Fx::MIN),
            FxPoint::new(Fx::MIN, Fx::MAX),
            FxPoint::new(Fx::MAX, Fx::MAX),
        );
    let dsc = VectorDsc {
        stroke: Some((
            Paint::Solid(Color::WHITE),
            Stroke {
                width: Fx::MAX,
                join: LineJoin::Miter,
                cap: LineCap::Square,
                miter_limit: Fx::MAX,
                dash: None,
            },
        )),
        ..VectorDsc::default()
    };
    draw(|p| p.vector(&path, &dsc));
}
