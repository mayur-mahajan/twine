//! Renderer gallery pages, shared by the `render_gallery` example and the renderer's snapshot
//! tests (which include this file by path). Depends only on `twine-core` and `twine-render`.
//!
//! Every page draws a 320 × 240 screen; `frame` animates the pages that move.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

use twine_core::{Angle, Color, Fx, Opa, Point, Rect, Scale, Transform};
use twine_render::{
    ArcDsc, BlendMode, BlitDsc, BorderSide, FillRule, GradExtend, GradKind, GradStop, Gradient, ImagePixels,
    LayerDsc, LayerTransform, LineDsc, LineSide, Mask, Painter, RADIUS_CIRCLE, RectDsc, ShadowDsc,
    TriangleDsc,
};

/// Screen width of every page.
pub const W: i32 = 320;
/// Screen height of every page.
pub const H: i32 = 240;

/// A page: title, what to look at, and the drawing function.
pub struct Page {
    /// Title (logged when the page is shown).
    pub title: &'static str,
    /// What the page demonstrates.
    pub hint: &'static str,
    /// Draws the page for animation frame `frame`.
    pub draw: fn(&mut Painter<'_>, u32),
}

/// All pages in order.
pub const PAGES: &[Page] = &[
    Page {
        title: "Fills & opacity",
        hint: "palette at opa 255/191/127/63 over a checkerboard; one rect per blend mode over stripes",
        draw: page_fills,
    },
    Page {
        title: "Rounded rects, borders & outlines",
        hint: "radius 0/3/8/16/circle at opa 255 and 128, a 3x3 rect with radius 10, borders per side, outlines",
        draw: page_rects,
    },
    Page {
        title: "Gradients",
        hint: "hor, ver, linear, radial, focal, conical, repeat/reflect, dithered 565, rounded",
        draw: page_gradients,
    },
    Page {
        title: "Shadows",
        hint: "blur 5/15/30, offset, spread, colored shadow, round button",
        draw: page_shadows,
    },
    Page {
        title: "Masks",
        hint: "radius clip, angle, line, fade, bitmap map, radius + fade",
        draw: page_masks,
    },
    Page {
        title: "Layers & blend modes",
        hint: "group opacity (no darker overlap) vs individual; layers per blend mode",
        draw: page_layers,
    },
    Page {
        title: "Lines",
        hint: "fan of 36 lines, widths 1-12, dashes, butt vs round caps, zigzag polyline",
        draw: page_lines,
    },
    Page {
        title: "Arcs",
        hint: "gauges, rounded vs flat ends, image-filled arc, spinning arc",
        draw: page_arcs,
    },
    Page {
        title: "Triangles & polygons",
        hint: "stars (non-zero vs even-odd), arrow, concave shape, gradient triangle",
        draw: page_polygons,
    },
    Page {
        title: "Transforms",
        hint: "rotating image with/without AA, scale pulse, rotating card layer",
        draw: page_transforms,
    },
];

/// Eight palette colors.
pub const PALETTE: [Color; 8] = [
    Color::hex(0xE53935),
    Color::hex(0xFB8C00),
    Color::hex(0xFDD835),
    Color::hex(0x43A047),
    Color::hex(0x1E88E5),
    Color::hex(0x8E24AA),
    Color::hex(0x00ACC1),
    Color::hex(0x6D4C41),
];

fn screen() -> Rect {
    Rect::from_xywh(0, 0, W, H)
}

/// A checkerboard of `size`-pixel squares in two grays.
pub fn checkerboard(p: &mut Painter<'_>, area: Rect, size: i32) {
    p.fill(area, Color::hex(0xDDDDDD), Opa::COVER);
    let mut y = area.y0;
    while y < area.y1 {
        let mut x = area.x0 + if ((y - area.y0) / size) % 2 == 0 { 0 } else { size };
        while x < area.x1 {
            p.fill(
                Rect::new(x, y, (x + size).min(area.x1), (y + size).min(area.y1)),
                Color::hex(0x999999),
                Opa::COVER,
            );
            x += 2 * size;
        }
        y += size;
    }
}

fn rect_dsc(color: Color) -> RectDsc<'static> {
    RectDsc {
        bg_color: color,
        bg_opa: Opa::COVER,
        ..RectDsc::default()
    }
}

