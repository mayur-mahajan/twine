//! Blending core and `Painter::fill`.

use std::cell::Cell;

use proptest::prelude::*;
use twine_core::color::{Argb8888, L8, Rgb565, Rgb565Swapped, Rgb888, Xrgb8888};
use twine_core::math::udiv255;
use twine_core::{Color, ColorFormat, Opa, PixelFormat, Rect};
use twine_render::{AccelResult, BlendMode, DrawAccel, DrawBuf, ImagePixels, Painter, Source, blend_span};
use twine_testing::RenderHarness;

const FORMATS: [ColorFormat; 7] = [
    ColorFormat::Rgb565,
    ColorFormat::Rgb565Swapped,
    ColorFormat::Rgb888,
    ColorFormat::Xrgb8888,
    ColorFormat::Argb8888,
    ColorFormat::L8,
    ColorFormat::I1,
];

#[test]
fn fill_opaque_writes_exact_raw() {
    let c = Color::new(0x12, 0x9A, 0xF0);
    for f in FORMATS {
        let mut h = RenderHarness::new(16, 2, f);
        h.paint(|p| p.fill(Rect::from_xywh(0, 0, 16, 2), c, Opa::COVER));
        let expect: Vec<u8> = match f {
            ColorFormat::Rgb565 => Rgb565::from_color(c).to_le_bytes().to_vec(),
            ColorFormat::Rgb565Swapped => Rgb565Swapped::from_color(c).to_le_bytes().to_vec(),
            ColorFormat::Rgb888 => Rgb888::from_color(c).to_vec(),
            ColorFormat::Xrgb8888 => Xrgb8888::from_color(c).to_le_bytes().to_vec(),
            ColorFormat::Argb8888 => Argb8888::from_color(c).to_le_bytes().to_vec(),
            ColorFormat::L8 => vec![L8::from_color(c)],
            // Luminance of the color < 128 → bits 0.
            _ => vec![if c.luminance() >= 128 { 0xFF } else { 0x00 }],
        };
        for px in h.data().chunks(expect.len()) {
            assert_eq!(px, &expect[..], "{f}");
        }
    }
}

#[test]
fn fill_opa_50_matches_color_mix() {
    let (fg, bg) = (Color::new(200, 40, 90), Color::new(10, 220, 130));
    for f in [ColorFormat::Rgb888, ColorFormat::Xrgb8888, ColorFormat::Argb8888] {
        let mut h = RenderHarness::new(4, 1, f);
        h.clear(bg);
        h.paint(|p| p.fill(Rect::from_xywh(0, 0, 4, 1), fg, Opa::P50));
        let m = Color::mix(fg, bg, Opa::P50);
        let rgb = h.rgb888();
        assert_eq!(&rgb[..3], &[m.r, m.g, m.b], "{f}");
    }
    // RGB565: the normative `Rgb565::mix` of the quantized colors.
    let mut h = RenderHarness::new(4, 1, ColorFormat::Rgb565);
    h.clear(bg);
    h.paint(|p| p.fill(Rect::from_xywh(0, 0, 4, 1), fg, Opa::P50));
    let expect = Rgb565::mix(Rgb565::from_color(fg), Rgb565::from_color(bg), Opa::P50);
    assert_eq!(&h.data()[..2], &expect.to_le_bytes());
}

#[test]
fn mask_zero_leaves_dst() {
    let mut row = [7u8; 3 * 4];
    let before = row;
    blend_span::<Rgb888>(
        &mut row,
        4,
        Source::Solid(Color::RED),
        Some(&[0, 0, 1, 2]),
        Opa::COVER,
        BlendMode::Normal,
    );
    assert_eq!(row, before);
    blend_span::<Rgb888>(
        &mut row,
        4,
        Source::Solid(Color::RED),
        Some(&[0, 255, 0, 0]),
        Opa::COVER,
        BlendMode::Normal,
    );
    assert_eq!(&row[3..6], &[0, 0, 255]);
    assert_eq!(&row[..3], &before[..3]);
}

#[test]
fn additive_saturates() {
    let mut row = Rgb888::from_color(Color::new(200, 100, 0)).to_vec();
    blend_span::<Rgb888>(
        &mut row,
        1,
        Source::Solid(Color::new(100, 100, 100)),
        None,
        Opa::COVER,
        BlendMode::Additive,
    );
    assert_eq!(
        Rgb888::to_color([row[0], row[1], row[2]]),
        Color::new(255, 200, 100)
    );
    blend_span::<Rgb888>(
        &mut row,
        1,
        Source::Solid(Color::new(100, 250, 0)),
        None,
        Opa::COVER,
        BlendMode::Subtractive,
    );
    assert_eq!(
        Rgb888::to_color([row[0], row[1], row[2]]),
        Color::new(155, 0, 100)
    );
    blend_span::<Rgb888>(
        &mut row,
        1,
        Source::Solid(Color::new(200, 50, 100)),
        None,
        Opa::COVER,
        BlendMode::Difference,
    );
    assert_eq!(Rgb888::to_color([row[0], row[1], row[2]]), Color::new(45, 50, 0));
}

