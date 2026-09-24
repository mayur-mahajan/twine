//! Paints: gradients, image patterns, opacity and blend modes, in fills and strokes.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Rect, Transform};
use twine_render::{BlendMode, GradExtend, GradStop, ImagePixels};
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_vector::{FillRule, FxPoint, LineJoin, Paint, PainterVectorExt, Path, Stops, Stroke, VectorDsc};

static SUNSET: [GradStop; 3] = [
    GradStop::new(Color::hex(0x3949AB), 0),
    GradStop::new(Color::hex(0xE91E63), 140),
    GradStop::new(Color::hex(0xFFC107), 255),
];

static BW: [GradStop; 2] = [GradStop::new(Color::BLACK, 0), GradStop::new(Color::WHITE, 255)];

fn pt(x: i32, y: i32) -> FxPoint {
    FxPoint::from_int(x, y)
}

fn linear(a: FxPoint, b: FxPoint, stops: &'static [GradStop], extend: GradExtend) -> Paint {
    Paint::Linear {
        start: a,
        end: b,
        stops: Stops::Static(stops),
        extend,
        transform: Transform::IDENTITY,
    }
}

fn radial(
    c: FxPoint,
    r: i32,
    focal: Option<FxPoint>,
    stops: &'static [GradStop],
    extend: GradExtend,
) -> Paint {
    Paint::Radial {
        center: c,
        radius: Fx::from_int(r),
        focal,
        stops: Stops::Static(stops),
        extend,
        transform: Transform::IDENTITY,
    }
}

fn fill(paint: Paint) -> VectorDsc {
    VectorDsc {
        fill: Some((paint, FillRule::NonZero)),
        ..VectorDsc::default()
    }
}

fn rect(r: Rect, radius: i32) -> Path {
    let mut p = Path::new();
    p.rect(r, radius);
    p
}

#[test]
fn linear_gradient_rect() {
    let mut h = RenderHarness::new(120, 70, ColorFormat::Rgb565);
    let p = rect(Rect::new(10, 10, 110, 60), 10);
    h.paint(|pa| pa.vector(&p, &fill(linear(pt(10, 0), pt(110, 0), &SUNSET, GradExtend::Pad))));
    assert_render_snapshot!(h, "linear_gradient_rect");
    // Ends match the stops.
    let rgb = h.rgb888();
    let at = |x: usize, y: usize| rgb[(y * 120 + x) * 3..(y * 120 + x) * 3 + 3].to_vec();
    let c0 = at(12, 35);
    assert!(c0[2] > 150 && c0[0] < 100, "{c0:?}");
    let c1 = at(108, 35);
    assert!(c1[0] > 240 && c1[1] > 170, "{c1:?}");
}

#[test]
fn linear_gradient_transformed_matches_rotated_axis() {
    // A gradient along x, rotated 90° by the draw transform, varies along y on screen.
    let mut h = RenderHarness::new(40, 40, ColorFormat::L8);
    let p = rect(Rect::new(0, 0, 40, 40), 0);
    let dsc = VectorDsc {
        transform: Transform::rotate(Angle::deg(90)).then(Transform::translate(Fx::from_int(40), Fx::ZERO)),
        ..fill(linear(pt(0, 0), pt(40, 0), &BW, GradExtend::Pad))
    };
    h.paint(|pa| pa.vector(&p, &dsc));
    let d = h.data();
    assert_eq!(d[5 * 40 + 3], d[5 * 40 + 30]);
    assert!(d[35 * 40 + 20] > d[5 * 40 + 20] + 150);
}

#[test]
fn radial_gradient_circle() {
    let mut h = RenderHarness::new(100, 100, ColorFormat::Rgb565);
    let mut p = Path::new();
    p.circle(pt(50, 50), Fx::from_int(45));
    h.paint(|pa| pa.vector(&p, &fill(radial(pt(50, 50), 45, None, &SUNSET, GradExtend::Pad))));
    assert_render_snapshot!(h, "radial_gradient_circle");
    // Symmetric around the center (pixel centers 29.5 and 70.5 are 20.5 px away).
    let rgb = h.rgb888();
    let at = |x: usize, y: usize| rgb[(y * 100 + x) * 3..(y * 100 + x) * 3 + 3].to_vec();
    assert_eq!(at(29, 49), at(70, 49));
    assert_eq!(at(49, 29), at(49, 70));
}

