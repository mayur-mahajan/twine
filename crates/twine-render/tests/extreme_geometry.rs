//! Regression tests of R5.S13 (panic-site cleanup of the renderer): geometry at the ends of the
//! `i32` range used to overflow intermediate values (a panic with `overflow-checks`, wrong
//! pixels without). It is now drawn without panicking; where a value had to be bounded the
//! visible result is checked (`docs/safety/profile.md` § 4.1).

use twine_core::{Angle, Color, ColorFormat, Fraction, Opa, Point, Rect};
use twine_render::{
    ArcDsc, DrawBuf, GradKind, GradStop, Gradient, LineDsc, LineSide, Mask, Painter, RADIUS_CIRCLE, RectDsc,
    RenderCaches, ShadowDsc, TriangleDsc, rotate_buffer,
};

const W: i32 = 16;

/// Draws `f` into a black 16 × 16 `L8` buffer and returns its pixels.
fn draw(f: impl FnOnce(&mut Painter<'_>)) -> Vec<u8> {
    let mut caches = RenderCaches::default();
    let mut px = vec![0u8; (W * W) as usize];
    let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, W, W)).unwrap();
    f(&mut Painter::new(buf, &mut caches));
    px
}

const MIN: i32 = i32::MIN;
const MAX: i32 = i32::MAX;

#[test]
fn regression_huge_rounded_rect_radius_is_bounded() {
    // The circle coverage of a radius near 2^30 overflowed `i64` (`(8r)²`). The radius is
    // now bounded (2^22 px): the visible middle of the huge rounded rectangle is still full.
    let px = draw(|p| {
        let area = Rect::new(-(1 << 30), -(1 << 30), 1 << 30, 1 << 30);
        let dsc = RectDsc {
            radius: RADIUS_CIRCLE,
            bg_color: Color::WHITE,
            bg_opa: Opa::COVER,
            border_width: 3,
            border_opa: Opa::COVER,
            outline_width: 4,
            outline_pad: MAX,
            outline_opa: Opa::COVER,
            ..RectDsc::default()
        };
        p.rect(area, &dsc);
    });
    assert!(px.iter().all(|&v| v == 255));
}

#[test]
fn regression_extreme_lines_do_not_overflow() {
    // Horizontal (`|b − a| + 1` in `i32`) and diagonal (`dx² + dy²` in `i64`) lines across the
    // whole coordinate range, round caps, dashes, the widest widths.
    let px = draw(|p| {
        let solid = LineDsc {
            color: Color::WHITE,
            width: 3,
            ..LineDsc::default()
        };
        p.line(Point::new(MIN, 2), Point::new(MAX, 2), &solid);
        p.line(Point::new(MIN, 8), Point::new(MAX, 9), &solid);
        let wide = LineDsc {
            width: MAX,
            round_start: true,
            round_end: true,
            dash_width: MAX,
            dash_gap: MAX,
            ..solid
        };
        p.line(Point::new(MIN, MIN), Point::new(MAX, MAX), &wide);
        p.polyline(
            &[Point::new(MIN, 0), Point::new(MAX, MAX), Point::new(0, MIN)],
            &wide,
        );
        p.line(Point::new(MAX, MAX), Point::new(MAX, MAX), &wide);
    });
    // The horizontal line covers rows 1..=3 everywhere.
    assert!(px[(2 * W) as usize..(3 * W) as usize].iter().all(|&v| v == 255));
}

#[test]
fn regression_extreme_polygons_do_not_overflow() {
    // Edge crossings `(ys − y0)(x1 − x0)` overflowed `i64` for vertices 2^31 apart; the
    // triangle's orientation test overflowed `i32`.
    let px = draw(|p| {
        let pts = [Point::new(MIN, MIN), Point::new(MAX, 0), Point::new(0, MAX)];
        p.triangle(
            pts,
            &TriangleDsc {
                color: Color::WHITE,
                ..TriangleDsc::default()
            },
        );
    });
    // The pixels around the origin are deep inside the triangle.
    assert!(px.iter().all(|&v| v == 255), "{px:?}");
}

