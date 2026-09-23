//! No allocation while drawing lines, arcs, polygons, blits and transformed layers.

use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Point, Rect, Transform, XorShift32};
use twine_render::{
    ArcDsc, BlitDsc, ImagePixels, LayerDsc, LayerTransform, LineDsc, Painter, RectDsc, TriangleDsc,
};
use twine_testing::RenderHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

fn assert_no_alloc(name: &str, mut f: impl FnMut(&mut Painter<'_>)) {
    let mut h = RenderHarness::new(200, 150, ColorFormat::Rgb565);
    h.paint(&mut f);
    let ((), stats) = count_allocs(|| h.paint(&mut f));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{name}: {stats:?}");
}

#[test]
fn lines_do_not_allocate() {
    assert_no_alloc("lines", |p| {
        let mut rng = XorShift32::new(3);
        for _ in 0..200 {
            let a = Point::new(rng.range(-20, 220), rng.range(-20, 170));
            let b = Point::new(rng.range(-20, 220), rng.range(-20, 170));
            p.line(
                a,
                b,
                &LineDsc {
                    width: rng.range(1, 12),
                    round_start: rng.next_bool(),
                    round_end: rng.next_bool(),
                    dash_width: rng.range(0, 6),
                    dash_gap: rng.range(0, 4),
                    color: Color::BLUE,
                    ..LineDsc::default()
                },
            );
        }
        p.polyline(
            &[Point::new(5, 5), Point::new(50, 60), Point::new(90, 10)],
            &LineDsc {
                width: 5,
                round_end: true,
                ..LineDsc::default()
            },
        );
    });
}

#[test]
fn arcs_do_not_allocate() {
    let img = vec![200u8; 60 * 60 * 4];
    assert_no_alloc("arcs", |p| {
        for i in 0..20 {
            p.arc(
                Point::new(100, 75),
                30,
                Angle::deg(i * 18),
                Angle::deg(i * 18 + 70),
                &ArcDsc {
                    width: 6,
                    rounded: true,
                    ..ArcDsc::default()
                },
            );
        }
        let pix = ImagePixels::new(ColorFormat::Argb8888, 60, 60, &img);
        p.arc(
            Point::new(50, 50),
            30,
            Angle::deg(0),
            Angle::deg(200),
            &ArcDsc {
                width: 8,
                image: Some(&pix),
                ..ArcDsc::default()
            },
        );
    });
}

#[test]
fn polygons_do_not_allocate() {
    let star: Vec<Point> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { 60 } else { 25 };
            let a = Angle::deg(i * 36);
            Point::new(
                100 + r * twine_core::math::cos(a) / 32767,
                75 + r * twine_core::math::sin(a) / 32767,
            )
        })
        .collect();
    assert_no_alloc("polygons", |p| {
        for _ in 0..10 {
            p.polygon(&star, &TriangleDsc::default());
            p.triangle(
                [Point::new(0, 0), Point::new(150, 20), Point::new(30, 140)],
                &TriangleDsc {
                    opa: Opa(100),
                    ..TriangleDsc::default()
                },
            );
        }
    });
}

#[test]
fn blits_and_transformed_layers_do_not_allocate() {
    let img = vec![90u8; 32 * 32 * 4];
    assert_no_alloc("blits", |p| {
        let pix = ImagePixels::new(ColorFormat::Argb8888, 32, 32, &img);
        p.blit(
            Point::new(3, 4),
            &pix,
            Opa::COVER,
            twine_render::BlendMode::Normal,
        );
        for i in 0..10 {
            p.blit_transformed(
                &pix,
                &BlitDsc {
                    transform: Transform::rotate(Angle::deg(i * 17))
                        .then(Transform::translate(Fx::from_int(80), Fx::from_int(60))),
                    ..BlitDsc::default()
                },
            );
        }
        let area = Rect::from_xywh(40, 40, 60, 50);
        p.layer(
            area,
            &LayerDsc {
                transform: Some(LayerTransform {
                    rotation: Angle::deg(30),
                    pivot: Point::new(30, 25),
                    ..LayerTransform::default()
                }),
                ..LayerDsc::default()
            },
            |p| {
                p.rect(
                    area,
                    &RectDsc {
                        radius: 8,
                        bg_opa: Opa::COVER,
                        ..RectDsc::default()
                    },
                );
            },
        );
    });
}