/// Page 1: fills, opacity and blend modes.
pub fn page_fills(p: &mut Painter<'_>, _frame: u32) {
    checkerboard(p, screen(), 8);
    let opas = [255u8, 191, 127, 63];
    for (row, &o) in opas.iter().enumerate() {
        for col in 0..6 {
            let r = Rect::from_xywh(10 + col * 50, 8 + row as i32 * 36, 44, 30);
            p.fill(r, PALETTE[col as usize], Opa(o));
        }
    }
    // Stripes, then one rect per blend mode.
    let stripes = Rect::new(0, 160, W, H);
    for i in 0..W / 8 {
        let c = Color::lerp(
            Color::hex(0x2040A0),
            Color::hex(0xF0E040),
            (i * 1024 / (W / 8)) as u16,
        );
        p.fill(Rect::new(i * 8, stripes.y0, i * 8 + 8, stripes.y1), c, Opa::COVER);
    }
    for (i, mode) in BlendMode::ALL.iter().enumerate() {
        let r = Rect::from_xywh(12 + i as i32 * 62, 172, 50, 56);
        p.fill_mode(r, Color::hex(0xC05030), Opa::COVER, *mode);
    }
}

/// Page 2: rounded rectangles, borders and outlines.
pub fn page_rects(p: &mut Painter<'_>, _frame: u32) {
    p.fill(screen(), Color::hex(0xF4F4F4), Opa::COVER);
    let radii = [0, 3, 8, 16, RADIUS_CIRCLE];
    for (row, opa) in [Opa::COVER, Opa(128)].into_iter().enumerate() {
        for (i, &r) in radii.iter().enumerate() {
            let a = Rect::from_xywh(10 + i as i32 * 58, 8 + row as i32 * 56, 50, 48);
            p.rect(
                a,
                &RectDsc {
                    radius: r,
                    bg_color: PALETTE[4],
                    bg_opa: opa,
                    ..RectDsc::default()
                },
            );
        }
    }
    // Tiny rect with a big radius.
    p.rect(
        Rect::from_xywh(302, 30, 3, 3),
        &RectDsc {
            radius: 10,
            ..rect_dsc(Color::BLACK)
        },
    );
    // Borders on each side and full.
    let sides = [
        BorderSide::TOP,
        BorderSide::BOTTOM,
        BorderSide::LEFT,
        BorderSide::RIGHT,
        BorderSide::FULL,
    ];
    for (i, &s) in sides.iter().enumerate() {
        p.rect(
            Rect::from_xywh(10 + i as i32 * 58, 124, 50, 40),
            &RectDsc {
                radius: 8,
                border_width: 4,
                border_color: PALETTE[0],
                border_opa: Opa::COVER,
                border_side: s,
                ..rect_dsc(Color::WHITE)
            },
        );
    }
    // Thick rounded border, translucent border over a colored bg, outline with pad.
    p.rect(
        Rect::from_xywh(14, 178, 80, 54),
        &RectDsc {
            radius: 20,
            border_width: 8,
            border_color: PALETTE[5],
            border_opa: Opa::COVER,
            ..rect_dsc(Color::WHITE)
        },
    );
    p.rect(
        Rect::from_xywh(112, 178, 80, 54),
        &RectDsc {
            radius: 12,
            border_width: 6,
            border_color: Color::BLACK,
            border_opa: Opa(100),
            ..rect_dsc(PALETTE[2])
        },
    );
    p.rect(
        Rect::from_xywh(216, 186, 88, 40),
        &RectDsc {
            radius: RADIUS_CIRCLE,
            outline_width: 3,
            outline_pad: 3,
            outline_color: PALETTE[4],
            outline_opa: Opa::COVER,
            ..rect_dsc(PALETTE[6])
        },
    );
}

fn two_stops(a: Color, b: Color) -> [GradStop; 2] {
    [GradStop::new(a, 0), GradStop::new(b, 255)]
}

