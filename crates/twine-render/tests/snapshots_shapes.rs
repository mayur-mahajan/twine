//! Snapshots and tests of lines, arcs, polygons, transformed blits and transformed layers.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use std::cell::Cell;

use proptest::prelude::*;
use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Point, Rect, Scale, Transform};
use twine_render::{
    AccelResult, ArcDsc, BlendMode, BlitDsc, DrawAccel, DrawBuf, FillRule, GradKind, GradStop, Gradient,
    ImagePixels, LayerDsc, LayerTransform, LineDsc, Painter, RectDsc, RenderCaches, RenderConfig, ShadowDsc,
    TriangleDsc, transformed_bounds,
};
use twine_testing::{RenderHarness, assert_render_snapshot};

const BLUE: Color = Color::hex(0x1E88E5);
const RED: Color = Color::hex(0xE53935);
const GREEN: Color = Color::hex(0x43A047);

fn draw(w: u16, h: u16, f: impl FnOnce(&mut Painter<'_>)) -> RenderHarness {
    let mut hh = RenderHarness::new(w, h, ColorFormat::Rgb565);
    hh.paint(f);
    hh
}

fn ld(width: i32, color: Color) -> LineDsc {
    LineDsc {
        width,
        color,
        ..LineDsc::default()
    }
}

// ---------------------------------------------------------------- lines

#[test]
fn line_hor_ver_widths() {
    let h = draw(120, 70, |p| {
        for (i, w) in [1, 2, 5, 10].into_iter().enumerate() {
            let y = 6 + i as i32 * 12;
            p.line(Point::new(4, y), Point::new(60, y), &ld(w, Color::BLACK));
            let x = 72 + i as i32 * 12;
            p.line(Point::new(x, 4), Point::new(x, 64), &ld(w, BLUE));
        }
    });
    assert_render_snapshot!(h, "line_hor_ver_widths");
}

fn deg_end(c: Point, len: i32, deg: i32) -> Point {
    let a = Angle::deg(deg);
    Point::new(
        c.x + len * twine_core::math::cos(a) / 32767,
        c.y + len * twine_core::math::sin(a) / 32767,
    )
}

#[test]
fn line_diagonal_aa() {
    let h = draw(100, 80, |p| {
        for (i, deg) in [10, 30, 45, 60].into_iter().enumerate() {
            let s = Point::new(4 + i as i32 * 4, 76);
            p.line(s, deg_end(s, 70, 360 - deg), &ld(1, Color::BLACK));
            let s = Point::new(36, 4);
            p.line(s, deg_end(s, 60, deg), &ld(3, RED));
        }
    });
    assert_render_snapshot!(h, "line_diagonal_aa");
}

#[test]
fn line_caps() {
    let h = draw(100, 60, |p| {
        let d = ld(12, BLUE);
        p.line(Point::new(15, 15), Point::new(85, 25), &d);
        p.line(
            Point::new(15, 42),
            Point::new(85, 52),
            &LineDsc {
                round_start: true,
                round_end: true,
                ..d
            },
        );
    });
    assert_render_snapshot!(h, "line_caps");
}

#[test]
fn line_dashes() {
    let h = draw(100, 60, |p| {
        let d = LineDsc {
            width: 3,
            dash_width: 6,
            dash_gap: 4,
            color: RED,
            ..LineDsc::default()
        };
        p.line(Point::new(4, 6), Point::new(95, 6), &d);
        p.line(Point::new(4, 12), Point::new(4, 56), &d);
        p.line(Point::new(12, 56), Point::new(95, 14), &d);
    });
    assert_render_snapshot!(h, "line_dashes");
}

#[test]
fn polyline_zigzag_round() {
    let h = draw(100, 50, |p| {
        let pts = [
            Point::new(8, 40),
            Point::new(24, 10),
            Point::new(40, 40),
            Point::new(56, 10),
            Point::new(72, 40),
            Point::new(92, 12),
        ];
        p.polyline(
            &pts,
            &LineDsc {
                width: 6,
                color: GREEN,
                round_start: true,
                round_end: true,
                opa: Opa(200),
                ..LineDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "polyline_zigzag_round");
}

fn lines_scene(p: &mut Painter<'_>) {
    for i in 0..12 {
        let c = Point::new(50, 30);
        p.line(c, deg_end(c, 45, i * 30 + 7), &ld(1 + i % 4, Color::BLACK));
    }
    p.line(
        Point::new(5, 55),
        Point::new(95, 5),
        &LineDsc {
            width: 5,
            dash_width: 7,
            dash_gap: 3,
            round_start: true,
            color: RED,
            ..LineDsc::default()
        },
    );
}

#[test]
fn line_clipped_through_chunks() {
    let full = draw(100, 60, lines_scene);
    let mut ch = RenderHarness::new(100, 60, ColorFormat::Rgb565);
    ch.paint_chunked(3, lines_scene);
    assert!(full.rgb888() == ch.rgb888());
}

struct CountFills(Cell<u32>);
impl DrawAccel for CountFills {
    fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult {
        self.0.set(self.0.get() + 1);
        AccelResult::Unsupported
    }
    fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult {
        AccelResult::Unsupported
    }
    fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult {
        AccelResult::Unsupported
    }
    fn wait(&mut self) {}
}

#[test]
fn hor_line_uses_fill_path() {
    let mut acc = CountFills(Cell::new(0));
    let mut caches = RenderCaches::default();
    let mut data = vec![0u8; 320 * 20];
    {
        let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 320, 20)).unwrap();
        let mut p = Painter::new(buf, &mut caches).with_accel(&mut acc);
        p.line(Point::new(0, 10), Point::new(300, 10), &ld(5, Color::WHITE));
        p.line(Point::new(0, 10), Point::new(300, 12), &ld(5, Color::WHITE)); // diagonal: no fill
    }
    assert_eq!(acc.0.get(), 1);
    // Width 5 centered on y = 10: rows 8..=12.
    assert_eq!(data[7 * 320 + 100], 0);
    assert_eq!(data[8 * 320 + 100], 255);
    assert_eq!(data[12 * 320 + 100], 255);
}

#[test]
fn degenerate_point_line() {
    let mut h = RenderHarness::new(20, 20, ColorFormat::L8);
    h.clear(Color::BLACK);
    h.paint(|p| {
        p.line(Point::new(5, 5), Point::new(5, 5), &ld(3, Color::WHITE));
        p.line(Point::new(1, 1), Point::new(18, 18), &ld(0, Color::WHITE));
    });
    let lit: Vec<(usize, usize)> = (0..400)
        .filter(|&i| h.data()[i] > 0)
        .map(|i| (i % 20, i / 20))
        .collect();
    assert_eq!(lit.len(), 9);
    assert!(
        lit.iter()
            .all(|&(x, y)| (4..=6).contains(&x) && (4..=6).contains(&y))
    );
    // Round caps: a disc.
    h.clear(Color::BLACK);
    h.paint(|p| {
        p.line(
            Point::new(10, 10),
            Point::new(10, 10),
            &LineDsc {
                width: 7,
                round_start: true,
                color: Color::WHITE,
                ..LineDsc::default()
            },
        );
    });
    assert_eq!(h.data()[10 * 20 + 10], 255);
    assert!(
        h.data()[7 * 20 + 7] < 128,
        "corner of the 7×7 box is outside the disc"
    );
}

// ---------------------------------------------------------------- arcs

fn arc(color: Color, width: i32, rounded: bool) -> ArcDsc<'static> {
    ArcDsc {
        color,
        width,
        rounded,
        ..ArcDsc::default()
    }
}

#[test]
fn arc_quarters() {
    let h = draw(90, 90, |p| {
        let c = Point::new(45, 45);
        let cols = [RED, GREEN, BLUE, Color::BLACK];
        for (i, col) in cols.into_iter().enumerate() {
            let s = i as i32 * 90;
            p.arc(c, 40, Angle::deg(s + 5), Angle::deg(s + 85), &arc(col, 10, false));
        }
    });
    assert_render_snapshot!(h, "arc_quarters");
}

#[test]
fn arc_full_ring() {
    let h = draw(80, 80, |p| {
        p.arc(
            Point::new(40, 40),
            36,
            Angle::deg(-10),
            Angle::deg(350),
            &arc(BLUE, 8, false),
        );
    });
    assert_render_snapshot!(h, "arc_full_ring");
}

#[test]
fn arc_rounded_ends_thick() {
    let h = draw(90, 90, |p| {
        p.arc(
            Point::new(45, 45),
            40,
            Angle::deg(135),
            Angle::deg(45),
            &arc(GREEN, 16, true),
        );
    });
    assert_render_snapshot!(h, "arc_rounded_ends_thick");
}

#[test]
fn arc_thin_1px() {
    let h = draw(80, 80, |p| {
        p.arc(
            Point::new(40, 40),
            36,
            Angle::deg(0),
            Angle::deg(270),
            &arc(Color::BLACK, 1, false),
        );
        p.arc(
            Point::new(40, 40),
            20,
            Angle::deg(90),
            Angle::deg(450),
            &arc(RED, 1, false),
        );
    });
    assert_render_snapshot!(h, "arc_thin_1px");
}

#[test]
fn arc_pie_sector() {
    let h = draw(80, 80, |p| {
        p.arc(
            Point::new(40, 40),
            36,
            Angle::deg(-60),
            Angle::deg(30),
            &arc(RED, 36, false),
        );
    });
    assert_render_snapshot!(h, "arc_pie_sector");
}

fn gradient_image(size: u16) -> Vec<u8> {
    let s = i32::from(size);
    (0..s * s)
        .flat_map(|i| {
            let (x, y) = (i % s, i / s);
            let c = Color::lerp(
                Color::lerp(RED, BLUE, (x * 1024 / s) as u16),
                Color::YELLOW,
                (y * 512 / s) as u16,
            );
            [c.b, c.g, c.r, 255]
        })
        .collect()
}

#[test]
fn arc_image_source() {
    let img = gradient_image(72);
    let pix = ImagePixels::new(ColorFormat::Argb8888, 72, 72, &img);
    let h = draw(80, 80, |p| {
        p.arc(
            Point::new(40, 40),
            36,
            Angle::deg(0),
            Angle::deg(300),
            &ArcDsc {
                width: 14,
                image: Some(&pix),
                rounded: true,
                ..ArcDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "arc_image_source");
}

#[test]
fn arc_opa_50_rounded() {
    let h = draw(90, 90, |p| {
        p.fill(Rect::from_xywh(0, 40, 90, 10), Color::BLACK, Opa::COVER);
        p.arc(
            Point::new(45, 45),
            40,
            Angle::deg(180),
            Angle::deg(0),
            &ArcDsc {
                opa: Opa(128),
                ..arc(BLUE, 16, true)
            },
        );
    });
    // No double blending: every arc pixel over white is at most 50 % blue.
    let rgb = h.rgb888();
    let bluest = rgb
        .chunks(3)
        .enumerate()
        .filter(|(i, _)| !(40..50).contains(&(i / 90)))
        .map(|(_, c)| 255 - c[0])
        .max()
        .unwrap();
    assert!(bluest <= 128, "{bluest}");
    assert_render_snapshot!(h, "arc_opa_50_rounded");
}

#[test]
fn zero_length_arc_draws_nothing() {
    let h = draw(40, 40, |p| {
        p.arc(
            Point::new(20, 20),
            15,
            Angle::deg(30),
            Angle::deg(30),
            &arc(RED, 4, true),
        );
        p.arc(
            Point::new(20, 20),
            0,
            Angle::deg(0),
            Angle::deg(90),
            &arc(RED, 4, true),
        );
        p.arc(
            Point::new(20, 20),
            15,
            Angle::deg(0),
            Angle::deg(90),
            &arc(RED, 0, true),
        );
        assert!(p.touched_area().is_none());
    });
    assert!(h.rgb888().iter().all(|&v| v == 255));
}

#[test]
fn angles_wrap_negative_and_over_3600() {
    let a = draw(60, 60, |p| {
        p.arc(
            Point::new(30, 30),
            25,
            Angle::deg(-90),
            Angle::deg(90),
            &arc(RED, 6, false),
        );
    });
    let b = draw(60, 60, |p| {
        p.arc(
            Point::new(30, 30),
            25,
            Angle::deg(270),
            Angle::deg(450),
            &arc(RED, 6, false),
        );
    });
    let c = draw(60, 60, |p| {
        p.arc(
            Point::new(30, 30),
            25,
            Angle::deg(630),
            Angle::deg(810),
            &arc(RED, 6, false),
        );
    });
    assert!(a.rgb888() == b.rgb888());
    assert!(a.rgb888() == c.rgb888());
    let full1 = draw(60, 60, |p| {
        p.arc(
            Point::new(30, 30),
            25,
            Angle::deg(0),
            Angle::deg(360),
            &arc(RED, 6, false),
        );
    });
    let full2 = draw(60, 60, |p| {
        p.arc(
            Point::new(30, 30),
            25,
            Angle::deg(45),
            Angle::deg(500),
            &arc(RED, 6, false),
        );
    });
    assert!(full1.rgb888() == full2.rgb888());
}

// ---------------------------------------------------------------- polygons

fn solid(c: Color) -> TriangleDsc<'static> {
    TriangleDsc {
        color: c,
        ..TriangleDsc::default()
    }
}

#[test]
fn triangle_orientations() {
    let pts = [Point::new(5, 5), Point::new(70, 20), Point::new(20, 55)];
    let cw = draw(80, 60, |p| p.triangle(pts, &solid(BLUE)));
    let ccw = draw(80, 60, |p| p.triangle([pts[2], pts[1], pts[0]], &solid(BLUE)));
    assert!(cw.rgb888() == ccw.rgb888());
    assert_render_snapshot!(cw, "triangle_orientations");
}

#[test]
fn triangle_thin_sliver_aa() {
    let h = draw(100, 30, |p| {
        p.triangle(
            [Point::new(2, 5), Point::new(98, 10), Point::new(2, 7)],
            &solid(Color::BLACK),
        );
        p.triangle(
            [Point::new(2, 20), Point::new(98, 28), Point::new(98, 26)],
            &solid(RED),
        );
    });
    assert_render_snapshot!(h, "triangle_thin_sliver_aa");
}

fn pentagram(c: Point, r: i32) -> Vec<Point> {
    (0..5).map(|i| deg_end(c, r, -90 + i * 144)).collect()
}

#[test]
fn polygon_star_nonzero_vs_evenodd() {
    let h = draw(130, 64, |p| {
        p.polygon_with_rule(
            &pentagram(Point::new(32, 32), 30),
            FillRule::NonZero,
            &solid(BLUE),
        );
        p.polygon_with_rule(&pentagram(Point::new(96, 32), 30), FillRule::EvenOdd, &solid(RED));
    });
    assert_render_snapshot!(h, "polygon_star_nonzero_vs_evenodd");
}

fn concave() -> [Point; 8] {
    [
        Point::new(5, 5),
        Point::new(75, 5),
        Point::new(75, 20),
        Point::new(25, 20),
        Point::new(40, 35),
        Point::new(75, 40),
        Point::new(60, 55),
        Point::new(5, 55),
    ]
}

#[test]
fn polygon_concave() {
    let h = draw(80, 60, |p| p.polygon(&concave(), &solid(GREEN)));
    assert_render_snapshot!(h, "polygon_concave");
}

#[test]
fn polygon_gradient_fill() {
    let g = Gradient::new(
        GradKind::Ver,
        &[GradStop::new(Color::YELLOW, 0), GradStop::new(RED, 255)],
    );
    let h = draw(80, 60, |p| {
        p.polygon(
            &concave(),
            &TriangleDsc {
                grad: Some(&g),
                opa: Opa(230),
                ..TriangleDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "polygon_gradient_fill");
}

#[test]
fn polygon_clipped_chunks_identical() {
    let scene = |p: &mut Painter<'_>| {
        p.polygon(&concave(), &solid(GREEN));
        p.polygon_with_rule(
            &pentagram(Point::new(40, 30), 28),
            FillRule::EvenOdd,
            &solid(BLUE),
        );
    };
    let full = draw(80, 60, scene);
    let mut ch = RenderHarness::new(80, 60, ColorFormat::Rgb565);
    ch.paint_chunked(5, scene);
    assert!(full.rgb888() == ch.rgb888());
}

#[test]
fn collinear_triangle_draws_nothing() {
    let h = draw(40, 40, |p| {
        p.triangle(
            [Point::new(1, 1), Point::new(10, 10), Point::new(30, 30)],
            &solid(RED),
        );
        p.polygon(&[Point::new(1, 1), Point::new(30, 1)], &solid(RED));
        assert!(p.touched_area().is_none());
    });
    assert!(h.rgb888().iter().all(|&v| v == 255));
}

#[test]
fn too_many_points_truncates_and_warns() {
    let pts: Vec<Point> = (0..100)
        .map(|i| deg_end(Point::new(50, 50), 40, i * 360 / 100))
        .collect();
    let a = draw(100, 100, |p| p.polygon(&pts, &solid(BLUE)));
    let b = draw(100, 100, |p| {
        p.polygon(&pts[..twine_render::MAX_POLYGON_POINTS], &solid(BLUE));
    });
    assert!(a.rgb888() == b.rgb888());
}

proptest! {
    #[test]
    fn polygon_coverage_within_bbox(pts in proptest::collection::vec((-10i32..70, -10i32..70), 3..10)) {
        let pts: Vec<Point> = pts.into_iter().map(|(x, y)| Point::new(x, y)).collect();
        let bb = pts.iter().fold(Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN), |r, p| {
            Rect::new(r.x0.min(p.x), r.y0.min(p.y), r.x1.max(p.x), r.y1.max(p.y))
        });
        let mut h = RenderHarness::new(64, 64, ColorFormat::L8);
        h.clear(Color::BLACK);
        h.paint(|p| p.polygon_with_rule(&pts, FillRule::EvenOdd, &solid(Color::WHITE)));
        for (i, &v) in h.data().iter().enumerate() {
            let (x, y) = ((i % 64) as i32, (i / 64) as i32);
            if !bb.contains(Point::new(x, y)) {
                prop_assert_eq!(v, 0, "pixel ({}, {}) outside {}", x, y, bb);
            }
        }
    }
}

// ---------------------------------------------------------------- blits

fn test_image() -> Vec<u8> {
    let mut v = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32i32 {
        for x in 0..32i32 {
            let mut c = if (x / 8 + y / 8) % 2 == 0 {
                Color::WHITE
            } else {
                Color::hex(0x90CAF9)
            };
            if x == 0 || y == 0 || x == 31 || y == 31 {
                c = Color::hex(0x0D47A1);
            }
            if ((6..20).contains(&x) && (14..18).contains(&y))
                || ((20..28).contains(&x) && (y - 16).abs() <= 27 - x)
            {
                c = RED;
            }
            v.extend_from_slice(&[c.b, c.g, c.r, 255]);
        }
    }
    v
}

fn blit(t: Transform, aa: bool) -> RenderHarness {
    let img = test_image();
    let pix = ImagePixels::new(ColorFormat::Argb8888, 32, 32, &img);
    draw(80, 80, |p| {
        p.blit_transformed(
            &pix,
            &BlitDsc {
                transform: t,
                antialias: aa,
                ..BlitDsc::default()
            },
        );
    })
}

fn around_center(t: Transform) -> Transform {
    // Image center (16, 16) to screen center (40, 40).
    Transform::translate(Fx::from_int(-16), Fx::from_int(-16))
        .then(t)
        .then(Transform::translate(Fx::from_int(40), Fx::from_int(40)))
}

#[test]
fn blit_identity() {
    let h = blit(Transform::translate(Fx::from_int(24), Fx::from_int(24)), true);
    assert_render_snapshot!(h, "blit_identity");
}

#[test]
fn identity_equals_untransformed_blit() {
    let img = test_image();
    let pix = ImagePixels::new(ColorFormat::Argb8888, 32, 32, &img);
    let a = draw(80, 80, |p| {
        p.blit(Point::new(24, 24), &pix, Opa::COVER, BlendMode::Normal);
    });
    for aa in [false, true] {
        let b = blit(Transform::translate(Fx::from_int(24), Fx::from_int(24)), aa);
        assert!(a.rgb888() == b.rgb888());
        // A near-identity (non translation-only) transform takes the transformed path.
        let c = blit(
            Transform {
                a: Fx(65536),
                d: Fx(65536),
                b: Fx(0),
                c: Fx(0),
                tx: Fx::from_int(24),
                ty: Fx::from_int(24),
            }
            .then(Transform::scale(Fx::ONE, Fx::ONE)),
            aa,
        );
        assert!(a.rgb888() == c.rgb888());
    }
}

#[test]
fn blit_rotate_30_aa() {
    assert_render_snapshot!(
        blit(around_center(Transform::rotate(Angle::deg(30))), true),
        "blit_rotate_30_aa"
    );
    assert_render_snapshot!(
        blit(around_center(Transform::rotate(Angle::deg(30))), false),
        "blit_rotate_30_nearest"
    );
}

#[test]
fn blit_rotate_90_exact() {
    let img = test_image();
    let h = blit(around_center(Transform::rotate(Angle::deg(90))), false);
    // Pixel (x, y) of the image lands at (39 − (y − 16) ... ): check every pixel against the
    // pixel-rotated source.
    let rgb = h.rgb888();
    for y in 0..32usize {
        for x in 0..32usize {
            // 90° clockwise around (16, 16): (x, y) → (16 − (y − 16), 16 + (x − 16)) + 24.
            let (dx, dy) = (24 + 31 - y, 24 + x);
            let s = &img[(y * 32 + x) * 4..(y * 32 + x) * 4 + 3];
            let d = &rgb[(dy * 80 + dx) * 3..(dy * 80 + dx) * 3 + 3];
            let q = Color::from_rgb565(Color::new(s[2], s[1], s[0]).to_rgb565());
            assert_eq!(d, &[q.r, q.g, q.b], "src ({x},{y}) → ({dx},{dy})");
        }
    }
    assert_render_snapshot!(h, "blit_rotate_90_exact");
}

#[test]
fn blit_scale_2x_bilinear() {
    let s = Transform::scale(Fx::from_int(2), Fx::from_int(2));
    assert_render_snapshot!(blit(around_center(s), true), "blit_scale_2x_bilinear");
}

#[test]
fn blit_scale_half() {
    let s = Transform::scale(Fx::HALF, Fx::HALF);
    assert_render_snapshot!(blit(around_center(s), true), "blit_scale_half");
}

#[test]
fn blit_skew_x() {
    assert_render_snapshot!(
        blit(around_center(Transform::skew(Angle::deg(25), Angle(0))), true),
        "blit_skew_x"
    );
}

#[test]
fn blit_recolor() {
    let img = test_image();
    let pix = ImagePixels::new(ColorFormat::Argb8888, 32, 32, &img);
    let h = draw(80, 40, |p| {
        p.blit_transformed(
            &pix,
            &BlitDsc {
                transform: Transform::translate(Fx::from_int(4), Fx::from_int(4)),
                recolor: Some((GREEN, Opa(160))),
                ..BlitDsc::default()
            },
        );
        p.blit_transformed(
            &pix,
            &BlitDsc {
                transform: around_center(Transform::rotate(Angle::deg(15)))
                    .then(Transform::translate(Fx::from_int(14), Fx::from_int(-20))),
                recolor: Some((RED, Opa(100))),
                opa: Opa(200),
                ..BlitDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "blit_recolor");
}

#[test]
fn singular_transform_draws_nothing() {
    let h = blit(Transform::scale(Fx::ZERO, Fx::ONE), true);
    assert!(h.rgb888().iter().all(|&v| v == 255));
}

// ---------------------------------------------------------------- transformed layers

fn card(p: &mut Painter<'_>, a: Rect) {
    p.rect(
        a.expand(-8),
        &RectDsc {
            radius: 8,
            bg_color: Color::WHITE,
            bg_opa: Opa::COVER,
            border_width: 2,
            border_color: BLUE,
            border_opa: Opa::COVER,
            shadow: ShadowDsc {
                width: 8,
                opa: Opa(140),
                ..ShadowDsc::default()
            },
            ..RectDsc::default()
        },
    );
    p.fill(Rect::from_xywh(a.x0 + 16, a.y0 + 16, 24, 6), RED, Opa::COVER);
}

fn layer_t(t: LayerTransform, opa: Opa) -> RenderHarness {
    let area = Rect::from_xywh(20, 20, 60, 50);
    draw(100, 90, |p| {
        p.fill(Rect::from_xywh(0, 40, 100, 10), Color::hex(0x999999), Opa::COVER);
        p.layer(
            area,
            &LayerDsc {
                opa,
                blend_mode: BlendMode::Normal,
                transform: Some(t),
            },
            |p| card(p, area),
        );
    })
}

fn lt() -> LayerTransform {
    LayerTransform {
        pivot: Point::new(30, 25),
        ..LayerTransform::default()
    }
}

#[test]
fn layer_rotate_45_card() {
    assert_render_snapshot!(
        layer_t(
            LayerTransform {
                rotation: Angle::deg(45),
                ..lt()
            },
            Opa::COVER
        ),
        "layer_rotate_45_card"
    );
}

#[test]
fn layer_scale_150() {
    let s = Scale::from_percent(150);
    assert_render_snapshot!(
        layer_t(
            LayerTransform {
                scale_x: s,
                scale_y: s,
                ..lt()
            },
            Opa::COVER
        ),
        "layer_scale_150"
    );
}

#[test]
fn layer_skew() {
    assert_render_snapshot!(
        layer_t(
            LayerTransform {
                skew_x: Angle::deg(20),
                ..lt()
            },
            Opa::COVER
        ),
        "layer_skew"
    );
}

#[test]
fn layer_transform_opa() {
    assert_render_snapshot!(
        layer_t(
            LayerTransform {
                rotation: Angle::deg(-20),
                ..lt()
            },
            Opa(128)
        ),
        "layer_transform_opa"
    );
}

#[test]
fn layer_transform_too_big_falls_back() {
    let area = Rect::from_xywh(20, 20, 60, 50);
    let small = RenderConfig {
        layer_buf_bytes: 60 * 4 * 10,
        ..RenderConfig::default()
    };
    let mut h = RenderHarness::new(100, 90, ColorFormat::Rgb565).with_config(small);
    h.paint(|p| {
        p.layer(
            area,
            &LayerDsc {
                transform: Some(LayerTransform {
                    rotation: Angle::deg(45),
                    ..lt()
                }),
                ..LayerDsc::default()
            },
            |p| card(p, area),
        );
    });
    let mut plain = RenderHarness::new(100, 90, ColorFormat::Rgb565).with_config(small);
    plain.paint(|p| p.layer(area, &LayerDsc::default(), |p| card(p, area)));
    assert!(h.rgb888() == plain.rgb888(), "drawn untransformed");
}

proptest! {
    #[test]
    fn transformed_bounds_contains_rotated_corners(
        deg in -720i32..720, sx in 64u16..1024, sy in 64u16..1024, px in -20i32..80, py in -20i32..80,
    ) {
        let area = Rect::from_xywh(10, 20, 60, 40);
        let t = LayerTransform {
            rotation: Angle::deg(deg),
            scale_x: Scale(sx),
            scale_y: Scale(sy),
            pivot: Point::new(px, py),
            ..LayerTransform::default()
        };
        let b = transformed_bounds(area, &t);
        let m = t.to_transform(area);
        for c in area.corners() {
            let q = m.map_point(c);
            prop_assert!(b.expand(1).contains(q), "{} not in {}", q, b);
        }
    }
}
