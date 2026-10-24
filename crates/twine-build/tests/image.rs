//! `twine_build::image`: every format round-trips through `twine-image` and the renderer,
//! compression, the background, determinism, the generated source and the palette quantizer.

#![cfg(feature = "image")]

use std::path::Path;

use twine_build::image::{
    Color, ColorFormat, Compression, FORMAT_NAMES, GeneratedImage, Image, convert, median_cut,
};
use twine_core::Rect;
use twine_image::{ImageData, ImageFlags, pixels_of};
use twine_render::ImageDsc;
use twine_testing::RenderHarness;

const W: u32 = 32;
const H: u32 = 32;

/// A pattern with at most `colors` distinct opaque/translucent RGBA values.
fn pattern(colors: u32) -> Vec<u8> {
    pattern_alpha(colors, true)
}

fn pattern_alpha(colors: u32, alpha: bool) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let k = (x / 4 + (y / 4) * 8) % colors.max(1);
            let r = (k * 97 % 256) as u8;
            let g = (k * 53 % 256) as u8;
            let b = (255 - k * 31 % 256) as u8;
            let a = if alpha && y >= H / 2 {
                (255 - x * 6) as u8
            } else {
                255
            };
            v.extend_from_slice(&[r, g, b, a]);
        }
    }
    v
}

fn image(src: &[u8], w: u32, h: u32, format: ColorFormat, compress: Compression) -> Image {
    Image::from_rgba(w, h, src.to_vec())
        .format(format)
        .compress(compress)
        .name("TEST")
        .crate_path("twine_image")
}

fn build(src: &[u8], format: ColorFormat, compress: Compression) -> GeneratedImage {
    image(src, W, H, format, compress).build().unwrap()
}

/// Renders `data` over opaque white and returns RGB.
fn render(header: twine_image::ImageHeader, data: &[u8]) -> Vec<u8> {
    let px = pixels_of(&header, data).expect("valid converted data");
    let mut h = RenderHarness::new(W as u16, H as u16, ColorFormat::Argb8888);
    h.paint(|p| {
        p.image(
            Rect::from_xywh(0, 0, W as i32, H as i32),
            &px,
            &ImageDsc::default(),
        );
    });
    h.rgb888()
}