/// Page 3: gradients.
pub fn page_gradients(p: &mut Painter<'_>, _frame: u32) {
    p.fill(screen(), Color::WHITE, Opa::COVER);
    let tile = |i: i32| Rect::from_xywh(8 + (i % 3) * 104, 6 + (i / 3) * 78, 96, 72);
    let rainbow = [
        GradStop::new(Color::RED, 0),
        GradStop::new(Color::YELLOW, 64),
        GradStop::new(Color::GREEN, 128),
        GradStop::new(Color::CYAN, 192),
        GradStop::new(Color::BLUE, 255),
    ];
    let grads = [
        Gradient::new(GradKind::Hor, &rainbow),
        Gradient::new(GradKind::Ver, &two_stops(PALETTE[4], PALETTE[0])),
        Gradient::new(
            GradKind::Linear {
                start: Point::new(0, 0),
                end: Point::new(95, 71),
            },
            &two_stops(Color::BLACK, PALETTE[2]),
        ),
        Gradient::new(
            GradKind::Radial {
                center: Point::new(48, 36),
                radius: 40,
                focal: Point::new(48, 36),
                focal_radius: 0,
            },
            &two_stops(Color::WHITE, PALETTE[5]),
        ),
        Gradient::new(
            GradKind::Radial {
                center: Point::new(48, 36),
                radius: 44,
                focal: Point::new(30, 22),
                focal_radius: 4,
            },
            &two_stops(Color::YELLOW, PALETTE[0]),
        ),
        Gradient::new(
            GradKind::Conical {
                center: Point::new(48, 36),
                start_angle: Angle::deg(0),
                end_angle: Angle::deg(360),
            },
            &rainbow,
        ),
        Gradient::new(
            GradKind::Linear {
                start: Point::new(30, 0),
                end: Point::new(48, 0),
            },
            &two_stops(PALETTE[3], Color::WHITE),
        )
        .extend(GradExtend::Repeat),
        Gradient::new(
            GradKind::Ver,
            &two_stops(Color::hex(0x202830), Color::hex(0x405060)),
        )
        .dither(true),
        Gradient::new(GradKind::Hor, &two_stops(PALETTE[6], PALETTE[4])),
    ];
    for (i, g) in grads.iter().enumerate() {
        let r = if i == 8 { RADIUS_CIRCLE } else { 0 };
        p.rect(
            tile(i as i32),
            &RectDsc {
                radius: r,
                bg_opa: Opa::COVER,
                bg_grad: Some(g),
                ..RectDsc::default()
            },
        );
    }
    // Reflect in the lower part of the repeat tile.
    let g = Gradient::new(
        GradKind::Linear {
            start: Point::new(30, 0),
            end: Point::new(48, 0),
        },
        &two_stops(PALETTE[3], Color::WHITE),
    )
    .extend(GradExtend::Reflect);
    let t = tile(6);
    p.fill_gradient(Rect::new(t.x0, t.y0 + 36, t.x1, t.y1), &g, Opa::COVER);
}

fn card(p: &mut Painter<'_>, a: Rect, radius: i32, shadow: ShadowDsc) {
    p.rect(
        a,
        &RectDsc {
            radius,
            shadow,
            border_width: 1,
            border_color: Color::hex(0xDDDDDD),
            border_opa: Opa::COVER,
            ..rect_dsc(Color::WHITE)
        },
    );
}

/// Page 4: shadows.
pub fn page_shadows(p: &mut Painter<'_>, _frame: u32) {
    p.fill(screen(), Color::hex(0xECEFF1), Opa::COVER);
    let sh = |width, ofs_x, ofs_y, spread, color, opa| ShadowDsc {
        width,
        ofs_x,
        ofs_y,
        spread,
        color,
        opa,
    };
    card(
        p,
        Rect::from_xywh(20, 20, 70, 60),
        8,
        sh(5, 0, 0, 0, Color::BLACK, Opa(160)),
    );
    card(
        p,
        Rect::from_xywh(125, 20, 70, 60),
        8,
        sh(15, 0, 0, 0, Color::BLACK, Opa(160)),
    );
    card(
        p,
        Rect::from_xywh(230, 20, 70, 60),
        8,
        sh(30, 0, 0, 0, Color::BLACK, Opa(160)),
    );
    card(
        p,
        Rect::from_xywh(20, 130, 70, 60),
        4,
        sh(12, 8, 8, 0, Color::BLACK, Opa(140)),
    );
    card(
        p,
        Rect::from_xywh(125, 130, 70, 60),
        12,
        sh(10, 0, 0, 6, Color::BLACK, Opa(120)),
    );
    card(
        p,
        Rect::from_xywh(230, 130, 70, 60),
        8,
        sh(20, 0, 4, 2, PALETTE[4], Opa::COVER),
    );
    p.rect(
        Rect::from_xywh(140, 206, 40, 28),
        &RectDsc {
            radius: RADIUS_CIRCLE,
            shadow: sh(10, 0, 3, 0, Color::BLACK, Opa(150)),
            ..rect_dsc(PALETTE[0])
        },
    );
}

