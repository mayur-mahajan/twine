//! Image source formats, buffer formats and small API surface checks.

use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Point, Rect, Transform};
use twine_render::{
    BlendMode, BlitDsc, ImagePixels, RenderCaches, RenderConfig, Source, blend_span, is_draw_format,
    is_format_enabled,
};
use twine_testing::RenderHarness;

/// A 2 × 2 image of `format`: pixel (0, 0) red, the others per format.
fn image(format: ColorFormat) -> (Vec<u8>, Option<Vec<u8>>) {
    let red565 = Color::RED.to_rgb565();
    match format {
        ColorFormat::Rgb565 => (red565.to_le_bytes().repeat(4), None),
        ColorFormat::Rgb565Swapped => (red565.swap_bytes().to_le_bytes().repeat(4), None),
        ColorFormat::Rgb888 => ([0u8, 0, 255].repeat(4), None),
        ColorFormat::Xrgb8888 => ([0u8, 0, 255, 0].repeat(4), None),
        ColorFormat::Argb8888 => ([0u8, 0, 255, 255].repeat(4), None),
        ColorFormat::Argb8888Premultiplied => ([0u8, 0, 255, 255, 0, 0, 128, 128].repeat(2), None),
        ColorFormat::L8 | ColorFormat::A8 => (vec![255; 4], None),
        ColorFormat::A4 => (vec![0xFF; 2], None),
        ColorFormat::A2 => (vec![0xF0; 2], None),
        ColorFormat::A1 => (vec![0xC0; 2], None),
        ColorFormat::I1 | ColorFormat::I2 | ColorFormat::I4 => {
            (vec![0; 2], Some([0u8, 0, 255, 255].repeat(16)))
        }
        ColorFormat::I8 => (vec![0; 4], Some([0u8, 0, 255, 255].repeat(256))),
        ColorFormat::Rgb565A8 => {
            let mut v = red565.to_le_bytes().repeat(4);
            v.extend_from_slice(&[255; 4]);
            (v, None)
        }
    }
}

#[test]
fn every_source_format_blits() {
    for f in ColorFormat::ALL {
        let (data, palette) = image(f);
        let mut img = ImagePixels::new(f, 2, 2, &data);
        img.palette = palette.as_deref();
        let expect = match f {
            ColorFormat::L8 => Color::WHITE,
            ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8 => Color::GREEN,
            _ => Color::RED,
        };
        let rc = f.is_alpha_only().then_some((Color::GREEN, Opa::COVER));
        // Untransformed (direct or converted rows) and transformed (texel fetch).
        for transformed in [false, true] {
            let mut h = RenderHarness::new(8, 8, ColorFormat::Rgb888);
            h.paint(|p| {
                if transformed {
                    p.blit_transformed(
                        &img,
                        &BlitDsc {
                            transform: Transform::scale(Fx::from_int(2), Fx::from_int(2)),
                            antialias: false,
                            recolor: rc,
                            ..BlitDsc::default()
                        },
                    );
                } else {
                    p.blit_recolor(Point::new(0, 0), &img, Opa::COVER, BlendMode::Normal, rc);
                }
            });
            let rgb = h.rgb888();
            assert_eq!(
                &rgb[..3],
                &[expect.r, expect.g, expect.b],
                "{f} transformed={transformed}"
            );
            if f == ColorFormat::Argb8888Premultiplied {
                // Pixel (1, 0) is red at alpha 128 (premultiplied 128): un-premultiplied over white.
                let x = if transformed { 2 } else { 1 };
                assert_eq!(&rgb[x * 3..x * 3 + 3], &[255, 127, 127]);
            }
        }
    }
}

#[test]
fn draw_formats_and_features() {
    for f in ColorFormat::ALL {
        let draw = matches!(
            f,
            ColorFormat::Rgb565
                | ColorFormat::Rgb565Swapped
                | ColorFormat::Rgb888
                | ColorFormat::Xrgb8888
                | ColorFormat::Argb8888
                | ColorFormat::L8
                | ColorFormat::I1
        );
        assert_eq!(is_draw_format(f), draw, "{f}");
        assert_eq!(
            is_format_enabled(f),
            draw,
            "{f}: tests enable every color feature"
        );
    }
}

#[test]
fn caches_budget_and_stats() {
    let cfg = RenderConfig {
        max_span: 100,
        layer_buf_bytes: 1000,
        ..RenderConfig::default()
    };
    let c = RenderCaches::new(&cfg);
    assert_eq!(c.max_span(), 100);
    assert_eq!(c.config(), &cfg);
    assert!(c.bytes_reserved() >= 100 * 2 + 400 + 200 + 1000);
    assert_eq!(c.stats(), twine_render::RenderCacheStats::default());
}

#[test]
fn source_helpers_and_unsupported_formats() {
    let d = [1u8, 2, 3, 4, 5, 6];
    let s = Source::Pixels {
        data: &d,
        format: ColorFormat::Rgb888,
    };
    assert_eq!(s.len(), 2);
    assert_eq!(s.offset(1).len(), 1);
    assert!(s.offset(5).is_empty());
    assert_eq!(Source::Argb(&d).len(), 1);
    assert_eq!(Source::Solid(Color::RED).len(), usize::MAX);
    // An unsupported source format draws nothing.
    let mut row = [9u8; 6];
    blend_span::<twine_core::color::Rgb888>(
        &mut row,
        2,
        Source::Pixels {
            data: &d,
            format: ColorFormat::A8,
        },
        None,
        Opa::COVER,
        BlendMode::Normal,
    );
    assert_eq!(row, [9; 6]);
}

#[test]
fn i1_buffer_thresholds_luminance() {
    let mut h = RenderHarness::new(16, 4, ColorFormat::I1);
    h.clear(Color::BLACK);
    h.paint(|p| {
        p.fill(Rect::from_xywh(0, 0, 8, 4), Color::WHITE, Opa::COVER);
        p.fill(Rect::from_xywh(8, 0, 8, 4), Color::WHITE, Opa(100)); // too dark: stays black
        p.arc(
            Point::new(8, 2),
            2,
            Angle::deg(0),
            Angle::deg(90),
            &twine_render::ArcDsc {
                color: Color::WHITE,
                width: 2,
                ..Default::default()
            },
        );
    });
    assert_eq!(h.data()[0], 0xFF);
    assert_eq!(h.data()[1] & 0x0F, 0x00);
}
