//! Snapshots of masks and layers.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use twine_core::{Angle, Color, ColorFormat, Opa, Point, Rect};
use twine_render::{
    BlendMode, LayerDsc, LineSide, Mask, MaskId, Painter, RADIUS_CIRCLE, RectDsc, RenderConfig,
};
use twine_testing::{RenderHarness, assert_render_snapshot};

const PAL: [Color; 6] = [
    Color::hex(0xE53935),
    Color::hex(0xFB8C00),
    Color::hex(0xFDD835),
    Color::hex(0x43A047),
    Color::hex(0x1E88E5),
    Color::hex(0x8E24AA),
];

fn stripes(p: &mut Painter<'_>, r: Rect) {
    let mut x = r.x0;
    let mut i = 0;
    while x < r.x1 {
        p.fill(
            Rect::new(x, r.y0, (x + 5).min(r.x1), r.y1),
            PAL[i % PAL.len()],
            Opa::COVER,
        );
        x += 5;
        i += 1;
    }
}

fn masked(name: &str, m: Mask<'static>) {
    let mut h = RenderHarness::new(80, 64, ColorFormat::Rgb565);
    h.paint(|p| {
        let id = p.push_mask(m);
        stripes(p, Rect::from_xywh(0, 0, 80, 64));
        p.pop_mask(id);
        // After popping, drawing is unmasked again.
        p.fill(Rect::from_xywh(0, 60, 8, 4), Color::BLACK, Opa::COVER);
    });
    h.assert_snapshot_with(&twine_testing::snapshot_config!(), name);
}

#[test]
fn mask_radius_clip_children() {
    masked(
        "mask_radius_clip_children",
        Mask::Radius {
            area: Rect::from_xywh(6, 4, 68, 52),
            radius: 18,
            outer: false,
        },
    );
    masked(
        "mask_radius_outer",
        Mask::Radius {
            area: Rect::from_xywh(6, 4, 68, 52),
            radius: RADIUS_CIRCLE,
            outer: true,
        },
    );
}

#[test]
fn mask_angle_quarter() {
    masked(
        "mask_angle_quarter",
        Mask::Angle {
            center: Point::new(40, 32),
            start: Angle::deg(270),
            end: Angle::deg(360),
        },
    );
    masked(
        "mask_angle_wide",
        Mask::Angle {
            center: Point::new(40, 32),
            start: Angle::deg(30),
            end: Angle::deg(300),
        },
    );
}

#[test]
fn mask_line_diagonal() {
    masked(
        "mask_line_diagonal",
        Mask::Line {
            p1: Point::new(0, 60),
            p2: Point::new(80, 10),
            side: LineSide::Bottom,
        },
    );
}

#[test]
fn mask_fade_vertical() {
    masked(
        "mask_fade_vertical",
        Mask::Fade {
            area: Rect::from_xywh(10, 0, 60, 64),
            y_top: 8,
            y_bottom: 56,
            opa_top: Opa::COVER,
            opa_bottom: Opa::TRANSP,
        },
    );
}

static CHECKER: [u8; 40 * 32] = {
    let mut a = [0u8; 40 * 32];
    let mut i = 0;
    while i < a.len() {
        let (x, y) = (i % 40, i / 40);
        a[i] = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 60 };
        i += 1;
    }
    a
};

#[test]
fn mask_map_checker() {
    masked(
        "mask_map_checker",
        Mask::Map {
            area: Rect::from_xywh(20, 16, 40, 32),
            alpha: &CHECKER,
        },
    );
}

#[test]
fn mask_stack_radius_and_fade() {
    let mut h = RenderHarness::new(80, 64, ColorFormat::Rgb565);
    h.paint(|p| {
        let a = p.push_mask(Mask::Radius {
            area: Rect::from_xywh(4, 4, 72, 56),
            radius: RADIUS_CIRCLE,
            outer: false,
        });
        let b = p.push_mask(Mask::Fade {
            area: Rect::from_xywh(0, 0, 80, 64),
            y_top: 4,
            y_bottom: 60,
            opa_top: Opa::COVER,
            opa_bottom: Opa(30),
        });
        stripes(p, Rect::from_xywh(0, 0, 80, 64));
        // Masks apply to every primitive, not just fills.
        p.rect(
            Rect::from_xywh(20, 20, 40, 30),
            &RectDsc {
                radius: 6,
                bg_color: Color::WHITE,
                bg_opa: Opa::COVER,
                ..RectDsc::default()
            },
        );
        p.pop_mask(b);
        p.pop_mask(a);
    });
    assert_render_snapshot!(h, "mask_stack_radius_and_fade");
}

#[test]
fn stack_overflow_warns_and_ignores() {
    let mut h = RenderHarness::new(8, 8, ColorFormat::L8);
    h.clear(Color::BLACK);
    h.paint(|p| {
        let area = Rect::from_xywh(0, 0, 8, 8);
        let ids: Vec<MaskId> = (0..twine_render::MAX_MASKS)
            .map(|_| {
                p.push_mask(Mask::Radius {
                    area,
                    radius: 0,
                    outer: false,
                })
            })
            .collect();
        assert!(ids.iter().all(|&id| id != MaskId::INVALID));
        // A 17th mask (hiding everything) is ignored.
        let id = p.push_mask(Mask::Radius {
            area: Rect::ZERO,
            radius: 0,
            outer: false,
        });
        assert_eq!(id, MaskId::INVALID);
        p.pop_mask(id);
        p.fill(area, Color::WHITE, Opa::COVER);
        assert!(p.has_masks());
    });
    assert!(h.data().iter().all(|&v| v == 255));
}

