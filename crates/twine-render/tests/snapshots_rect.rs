//! Snapshots of rectangles, borders, outlines, gradients and shadows.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use twine_core::{Angle, Color, ColorFormat, Opa, Point, Rect};
use twine_render::{
    BorderSide, GradExtend, GradKind, GradStop, Gradient, Painter, RADIUS_CIRCLE, RectDsc, ShadowDsc,
};
use twine_testing::{RenderHarness, assert_render_snapshot};

const BLUE: Color = Color::hex(0x1E88E5);
const RED: Color = Color::hex(0xE53935);

fn bg(color: Color) -> RectDsc<'static> {
    RectDsc {
        bg_color: color,
        bg_opa: Opa::COVER,
        ..RectDsc::default()
    }
}

fn harness(w: u16, h: u16, format: ColorFormat, f: impl FnOnce(&mut Painter<'_>)) -> RenderHarness {
    let mut h = RenderHarness::new(w, h, format);
    h.paint(f);
    h
}

fn radii_scene(p: &mut Painter<'_>) {
    for (i, r) in [0, 4, 16, RADIUS_CIRCLE].into_iter().enumerate() {
        p.rect(
            Rect::from_xywh(4 + i as i32 * 44, 4, 40, 40),
            &RectDsc {
                radius: r,
                ..bg(BLUE)
            },
        );
    }
}

#[test]
fn rect_radius_0_4_16_circle() {
    assert_render_snapshot!(
        harness(184, 48, ColorFormat::Rgb565, radii_scene),
        "rect_radius_0_4_16_circle_565"
    );
    assert_render_snapshot!(
        harness(184, 48, ColorFormat::Argb8888, radii_scene),
        "rect_radius_0_4_16_circle_argb"
    );
}

#[test]
fn rect_opa_128_radius_10() {
    let scene = |p: &mut Painter<'_>| {
        p.fill(Rect::from_xywh(0, 20, 80, 20), Color::BLACK, Opa::COVER);
        p.rect(
            Rect::from_xywh(10, 5, 60, 50),
            &RectDsc {
                radius: 10,
                bg_opa: Opa(128),
                ..bg(RED)
            },
        );
    };
    assert_render_snapshot!(
        harness(80, 60, ColorFormat::Rgb565, scene),
        "rect_opa_128_radius_10_565"
    );
    assert_render_snapshot!(
        harness(80, 60, ColorFormat::Argb8888, scene),
        "rect_opa_128_radius_10_argb"
    );
}

#[test]
fn rect_clipped_corner() {
    let scene = |p: &mut Painter<'_>| {
        p.with_clip(Rect::new(0, 0, 40, 35), |p| {
            p.rect(
                Rect::from_xywh(10, 10, 50, 40),
                &RectDsc {
                    radius: 20,
                    ..bg(BLUE)
                },
            );
        });
    };
    assert_render_snapshot!(
        harness(64, 56, ColorFormat::Rgb565, scene),
        "rect_clipped_corner_565"
    );
    assert_render_snapshot!(
        harness(64, 56, ColorFormat::Argb8888, scene),
        "rect_clipped_corner_argb"
    );
}

fn chunk_scene(p: &mut Painter<'_>) {
    radii_scene(p);
    p.rect(
        Rect::from_xywh(20, 10, 120, 30),
        &RectDsc {
            radius: 12,
            bg_opa: Opa(150),
            border_width: 3,
            border_color: RED,
            border_opa: Opa::COVER,
            shadow: ShadowDsc {
                width: 10,
                opa: Opa(120),
                ..ShadowDsc::default()
            },
            ..bg(Color::YELLOW)
        },
    );
}

#[test]
fn rect_chunked_identical() {
    for f in [ColorFormat::Rgb565, ColorFormat::Argb8888] {
        let full = harness(184, 48, f, chunk_scene);
        let mut ch = RenderHarness::new(184, 48, f);
        ch.paint_chunked(7, chunk_scene);
        assert!(full.rgb888() == ch.rgb888(), "{f}");
    }
}

fn bordered(side: BorderSide, radius: i32, width: i32) -> RectDsc<'static> {
    RectDsc {
        radius,
        border_width: width,
        border_color: RED,
        border_opa: Opa::COVER,
        border_side: side,
        ..bg(Color::hex(0xFFF3E0))
    }
}

#[test]
fn border_sides_each() {
    let h = harness(236, 44, ColorFormat::Rgb565, |p| {
        let sides = [
            BorderSide::TOP,
            BorderSide::BOTTOM,
            BorderSide::LEFT,
            BorderSide::RIGHT,
            BorderSide::FULL,
        ];
        for (i, s) in sides.into_iter().enumerate() {
            p.rect(Rect::from_xywh(4 + i as i32 * 46, 4, 42, 36), &bordered(s, 8, 4));
        }
    });
    assert_render_snapshot!(h, "border_sides_each");
}

#[test]
fn border_radius_thick() {
    let h = harness(100, 70, ColorFormat::Rgb565, |p| {
        p.rect(Rect::from_xywh(5, 5, 90, 60), &bordered(BorderSide::FULL, 20, 8));
    });
    assert_render_snapshot!(h, "border_radius_thick");
}