#[test]
fn regression_extreme_masks_do_not_overflow() {
    let px = draw(|p| {
        let ids = [
            p.push_mask(Mask::Line {
                p1: Point::new(MIN, 0),
                p2: Point::new(MAX, 1),
                side: LineSide::Bottom,
            }),
            p.push_mask(Mask::Angle {
                center: Point::new(MIN, MIN),
                start: Angle::deg(0),
                end: Angle::deg(90),
            }),
            p.push_mask(Mask::Fade {
                area: Rect::new(MIN, MIN, MAX, MAX),
                y_top: MIN,
                y_bottom: MAX,
                opa_top: Opa::COVER,
                opa_bottom: Opa::COVER,
            }),
            p.push_mask(Mask::Radius {
                area: Rect::new(MIN, MIN, MAX, MAX),
                radius: MAX,
                outer: false,
            }),
            p.push_mask(Mask::Map {
                area: Rect::new(MIN, MIN, MAX, MAX),
                alpha: &[255; 4],
            }),
        ];
        p.fill(Rect::from_xywh(0, 0, W, W), Color::WHITE, Opa::COVER);
        for id in ids {
            p.pop_mask(id);
        }
    });
    // Nothing panicked; the far map mask leaves these pixels transparent.
    assert!(px.iter().all(|&v| v == 0));
}

#[test]
fn regression_extreme_gradients_do_not_overflow() {
    const STOPS: [GradStop; 2] = [
        GradStop::new(Color::BLACK, Fraction::ZERO),
        GradStop::new(Color::WHITE, Fraction::ONE),
    ];
    let kinds = [
        GradKind::Linear {
            start: Point::new(MIN, MIN),
            end: Point::new(MAX, MAX),
        },
        GradKind::Radial {
            center: Point::new(MAX, MAX),
            radius: MAX,
            focal: Point::new(MIN, MIN),
            focal_radius: MAX,
        },
        GradKind::Conical {
            center: Point::new(MIN, MAX),
            start_angle: Angle::deci_deg(MIN),
            end_angle: Angle::deci_deg(MAX),
        },
        GradKind::Hor,
    ];
    for kind in kinds {
        let g = Gradient::new(kind, &STOPS);
        draw(|p| {
            p.fill_gradient(Rect::new(MIN, MIN, MAX, MAX), &g, Opa::COVER);
            p.polygon(
                &[Point::new(MIN, MIN), Point::new(MAX, MIN), Point::new(MAX, MAX)],
                &TriangleDsc {
                    grad: Some(&g),
                    ..TriangleDsc::default()
                },
            );
        });
    }
}

#[test]
fn regression_extreme_arcs_and_shadows_do_not_overflow() {
    draw(|p| {
        let dsc = ArcDsc {
            color: Color::WHITE,
            width: MAX,
            rounded: true,
            ..ArcDsc::default()
        };
        p.arc(Point::new(MAX, MAX), MAX, Angle::deg(10), Angle::deg(100), &dsc);
        p.arc(
            Point::new(8, 8),
            MAX,
            Angle::deci_deg(MIN),
            Angle::deci_deg(MAX),
            &dsc,
        );
        let shadow = RectDsc {
            radius: MAX,
            shadow: ShadowDsc {
                width: MAX,
                ofs_x: MAX,
                ofs_y: MIN,
                spread: MAX,
                color: Color::WHITE,
                opa: Opa::COVER,
            },
            ..RectDsc::default()
        };
        p.rect(Rect::new(MIN, MIN, MAX, MAX), &shadow);
        p.rect(Rect::from_xywh(2, 2, 8, 8), &shadow);
    });
}

#[test]
fn regression_impossible_buffer_sizes_are_refused() {
    // Size computations overflowed `usize` before reaching the size checks.
    let mut dst = [0u8; 4];
    assert!(
        rotate_buffer(
            &[0; 4],
            usize::MAX,
            &mut dst,
            2,
            usize::MAX,
            2,
            twine_core::Rotation::Deg90,
            4
        )
        .is_err()
    );
    assert!(twine_render::convert_l8_to_i1(&[0; 4], &mut dst, usize::MAX, 3).is_err());
}
