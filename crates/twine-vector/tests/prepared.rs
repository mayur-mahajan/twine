//! Prepared geometry of scene items: drawing a [`VectorScene`] (which keeps each item's
//! flattened fill and flattened, dashed, indexed stroke between draws) gives the same pixels as
//! drawing every path directly, in any clip, after transform changes and after edits; a warm
//! scene draws without allocating.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use proptest::prelude::*;
use twine_core::{Angle, Color, ColorFormat, Fraction, Fx, Opa, Point, Rect, Transform};
use twine_render::{FillRule, GradExtend, GradStop};
use twine_testing::RenderHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_vector::{
    Dash, DrawParams, FxPoint, LineCap, LineJoin, Paint, PainterVectorExt, Path, Stops, Stroke, VectorDsc,
    VectorScene,
};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

const W: i32 = 160;
const H: i32 = 120;

static STOPS: [GradStop; 2] = [
    GradStop {
        color: Color::hex(0x1E88E5),
        opa: Opa::COVER,
        frac: Fraction::from_raw(0),
    },
    GradStop {
        color: Color::hex(0xE53935),
        opa: Opa::COVER,
        frac: Fraction::from_raw(255),
    },
];

/// A polyline of `n` points zig-zagging across the canvas (long enough for several chunks).
fn wiggle(n: i32, seed: i32, closed: bool) -> Path {
    let mut p = Path::new();
    for i in 0..n {
        let x = 4 + i * (W - 8) / n.max(1);
        let y = 10 + ((i * 37 + seed * 13) % 97);
        let q = FxPoint::from_int(x, y);
        if i == 0 {
            p.move_to(q);
        } else {
            p.line_to(q);
        }
    }
    if closed {
        p.close();
    }
    p
}

/// A scene with a gradient disc, a solid fill, a long polyline, dashed rings and a short
/// closed stroke.
fn scene(seed: i32, dash: (i32, i32), width_tenths: i32, join: LineJoin, cap: LineCap) -> VectorScene {
    let mut s = VectorScene::new();
    let c = FxPoint::from_int(80, 60);
    let mut disc = Path::new();
    disc.circle(c, Fx::from_int(55));
    s.add(
        disc,
        VectorDsc {
            fill: Some((
                Paint::Radial {
                    center: c,
                    radius: Fx::from_int(55),
                    focal: None,
                    stops: Stops::Static(&STOPS),
                    extend: GradExtend::Pad,
                    transform: Transform::IDENTITY,
                },
                FillRule::NonZero,
            )),
            ..VectorDsc::default()
        },
    );
    let mut blob = Path::new();
    blob.circle(FxPoint::from_int(30 + seed % 40, 90), Fx::from_int(14));
    s.add(blob, VectorDsc::fill(Color::hex(0x43A047)));
    let stroke = |w: i32, dash: Option<Dash>| VectorDsc {
        stroke: Some((
            Paint::Solid(Color::hex(0xFFFFFF)),
            Stroke {
                width: Fx::from_ratio(w, 10),
                join,
                cap,
                dash,
                ..Stroke::default()
            },
        )),
        stroke_opa: Opa::P80,
        ..VectorDsc::default()
    };
    s.add(
        wiggle(150 + seed % 50, seed, seed % 2 == 0),
        stroke(width_tenths, None),
    );
    let mut rings = Path::new();
    for r in [12, 25, 38, 51] {
        rings.circle(c, Fx::from_int(r));
    }
    let d = Dash::new(
        &[Fx::from_int(dash.0), Fx::from_int(dash.1)],
        Fx::from_int(seed % 5),
    );
    s.add(rings, stroke(10, Some(d)));
    s.add(wiggle(12, seed + 3, true), stroke(width_tenths + 20, None));
    s
}

/// Every item drawn directly (no prepared geometry) with `t` after its own transform.
fn reference(s: &VectorScene, t: Transform) -> RenderHarness {
    let mut h = RenderHarness::new(W as u16, H as u16, ColorFormat::Rgb565);
    h.paint(|pa| {
        for (path, dsc) in s.items() {
            pa.vector_params(path, dsc, &DrawParams::transform(t));
        }
    });
    h
}