#[test]
fn multiply_by_white_is_identity() {
    let c = Color::new(12, 150, 250);
    let mut row = Rgb888::from_color(c).to_vec();
    blend_span::<Rgb888>(
        &mut row,
        1,
        Source::Solid(Color::WHITE),
        None,
        Opa::COVER,
        BlendMode::Multiply,
    );
    assert_eq!(Rgb888::to_color([row[0], row[1], row[2]]), c);
    blend_span::<Rgb888>(
        &mut row,
        1,
        Source::Solid(Color::BLACK),
        None,
        Opa::COVER,
        BlendMode::Multiply,
    );
    assert_eq!(Rgb888::to_color([row[0], row[1], row[2]]), Color::BLACK);
}

#[test]
fn pixels_same_format_copies() {
    let src: Vec<u8> = (0..40u8).collect();
    let mut dst = vec![0u8; 40];
    blend_span::<Rgb565>(
        &mut dst,
        20,
        Source::Pixels {
            data: &src,
            format: ColorFormat::Rgb565,
        },
        None,
        Opa::COVER,
        BlendMode::Normal,
    );
    assert_eq!(dst, src);
    // A different format converts.
    let mut dst = vec![0u8; 3];
    blend_span::<Rgb888>(
        &mut dst,
        1,
        Source::Pixels {
            data: &[0x00, 0xF8],
            format: ColorFormat::Rgb565,
        },
        None,
        Opa::COVER,
        BlendMode::Normal,
    );
    assert_eq!(dst, [0, 0, 255]);
    // An image blit uses the same path.
    let mut h = RenderHarness::new(4, 1, ColorFormat::Rgb565);
    let img = ImagePixels::new(ColorFormat::Rgb565, 2, 1, &[0x00, 0xF8, 0x1F, 0x00]);
    h.paint(|p| p.blit(twine_core::Point::new(1, 0), &img, Opa::COVER, BlendMode::Normal));
    assert_eq!(h.data(), &[0xFF, 0xFF, 0x00, 0xF8, 0x1F, 0x00, 0xFF, 0xFF]);
}

#[test]
fn fill_respects_clip() {
    let mut h = RenderHarness::new(8, 8, ColorFormat::L8);
    h.clear(Color::BLACK);
    h.paint(|p| {
        p.with_clip(Rect::from_xywh(2, 2, 3, 3), |p| {
            p.fill(Rect::from_xywh(0, 0, 8, 8), Color::WHITE, Opa::COVER);
        });
        assert_eq!(p.touched_area(), Some(Rect::from_xywh(2, 2, 3, 3)));
    });
    for y in 0..8 {
        for x in 0..8 {
            let inside = (2..5).contains(&x) && (2..5).contains(&y);
            assert_eq!(h.data()[y * 8 + x], if inside { 255 } else { 0 }, "({x},{y})");
        }
    }
}

struct FakeAccel {
    fills: Cell<u32>,
    waits: Cell<u32>,
    result: AccelResult,
}

impl DrawAccel for FakeAccel {
    fn fill(&mut self, dst: &mut DrawBuf<'_>, area: Rect, color: Color, _opa: Opa) -> AccelResult {
        self.fills.set(self.fills.get() + 1);
        if self.result != AccelResult::Unsupported {
            // A "hardware" fill: write L8 directly.
            for y in area.y0..area.y1 {
                dst.row_mut(y, area.x0, area.x1).fill(color.luminance());
            }
        }
        self.result
    }
    fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult {
        AccelResult::Unsupported
    }
    fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult {
        AccelResult::Unsupported
    }
    fn wait(&mut self) {
        self.waits.set(self.waits.get() + 1);
    }
}

#[test]
fn accel_fill_called_for_large_area() {
    for result in [AccelResult::Done, AccelResult::Queued, AccelResult::Unsupported] {
        let mut accel = FakeAccel {
            fills: Cell::new(0),
            waits: Cell::new(0),
            result,
        };
        let mut caches = twine_render::RenderCaches::default();
        let mut data = vec![0u8; 64 * 64];
        {
            let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 64, 64)).unwrap();
            let mut p = Painter::new(buf, &mut caches).with_accel(&mut accel);
            p.fill(Rect::from_xywh(0, 0, 10, 10), Color::WHITE, Opa::COVER); // 100 px: software
            p.fill(Rect::from_xywh(0, 16, 64, 32), Color::WHITE, Opa::COVER); // 2048 px: accel
            p.fill(Rect::from_xywh(0, 60, 4, 4), Color::WHITE, Opa::COVER); // software write
        }
        assert_eq!(accel.fills.get(), 1, "{result:?}");
        assert_eq!(
            accel.waits.get(),
            u32::from(result == AccelResult::Queued),
            "{result:?}"
        );
        assert!(
            data[16 * 64..48 * 64].iter().all(|&v| v == 255),
            "{result:?}: filled either way"
        );
    }
}