#[test]
fn radial_focal() {
    let mut h = RenderHarness::new(100, 100, ColorFormat::Rgb565);
    let mut p = Path::new();
    p.circle(pt(50, 50), Fx::from_int(45));
    h.paint(|pa| {
        pa.vector(
            &p,
            &fill(radial(pt(50, 50), 45, Some(pt(30, 30)), &SUNSET, GradExtend::Pad)),
        );
    });
    assert_render_snapshot!(h, "radial_focal");
    // t = 0 (first stop) at the focal point.
    let rgb = h.rgb888();
    let f = &rgb[(30 * 100 + 30) * 3..(30 * 100 + 30) * 3 + 3];
    assert!(f[2] > 150 && f[0] < 100, "{f:?}");
    // A focal point outside the circle is clamped (no panic, still draws).
    let mut h2 = RenderHarness::new(100, 100, ColorFormat::Rgb565);
    h2.paint(|pa| {
        pa.vector(
            &p,
            &fill(radial(
                pt(50, 50),
                45,
                Some(pt(200, 50)),
                &SUNSET,
                GradExtend::Pad,
            )),
        );
    });
}

#[test]
fn extend_repeat_reflect() {
    let mut h = RenderHarness::new(160, 90, ColorFormat::Rgb565);
    let rows = [GradExtend::Pad, GradExtend::Repeat, GradExtend::Reflect];
    let paths: Vec<Path> = (0..3)
        .map(|i| rect(Rect::new(5, 5 + i * 28, 155, 29 + i * 28), 0))
        .collect();
    h.paint(|pa| {
        for (i, e) in rows.into_iter().enumerate() {
            pa.vector(&paths[i], &fill(linear(pt(55, 0), pt(95, 0), &SUNSET, e)));
        }
    });
    assert_render_snapshot!(h, "extend_repeat_reflect");
    let mut h = RenderHarness::new(160, 60, ColorFormat::Rgb565);
    let mut c = Path::new();
    c.circle(pt(80, 30), Fx::from_int(80));
    h.paint(|pa| {
        pa.vector(
            &c,
            &fill(radial(pt(80, 30), 12, None, &SUNSET, GradExtend::Reflect)),
        );
    });
    assert_render_snapshot!(h, "extend_radial_reflect");
}

/// A 4×4 checkerboard with an alpha hole.
static CHECKER: [u8; 4 * 4 * 4] = {
    let mut d = [0u8; 64];
    let mut i = 0;
    while i < 16 {
        let (x, y) = (i % 4, i / 4);
        let on = (x + y) % 2 == 0;
        let o = i * 4;
        // B, G, R, A
        d[o] = if on { 0x30 } else { 0xF0 };
        d[o + 1] = if on { 0x30 } else { 0xC0 };
        d[o + 2] = if on { 0x30 } else { 0x40 };
        d[o + 3] = if x == 3 && y == 3 { 0 } else { 255 };
        i += 1;
    }
    d
};

#[test]
fn image_pattern_fill() {
    let img = ImagePixels::new(ColorFormat::Argb8888, 4, 4, &CHECKER);
    let mut h = RenderHarness::new(140, 70, ColorFormat::Rgb565);
    let mut c = Path::new();
    c.circle(pt(35, 35), Fx::from_int(30));
    let r = rect(Rect::new(75, 5, 135, 65), 12);
    h.paint(|pa| {
        let tile = |aa, extend| Paint::Image {
            image: img,
            transform: Transform::scale(Fx::from_int(5), Fx::from_int(5))
                .then(Transform::rotate(Angle::deg(20))),
            extend,
            antialias: aa,
        };
        pa.vector(&c, &fill(tile(false, GradExtend::Repeat)));
        pa.vector(&r, &fill(tile(true, GradExtend::Reflect)));
    });
    assert_render_snapshot!(h, "image_pattern_fill");
}