#[test]
fn border_wider_than_half() {
    let h = harness(100, 50, ColorFormat::Rgb565, |p| {
        p.rect(Rect::from_xywh(5, 5, 40, 30), &bordered(BorderSide::FULL, 6, 20));
        p.rect(Rect::from_xywh(55, 5, 40, 30), &bordered(BorderSide::FULL, 0, 16));
    });
    assert_render_snapshot!(h, "border_wider_than_half");
}

#[test]
fn outline_with_pad() {
    let h = harness(120, 60, ColorFormat::Rgb565, |p| {
        let d = RectDsc {
            radius: 10,
            outline_width: 3,
            outline_pad: 4,
            outline_color: BLUE,
            outline_opa: Opa::COVER,
            ..bg(Color::hex(0xB2DFDB))
        };
        p.rect(Rect::from_xywh(12, 12, 40, 36), &d);
        p.rect(
            Rect::from_xywh(72, 12, 36, 36),
            &RectDsc {
                radius: 0,
                outline_pad: 0,
                outline_opa: Opa(128),
                ..d
            },
        );
    });
    assert_render_snapshot!(h, "outline_with_pad");
}

#[test]
fn border_opa_over_bg() {
    let h = harness(100, 60, ColorFormat::Rgb565, |p| {
        p.rect(
            Rect::from_xywh(5, 5, 90, 50),
            &RectDsc {
                radius: 12,
                border_width: 6,
                border_color: Color::BLACK,
                border_opa: Opa(100),
                ..bg(Color::YELLOW)
            },
        );
    });
    assert_render_snapshot!(h, "border_opa_over_bg");
}

fn stops2(a: Color, b: Color) -> [GradStop; 2] {
    [GradStop::new(a, 0), GradStop::new(b, 255)]
}

fn grad_tile(p: &mut Painter<'_>, area: Rect, g: &Gradient, radius: i32) {
    p.rect(
        area,
        &RectDsc {
            radius,
            bg_opa: Opa::COVER,
            bg_grad: Some(g),
            ..RectDsc::default()
        },
    );
}

#[test]
fn grad_hor_ver() {
    let h = harness(136, 64, ColorFormat::Rgb888, |p| {
        grad_tile(
            p,
            Rect::from_xywh(4, 4, 60, 56),
            &Gradient::new(GradKind::Hor, &stops2(RED, BLUE)),
            0,
        );
        let three = [
            GradStop::new(RED, 0),
            GradStop::with_opa(Color::GREEN, Opa(64), 128),
            GradStop::new(BLUE, 255),
        ];
        grad_tile(
            p,
            Rect::from_xywh(72, 4, 60, 56),
            &Gradient::new(GradKind::Ver, &three),
            0,
        );
    });
    assert_render_snapshot!(h, "grad_hor_ver");
}

#[test]
fn grad_linear_diagonal() {
    let h = harness(80, 60, ColorFormat::Rgb888, |p| {
        let g = Gradient::new(
            GradKind::Linear {
                start: Point::new(10, 10),
                end: Point::new(70, 50),
            },
            &stops2(Color::BLACK, Color::YELLOW),
        );
        grad_tile(p, Rect::from_xywh(0, 0, 80, 60), &g, 0);
    });
    assert_render_snapshot!(h, "grad_linear_diagonal");
}

#[test]
fn grad_radial_centered() {
    let h = harness(80, 80, ColorFormat::Rgb888, |p| {
        let g = Gradient::new(
            GradKind::Radial {
                center: Point::new(40, 40),
                radius: 36,
                focal: Point::new(40, 40),
                focal_radius: 0,
            },
            &stops2(Color::WHITE, BLUE),
        );
        grad_tile(p, Rect::from_xywh(0, 0, 80, 80), &g, 0);
    });
    assert_render_snapshot!(h, "grad_radial_centered");
}

#[test]
fn grad_radial_focal() {
    let h = harness(80, 80, ColorFormat::Rgb888, |p| {
        let g = Gradient::new(
            GradKind::Radial {
                center: Point::new(40, 40),
                radius: 38,
                focal: Point::new(25, 25),
                focal_radius: 5,
            },
            &stops2(Color::YELLOW, RED),
        );
        grad_tile(p, Rect::from_xywh(0, 0, 80, 80), &g, 0);
    });
    assert_render_snapshot!(h, "grad_radial_focal");
}

#[test]
fn grad_conical() {
    let h = harness(80, 80, ColorFormat::Rgb888, |p| {
        let g = Gradient::new(
            GradKind::Conical {
                center: Point::new(40, 40),
                start_angle: Angle::deg(45),
                end_angle: Angle::deg(315),
            },
            &stops2(Color::WHITE, Color::hex(0x6A1B9A)),
        );
        grad_tile(p, Rect::from_xywh(0, 0, 80, 80), &g, 0);
    });
    assert_render_snapshot!(h, "grad_conical");
}

