//! Vector graphics (the scenes of LVGL's `lv_demo_vector_graphic`), one tab per topic:
//!
//! - **Shapes**: rectangles, rounded rectangles, circles, ellipses, curves (quadratic, cubic,
//!   arcs) and the two fill rules (a pentagram filled non-zero and even-odd).
//! - **Gradients**: linear (two and three stops), radial, radial with a focal point, and the
//!   pad / repeat / reflect spreads.
//! - **Strokes**: miter, round and bevel joins; butt, round and square caps; dashes and dots.
//! - **SVG & motion**: four SVG icons (`ImageSource::Svg`, drawn as vectors at 1× and 2×) and
//!   a rotating, pulsing star animated from a signal: only its bounds are redrawn, and a frame
//!   allocates nothing.
//!
//! Everything is integer geometry (16.16 fixed point), rasterized with anti-aliasing through
//! `twine-vector`; scenes are rebuilt only when a signal they read changes.

use twine::core::{Fx, Transform};
use twine::prelude::*;
use twine::render::{FillRule, GradExtend, GradStop};
use twine::vector::{
    Dash, FxPoint, FxRect, LineCap, LineJoin, Paint, Path, Stops, Stroke, VectorDsc, VectorScene,
};

/// The SVG icons of the last tab.
pub static ICONS: [&[u8]; 4] = [
    include_bytes!("../../../assets/images/svg/home.svg"),
    include_bytes!("../../../assets/images/svg/star.svg"),
    include_bytes!("../../../assets/images/svg/badge.svg"),
    include_bytes!("../../../assets/images/svg/chart.svg"),
];

const fn stop(rgb: u32, frac: u8) -> GradStop {
    GradStop {
        color: Color::hex(rgb),
        opa: Opa::COVER,
        frac,
    }
}

static SUNSET: [GradStop; 3] = [
    stop(0x00FF_D54F, 0),
    stop(0x00FF_7043, 128),
    stop(0x008E_24AA, 255),
];
static OCEAN: [GradStop; 2] = [stop(0x0080_DEEA, 0), stop(0x0001_579B, 255)];
static GLOW: [GradStop; 2] = [stop(0x00FF_FFFF, 0), stop(0x0043_A047, 255)];
static STRIPES: [GradStop; 2] = [stop(0x00EC_407A, 0), stop(0x0029_B6F6, 255)];

fn p(x: i32, y: i32) -> FxPoint {
    FxPoint::from_int(x, y)
}

fn fx(v: i32) -> Fx {
    Fx::from_int(v)
}

fn fill(color: u32) -> VectorDsc {
    VectorDsc::fill(Color::hex(color))
}

fn stroked(color: u32, width: i32, join: LineJoin, cap: LineCap, dash: Option<Dash>) -> VectorDsc {
    VectorDsc {
        stroke: Some((
            Paint::Solid(Color::hex(color)),
            Stroke {
                width: fx(width),
                join,
                cap,
                dash,
                ..Stroke::default()
            },
        )),
        ..VectorDsc::default()
    }
}

/// A five-pointed star (outer radius `r`), as a path of ten points (`pentagram`: the five
/// points joined crosswise, which fills differently with the two fill rules).
fn star(path: &mut Path, c: FxPoint, r: i32, pentagram: bool) {
    let n = if pentagram { 5 } else { 10 };
    for k in 0..n {
        let (radius, step) = if pentagram {
            (r, 1440)
        } else if k % 2 == 0 {
            (r, 360)
        } else {
            (r * 2 / 5, 360)
        };
        let (x, y) = Transform::rotate(Angle(k * step)).map(Fx::ZERO, fx(-radius));
        let q = FxPoint::new(c.x + x, c.y + y);
        if k == 0 {
            path.move_to(q);
        } else {
            path.line_to(q);
        }
    }
    path.close();
}