/// The alpha map of the masks page (masks must live as long as the painter, so it is static).
static DOTS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// Page 5: masks.
pub fn page_masks(p: &mut Painter<'_>, _frame: u32) {
    p.fill(screen(), Color::WHITE, Opa::COVER);
    let tile = |i: i32| Rect::from_xywh(8 + (i % 3) * 104, 12 + (i / 3) * 112, 96, 100);
    let stripes = |p: &mut Painter<'_>, r: Rect| {
        for i in 0..=(r.width() / 6) {
            let c = PALETTE[(i % 8) as usize];
            p.fill(
                Rect::new(r.x0 + i * 6, r.y0, (r.x0 + i * 6 + 6).min(r.x1), r.y1),
                c,
                Opa::COVER,
            );
        }
    };
    // Radius clip.
    let t = tile(0);
    let id = p.push_mask(Mask::Radius {
        area: t,
        radius: 24,
        outer: false,
    });
    stripes(p, t);
    p.pop_mask(id);
    // Angle (a quarter plus a bit).
    let t = tile(1);
    let id = p.push_mask(Mask::Angle {
        center: t.center(),
        start: Angle::deg(-30),
        end: Angle::deg(100),
    });
    stripes(p, t);
    p.pop_mask(id);
    // Diagonal line.
    let t = tile(2);
    let id = p.push_mask(Mask::Line {
        p1: Point::new(t.x0, t.y1),
        p2: Point::new(t.x1, t.y0 + 20),
        side: LineSide::Top,
    });
    stripes(p, t);
    p.pop_mask(id);
    // Vertical fade.
    let t = tile(3);
    let id = p.push_mask(Mask::Fade {
        area: t,
        y_top: t.y0 + 10,
        y_bottom: t.y1 - 10,
        opa_top: Opa::COVER,
        opa_bottom: Opa::TRANSP,
    });
    stripes(p, t);
    p.pop_mask(id);
    // Bitmap map: a checker of soft dots.
    let t = tile(4);
    let alpha = DOTS.get_or_init(|| {
        (0..100)
            .flat_map(|y: i32| {
                (0..96).map(move |x: i32| {
                    let (dx, dy) = ((x % 16) - 8, (y % 16) - 8);
                    (255 - (dx * dx + dy * dy) * 4).clamp(0, 255) as u8
                })
            })
            .collect()
    });
    let id = p.push_mask(Mask::Map { area: t, alpha });
    stripes(p, t);
    p.pop_mask(id);
    // Radius + fade stacked.
    let t = tile(5);
    let a = p.push_mask(Mask::Radius {
        area: t,
        radius: RADIUS_CIRCLE,
        outer: false,
    });
    let b = p.push_mask(Mask::Fade {
        area: t,
        y_top: t.y0,
        y_bottom: t.y1,
        opa_top: Opa::COVER,
        opa_bottom: Opa(40),
    });
    stripes(p, t);
    p.pop_mask(b);
    p.pop_mask(a);
}

/// Page 6: layers and blend modes.
pub fn page_layers(p: &mut Painter<'_>, _frame: u32) {
    checkerboard(p, screen(), 10);
    // Individually at 50 %: the overlap is darker.
    let pair = |p: &mut Painter<'_>, x: i32, opa: Opa| {
        p.fill(Rect::from_xywh(x, 20, 70, 60), PALETTE[0], opa);
        p.fill(Rect::from_xywh(x + 40, 50, 70, 60), PALETTE[4], opa);
    };
    pair(p, 20, Opa(128));
    // In a layer at 50 %: uniform.
    p.layer(
        Rect::from_xywh(170, 20, 110, 90),
        &LayerDsc {
            opa: Opa(128),
            ..LayerDsc::default()
        },
        |p| pair(p, 170, Opa::COVER),
    );
    for (i, mode) in BlendMode::ALL.iter().enumerate() {
        let a = Rect::from_xywh(8 + i as i32 * 62, 140, 56, 90);
        p.layer(
            a,
            &LayerDsc {
                opa: Opa(220),
                blend_mode: *mode,
                transform: None,
            },
            |p| {
                p.rect(
                    a,
                    &RectDsc {
                        radius: 12,
                        ..rect_dsc(Color::hex(0x80C040))
                    },
                );
                p.fill(
                    Rect::new(a.x0 + 10, a.y0 + 30, a.x1 - 10, a.y0 + 60),
                    PALETTE[5],
                    Opa::COVER,
                );
            },
        );
    }
}