/// The scene drawn through its prepared geometry, tile by tile.
fn tiled(s: &VectorScene, t: Transform, tile: i32) -> RenderHarness {
    let mut h = RenderHarness::new(W as u16, H as u16, ColorFormat::Rgb565);
    for y in (0..H).step_by(tile as usize) {
        for x in (0..W).step_by(tile as usize) {
            let r = Rect::new(x, y, (x + tile).min(W), (y + tile).min(H));
            h.paint(|pa| pa.with_clip(r, |pa| s.draw(pa, &t)));
        }
    }
    h
}

#[test]
fn scene_draws_match_direct_draws_across_clips_transforms_and_edits() {
    let mut s = scene(7, (3, 5), 15, LineJoin::Round, LineCap::Round);
    let moved = Transform::translate(Fx::from_int(6), Fx::from_int(-4));
    let zoomed = Transform::scale(Fx::from_ratio(3, 2), Fx::from_ratio(3, 2))
        .then(Transform::translate(Fx::from_int(-30), Fx::from_int(-20)));
    // The same scene drawn again (prepared geometry reused), moved (fills re-flattened,
    // strokes reused) and zoomed (both rebuilt), then back.
    for (t, tile) in [
        (Transform::IDENTITY, 9),
        (Transform::IDENTITY, 32),
        (moved, 17),
        (zoomed, 13),
        (Transform::IDENTITY, 5),
    ] {
        assert!(
            tiled(&s, t, tile).data() == reference(&s, t).data(),
            "{t:?} in tiles of {tile}"
        );
    }
    assert!(s.prepared_bytes() > 0, "the items keep prepared geometry");
    // Editing an item replaces its prepared geometry.
    if let Some((p, d)) = s.path_mut() {
        *p = wiggle(80, 11, false);
        *d = VectorDsc {
            stroke: Some((
                Paint::Solid(Color::hex(0xFDD835)),
                Stroke {
                    width: Fx::from_int(3),
                    ..Stroke::default()
                },
            )),
            ..VectorDsc::default()
        };
    }
    assert!(
        tiled(&s, Transform::IDENTITY, 11).data() == reference(&s, Transform::IDENTITY).data(),
        "after an edit"
    );
    s.clear();
    let mut q = Path::new();
    q.circle(FxPoint::from_int(40, 40), Fx::from_int(20));
    s.add(q, VectorDsc::fill(Color::hex(0x8E24AA)));
    assert!(
        tiled(&s, Transform::IDENTITY, 8).data() == reference(&s, Transform::IDENTITY).data(),
        "after a rebuild"
    );
}

#[test]
fn warm_scene_draws_without_allocating() {
    let s = scene(4, (4, 4), 20, LineJoin::Miter, LineCap::Butt);
    let mut h = RenderHarness::new(W as u16, H as u16, ColorFormat::Rgb565);
    let tiles: Vec<Rect> = (0..12)
        .map(|i| Rect::new((i % 4) * 40, (i / 4) * 40, (i % 4) * 40 + 40, (i / 4) * 40 + 40))
        .collect();
    let draw = |h: &mut RenderHarness| {
        for &r in &tiles {
            h.paint(|pa| pa.with_clip(r, |pa| s.draw(pa, &Transform::IDENTITY)));
        }
    };
    draw(&mut h);
    let ((), stats) = count_allocs(|| draw(&mut h));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn prepared_scenes_match_direct_draws(
        seed in 0i32..1000,
        on in 1i32..12, off in 1i32..12,
        width in 5i32..80,
        join in 0usize..3, cap in 0usize..3,
        angle in -180i32..180, scale in 60i32..180,
        tile in 4i32..60,
    ) {
        let join = [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel][join];
        let cap = [LineCap::Butt, LineCap::Round, LineCap::Square][cap];
        let s = scene(seed, (on, off), width, join, cap);
        let t = Transform::rotate(Angle::deg(angle))
            .around(Point::new(80, 60))
            .then(Transform::scale(Fx::from_ratio(scale, 100), Fx::from_ratio(scale, 100)));
        prop_assert!(tiled(&s, t, tile).data() == reference(&s, t).data());
        // Drawn again from the prepared geometry.
        prop_assert!(tiled(&s, t, tile.max(7) - 3).data() == reference(&s, t).data());
    }
}
