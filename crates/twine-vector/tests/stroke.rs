//! Strokes: joins, caps, dashes, miter limit, closed outlines, dots, watertightness.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

use twine_core::{Angle, Color, ColorFormat, Fx, Rect, Transform};
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_vector::{Dash, FxPoint, LineCap, LineJoin, Paint, PainterVectorExt, Path, Stroke, VectorDsc};

const INK: Color = Color::hex(0x263238);

fn pt(x: i32, y: i32) -> FxPoint {
    FxPoint::from_int(x, y)
}

fn stroke(width: i32, join: LineJoin, cap: LineCap) -> VectorDsc {
    VectorDsc {
        stroke: Some((
            Paint::Solid(INK),
            Stroke {
                width: Fx::from_int(width),
                join,
                cap,
                ..Stroke::default()
            },
        )),
        ..VectorDsc::default()
    }
}

/// A zig-zag with a sharp, a right and an obtuse angle.
fn zigzag() -> Path {
    let mut p = Path::new();
    p.move_to(pt(12, 70))
        .line_to(pt(40, 14))
        .line_to(pt(62, 70))
        .line_to(pt(98, 34))
        .line_to(pt(128, 44));
    p
}

fn joins(join: LineJoin, name: &str) {
    let mut h = RenderHarness::new(140, 84, ColorFormat::Rgb565);
    let z = zigzag();
    h.paint(|p| {
        p.vector(&z, &stroke(12, join, LineCap::Butt));
        // The centerline on top, to show the geometry.
        p.vector(
            &z,
            &VectorDsc {
                stroke: Some((
                    Paint::Solid(Color::hex(0xFF7043)),
                    Stroke {
                        width: Fx::ONE,
                        ..Stroke::default()
                    },
                )),
                ..VectorDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, name);
}

#[test]
fn stroke_joins_miter() {
    joins(LineJoin::Miter, "stroke_joins_miter");
}

#[test]
fn stroke_joins_round() {
    joins(LineJoin::Round, "stroke_joins_round");
}

#[test]
fn stroke_joins_bevel() {
    joins(LineJoin::Bevel, "stroke_joins_bevel");
}

fn caps(cap: LineCap, name: &str) {
    let mut h = RenderHarness::new(120, 60, ColorFormat::Rgb565);
    let mut p = Path::new();
    p.move_to(pt(20, 18)).line_to(pt(100, 18));
    p.move_to(pt(24, 48)).line_to(pt(96, 36));
    h.paint(|pa| {
        pa.fill(
            Rect::new(20, 0, 21, 60),
            Color::hex(0xFF7043),
            twine_core::Opa::COVER,
        );
        pa.fill(
            Rect::new(100, 0, 101, 60),
            Color::hex(0xFF7043),
            twine_core::Opa::COVER,
        );
        pa.vector(&p, &stroke(14, LineJoin::Miter, cap));
    });
    assert_render_snapshot!(h, name);
}

#[test]
fn stroke_caps_butt() {
    caps(LineCap::Butt, "stroke_caps_butt");
}

#[test]
fn stroke_caps_round() {
    caps(LineCap::Round, "stroke_caps_round");
}

#[test]
fn stroke_caps_square() {
    caps(LineCap::Square, "stroke_caps_square");
}

#[test]
fn dash_pattern_snapshot() {
    let mut h = RenderHarness::new(160, 90, ColorFormat::Rgb565);
    let mut line = Path::new();
    line.move_to(pt(10, 14)).line_to(pt(150, 14));
    let mut curve = Path::new();
    curve
        .move_to(pt(10, 70))
        .cubic_to(pt(50, 10), pt(110, 110), pt(150, 40));
    let mut rect = Path::new();
    rect.rect(Rect::new(20, 28, 60, 50), 6);
    h.paint(|p| {
        let d = |w: i32, cap, pat: &[i32], off: i32| VectorDsc {
            stroke: Some((
                Paint::Solid(INK),
                Stroke {
                    width: Fx::from_int(w),
                    cap,
                    join: LineJoin::Round,
                    dash: Some(Dash::new(
                        &pat.iter().map(|&v| Fx::from_int(v)).collect::<Vec<_>>(),
                        Fx::from_int(off),
                    )),
                    ..Stroke::default()
                },
            )),
            ..VectorDsc::default()
        };
        p.vector(&line, &d(4, LineCap::Butt, &[12, 6, 2, 6], 0));
        p.vector(&curve, &d(5, LineCap::Round, &[0, 10], 0));
        p.vector(&rect, &d(3, LineCap::Butt, &[8, 4], 3));
    });
    assert_render_snapshot!(h, "dash_pattern");
}

/// Coverage profile of row `y` in an L8 harness drawn white on black.
fn l8(w: u16, hh: u16, f: impl FnOnce(&mut twine_render::Painter<'_>)) -> RenderHarness {
    let mut h = RenderHarness::new(w, hh, ColorFormat::L8);
    h.clear(Color::BLACK);
    h.paint(f);
    h
}

fn white(width: i32, join: LineJoin, cap: LineCap, limit: i32) -> VectorDsc {
    VectorDsc {
        stroke: Some((
            Paint::Solid(Color::WHITE),
            Stroke {
                width: Fx::from_int(width),
                join,
                cap,
                miter_limit: Fx::from_int(limit),
                dash: None,
            },
        )),
        ..VectorDsc::default()
    }
}

fn lit(h: &RenderHarness) -> usize {
    h.data().iter().filter(|&&v| v > 0).count()
}

#[test]
fn miter_limit_falls_back_to_bevel() {
    // A 30° corner: miter ratio 1/sin(15°) ≈ 3.86.
    let mut p = Path::new();
    p.move_to(pt(10, 60)).line_to(pt(80, 60)).line_to(pt(19, 25));
    let miter = l8(100, 100, |pa| {
        pa.vector(&p, &white(8, LineJoin::Miter, LineCap::Butt, 4));
    });
    let limited = l8(100, 100, |pa| {
        pa.vector(&p, &white(8, LineJoin::Miter, LineCap::Butt, 3));
    });
    let bevel = l8(100, 100, |pa| {
        pa.vector(&p, &white(8, LineJoin::Bevel, LineCap::Butt, 4));
    });
    assert!(limited.data() == bevel.data(), "limit 3 must bevel");
    assert!(
        lit(&miter) > lit(&bevel) + 10,
        "{} vs {}",
        lit(&miter),
        lit(&bevel)
    );
    // The miter tip reaches ≈ 15 px right of the vertex, along (0.97, 0.26).
    assert!(miter.data()[63 * 100 + 92] > 0);
    assert_eq!(bevel.data()[63 * 100 + 92], 0);
}

#[test]
fn closed_rect_stroke_no_cap_artifacts() {
    let mut open = Path::new();
    open.move_to(pt(10, 10))
        .line_to(pt(50, 10))
        .line_to(pt(50, 40))
        .line_to(pt(10, 40))
        .line_to(pt(10, 10));
    let mut closed = Path::new();
    closed.rect(Rect::new(10, 10, 50, 40), 0);
    for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
        let h = l8(60, 50, |pa| {
            pa.vector(&closed, &white(6, LineJoin::Miter, cap, 4));
        });
        // Caps do not apply to closed paths: every cap gives the same image.
        let h0 = l8(60, 50, |pa| {
            pa.vector(&closed, &white(6, LineJoin::Miter, LineCap::Butt, 4));
        });
        assert!(h.data() == h0.data(), "{cap:?}");
        // Full-strength corners and edges, empty inside.
        let px = |x: usize, y: usize| h.data()[y * 60 + x];
        assert_eq!(px(8, 8), 255);
        assert_eq!(px(30, 10), 255);
        assert_eq!(px(30, 25), 0);
        assert_eq!(px(5, 25), 0);
    }
    // The open path leaves a notch at the start/end corner that the closed one does not.
    let o = l8(60, 50, |pa| {
        pa.vector(&open, &white(6, LineJoin::Miter, LineCap::Butt, 4));
    });
    assert_eq!(o.data()[8 * 60 + 8], 0);
}

#[test]
fn zero_length_segment_round_cap_is_dot() {
    let mut p = Path::new();
    p.move_to(pt(20, 20)).line_to(pt(20, 20));
    let round = l8(40, 40, |pa| {
        pa.vector(&p, &white(10, LineJoin::Miter, LineCap::Round, 4));
    });
    let px = |h: &RenderHarness, x: usize, y: usize| h.data()[y * 40 + x];
    assert_eq!(px(&round, 20, 20), 255);
    assert_eq!(px(&round, 16, 20), 255);
    assert_eq!(px(&round, 24, 24), 0); // outside the circle of radius 5
    // Area ≈ π·25 ≈ 78.5 px.
    let sum: u32 = round.data().iter().map(|&v| u32::from(v)).sum();
    assert!((sum / 255).abs_diff(78) <= 2, "{}", sum / 255);
    let square = l8(40, 40, |pa| {
        pa.vector(&p, &white(10, LineJoin::Miter, LineCap::Square, 4));
    });
    assert_eq!(lit(&square), 100);
    let butt = l8(40, 40, |pa| {
        pa.vector(&p, &white(10, LineJoin::Miter, LineCap::Butt, 4));
    });
    assert_eq!(lit(&butt), 0);
}

#[test]
fn stroke_zoom_4x() {
    // A star outline and a curve scaled 4×: joins and segment seams must show no gaps.
    let mut p = Path::new();
    p.move_to(pt(20, 2))
        .line_to(pt(26, 16))
        .line_to(pt(38, 16))
        .line_to(pt(28, 25))
        .line_to(pt(32, 38))
        .line_to(pt(20, 30))
        .line_to(pt(8, 38))
        .line_to(pt(12, 25))
        .line_to(pt(2, 16))
        .line_to(pt(14, 16))
        .close();
    p.move_to(pt(4, 44)).cubic_to(pt(14, 34), pt(26, 54), pt(36, 44));
    let t = Transform::scale(Fx::from_int(4), Fx::from_int(4)).then(Transform::rotate(Angle::deg(3)));
    let mut h = RenderHarness::new(170, 200, ColorFormat::Rgb565);
    h.paint(|pa| {
        for (join, w, c) in [(LineJoin::Round, 3, 0x42A5F5), (LineJoin::Miter, 1, 0x263238)] {
            pa.vector(
                &p,
                &VectorDsc {
                    transform: t,
                    stroke: Some((
                        Paint::Solid(Color::hex(c)),
                        Stroke {
                            width: Fx::from_int(w),
                            join,
                            ..Stroke::default()
                        },
                    )),
                    opa: twine_core::Opa::P70,
                    ..VectorDsc::default()
                },
            );
        }
    });
    assert_render_snapshot!(h, "stroke_zoom_4x");
    // Watertight: in coverage (L8, white on black, 4× without rotation) every pixel on the
    // centerline of every segment and around every vertex is fully covered.
    let pts = [
        (20, 2),
        (26, 16),
        (38, 16),
        (28, 25),
        (32, 38),
        (20, 30),
        (8, 38),
        (12, 25),
        (2, 16),
        (14, 16),
    ];
    let s4 = Transform::scale(Fx::from_int(4), Fx::from_int(4))
        .then(Transform::translate(Fx::from_int(4), Fx::from_int(4)));
    let cov = l8(170, 200, |pa| {
        pa.vector(
            &p,
            &VectorDsc {
                transform: s4,
                ..white(3, LineJoin::Miter, LineCap::Butt, 4)
            },
        );
    });
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        for k in 0..=20 {
            let x = 4 + (a.0 * 4 * (20 - k) + b.0 * 4 * k) / 20;
            let y = 4 + (a.1 * 4 * (20 - k) + b.1 * 4 * k) / 20;
            let v = cov.data()[y as usize * 170 + x as usize];
            assert_eq!(v, 255, "gap at ({x}, {y}) on segment {i}");
        }
    }
}
