//! No allocation while drawing rectangles, borders, gradients, shadows, masks and layers
//! (after one warm-up draw that fills the caches).

use twine_core::{Color, ColorFormat, Opa, Point, Rect};
use twine_render::{
    GradKind, GradStop, Gradient, LayerDsc, Mask, Painter, RADIUS_CIRCLE, RectDsc, ShadowDsc,
};
use twine_testing::RenderHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

fn assert_no_alloc(name: &str, mut f: impl FnMut(&mut Painter<'_>)) {
    let mut h = RenderHarness::new(200, 150, ColorFormat::Rgb565);
    h.paint(&mut f); // warm-up
    let ((), stats) = count_allocs(|| h.paint(&mut f));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{name}: {stats:?}");
}

#[test]
fn rounded_rects_do_not_allocate() {
    assert_no_alloc("rects", |p| {
        for i in 0..100 {
            let r = [2, 8, 16, 32][i % 4];
            p.rect(
                Rect::from_xywh((i as i32 * 7) % 150, (i as i32 * 5) % 100, 45, 40),
                &RectDsc {
                    radius: r,
                    bg_color: Color::RED,
                    bg_opa: Opa::COVER,
                    border_width: 3,
                    border_opa: Opa::COVER,
                    outline_width: 2,
                    outline_opa: Opa(128),
                    ..RectDsc::default()
                },
            );
        }
    });
}

#[test]
fn gradients_do_not_allocate() {
    let grads: Vec<Gradient> = [
        GradKind::Hor,
        GradKind::Ver,
        GradKind::Linear {
            start: Point::new(0, 0),
            end: Point::new(30, 30),
        },
        GradKind::Radial {
            center: Point::new(20, 20),
            radius: 20,
            focal: Point::new(10, 10),
            focal_radius: 2,
        },
        GradKind::Conical {
            center: Point::new(20, 20),
            start_angle: twine_core::Angle::deg(0),
            end_angle: twine_core::Angle::deg(270),
        },
    ]
    .into_iter()
    .map(|k| {
        Gradient::new(
            k,
            &[GradStop::new(Color::RED, 0), GradStop::new(Color::BLUE, 255)],
        )
        .dither(true)
    })
    .collect();
    assert_no_alloc("gradients", |p| {
        for i in 0..50 {
            p.rect(
                Rect::from_xywh((i * 3) % 150, (i * 2) % 100, 50, 40),
                &RectDsc {
                    radius: RADIUS_CIRCLE,
                    bg_opa: Opa::COVER,
                    bg_grad: Some(&grads[i as usize % grads.len()]),
                    ..RectDsc::default()
                },
            );
        }
    });
}

#[test]
fn shadows_do_not_allocate() {
    assert_no_alloc("shadows", |p| {
        for i in 0..20 {
            p.rect(
                Rect::from_xywh(20 + i, 20, 100, 80),
                &RectDsc {
                    radius: 10,
                    bg_opa: Opa::COVER,
                    shadow: ShadowDsc {
                        width: 20,
                        opa: Opa(160),
                        ..ShadowDsc::default()
                    },
                    ..RectDsc::default()
                },
            );
        }
    });
}

#[test]
fn masked_fills_do_not_allocate() {
    static MAP: [u8; 64] = [128; 64];
    assert_no_alloc("masks", |p| {
        let a = p.push_mask(Mask::Radius {
            area: Rect::from_xywh(10, 10, 150, 100),
            radius: 20,
            outer: false,
        });
        let b = p.push_mask(Mask::Fade {
            area: Rect::from_xywh(0, 0, 200, 150),
            y_top: 10,
            y_bottom: 120,
            opa_top: Opa::COVER,
            opa_bottom: Opa(20),
        });
        let c = p.push_mask(Mask::Map {
            area: Rect::from_xywh(40, 40, 8, 8),
            alpha: &MAP,
        });
        p.pop_mask(c);
        let d = p.push_mask(Mask::Angle {
            center: Point::new(100, 70),
            start: twine_core::Angle::deg(10),
            end: twine_core::Angle::deg(250),
        });
        p.fill(Rect::from_xywh(0, 0, 200, 150), Color::GREEN, Opa::COVER);
        p.pop_mask(d);
        p.pop_mask(b);
        p.pop_mask(a);
    });
}

#[test]
fn layers_with_strips_do_not_allocate() {
    let mut h = RenderHarness::new(200, 150, ColorFormat::Rgb565).with_config(twine_render::RenderConfig {
        layer_buf_bytes: 4000,
        ..twine_render::RenderConfig::default()
    });
    let f = |p: &mut Painter<'_>| {
        p.layer(
            Rect::from_xywh(10, 10, 150, 120),
            &LayerDsc {
                opa: Opa(128),
                ..LayerDsc::default()
            },
            |p| {
                p.rect(
                    Rect::from_xywh(20, 20, 120, 100),
                    &RectDsc {
                        radius: 12,
                        bg_opa: Opa::COVER,
                        ..RectDsc::default()
                    },
                );
            },
        );
    };
    h.paint(f);
    let ((), stats) = count_allocs(|| h.paint(f));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}
