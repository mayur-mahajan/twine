//! `Painter::image` without transformation: every source format, fast paths, opacity,
//! recolor, chroma key, tiling, clip radius, bitmap mask, chunking, clipping and allocation.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

mod common;

use common::images::{Encoded, N, checker, encode};
use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::{ImageDsc, ImagePixels, Painter, read_row_argb};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{RenderHarness, assert_render_snapshot};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

/// Every buffer format the renderer can draw into.
const DESTS: [ColorFormat; 7] = [
    ColorFormat::Rgb565,
    ColorFormat::Rgb565Swapped,
    ColorFormat::Rgb888,
    ColorFormat::Xrgb8888,
    ColorFormat::Argb8888,
    ColorFormat::L8,
    ColorFormat::I1,
];

fn name(f: ColorFormat) -> String {
    f.name().to_lowercase()
}

/// The image at (8, 8) of a 48 × 48 checkerboard.
fn on_checker(dest: ColorFormat, img: &ImagePixels<'_>, dsc: &ImageDsc<'_>) -> RenderHarness {
    let mut h = RenderHarness::new(48, 48, dest);
    h.paint(|p| {
        checker(p, Rect::from_xywh(0, 0, 48, 48));
        p.image(Rect::from_xywh(8, 8, i32::from(N), i32::from(N)), img, dsc);
    });
    h
}

#[test]
fn snap_blit_each_format_on_checker() {
    for f in ColorFormat::ALL {
        let e = encode(f);
        let h = on_checker(ColorFormat::Rgb565, &e.pixels(), &ImageDsc::default());
        assert_render_snapshot!(h, &format!("image_format_{}", name(f)));
    }
}

/// The same pixels as `Argb8888` (read through `read_row_argb`).
fn as_argb(e: &Encoded) -> Vec<u8> {
    let px = e.pixels();
    let mut out = vec![0u8; usize::from(N) * usize::from(N) * 4];
    for y in 0..N {
        let s = usize::from(y) * usize::from(N) * 4;
        read_row_argb(&px, y, 0, N, &mut out[s..s + usize::from(N) * 4]);
    }
    out
}

/// Every source format on every destination format matches drawing the same pixels
/// converted to `Argb8888` first (different code paths: direct, alpha-as-coverage, generic).
#[test]
fn every_source_on_every_destination_matches_argb_reference() {
    for dest in DESTS {
        for f in ColorFormat::ALL {
            let e = encode(f);
            for dsc in [
                ImageDsc::default(),
                ImageDsc {
                    opa: Opa(160),
                    ..ImageDsc::default()
                },
            ] {
                let got = on_checker(dest, &e.pixels(), &dsc);
                let argb = as_argb(&e);
                let reference = on_checker(dest, &ImagePixels::new(ColorFormat::Argb8888, N, N, &argb), &dsc);
                let (a, b) = (got.rgb888(), reference.rgb888());
                let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
                // Pixels are mixed from identical colors and alphas, only the order of
                // conversion differs.
                assert!(worst <= 2, "{f} on {dest} opa {}: channel delta {worst}", dsc.opa.0);
            }
        }
    }
}

#[test]
fn blit_same_format_uses_copy_path() {
    let e = encode(ColorFormat::Rgb565);
    let mut h = RenderHarness::new(48, 48, ColorFormat::Rgb565);
    let area = Rect::from_xywh(4, 4, i32::from(N), i32::from(N));
    h.paint(|p| p.image(area, &e.pixels(), &ImageDsc::default()));
    assert_eq!(h.caches().stats().image_copy_rows, u32::from(N));
    // Opacity, recolor or another format take other paths.
    h.paint(|p| {
        p.image(area, &e.pixels(), &ImageDsc { opa: Opa(100), ..ImageDsc::default() });
        p.image(
            area,
            &e.pixels(),
            &ImageDsc {
                recolor: Color::RED,
                recolor_opa: Opa(100),
                ..ImageDsc::default()
            },
        );
        let e888 = encode(ColorFormat::Rgb888);
        p.image(area, &e888.pixels(), &ImageDsc::default());
    });
    assert_eq!(h.caches().stats().image_copy_rows, u32::from(N));
    // The copy is exact.
    let mut h = RenderHarness::new(N, N, ColorFormat::Rgb565);
    h.paint(|p| p.image(Rect::from_xywh(0, 0, i32::from(N), i32::from(N)), &e.pixels(), &ImageDsc::default()));
    assert_eq!(h.data(), &e.data[..]);
}