/// Page 7: lines.
pub fn page_lines(p: &mut Painter<'_>, _frame: u32) {
    p.fill(screen(), Color::WHITE, Opa::COVER);
    // Fan: 36 lines at 10° steps, width growing 1..=12.
    let c = Point::new(80, 90);
    for i in 0..36 {
        let a = Angle::deg(i * 10);
        let (s, co) = (twine_core::math::sin(a), twine_core::math::cos(a));
        let e = Point::new(c.x + 70 * co / 32767, c.y + 70 * s / 32767);
        let f = Point::new(c.x + 18 * co / 32767, c.y + 18 * s / 32767);
        p.line(
            f,
            e,
            &LineDsc {
                color: PALETTE[(i % 8) as usize],
                width: 1 + (i % 12) / 3,
                ..LineDsc::default()
            },
        );
    }
    // Widths.
    for (i, w) in [1, 2, 3, 5, 8, 12].into_iter().enumerate() {
        let y = 14 + i as i32 * 22;
        p.line(
            Point::new(180, y),
            Point::new(310, y + 10),
            &LineDsc {
                width: w,
                color: Color::BLACK,
                ..LineDsc::default()
            },
        );
    }
    // Caps: butt vs round.
    let thick = LineDsc {
        width: 12,
        color: PALETTE[4],
        ..LineDsc::default()
    };
    p.line(Point::new(20, 190), Point::new(90, 175), &thick);
    p.line(
        Point::new(20, 220),
        Point::new(90, 205),
        &LineDsc {
            round_start: true,
            round_end: true,
            ..thick
        },
    );
    // Dashes: horizontal, vertical, diagonal.
    let dash = LineDsc {
        width: 3,
        dash_width: 8,
        dash_gap: 5,
        color: PALETTE[0],
        ..LineDsc::default()
    };
    p.line(Point::new(110, 180), Point::new(200, 180), &dash);
    p.line(Point::new(110, 190), Point::new(110, 234), &dash);
    p.line(Point::new(120, 234), Point::new(200, 192), &dash);
    // Zigzag polyline with round joins.
    let pts = [
        Point::new(215, 225),
        Point::new(235, 180),
        Point::new(255, 225),
        Point::new(275, 180),
        Point::new(295, 225),
        Point::new(312, 190),
    ];
    p.polyline(
        &pts,
        &LineDsc {
            width: 6,
            color: PALETTE[3],
            round_start: true,
            round_end: true,
            ..LineDsc::default()
        },
    );
}

/// A generated `size × size` `Argb8888` image: a hue gradient ring pattern.
pub fn gradient_image(size: u16) -> Vec<u8> {
    let s = i32::from(size);
    let mut v = Vec::with_capacity(usize::from(size) * usize::from(size) * 4);
    for y in 0..s {
        for x in 0..s {
            let c = Color::lerp(
                Color::lerp(Color::RED, Color::BLUE, (x * 1024 / s) as u16),
                Color::GREEN,
                (y * 1024 / s / 2) as u16,
            );
            v.extend_from_slice(&[c.b, c.g, c.r, 255]);
        }
    }
    v
}

