//! `Painter::image` with rotation and scale: bounds, identities, snapshots, chunking and a
//! property test that nothing is written outside the clip.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

mod common;

use common::images::{N, checker, encode};
use proptest::prelude::*;
use twine_core::{Angle, Color, ColorFormat, Opa, Point, Rect, Scale};
use twine_render::{ImageDsc, Painter, transformed_area};
use twine_testing::{RenderHarness, assert_render_snapshot};

const SIZE: i32 = N as i32;

fn center() -> Point {
    Point::new(SIZE / 2, SIZE / 2)
}

#[test]
fn transformed_area_rotation_90_swaps_wh() {
    let a = Rect::from_xywh(10, 20, 100, 50);
    let r = transformed_area(a, Angle::deg(90), Scale::ONE, Scale::ONE, Point::new(50, 25));
    assert_eq!((r.width(), r.height()), (50, 100));
    // Same center.
    assert_eq!((r.x0 + r.x1, r.y0 + r.y1), (a.x0 + a.x1, a.y0 + a.y1));
    let r = transformed_area(a, Angle::deg(45), Scale::ONE, Scale::ONE, Point::new(50, 25));
    assert!(r.width() >= 106 && r.width() <= 108, "{r}");
}

#[test]
fn transformed_area_scale_2x() {
    let a = Rect::from_xywh(0, 0, 40, 20);
    let r = transformed_area(a, Angle(0), Scale(512), Scale(512), Point::new(20, 10));
    assert_eq!(r, Rect::new(-20, -10, 60, 30));
    let r = transformed_area(a, Angle(0), Scale(512), Scale(256), Point::new(0, 0));
    assert_eq!(r, Rect::new(0, 0, 80, 20));
}

fn draw(dsc: &ImageDsc<'_>, f: ColorFormat) -> RenderHarness {
    let e = encode(f);
    let mut h = RenderHarness::new(64, 64, ColorFormat::Rgb565);
    h.paint(|p| {
        checker(p, Rect::from_xywh(0, 0, 64, 64));
        p.image(Rect::from_xywh(16, 16, SIZE, SIZE), &e.pixels(), dsc);
    });
    h
}

#[test]
fn rotate_0_equals_untransformed_blit() {
    for f in [ColorFormat::Argb8888, ColorFormat::Rgb565, ColorFormat::I4, ColorFormat::A8] {
        let plain = draw(&ImageDsc::default(), f);
        let pivot = draw(
            &ImageDsc {
                angle: Angle(0),
                pivot: Point::new(3, 29),
                ..ImageDsc::default()
            },
            f,
        );
        assert_eq!(plain.data(), pivot.data(), "{f}");
    }
}

#[test]
fn rotate_360_equals_0() {
    let a = draw(&ImageDsc::default(), ColorFormat::Argb8888);
    for deg in [360, -360, 720] {
        let b = draw(
            &ImageDsc {
                angle: Angle::deg(deg),
                pivot: center(),
                ..ImageDsc::default()
            },
            ColorFormat::Argb8888,
        );
        assert_eq!(a.data(), b.data(), "{deg}");
    }
}

#[test]
fn scale_256_identity() {
    let a = draw(&ImageDsc::default(), ColorFormat::Rgb565A8);
    let b = draw(
        &ImageDsc {
            scale_x: Scale(256),
            scale_y: Scale(256),
            pivot: Point::new(5, 7),
            ..ImageDsc::default()
        },
        ColorFormat::Rgb565A8,
    );
    assert_eq!(a.data(), b.data());
    // A 90° rotation without AA moves pixels exactly: the image's top-left pixel lands at
    // the top-right of the rotated square.
    let e = encode(ColorFormat::Argb8888);
    let mut h = RenderHarness::new(64, 64, ColorFormat::Argb8888);
    h.paint(|p| {
        p.image(
            Rect::from_xywh(16, 16, SIZE, SIZE),
            &e.pixels(),
            &ImageDsc {
                angle: Angle::deg(90),
                pivot: center(),
                antialias: false,
                ..ImageDsc::default()
            },
        );
    });
    let rgb = h.rgb888();
    let px = |x: usize, y: usize| rgb[(y * 64 + x) * 3..(y * 64 + x) * 3 + 3].to_vec();
    // Pixel (13, 5) of the image is white (the dot); rotated 90° clockwise around the center
    // it lands at (31 - 5, 13) = (26, 13) of the square.
    assert_eq!(px(16 + 26, 16 + 13), vec![255, 255, 255]);
}