#[test]
fn opa_and_multiply_blend() {
    let mut h = RenderHarness::new(120, 70, ColorFormat::Rgb565);
    let a = rect(Rect::new(10, 10, 70, 60), 8);
    let mut b = Path::new();
    b.circle(pt(75, 35), Fx::from_int(30));
    h.paint(|pa| {
        pa.vector(&a, &fill(Paint::Solid(Color::hex(0x42A5F5))));
        pa.vector(
            &b,
            &VectorDsc {
                fill: Some((
                    linear(pt(45, 0), pt(105, 0), &SUNSET, GradExtend::Pad),
                    FillRule::NonZero,
                )),
                opa: Opa::P80,
                blend_mode: BlendMode::Multiply,
                ..VectorDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "opa_and_multiply_blend");
    // Multiply over white keeps the gradient color, over blue darkens it.
    let rgb = h.rgb888();
    let at = |x: usize, y: usize| rgb[(y * 120 + x) * 3..(y * 120 + x) * 3 + 3].to_vec();
    let over_blue = at(60, 35);
    let over_white = at(95, 35);
    let sum = |c: &[u8]| c.iter().map(|&v| u32::from(v)).sum::<u32>();
    assert!(sum(&over_blue) < sum(&over_white), "{over_blue:?} {over_white:?}");
}

#[test]
fn gradient_strokes() {
    let mut h = RenderHarness::new(120, 80, ColorFormat::Rgb565);
    let mut p = Path::new();
    p.move_to(pt(10, 60))
        .cubic_to(pt(40, -10), pt(80, 130), pt(110, 20));
    let mut ring = Path::new();
    ring.circle(pt(60, 40), Fx::from_int(25));
    h.paint(|pa| {
        pa.vector(
            &p,
            &VectorDsc {
                stroke: Some((
                    linear(pt(10, 0), pt(110, 0), &SUNSET, GradExtend::Pad),
                    Stroke {
                        width: Fx::from_int(8),
                        join: LineJoin::Round,
                        cap: twine_vector::LineCap::Round,
                        ..Stroke::default()
                    },
                )),
                ..VectorDsc::default()
            },
        );
        pa.vector(
            &ring,
            &VectorDsc {
                stroke: Some((
                    radial(pt(60, 40), 30, None, &BW, GradExtend::Reflect),
                    Stroke {
                        width: Fx::from_int(5),
                        ..Stroke::default()
                    },
                )),
                stroke_opa: Opa::P60,
                ..VectorDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "gradient_strokes");
}

#[test]
fn gradient_lut_cached() {
    let mut h = RenderHarness::new(60, 60, ColorFormat::Rgb565);
    let p = rect(Rect::new(5, 5, 55, 55), 0);
    let d = fill(linear(pt(5, 0), pt(55, 0), &SUNSET, GradExtend::Pad));
    h.paint(|pa| pa.vector(&p, &d));
    let s1 = h.caches().stats().gradient;
    h.paint(|pa| pa.vector(&p, &d));
    let s2 = h.caches().stats().gradient;
    assert_eq!(s2.misses, s1.misses, "second draw must not rebuild the LUT");
    assert_eq!(s2.hits, s1.hits + 1);
    // More than 8 stops: built directly (no cache entry), still drawn.
    let many: Vec<GradStop> = (0..12)
        .map(|i| GradStop::new(Color::new(i * 20, 0, 0), i * 23))
        .collect();
    let d = fill(Paint::Linear {
        start: pt(5, 0),
        end: pt(55, 0),
        stops: Stops::Owned(many),
        extend: GradExtend::Pad,
        transform: Transform::IDENTITY,
    });
    h.paint(|pa| pa.vector(&p, &d));
    assert_eq!(h.caches().stats().gradient, s2);
}
