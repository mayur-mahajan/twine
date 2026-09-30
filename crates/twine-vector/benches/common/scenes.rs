//! Vector benchmark scenes shared by the criterion and iai benches: each draws into a
//! 320 × 240 RGB565 buffer with warm caches (the steady state of an animated UI).

use std::hint::black_box;

use twine_core::{Color, ColorFormat, Fx, Rect};
use twine_render::{DrawBuf, GradExtend, GradStop, Painter, RenderCaches};
use twine_vector::{FxPoint, LineJoin, Paint, PainterVectorExt, Path, Stops, Stroke, VectorDsc, parse_svg};

pub const W: i32 = 320;
pub const H: i32 = 240;

/// The SVG icon parsed by the `svg_parse_icon` scene.
pub static ICON: &[u8] = include_bytes!("../../../../assets/images/svg/badge.svg");

static STOPS: [GradStop; 2] = [
    GradStop {
        color: Color::hex(0x00FF_E082),
        opa: twine_core::Opa::COVER,
        frac: twine_core::Fraction::ZERO,
    },
    GradStop {
        color: Color::hex(0x00E6_5100),
        opa: twine_core::Opa::COVER,
        frac: twine_core::Fraction::ONE,
    },
];

/// A scene: a path and how to draw it.
pub struct Scene {
    pub name: &'static str,
    path: Path,
    dsc: VectorDsc,
}

/// Every drawing scene.
pub fn scenes() -> Vec<Scene> {
    let mut circle = Path::new();
    circle.circle(FxPoint::from_int(160, 120), Fx::from_int(50));
    let mut zigzag = Path::new();
    zigzag.move_to(FxPoint::from_int(60, 120));
    for k in 1..=10 {
        zigzag.line_to(FxPoint::from_int(60 + k * 20, if k % 2 == 0 { 100 } else { 140 }));
    }
    let mut square = Path::new();
    square.move_to(FxPoint::from_int(60, 20));
    square.line_to(FxPoint::from_int(260, 20));
    square.line_to(FxPoint::from_int(260, 220));
    square.line_to(FxPoint::from_int(60, 220));
    square.close();
    vec![
        Scene {
            name: "fill_circle_100",
            path: circle,
            dsc: VectorDsc::fill(Color::BLUE),
        },
        Scene {
            name: "stroke_polyline_200_round",
            path: zigzag,
            dsc: VectorDsc {
                stroke: Some((
                    Paint::Solid(Color::RED),
                    Stroke {
                        width: Fx::from_int(4),
                        join: LineJoin::Round,
                        ..Stroke::default()
                    },
                )),
                ..VectorDsc::default()
            },
        },
        Scene {
            name: "radial_gradient_200",
            path: square,
            dsc: VectorDsc {
                fill: Some((
                    Paint::Radial {
                        center: FxPoint::from_int(160, 120),
                        radius: Fx::from_int(100),
                        focal: None,
                        stops: Stops::Static(&STOPS),
                        extend: GradExtend::Pad,
                        transform: twine_core::Transform::IDENTITY,
                    },
                    twine_render::FillRule::NonZero,
                )),
                ..VectorDsc::default()
            },
        },
    ]
}

/// A render target with warm caches.
pub struct Target {
    px: Vec<u8>,
    caches: RenderCaches,
}

impl Target {
    pub fn new() -> Self {
        let mut t = Self {
            px: vec![0; (W * H * 2) as usize],
            caches: RenderCaches::default(),
        };
        for s in &scenes() {
            t.draw(s); // warm-up: grow the vector caches, fill the gradient cache
        }
        t
    }

    pub fn draw(&mut self, s: &Scene) {
        let area = Rect::from_xywh(0, 0, W, H);
        let buf = DrawBuf::new_packed(&mut self.px, ColorFormat::Rgb565, area).expect("buffer");
        let mut p = Painter::new(buf, &mut self.caches);
        p.vector(black_box(&s.path), black_box(&s.dsc));
    }
}

/// Parses the icon (the `svg_parse_icon` scene).
pub fn parse_icon() -> usize {
    parse_svg(black_box(ICON)).map_or(0, |d| d.scene.len())
}