#[test]
fn snap_rotate_30_aa() {
    let h = draw(
        &ImageDsc {
            angle: Angle::deg(30),
            pivot: center(),
            ..ImageDsc::default()
        },
        ColorFormat::Argb8888,
    );
    assert_render_snapshot!(h, "image_rotate_30_aa");
}

#[test]
fn snap_rotate_30_nearest() {
    let h = draw(
        &ImageDsc {
            angle: Angle::deg(30),
            pivot: center(),
            antialias: false,
            ..ImageDsc::default()
        },
        ColorFormat::Rgb565,
    );
    assert_render_snapshot!(h, "image_rotate_30_nearest");
}

#[test]
fn snap_zoom_150_pivot_center() {
    let h = draw(
        &ImageDsc {
            scale_x: Scale::from_percent(150),
            scale_y: Scale::from_percent(150),
            pivot: center(),
            ..ImageDsc::default()
        },
        ColorFormat::I8,
    );
    assert_render_snapshot!(h, "image_zoom_150_pivot_center");
}

#[test]
fn snap_zoom_50() {
    let h = draw(
        &ImageDsc {
            scale_x: Scale::from_percent(50),
            scale_y: Scale::from_percent(50),
            ..ImageDsc::default()
        },
        ColorFormat::Rgb565A8,
    );
    assert_render_snapshot!(h, "image_zoom_50");
}

#[test]
fn snap_rotate_recolor_clip_radius() {
    // Recolor, chroma key and clip radius also apply to transformed images.
    let h = draw(
        &ImageDsc {
            angle: Angle::deg(-20),
            scale_x: Scale::from_percent(120),
            scale_y: Scale::from_percent(80),
            pivot: center(),
            recolor: Color::hex(0x43A047),
            recolor_opa: Opa(128),
            clip_radius: 10,
            chroma_key: Some(Color::WHITE),
            ..ImageDsc::default()
        },
        ColorFormat::Xrgb8888,
    );
    assert_render_snapshot!(h, "image_rotate_recolor_clip_radius");
}

#[test]
fn transform_chunk_equivalence() {
    for f in ColorFormat::ALL {
        let e = encode(f);
        let paint = |p: &mut Painter<'_>| {
            checker(p, Rect::from_xywh(0, 0, 64, 64));
            for (i, deg) in [17, 135, 250].into_iter().enumerate() {
                p.image(
                    Rect::from_xywh(i as i32 * 12, i as i32 * 10, SIZE, SIZE),
                    &e.pixels(),
                    &ImageDsc {
                        angle: Angle::deg(deg),
                        scale_x: Scale(200 + i as u16 * 60),
                        scale_y: Scale(300 - i as u16 * 30),
                        pivot: Point::new(10, 20),
                        antialias: i != 1,
                        clip_radius: 6 * i as i32,
                        ..ImageDsc::default()
                    },
                );
            }
        };
        let mut one = RenderHarness::new(64, 64, ColorFormat::Rgb565);
        one.paint(paint);
        let mut chunked = RenderHarness::new(64, 64, ColorFormat::Rgb565);
        chunked.paint_chunked(7, paint);
        assert_eq!(one.data(), chunked.data(), "{f}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn transform_never_panics_or_writes_outside_clip(
        angle in -7200i32..7200,
        sx in 0u16..1200,
        sy in 0u16..1200,
        px in -50i32..80,
        py in -50i32..80,
        x in -40i32..60,
        y in -40i32..60,
        clip in (0i32..40, 0i32..40, 0i32..40, 0i32..40),
        aa in any::<bool>(),
        fi in 0usize..16,
    ) {
        let e = encode(ColorFormat::ALL[fi]);
        let mut h = RenderHarness::new(48, 48, ColorFormat::Argb8888);
        h.clear(Color::hex(0x123456));
        let before = h.data().to_vec();
        let clip = Rect::new(clip.0, clip.1, clip.0 + clip.2, clip.1 + clip.3);
        h.paint(|p| {
            p.with_clip(clip, |p| {
                p.image(
                    Rect::from_xywh(x, y, SIZE, SIZE),
                    &e.pixels(),
                    &ImageDsc {
                        angle: Angle(angle),
                        scale_x: Scale(sx),
                        scale_y: Scale(sy),
                        pivot: Point::new(px, py),
                        antialias: aa,
                        ..ImageDsc::default()
                    },
                );
            });
        });
        for (i, (a, b)) in h.data().chunks_exact(4).zip(before.chunks_exact(4)).enumerate() {
            let p = Point::new((i % 48) as i32, (i / 48) as i32);
            if !clip.contains(p) {
                prop_assert_eq!(a, b, "pixel {:?} outside clip {} changed", p, clip);
            }
        }
    }
}