/// Page 8: arcs.
pub fn page_arcs(p: &mut Painter<'_>, frame: u32) {
    p.fill(screen(), Color::hex(0x263238), Opa::COVER);
    let track = |p: &mut Painter<'_>, c: Point, r: i32, w: i32| {
        p.arc(
            c,
            r,
            Angle::deg(135),
            Angle::deg(405),
            &ArcDsc {
                color: Color::hex(0x455A64),
                width: w,
                ..ArcDsc::default()
            },
        );
    };
    // Gauges at several widths.
    for (i, w) in [4, 10, 18].into_iter().enumerate() {
        let c = Point::new(55 + i as i32 * 105, 60);
        track(p, c, 45, w);
        p.arc(
            c,
            45,
            Angle::deg(135),
            Angle::deg(135 + 90 + 60 * i as i32),
            &ArcDsc {
                color: PALETTE[i + 1],
                width: w,
                rounded: i != 0,
                ..ArcDsc::default()
            },
        );
    }
    // Rounded vs flat translucent, pie, full ring.
    let c = Point::new(55, 175);
    p.arc(
        c,
        40,
        Angle::deg(200),
        Angle::deg(340),
        &ArcDsc {
            color: PALETTE[0],
            width: 14,
            rounded: false,
            ..ArcDsc::default()
        },
    );
    p.arc(
        c,
        40,
        Angle::deg(20),
        Angle::deg(160),
        &ArcDsc {
            color: PALETTE[3],
            width: 14,
            rounded: true,
            opa: Opa(128),
            ..ArcDsc::default()
        },
    );
    p.arc(
        Point::new(160, 175),
        40,
        Angle::deg(-60),
        Angle::deg(60),
        &ArcDsc {
            color: PALETTE[2],
            width: 40,
            ..ArcDsc::default()
        },
    );
    // Image-filled ring.
    let img = gradient_image(80);
    let pix = ImagePixels::new(twine_core::ColorFormat::Argb8888, 80, 80, &img);
    p.arc(
        Point::new(160, 175),
        40,
        Angle::deg(90),
        Angle::deg(300),
        &ArcDsc {
            width: 8,
            image: Some(&pix),
            ..ArcDsc::default()
        },
    );
    // Spinner.
    let c = Point::new(265, 175);
    track(p, c, 30, 6);
    let a = (frame as i32 * 6) % 360;
    p.arc(
        c,
        30,
        Angle::deg(a),
        Angle::deg(a + 70),
        &ArcDsc {
            color: PALETTE[6],
            width: 6,
            rounded: true,
            ..ArcDsc::default()
        },
    );
}

fn star(c: Point, r_out: i32, r_in: i32, n: i32) -> Vec<Point> {
    (0..2 * n)
        .map(|i| {
            let a = Angle::decideg(-900 + i * 1800 / n);
            let r = if i % 2 == 0 { r_out } else { r_in };
            Point::new(
                c.x + r * twine_core::math::cos(a) / 32767,
                c.y + r * twine_core::math::sin(a) / 32767,
            )
        })
        .collect()
}

fn pentagram(c: Point, r: i32) -> Vec<Point> {
    (0..5)
        .map(|i| {
            let a = Angle::decideg(-900 + i * 1440);
            Point::new(
                c.x + r * twine_core::math::cos(a) / 32767,
                c.y + r * twine_core::math::sin(a) / 32767,
            )
        })
        .collect()
}

/// Page 9: triangles and polygons.
pub fn page_polygons(p: &mut Painter<'_>, _frame: u32) {
    p.fill(screen(), Color::WHITE, Opa::COVER);
    let solid = |c| TriangleDsc {
        color: c,
        ..TriangleDsc::default()
    };
    p.polygon(&star(Point::new(55, 60), 48, 20, 5), &solid(PALETTE[1]));
    p.polygon_with_rule(
        &pentagram(Point::new(160, 60), 48),
        FillRule::NonZero,
        &solid(PALETTE[4]),
    );
    p.polygon_with_rule(
        &pentagram(Point::new(265, 60), 48),
        FillRule::EvenOdd,
        &solid(PALETTE[5]),
    );
    // Arrow.
    let arrow = [
        Point::new(10, 150),
        Point::new(60, 150),
        Point::new(60, 130),
        Point::new(100, 165),
        Point::new(60, 200),
        Point::new(60, 180),
        Point::new(10, 180),
    ];
    p.polygon(&arrow, &solid(PALETTE[3]));
    // Concave "C" shape, translucent.
    let c = [
        Point::new(120, 125),
        Point::new(200, 125),
        Point::new(200, 145),
        Point::new(140, 145),
        Point::new(140, 195),
        Point::new(200, 195),
        Point::new(200, 215),
        Point::new(120, 215),
    ];
    p.polygon(
        &c,
        &TriangleDsc {
            color: PALETTE[0],
            opa: Opa(180),
            grad: None,
        },
    );
    // Gradient triangle and a thin sliver.
    let g = Gradient::new(GradKind::Hor, &two_stops(PALETTE[6], PALETTE[5]));
    p.triangle(
        [Point::new(215, 225), Point::new(265, 120), Point::new(315, 225)],
        &TriangleDsc {
            grad: Some(&g),
            ..TriangleDsc::default()
        },
    );
    p.triangle(
        [Point::new(5, 232), Point::new(310, 236), Point::new(5, 234)],
        &solid(Color::BLACK),
    );
}