#[test]
fn grad_extend_modes() {
    let h = harness(100, 64, ColorFormat::Rgb888, |p| {
        for (i, e) in [GradExtend::Pad, GradExtend::Repeat, GradExtend::Reflect]
            .into_iter()
            .enumerate()
        {
            let g = Gradient::new(
                GradKind::Linear {
                    start: Point::new(40, 0),
                    end: Point::new(55, 0),
                },
                &stops2(Color::hex(0x2E7D32), Color::WHITE),
            )
            .extend(e);
            grad_tile(p, Rect::from_xywh(0, 2 + i as i32 * 21, 100, 18), &g, 0);
        }
    });
    assert_render_snapshot!(h, "grad_extend_modes");
}

#[test]
fn grad_dither_565() {
    let h = harness(128, 48, ColorFormat::Rgb565, |p| {
        let g = Gradient::new(GradKind::Hor, &stops2(Color::hex(0x202020), Color::hex(0x404040)));
        grad_tile(p, Rect::from_xywh(0, 0, 128, 24), &g, 0);
        grad_tile(p, Rect::from_xywh(0, 24, 128, 24), &g.dither(true), 0);
    });
    assert_render_snapshot!(h, "grad_dither_565");
}

#[test]
fn grad_rounded_rect() {
    let h = harness(120, 60, ColorFormat::Rgb565, |p| {
        let g = Gradient::new(
            GradKind::Linear {
                start: Point::new(0, 0),
                end: Point::new(110, 50),
            },
            &stops2(Color::hex(0x00ACC1), Color::hex(0x8E24AA)),
        );
        p.rect(
            Rect::from_xywh(5, 5, 110, 50),
            &RectDsc {
                radius: 18,
                bg_opa: Opa::COVER,
                bg_grad: Some(&g),
                border_width: 2,
                border_color: Color::BLACK,
                border_opa: Opa::COVER,
                ..RectDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "grad_rounded_rect");
}

fn shadow_scene(area: Rect, radius: i32, sh: ShadowDsc, bg_opa: Opa) -> impl FnOnce(&mut Painter<'_>) {
    move |p| {
        p.fill(Rect::from_xywh(0, 0, 200, 200), Color::hex(0xECEFF1), Opa::COVER);
        p.rect(
            area,
            &RectDsc {
                radius,
                shadow: sh,
                bg_opa,
                ..bg(Color::WHITE)
            },
        );
    }
}

fn sh(width: i32, ofs_x: i32, ofs_y: i32, spread: i32, opa: u8) -> ShadowDsc {
    ShadowDsc {
        width,
        ofs_x,
        ofs_y,
        spread,
        color: Color::BLACK,
        opa: Opa(opa),
    }
}

#[test]
fn shadow_basic() {
    let h = harness(
        100,
        90,
        ColorFormat::Rgb565,
        shadow_scene(
            Rect::from_xywh(20, 20, 60, 50),
            8,
            sh(20, 0, 0, 0, 200),
            Opa::COVER,
        ),
    );
    assert_render_snapshot!(h, "shadow_basic");
}

#[test]
fn shadow_offset_spread() {
    let h = harness(
        100,
        90,
        ColorFormat::Rgb565,
        shadow_scene(
            Rect::from_xywh(15, 15, 60, 50),
            6,
            sh(12, 8, 6, 4, 180),
            Opa::COVER,
        ),
    );
    assert_render_snapshot!(h, "shadow_offset_spread");
}

#[test]
fn shadow_round_button() {
    let h = harness(
        100,
        60,
        ColorFormat::Rgb565,
        shadow_scene(
            Rect::from_xywh(15, 15, 70, 30),
            RADIUS_CIRCLE,
            sh(14, 0, 3, 0, 180),
            Opa::COVER,
        ),
    );
    assert_render_snapshot!(h, "shadow_round_button");
}

#[test]
fn shadow_width_0_is_solid() {
    let h = harness(
        100,
        80,
        ColorFormat::Rgb565,
        shadow_scene(
            Rect::from_xywh(15, 15, 60, 45),
            10,
            sh(0, 6, 6, 2, 255),
            Opa::COVER,
        ),
    );
    assert_render_snapshot!(h, "shadow_width_0_is_solid");
}

#[test]
fn shadow_under_transparent_bg() {
    let h = harness(
        100,
        90,
        ColorFormat::Rgb565,
        shadow_scene(Rect::from_xywh(20, 20, 60, 50), 8, sh(16, 0, 0, 0, 200), Opa(100)),
    );
    assert_render_snapshot!(h, "shadow_under_transparent_bg");
}

#[test]
fn shadow_cache_hits_after_warmup() {
    let mut h = RenderHarness::new(200, 100, ColorFormat::Rgb565);
    let d = RectDsc {
        radius: 8,
        shadow: sh(20, 0, 0, 0, 200),
        ..bg(Color::WHITE)
    };
    h.paint(|p| p.rect(Rect::from_xywh(20, 20, 60, 50), &d));
    let before = h.caches().stats().shadow;
    h.paint(|p| {
        for i in 0..5 {
            p.rect(Rect::from_xywh(10 + i * 20, 20, 60, 50), &d);
        }
    });
    let after = h.caches().stats().shadow;
    assert_eq!(after.misses, before.misses);
    assert_eq!(after.hits, before.hits + 5);
}