fn over_white(c: u8, a: u8) -> u8 {
    ((u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8
}

#[test]
fn convert_every_format_roundtrips_through_twine_image() {
    for (name, f) in FORMAT_NAMES {
        // Indexed formats get exactly as many colors as their palette holds.
        let colors = if f.is_indexed() {
            f.palette_len() as u32
        } else {
            64
        };
        let src = pattern_alpha(colors.min(64), !f.is_indexed());
        let g = build(&src, f, Compression::None);
        let img = twine_image::Image::new_owned(g.header, g.data.clone().into_boxed_slice());
        img.validate().unwrap();
        let got = render(g.header, &g.data);
        let bpp = u32::from(f.bpp());
        let tol: u8 = match f {
            ColorFormat::Rgb565 | ColorFormat::Rgb565Swapped | ColorFormat::Rgb565A8 => 6,
            ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 => (255 / ((1 << bpp) - 1) / 2 + 2) as u8,
            ColorFormat::Argb8888Premultiplied => 3,
            _ => 2,
        };
        for (i, s) in src.chunks_exact(4).enumerate() {
            let expected: [u8; 3] = match f {
                ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8 => {
                    [over_white(0, s[3]); 3]
                }
                ColorFormat::L8 => {
                    let l =
                        ((u32::from(s[0]) * 77 + u32::from(s[1]) * 150 + u32::from(s[2]) * 29) >> 8) as u8;
                    [l; 3]
                }
                ColorFormat::Rgb565
                | ColorFormat::Rgb565Swapped
                | ColorFormat::Rgb888
                | ColorFormat::Xrgb8888 => [s[0], s[1], s[2]],
                _ => [
                    over_white(s[0], s[3]),
                    over_white(s[1], s[3]),
                    over_white(s[2], s[3]),
                ],
            };
            let px = &got[i * 3..i * 3 + 3];
            let worst = px.iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap();
            assert!(worst <= tol, "{name}: pixel {i} got {px:?} expected {expected:?}");
        }
    }
}

#[test]
fn rle_and_lz4_outputs_decompress() {
    let src = pattern(16);
    for f in [
        ColorFormat::Rgb565A8,
        ColorFormat::Argb8888,
        ColorFormat::I4,
        ColorFormat::A2,
        ColorFormat::Rgb888,
    ] {
        let plain = build(&src, f, Compression::None);
        for c in [Compression::Rle, Compression::Lz4] {
            let g = build(&src, f, c);
            assert!(g.header.flags.contains(ImageFlags::COMPRESSED));
            if f.bpp() >= 8 {
                assert!(g.stored.len() < plain.stored.len(), "{f} {c:?} compresses");
            }
            let img = twine_image::Image {
                header: g.header,
                data: ImageData::Compressed {
                    method: if c == Compression::Rle {
                        twine_image::Compression::Rle
                    } else {
                        twine_image::Compression::Lz4
                    },
                    data: Box::leak(g.stored.clone().into_boxed_slice()),
                    decompressed_size: g.data.len() as u32,
                },
            };
            let mut out = vec![0u8; g.data.len()];
            let h = img.decompress_into(&mut out).unwrap();
            assert_eq!(out, plain.data, "{f} {c:?}");
            assert_eq!(h, plain.header);
            assert!(g.to_rust(Path::new("t.rs")).contains("ImageData::Compressed"));
        }
    }
}

/// A background: composited over the color with rounding (straight alpha), opaque in every
/// format, recorded in the command line; rejected for alpha-only formats.
#[test]
fn background_flattens_exactly() {
    // A half-transparent orange pixel and a transparent one, over #102030.
    let src = [200, 100, 50, 128, 0, 0, 0, 0];
    let bg = Color::hex(0x10_2030);
    let g = image(&src, 2, 1, ColorFormat::Rgb888, Compression::None)
        .background(bg)
        .build()
        .unwrap();
    // (c·128 + bg·127 + 127) / 255 = 108, 66, 49; stored as B, G, R.
    assert_eq!(g.data, [49, 66, 108, 0x30, 0x20, 0x10]);
    assert!(
        g.to_rust(Path::new("t.rs")).contains("--background #102030"),
        "recorded in the command"
    );
    let g = image(&src, 2, 1, ColorFormat::Argb8888, Compression::None)
        .background(bg)
        .build()
        .unwrap();
    assert_eq!(
        g.data,
        [49, 66, 108, 255, 0x30, 0x20, 0x10, 255],
        "opaque with alpha too"
    );
    assert!(
        image(&src, 2, 1, ColorFormat::A8, Compression::None)
            .background(bg)
            .build()
            .is_err(),
        "alpha-only formats refuse it"
    );
}

/// Without a background the source's alpha goes through unchanged (the conversion is the
/// plain `convert` of the pixels) and the command line does not mention it.
#[test]
fn no_background_keeps_alpha() {
    let src = pattern(8);
    for f in [
        ColorFormat::Rgb565,
        ColorFormat::Argb8888,
        ColorFormat::L8,
        ColorFormat::I4,
    ] {
        let g = build(&src, f, Compression::None);
        let plain = convert(&src, W as u16, H as u16, f, false, false).unwrap();
        assert_eq!(g.data, plain.data, "{f:?}");
        assert!(!g.to_rust(Path::new("t.rs")).contains("--background"));
    }
}

#[test]
fn deterministic_output() {
    let src = pattern(200);
    for f in [ColorFormat::I8, ColorFormat::Rgb565, ColorFormat::A4] {
        let a = image(&src, W, H, f, Compression::Rle)
            .dither(true)
            .build()
            .unwrap();
        let b = image(&src, W, H, f, Compression::Rle)
            .dither(true)
            .build()
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_rust(Path::new("x.rs")), b.to_rust(Path::new("x.rs")));
    }
    let g = build(&src, ColorFormat::Rgb565, Compression::None);
    let rs = g.to_rust(Path::new("assets/test.rs"));
    assert!(rs.starts_with("// Generated by `twine image`; do not edit.\n// Command: twine image --in <32×32 RGBA pixels> --format rgb565 --name TEST --out assets/test.rs\n"));
    assert!(rs.contains(&format!("// Input SHA-256: {}", twine_build::sha256_hex(&src))));
    assert!(rs.contains("include_bytes!(\"test.bin\")"));
    assert!(rs.contains("pub static TEST: twine_image::Image"));
    assert_eq!(
        g.summary(),
        "// twine-image: format=RGB565 w=32 h=32 stride=64 flags=0x00 compression=none stored=2048 size=2048"
    );
}

/// The inline form (the `include_image!` macro) holds the stored bytes as a byte string and
/// names no sidecar.
#[test]
fn inline_source_holds_the_data() {
    let g = image(&[1, 2, 3, 255], 1, 1, ColorFormat::Rgb888, Compression::None)
        .visibility("pub(crate)")
        .crate_path("twine::image")
        .build()
        .unwrap();
    let rs = g.to_rust_inline();
    assert!(rs.contains("pub(crate) static TEST: twine::image::Image"));
    assert!(rs.contains("b\"\\x03\\x02\\x01\""), "{rs}");
    assert!(!rs.contains("include_bytes!"));
}

#[test]
fn files_are_read_and_written() {
    let dir = std::env::temp_dir().join(format!("twine-build-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("power-on.png");
    ::image::RgbaImage::from_raw(W, H, pattern(8))
        .unwrap()
        .save(&png)
        .unwrap();
    let g = Image::from_file("power-on.png")
        .root(&dir)
        .format(ColorFormat::Rgb565A8)
        .compress(Compression::Rle)
        .build()
        .unwrap();
    assert_eq!(g.inputs(), std::slice::from_ref(&png));
    let out = dir.join("gen/power.rs");
    g.write(&out).unwrap();
    let rs = std::fs::read_to_string(&out).unwrap();
    assert!(
        rs.contains("pub static POWER_ON: twine::image::Image"),
        "named after the file"
    );
    assert!(rs.contains("--crate-path twine::image"));
    assert_eq!(std::fs::read(dir.join("gen/power.bin")).unwrap(), g.stored);
    assert!(
        Image::from_file("missing.png")
            .root(&dir)
            .build()
            .unwrap_err()
            .message()
            .contains("missing.png")
    );
    std::fs::write(dir.join("bad.png"), b"not a png").unwrap();
    assert!(Image::from_file("bad.png").root(&dir).build().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_options_are_errors() {
    let src = pattern(4);
    assert!(
        image(&src, W, H, ColorFormat::Rgb565, Compression::None)
            .premultiply(true)
            .build()
            .is_err()
    );
    assert!(
        image(&src, W, H + 1, ColorFormat::Rgb565, Compression::None)
            .build()
            .is_err(),
        "too few pixels"
    );
    assert!(
        image(&src, W, H, ColorFormat::Rgb565, Compression::None)
            .name("a b")
            .build()
            .is_err()
    );
    assert!(
        image(&src, W, H, ColorFormat::Rgb565, Compression::None)
            .crate_path("")
            .build()
            .is_err()
    );
}

/// The `.tbin` form decodes (as the engine does for `ImageSource::File`) to the same image as
/// the static of the same conversion.
#[test]
fn tbin_decodes_like_the_static_image() {
    let registry = twine_image::DecoderRegistry::with_defaults();
    for (format, compress) in [
        (ColorFormat::Rgb565, Compression::None),
        (ColorFormat::Argb8888, Compression::Rle),
        (ColorFormat::Rgb565A8, Compression::Lz4),
        (ColorFormat::I4, Compression::Rle),
    ] {
        let g = build(&pattern(8), format, compress);
        let file = g.to_tbin();
        assert_eq!(&file[..4], b"TWIN");
        let mut decoded = Vec::new();
        let header = registry.decode(&file, &mut decoded).unwrap();
        let mut static_header = g.header;
        static_header.flags.remove(ImageFlags::COMPRESSED);
        assert_eq!(header, static_header, "{format} {compress:?}");
        assert_eq!(
            render(header, &decoded),
            render(static_header, &g.data),
            "{format} {compress:?}"
        );
    }
}

#[test]
fn median_cut_palette_is_stable() {
    let src: Vec<[u8; 4]> = pattern(64)
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    let pal = median_cut(&src, 16);
    assert_eq!(pal.len(), 16);
    // Independent of pixel order.
    let mut shuffled = src.clone();
    shuffled.reverse();
    shuffled.rotate_left(97);
    assert_eq!(median_cut(&shuffled, 16), pal);
    // And of repetition.
    assert_eq!(median_cut(&[src.clone(), src].concat(), 16), pal);
}