fn pair(p: &mut Painter<'_>, opa: Opa) {
    p.fill(Rect::from_xywh(5, 5, 40, 30), PAL[0], opa);
    p.fill(Rect::from_xywh(25, 20, 40, 30), PAL[4], opa);
}

#[test]
fn layer_opa_group() {
    let mut h = RenderHarness::new(140, 56, ColorFormat::Rgb565);
    h.paint(|p| {
        pair(p, Opa(128));
        p.layer(
            Rect::from_xywh(70, 0, 70, 56),
            &LayerDsc {
                opa: Opa(128),
                ..LayerDsc::default()
            },
            |p| {
                p.fill(Rect::from_xywh(75, 5, 40, 30), PAL[0], Opa::COVER);
                p.fill(Rect::from_xywh(95, 20, 40, 30), PAL[4], Opa::COVER);
            },
        );
    });
    // In the layer the overlap has exactly the second rect's color at 50 %.
    let rgb = h.rgb888();
    let px = |x: usize, y: usize| &rgb[(y * 140 + x) * 3..(y * 140 + x) * 3 + 3];
    assert_eq!(px(105, 25), px(125, 45));
    assert_ne!(px(35, 25), px(55, 45));
    assert_render_snapshot!(h, "layer_opa_group");
}

#[test]
fn layer_blend_modes() {
    let mut h = RenderHarness::new(200, 44, ColorFormat::Rgb565);
    h.paint(|p| {
        stripes(p, Rect::from_xywh(0, 0, 200, 44));
        for (i, mode) in BlendMode::ALL.into_iter().enumerate() {
            let a = Rect::from_xywh(4 + i as i32 * 40, 4, 36, 36);
            p.layer(
                a,
                &LayerDsc {
                    opa: Opa(230),
                    blend_mode: mode,
                    transform: None,
                },
                |p| {
                    p.rect(
                        a,
                        &RectDsc {
                            radius: 8,
                            bg_color: Color::hex(0x808080),
                            bg_opa: Opa::COVER,
                            ..RectDsc::default()
                        },
                    );
                },
            );
        }
    });
    assert_render_snapshot!(h, "layer_blend_modes");
}

fn layer_scene(p: &mut Painter<'_>) {
    stripes(p, Rect::from_xywh(0, 0, 100, 60));
    p.layer(
        Rect::from_xywh(10, 5, 80, 50),
        &LayerDsc {
            opa: Opa(160),
            ..LayerDsc::default()
        },
        |p| {
            p.rect(
                Rect::from_xywh(10, 5, 80, 50),
                &RectDsc {
                    radius: 14,
                    bg_color: Color::WHITE,
                    bg_opa: Opa::COVER,
                    border_width: 3,
                    border_color: Color::BLACK,
                    border_opa: Opa::COVER,
                    ..RectDsc::default()
                },
            );
            p.fill(Rect::from_xywh(20, 20, 60, 10), PAL[5], Opa(200));
        },
    );
}

#[test]
fn layer_strips_equal_single() {
    let mut big = RenderHarness::new(100, 60, ColorFormat::Rgb565);
    big.paint(layer_scene);
    // 80 px × 4 B = 320 B per row; 3200 B → 10 rows per strip → 5 strips.
    let mut small = RenderHarness::new(100, 60, ColorFormat::Rgb565).with_config(RenderConfig {
        layer_buf_bytes: 3200,
        ..RenderConfig::default()
    });
    small.paint(layer_scene);
    assert!(big.rgb888() == small.rgb888());
    assert_render_snapshot!(big, "layer_strips_equal_single");
}

#[test]
fn layer_budget_too_small_falls_back() {
    let mut h = RenderHarness::new(100, 60, ColorFormat::Rgb565).with_config(RenderConfig {
        layer_buf_bytes: 100,
        ..RenderConfig::default()
    });
    h.paint(layer_scene);
    // Without a layer the content is drawn directly (no group opacity).
    let mut direct = RenderHarness::new(100, 60, ColorFormat::Rgb565);
    direct.paint(|p| {
        stripes(p, Rect::from_xywh(0, 0, 100, 60));
        p.rect(
            Rect::from_xywh(10, 5, 80, 50),
            &RectDsc {
                radius: 14,
                bg_color: Color::WHITE,
                bg_opa: Opa::COVER,
                border_width: 3,
                border_color: Color::BLACK,
                border_opa: Opa::COVER,
                ..RectDsc::default()
            },
        );
        p.fill(Rect::from_xywh(20, 20, 60, 10), PAL[5], Opa(200));
    });
    assert!(h.rgb888() == direct.rgb888());
}

#[test]
fn nested_layer_draws_into_outer() {
    let mut h = RenderHarness::new(40, 20, ColorFormat::Rgb888);
    h.paint(|p| {
        p.layer(
            Rect::from_xywh(0, 0, 40, 20),
            &LayerDsc {
                opa: Opa(128),
                ..LayerDsc::default()
            },
            |p| {
                p.layer(Rect::from_xywh(0, 0, 20, 20), &LayerDsc::default(), |p| {
                    p.fill(Rect::from_xywh(0, 0, 20, 20), Color::BLACK, Opa::COVER);
                });
            },
        );
    });
    let rgb = h.rgb888();
    assert_eq!(&rgb[..3], &[127, 127, 127]);
    assert_eq!(&rgb[30 * 3..31 * 3], &[255, 255, 255]);
}