/// A 32 × 32 `Argb8888` test image: checker, border and an arrow pointing right.
pub fn test_image() -> Vec<u8> {
    let mut v = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32i32 {
        for x in 0..32i32 {
            let mut c = if (x / 8 + y / 8) % 2 == 0 {
                Color::hex(0xFFFFFF)
            } else {
                Color::hex(0x90CAF9)
            };
            if x == 0 || y == 0 || x == 31 || y == 31 {
                c = Color::hex(0x0D47A1);
            }
            // Arrow: shaft and head.
            let shaft = (6..20).contains(&x) && (14..18).contains(&y);
            let head = (20..28).contains(&x) && (y - 16).abs() <= 27 - x;
            if shaft || head {
                c = Color::hex(0xD32F2F);
            }
            v.extend_from_slice(&[c.b, c.g, c.r, 255]);
        }
    }
    v
}

/// Page 10: transformed images and layers.
pub fn page_transforms(p: &mut Painter<'_>, frame: u32) {
    p.fill(screen(), Color::hex(0xFAFAFA), Opa::COVER);
    let img = test_image();
    let pix = ImagePixels::new(twine_core::ColorFormat::Argb8888, 32, 32, &img);
    let angle = Angle::deg((frame as i32 * 2) % 360);
    let place = |cx: i32, cy: i32, angle: Angle, scale: Scale| {
        Transform::from_rotate_scale(angle, scale, scale, Point::new(16, 16))
            .then(Transform::translate(Fx::from_int(cx - 16), Fx::from_int(cy - 16)))
    };
    let two = Scale(512);
    for (i, aa) in [false, true].into_iter().enumerate() {
        p.blit_transformed(
            &pix,
            &BlitDsc {
                transform: place(50 + i as i32 * 100, 60, angle, two),
                antialias: aa,
                ..BlitDsc::default()
            },
        );
    }
    // Scale pulse (100 % .. 200 %).
    let t = (frame % 60) as i32;
    let tri = if t < 30 { t } else { 60 - t };
    let s = Scale((256 + tri * 256 / 30) as u16);
    p.blit_transformed(
        &pix,
        &BlitDsc {
            transform: place(260, 60, Angle(0), s),
            ..BlitDsc::default()
        },
    );
    // A card rotating and pulsing inside a transformed layer.
    // 96 × 60 × 4 bytes fits the default 24 KiB layer buffer in one piece.
    let area = Rect::from_xywh(112, 150, 96, 60);
    p.layer(
        area,
        &LayerDsc {
            opa: Opa::COVER,
            blend_mode: BlendMode::Normal,
            transform: Some(LayerTransform {
                rotation: Angle::deg(-((frame as i32 * 3) % 360)),
                scale_x: Scale((230 + tri) as u16),
                scale_y: Scale((230 + tri) as u16),
                pivot: Point::new(48, 30),
                ..LayerTransform::default()
            }),
        },
        |p| {
            p.rect(
                area.expand(-6),
                &RectDsc {
                    radius: 10,
                    border_width: 2,
                    border_color: PALETTE[4],
                    border_opa: Opa::COVER,
                    shadow: ShadowDsc {
                        width: 8,
                        opa: Opa(120),
                        ..ShadowDsc::default()
                    },
                    ..rect_dsc(Color::WHITE)
                },
            );
            p.fill(
                Rect::from_xywh(area.x0 + 16, area.y0 + 14, 40, 8),
                PALETTE[0],
                Opa::COVER,
            );
            p.fill(
                Rect::from_xywh(area.x0 + 16, area.y0 + 28, 60, 6),
                Color::hex(0xB0BEC5),
                Opa::COVER,
            );
            p.fill(
                Rect::from_xywh(area.x0 + 16, area.y0 + 39, 48, 6),
                Color::hex(0xB0BEC5),
                Opa::COVER,
            );
        },
    );
}
