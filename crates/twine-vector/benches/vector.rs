//! Criterion benchmarks of vector drawing (320 × 240 RGB565, warm caches) and SVG parsing.
//!
//! Run with `cargo bench -p twine-vector --bench vector` or `cargo xtask bench`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

#[path = "common/scenes.rs"]
mod scenes;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use scenes::{ICON, Target, parse_icon, scenes};
use twine_core::{Color, Fx, Rect};
use twine_render::{FillRule, GradExtend};
use twine_vector::{Dash, FxPoint, LineCap, Paint, Path, Stops, Stroke, VectorDsc, parse_svg};

/// Scenes only the criterion bench runs: an even-odd star with a linear gradient, a dashed
/// curve stroke.
fn extra_scenes() -> Vec<scenes::Scene> {
    let mut star = Path::new();
    for k in 0..10 {
        // A 5-pointed star drawn as a pentagram (self-intersecting: even-odd matters).
        let a = f64::from(k * 144 % 360) * core::f64::consts::PI / 180.0;
        let p = FxPoint::from_int(160 + (100.0 * a.sin()) as i32, 120 - (100.0 * a.cos()) as i32);
        if k == 0 {
            star.move_to(p);
        } else {
            star.line_to(p);
        }
    }
    star.close();
    let mut curve = Path::new();
    curve.move_to(FxPoint::from_int(20, 200));
    curve.cubic_to(
        FxPoint::from_int(80, 0),
        FxPoint::from_int(240, 240),
        FxPoint::from_int(300, 40),
    );
    vec![
        scenes::Scene::new(
            "fill_star_linear_evenodd_200",
            star,
            VectorDsc {
                fill: Some((
                    Paint::Linear {
                        start: FxPoint::from_int(60, 20),
                        end: FxPoint::from_int(260, 220),
                        stops: Stops::Static(&scenes::STOPS),
                        extend: GradExtend::Pad,
                        transform: twine_core::Transform::IDENTITY,
                    },
                    FillRule::EvenOdd,
                )),
                ..VectorDsc::default()
            },
        ),
        scenes::Scene::new(
            "stroke_cubic_dash_w6",
            curve,
            VectorDsc {
                stroke: Some((
                    Paint::Solid(Color::GREEN),
                    Stroke {
                        width: Fx::from_int(6),
                        cap: LineCap::Round,
                        dash: Some(Dash::new(&[Fx::from_int(12), Fx::from_int(6)], Fx::ZERO)),
                        ..Stroke::default()
                    },
                )),
                ..VectorDsc::default()
            },
        ),
    ]
}

fn draw(c: &mut Criterion) {
    let mut t = Target::new();
    for s in scenes().iter().chain(&extra_scenes()) {
        t.draw(s);
        c.bench_function(s.name, |b| b.iter(|| t.draw(s)));
    }
    let doc = parse_svg(ICON).expect("icon");
    let tr = doc.transform_for(Rect::from_xywh(40, 0, 240, 240));
    t.draw_with(|p| doc.scene.draw(p, &tr));
    c.bench_function("svg_icon_draw_240", |b| {
        b.iter(|| t.draw_with(|p| black_box(&doc).scene.draw(p, &tr)));
    });
}

fn parse(c: &mut Criterion) {
    c.bench_function("svg_parse_icon", |b| b.iter(parse_icon));
}

criterion_group!(benches, draw, parse);
criterion_main!(benches);