fn shapes(scene: &mut VectorScene) {
    let (path, dsc) = scene.path_mut();
    path.rect(Rect::from_xywh(10, 10, 80, 60), 0);
    *dsc = fill(0x0042_A5F5);
    let (path, dsc) = scene.path_mut();
    path.rounded_rect(FxRect::from_xywh(fx(100), fx(10), fx(80), fx(60)), fx(16), fx(16));
    *dsc = fill(0x0066_BB6A);
    let (path, dsc) = scene.path_mut();
    path.circle(p(230, 40), fx(30));
    *dsc = fill(0x00FF_A726);
    let (path, dsc) = scene.path_mut();
    path.ellipse(p(330, 40), fx(50), fx(25));
    *dsc = fill(0x00AB_47BC);
    // Curves: a quadratic wave, a cubic S and an elliptical arc.
    let (path, dsc) = scene.path_mut();
    path.move_to(p(10, 150));
    path.quad_to(p(40, 90), p(70, 150));
    path.quad_to(p(100, 210), p(130, 150));
    path.line_to(p(130, 190));
    path.line_to(p(10, 190));
    path.close();
    *dsc = fill(0x0026_A69A);
    let (path, dsc) = scene.path_mut();
    path.move_to(p(150, 190));
    path.cubic_to(p(150, 90), p(230, 190), p(230, 100));
    path.line_to(p(250, 100));
    path.cubic_to(p(250, 200), p(170, 110), p(170, 190));
    path.close();
    *dsc = fill(0x00EF_5350);
    let (path, dsc) = scene.path_mut();
    path.move_to(p(270, 150));
    path.arc_to(p(30, 20), Angle(300), true, true, p(320, 160));
    path.line_to(p(295, 190));
    path.close();
    *dsc = fill(0x005C_6BC0);
    // Fill rules: the pentagram's center is inside for non-zero, outside for even-odd.
    for (cx, rule) in [(360, FillRule::NonZero), (420, FillRule::EvenOdd)] {
        let (path, dsc) = scene.path_mut();
        star(path, p(cx, 150), 30, true);
        *dsc = VectorDsc {
            fill: Some((Paint::Solid(Color::hex(0x008D_6E63)), rule)),
            ..VectorDsc::default()
        };
    }
}

fn gradients(scene: &mut VectorScene) {
    let linear = |start, end, stops: &'static [GradStop], extend| Paint::Linear {
        start,
        end,
        stops: Stops::Static(stops),
        extend,
        transform: Transform::IDENTITY,
    };
    let radial = |center, radius, focal, stops: &'static [GradStop], extend| Paint::Radial {
        center,
        radius,
        focal,
        stops: Stops::Static(stops),
        extend,
        transform: Transform::IDENTITY,
    };
    let items = [
        (
            Rect::from_xywh(10, 10, 130, 80),
            linear(p(10, 0), p(140, 0), &OCEAN, GradExtend::Pad),
        ),
        (
            Rect::from_xywh(155, 10, 130, 80),
            linear(p(155, 10), p(285, 90), &SUNSET, GradExtend::Pad),
        ),
        (
            Rect::from_xywh(300, 10, 130, 80),
            radial(p(365, 50), fx(50), None, &GLOW, GradExtend::Pad),
        ),
        (
            Rect::from_xywh(10, 110, 130, 80),
            radial(p(75, 150), fx(45), Some(p(55, 130)), &SUNSET, GradExtend::Pad),
        ),
        (
            Rect::from_xywh(155, 110, 130, 80),
            linear(p(200, 0), p(220, 0), &STRIPES, GradExtend::Repeat),
        ),
        (
            Rect::from_xywh(300, 110, 130, 80),
            linear(p(345, 0), p(365, 0), &STRIPES, GradExtend::Reflect),
        ),
    ];
    for (r, paint) in items {
        let (path, dsc) = scene.path_mut();
        path.rect(r, 10);
        *dsc = VectorDsc {
            fill: Some((paint, FillRule::NonZero)),
            ..VectorDsc::default()
        };
    }
}