fn gallery(p: &mut Painter<'_>, e: &Encoded, a8: &Encoded) {
    let px = e.pixels();
    let size = i32::from(N);
    checker(p, Rect::from_xywh(0, 0, 176, 48));
    let at = |i: i32| Rect::from_xywh(4 + i * 43, 8, size, size);
    p.image(at(0), &px, &ImageDsc { opa: Opa(96), ..ImageDsc::default() });
    p.image(
        at(1),
        &px,
        &ImageDsc {
            recolor: Color::hex(0xE53935),
            recolor_opa: Opa(160),
            ..ImageDsc::default()
        },
    );
    // Chroma key: the white dot and the frame color disappear.
    p.image(
        at(2),
        &px,
        &ImageDsc {
            chroma_key: Some(Color::new(255, 255, 255)),
            ..ImageDsc::default()
        },
    );
    // An alpha-only image recolored.
    p.image(
        at(3),
        &a8.pixels(),
        &ImageDsc {
            recolor: Color::hex(0x1E88E5),
            recolor_opa: Opa::COVER,
            ..ImageDsc::default()
        },
    );
}

#[test]
fn snap_blit_opa_recolor_chroma() {
    let (e, a8) = (encode(ColorFormat::Argb8888), encode(ColorFormat::A8));
    let mut h = RenderHarness::new(176, 48, ColorFormat::Rgb565);
    h.paint(|p| gallery(p, &e, &a8));
    assert_render_snapshot!(h, "image_opa_recolor_chroma");
}