// ---- reference implementation for the proptest ----

fn ref_eff(m: u8, opa: u8) -> u8 {
    udiv255(u32::from(m) * u32::from(opa)) as u8
}

/// Slow reference: per pixel, from first principles.
fn reference<F: PixelFormat>(
    dst: &mut [u8],
    n: usize,
    src: &Src,
    mask: Option<&[u8]>,
    opa: u8,
    mode: BlendMode,
) {
    if opa <= 2 {
        return;
    }
    let opa = if opa >= 253 { 255 } else { opa };
    for i in 0..n {
        let m = mask.map_or(255, |m| m[i]);
        let (fg, sa) = match src {
            // Solid colors are quantized to the destination format first.
            Src::Solid(c) => (F::to_color(F::from_color(*c)), 255u8),
            Src::Argb(v) => (Color::new(v[i * 4 + 2], v[i * 4 + 1], v[i * 4]), v[i * 4 + 3]),
            Src::Rgb888(v) => (Color::new(v[i * 3 + 2], v[i * 3 + 1], v[i * 3]), 255),
        };
        let a = ref_eff(ref_eff(sa, m), opa);
        if a <= 2 {
            continue;
        }
        let px = &mut dst[i * F::BYTES..(i + 1) * F::BYTES];
        if F::FORMAT == ColorFormat::Argb8888 {
            let (dc, da) = (Color::new(px[2], px[1], px[0]), px[3]);
            let fg = if da == 0 { fg } else { mode.color(fg, dc) };
            let (c, oa) = if a >= 253 {
                (fg, 255)
            } else if da <= 2 {
                (fg, a)
            } else if da == 255 {
                (Color::mix(fg, dc, Opa(a)), 255)
            } else {
                let ra = 255 - ref_eff(255 - a, 255 - da);
                let ratio = (u32::from(a) * 255 / u32::from(ra)).min(255) as u8;
                (Color::mix(fg, dc, Opa(ratio)), ra)
            };
            px.copy_from_slice(&[c.b, c.g, c.r, oa]);
        } else {
            let bg = F::to_color(F::read(px));
            let fg = mode.color(fg, bg);
            let out = if a >= 253 { fg } else { Color::mix(fg, bg, Opa(a)) };
            F::write(px, F::from_color(out));
        }
    }
}

#[derive(Clone, Debug)]
enum Src {
    Solid(Color),
    Argb(Vec<u8>),
    Rgb888(Vec<u8>),
}

fn run<F: PixelFormat>(dst0: &[u8], n: usize, src: &Src, mask: Option<&[u8]>, opa: u8, mode: BlendMode) {
    let mut a = dst0[..n * F::BYTES].to_vec();
    let mut b = a.clone();
    let s = match src {
        Src::Solid(c) => Source::Solid(*c),
        Src::Argb(v) => Source::Argb(v),
        Src::Rgb888(v) => Source::Pixels {
            data: v,
            format: ColorFormat::Rgb888,
        },
    };
    blend_span::<F>(&mut a, n, s, mask, Opa(opa), mode);
    reference::<F>(&mut b, n, src, mask, opa, mode);
    assert_eq!(a, b, "{:?} {src:?} mask {mask:?} opa {opa} {mode:?}", F::FORMAT);
}

proptest! {
    #[test]
    fn blend_span_equals_reference(
        fmt in 0usize..6,
        n in 1usize..24,
        dst in proptest::collection::vec(any::<u8>(), 24 * 4),
        srcbytes in proptest::collection::vec(any::<u8>(), 24 * 4),
        kind in 0u8..3,
        use_mask in any::<bool>(),
        mask in proptest::collection::vec(prop_oneof![Just(0u8), Just(255u8), any::<u8>()], 24),
        opa in prop_oneof![Just(255u8), Just(0u8), any::<u8>()],
        mode in 0usize..5,
    ) {
        let src = match kind {
            0 => Src::Solid(Color::new(srcbytes[0], srcbytes[1], srcbytes[2])),
            1 => Src::Argb(srcbytes.clone()),
            _ => Src::Rgb888(srcbytes[..72].to_vec()),
        };
        let mask = use_mask.then_some(&mask[..]);
        let mode = BlendMode::ALL[mode];
        match fmt {
            0 => run::<Rgb565>(&dst, n, &src, mask, opa, mode),
            1 => run::<Rgb565Swapped>(&dst, n, &src, mask, opa, mode),
            2 => run::<Rgb888>(&dst, n, &src, mask, opa, mode),
            3 => run::<Xrgb8888>(&dst, n, &src, mask, opa, mode),
            4 => run::<Argb8888>(&dst, n, &src, mask, opa, mode),
            _ => run::<L8>(&dst, n, &src, mask, opa, mode),
        }
    }
}