fn strokes(scene: &mut VectorScene) {
    // Joins: a sharp zigzag with each join.
    for (i, join) in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel]
        .into_iter()
        .enumerate()
    {
        let x = 20 + i as i32 * 140;
        let (path, dsc) = scene.path_mut();
        path.move_to(p(x, 80));
        path.line_to(p(x + 30, 20));
        path.line_to(p(x + 60, 80));
        path.line_to(p(x + 90, 20));
        *dsc = stroked(0x0037_474F, 12, join, LineCap::Butt, None);
    }
    // Caps: thick horizontal lines with each cap, over a thin guide.
    for (i, cap) in [LineCap::Butt, LineCap::Round, LineCap::Square]
        .into_iter()
        .enumerate()
    {
        let y = 115 + i as i32 * 24;
        let (path, dsc) = scene.path_mut();
        path.move_to(p(40, y));
        path.line_to(p(180, y));
        *dsc = stroked(0x00FF_7043, 14, LineJoin::Miter, cap, None);
        let (path, dsc) = scene.path_mut();
        path.move_to(p(40, y));
        path.line_to(p(180, y));
        *dsc = stroked(0x00FF_FFFF, 1, LineJoin::Miter, LineCap::Butt, None);
    }
    // Dashes on a circle, dots on a curve.
    let (path, dsc) = scene.path_mut();
    path.circle(p(270, 145), fx(40));
    *dsc = stroked(
        0x001E_88E5,
        6,
        LineJoin::Round,
        LineCap::Butt,
        Some(Dash::new(&[fx(14), fx(8)], Fx::ZERO)),
    );
    let (path, dsc) = scene.path_mut();
    path.move_to(p(330, 180));
    path.cubic_to(p(350, 90), p(400, 200), p(430, 110));
    *dsc = stroked(
        0x00D8_1B60,
        8,
        LineJoin::Round,
        LineCap::Round,
        Some(Dash::new(&[Fx::ZERO, fx(14)], Fx::ZERO)),
    );
}

/// A rotating, pulsing star (`angle` in 0.1°, `pulse` in ‰ of the radius).
fn motion(scene: &mut VectorScene, angle: i32, pulse: i32) {
    let (path, dsc) = scene.path_mut();
    star(path, p(80, 80), 40 + 20 * pulse / 1000, false);
    *dsc = VectorDsc {
        fill: Some((
            Paint::Radial {
                center: p(80, 80),
                radius: fx(60),
                focal: None,
                stops: Stops::Static(&SUNSET),
                extend: GradExtend::Pad,
                transform: Transform::IDENTITY,
            },
            FillRule::NonZero,
        )),
        transform: Transform::rotate(Angle(angle)).around(Point::new(80, 80)),
        ..VectorDsc::default()
    };
}

/// Height of the canvases.
const CANVAS_H: i32 = 200;

fn canvas(draw: fn(&mut VectorScene)) -> impl View {
    vector_canvas(draw).size(440, CANVAS_H)
}

/// The vector demo.
///
/// ```
/// use twine_testing::{TestUi, by_id};
///
/// let mut t = TestUi::new(480, 320).mount(twine_demos::vector::app);
/// t.advance(twine::core::Duration::ms(100));
/// assert!(t.find(by_id("motion")).coords().width() > 0);
/// ```
pub fn app(cx: Scope) -> impl View {
    let selected = cx.signal(0usize);
    let (angle, _spin) = cx.animation(
        Anim::new(0, 3600)
            .duration(Duration::secs(6))
            .repeat(Repeat::Infinite),
    );
    let (pulse, _beat) = cx.animation(
        Anim::new(0, 1000)
            .duration(Duration::ms(800))
            .easing(Easing::EaseInOut)
            .playback(Duration::ms(800))
            .repeat(Repeat::Infinite),
    );
    tabview(
        selected,
        (
            tab("Shapes", canvas(shapes)),
            tab("Gradients", canvas(gradients)),
            tab("Strokes", canvas(strokes)),
            tab(
                "SVG & motion",
                row((
                    vector_canvas(move |scene| motion(scene, angle.get(), pulse.get()))
                        .size(160, 160)
                        .test_id("motion"),
                    column((
                        row((
                            image(ImageSource::Svg(ICONS[0])),
                            image(ImageSource::Svg(ICONS[1])),
                            image(ImageSource::Svg(ICONS[2])),
                            image(ImageSource::Svg(ICONS[3])),
                        ))
                        .gap(12),
                        row((
                            image(ImageSource::Svg(ICONS[1])).scale(Scale(512)).size(96, 96),
                            image(ImageSource::Svg(ICONS[2])).scale(Scale(512)).size(96, 96),
                        ))
                        .gap(12),
                    ))
                    .gap(8),
                ))
                .gap(16),
            ),
        ),
    )
    .size(Length::pct(100), Length::pct(100))
}