#[test]
fn snap_blit_tiled() {
    let e = encode(ColorFormat::Rgb565A8);
    let mut h = RenderHarness::new(100, 80, ColorFormat::Rgb565);
    h.paint(|p| {
        checker(p, Rect::from_xywh(0, 0, 100, 80));
        // Tiling starts at the area's corner and is clipped by the area.
        p.image(
            Rect::from_xywh(5, 6, 90, 70),
            &e.pixels(),
            &ImageDsc {
                tile: true,
                ..ImageDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "image_tiled");
    // The tile period is the image size.
    let rgb = h.rgb888();
    let px = |x: usize, y: usize| &rgb[(y * 100 + x) * 3..(y * 100 + x) * 3 + 3];
    assert_eq!(px(10, 10), px(10 + 32, 10 + 32));
    assert_eq!(px(20, 12), px(20 + 64, 12));
}

#[test]
fn snap_blit_clip_radius() {
    let e = encode(ColorFormat::Rgb565);
    let mut h = RenderHarness::new(80, 48, ColorFormat::Argb8888);
    h.paint(|p| {
        checker(p, Rect::from_xywh(0, 0, 80, 48));
        p.image(
            Rect::from_xywh(4, 8, 32, 32),
            &e.pixels(),
            &ImageDsc {
                clip_radius: 16,
                ..ImageDsc::default()
            },
        );
        p.image(
            Rect::from_xywh(44, 8, 32, 32),
            &e.pixels(),
            &ImageDsc {
                clip_radius: 6,
                tile: true,
                ..ImageDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "image_clip_radius");
    // The corners of the circle-clipped image show the background.
    let mut bg = RenderHarness::new(80, 48, ColorFormat::Argb8888);
    bg.paint(|p| checker(p, Rect::from_xywh(0, 0, 80, 48)));
    let (a, b) = (h.rgb888(), bg.rgb888());
    let at = |v: &[u8], x: usize, y: usize| v[(y * 80 + x) * 3..(y * 80 + x) * 3 + 3].to_vec();
    assert_eq!(at(&a, 5, 9), at(&b, 5, 9));
    assert_ne!(at(&a, 20, 24), at(&b, 20, 24));
}

#[test]
fn snap_blit_bitmap_mask() {
    let e = encode(ColorFormat::Rgb888);
    // A radial A8 mask, smaller than the image: outside it everything is transparent.
    let mut mask = vec![0u8; 28 * 28];
    for y in 0..28i32 {
        for x in 0..28i32 {
            let d2 = (x - 14) * (x - 14) + (y - 14) * (y - 14);
            mask[(y * 28 + x) as usize] = (255 - (d2 * 255 / 196).min(255)) as u8;
        }
    }
    let m = ImagePixels::new(ColorFormat::A8, 28, 28, &mask);
    let mut h = RenderHarness::new(48, 48, ColorFormat::Rgb565);
    h.paint(|p| {
        checker(p, Rect::from_xywh(0, 0, 48, 48));
        p.image(
            Rect::from_xywh(8, 8, 32, 32),
            &e.pixels(),
            &ImageDsc {
                bitmap_mask: Some(m),
                ..ImageDsc::default()
            },
        );
    });
    assert_render_snapshot!(h, "image_bitmap_mask");
}

#[test]
fn blit_chunk_equivalence() {
    let a1 = encode(ColorFormat::A1);
    let mask = vec![180u8; 30 * 30];
    for dest in [ColorFormat::Rgb565, ColorFormat::Argb8888] {
        for f in ColorFormat::ALL {
            let e = encode(f);
            let draw = |p: &mut Painter<'_>| {
                checker(p, Rect::from_xywh(0, 0, 64, 64));
                let px = e.pixels();
                p.image(Rect::from_xywh(-5, 3, 32, 32), &px, &ImageDsc::default());
                p.image(
                    Rect::from_xywh(20, 10, 40, 50),
                    &px,
                    &ImageDsc {
                        tile: true,
                        clip_radius: 9,
                        opa: Opa(200),
                        ..ImageDsc::default()
                    },
                );
                p.image(
                    Rect::from_xywh(30, 30, 32, 32),
                    &px,
                    &ImageDsc {
                        bitmap_mask: Some(ImagePixels::new(ColorFormat::A8, 30, 30, &mask)),
                        recolor: Color::GREEN,
                        recolor_opa: Opa(90),
                        ..ImageDsc::default()
                    },
                );
                p.image(
                    Rect::from_xywh(2, 40, 32, 32),
                    &px,
                    &ImageDsc {
                        bitmap_mask: Some(a1.pixels()),
                        ..ImageDsc::default()
                    },
                );
            };
            let mut one = RenderHarness::new(64, 64, dest);
            one.paint(draw);
            let mut chunked = RenderHarness::new(64, 64, dest);
            chunked.paint_chunked(16, draw);
            assert_eq!(one.data(), chunked.data(), "{f} on {dest}");
        }
    }
}

#[test]
fn blit_partially_offscreen_negative_coords() {
    let e = encode(ColorFormat::Argb8888);
    let full = RenderHarness::new(64, 64, ColorFormat::Argb8888);
    let mut h = RenderHarness::new(64, 64, ColorFormat::Argb8888);
    let mut reference = RenderHarness::new(64, 64, ColorFormat::Argb8888);
    // Top-left corner off screen.
    h.paint(|p| {
        p.image(Rect::from_xywh(-20, -10, 32, 32), &e.pixels(), &ImageDsc::default());
        p.image(Rect::from_xywh(50, 50, 32, 32), &e.pixels(), &ImageDsc::default());
        p.image(Rect::from_xywh(-100, -100, 32, 32), &e.pixels(), &ImageDsc::default());
        p.image(
            Rect::from_xywh(-7, 40, 20, 20),
            &e.pixels(),
            &ImageDsc {
                tile: true,
                ..ImageDsc::default()
            },
        );
    });
    // Reference: draw the visible part with `read_row_argb` per pixel.
    let argb = as_argb(&e);
    reference.paint(|p| {
        for (x0, y0) in [(-20, -10), (50, 50)] {
            for y in 0..32 {
                for x in 0..32 {
                    let o = (y * 32 + x) * 4;
                    let a = argb[o + 3];
                    let c = Color::new(argb[o + 2], argb[o + 1], argb[o]);
                    p.fill(Rect::from_xywh(x0 + x as i32, y0 + y as i32, 1, 1), c, Opa(a));
                }
            }
        }
    });
    let rgb = h.rgb888();
    let rf = reference.rgb888();
    for y in 0..22 {
        for x in 0..12 {
            let i = (y * 64 + x) * 3;
            assert!(rgb[i..i + 3].iter().zip(&rf[i..i + 3]).all(|(a, b)| a.abs_diff(*b) <= 1), "({x}, {y})");
        }
    }
    // The tiled image starts mid-tile at x = 0 (source column 7).
    assert_ne!(h.data(), full.data());
}

#[test]
fn no_alloc_blit() {
    let images: Vec<Encoded> = ColorFormat::ALL.iter().map(|&f| encode(f)).collect();
    let mask = vec![128u8; 32 * 32];
    let mut h = RenderHarness::new(200, 150, ColorFormat::Rgb565);
    let draw = |p: &mut Painter<'_>| {
        for (i, e) in images.iter().enumerate() {
            let at = Rect::from_xywh((i as i32 % 6) * 33, (i as i32 / 6) * 40, 60, 40);
            p.image(
                at,
                &e.pixels(),
                &ImageDsc {
                    tile: i % 2 == 0,
                    clip_radius: 5,
                    opa: Opa(200),
                    chroma_key: Some(Color::WHITE),
                    recolor: Color::BLUE,
                    recolor_opa: Opa(i as u8 * 10),
                    bitmap_mask: Some(ImagePixels::new(ColorFormat::A8, 32, 32, &mask)),
                    ..ImageDsc::default()
                },
            );
        }
    };
    h.paint(draw);
    let ((), stats) = count_allocs(|| h.paint(draw));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}
