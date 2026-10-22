//! Criterion benchmarks of `twine-image`: decoding a 64 × 64 (QOI) or 128 × 128 image with
//! every in-house decoder (QOI, BMP 8/24/32 bpp and bit fields, PNG gray/RGB/RGBA/palette, GIF
//! still and animation frames) and RLE/LZ4 decompression. Run with `cargo bench -p twine-image`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use twine_image::decoders::bmp::BmpDecoder;
use twine_image::decoders::gif::{GifDecoder, GifPlayer};
use twine_image::decoders::png::PngDecoder;
use twine_image::decoders::qoi::{QoiDecoder, qoi_encode};
use twine_image::{Decoder, lz4_decompress, rle_compress, rle_decompress};

/// A 64 × 64 RGBA gradient with a few flat areas (typical UI artwork).
fn pixels() -> Vec<u8> {
    (0..64u32 * 64)
        .flat_map(|i| {
            let (x, y) = (i % 64, i / 64);
            if (16..48).contains(&x) && (16..48).contains(&y) {
                [255, 255, 255, 255]
            } else {
                [(x * 4) as u8, (y * 4) as u8, 128, 255]
            }
        })
        .collect()
}

/// Side of the BMP, PNG and GIF images.
const S: u32 = 128;

/// A `S` × `S` RGBA image: gradients, a flat square and a translucent band.
fn artwork() -> image::RgbaImage {
    image::RgbaImage::from_fn(S, S, |x, y| {
        if (32..96).contains(&x) && (32..96).contains(&y) {
            image::Rgba([255, 255, 255, 255])
        } else {
            image::Rgba([
                (x * 2) as u8,
                (y * 2) as u8,
                128,
                if (100..110).contains(&y) { 128 } else { 255 },
            ])
        }
    })
}

fn encode(img: &image::DynamicImage, fmt: image::ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, fmt).unwrap();
    out.into_inner()
}

/// A `S` × `S` 16-bit RGB565 `BI_BITFIELDS` BMP.
fn bmp565() -> Vec<u8> {
    let stride = (S * 2).div_ceil(4) * 4;
    let data = 14 + 40 + 12;
    let mut f = Vec::new();
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(data + stride * S).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&data.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(S as i32).to_le_bytes());
    f.extend_from_slice(&(S as i32).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    f.extend_from_slice(&[0; 20]);
    for m in [0xF800u32, 0x07E0, 0x001F] {
        f.extend_from_slice(&m.to_le_bytes());
    }
    f.extend((0..stride * S).map(|i| (i * 7) as u8));
    f
}

/// A `S` × `S` 8-bit PNG of `color` (palette images with a transparency chunk).
fn png_file(color: png::ColorType) -> Vec<u8> {
    let rgba = artwork().into_raw();
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, S, S);
        e.set_color(color);
        e.set_depth(png::BitDepth::Eight);
        if color == png::ColorType::Indexed {
            e.set_palette((0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect::<Vec<u8>>());
            e.set_trns((0..=255u8).map(|i| i.wrapping_mul(3)).collect::<Vec<u8>>());
        }
        let mut wr = e.write_header().unwrap();
        let data: Vec<u8> = match color {
            png::ColorType::Grayscale => rgba.chunks_exact(4).map(|c| c[0]).collect(),
            png::ColorType::Rgb => rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect(),
            png::ColorType::Indexed => rgba.chunks_exact(4).map(|c| c[0] ^ c[1]).collect(),
            _ => rgba,
        };
        wr.write_image_data(&data).unwrap();
    }
    out
}

/// A 3-frame `S` × `S` GIF animation (a moving square over a gradient).
fn gif_animation() -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..3u32 {
            let img = image::RgbaImage::from_fn(S, S, |x, y| {
                if x >= i * 30 && x < i * 30 + 40 && (40..80).contains(&y) {
                    image::Rgba([255, 255, 255, 255])
                } else {
                    image::Rgba([(x * 2) as u8, (y * 2) as u8, 128, 255])
                }
            });
            let frame = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(50, 1));
            enc.encode_frame(frame).unwrap();
        }
    }
    out
}

fn qoi_decode(c: &mut Criterion) {
    let file = qoi_encode(&pixels(), 64, 64, 4);
    let mut out = Vec::new();
    c.bench_function("qoi_decode_64x64", |b| {
        b.iter(|| QoiDecoder.decode(black_box(&file), &mut out).unwrap());
    });
}

fn bmp_decode(c: &mut Criterion) {
    let art = image::DynamicImage::ImageRgba8(artwork());
    let files = [
        (
            "bmp8_decode_128",
            encode(
                &image::DynamicImage::ImageLuma8(art.to_luma8()),
                image::ImageFormat::Bmp,
            ),
        ),
        (
            "bmp24_decode_128",
            encode(
                &image::DynamicImage::ImageRgb8(art.to_rgb8()),
                image::ImageFormat::Bmp,
            ),
        ),
        ("bmp32_decode_128", encode(&art, image::ImageFormat::Bmp)),
        ("bmp16_bitfields_decode_128", bmp565()),
    ];
    let mut out = Vec::new();
    for (name, file) in files {
        BmpDecoder.decode(&file, &mut out).expect(name);
        c.bench_function(name, |b| {
            b.iter(|| BmpDecoder.decode(black_box(&file), &mut out).unwrap());
        });
    }
}

fn png_decode(c: &mut Criterion) {
    let mut out = Vec::new();
    for (name, color) in [
        ("png_gray_decode_128", png::ColorType::Grayscale),
        ("png_rgb_decode_128", png::ColorType::Rgb),
        ("png_rgba_decode_128", png::ColorType::Rgba),
        ("png_palette_decode_128", png::ColorType::Indexed),
    ] {
        let file = png_file(color);
        PngDecoder.decode(&file, &mut out).expect(name);
        c.bench_function(name, |b| {
            b.iter(|| PngDecoder.decode(black_box(&file), &mut out).unwrap());
        });
    }
}

fn gif_decode(c: &mut Criterion) {
    let file = gif_animation();
    let mut out = Vec::new();
    c.bench_function("gif_decode_128", |b| {
        b.iter(|| GifDecoder.decode(black_box(&file), &mut out).unwrap());
    });
    let mut player = GifPlayer::new(&file).unwrap();
    c.bench_function("gif_advance_128", |b| b.iter(|| black_box(player.advance())));
}

fn rle(c: &mut Criterion) {
    // RGB565 (block size 2).
    let raw: Vec<u8> = pixels()
        .chunks_exact(4)
        .flat_map(|p| [p[0] & 0xF8, p[1]])
        .collect();
    let packed = rle_compress(&raw, 2);
    let mut out = vec![0u8; raw.len()];
    c.bench_function("rle_decompress_64x64", |b| {
        b.iter(|| rle_decompress(black_box(&packed), &mut out, 2).unwrap());
    });
    let packed = lz4_flex::block::compress(&raw);
    c.bench_function("lz4_decompress_64x64", |b| {
        b.iter(|| lz4_decompress(black_box(&packed), &mut out).unwrap());
    });
}

criterion_group!(benches, qoi_decode, bmp_decode, png_decode, gif_decode, rle);
criterion_main!(benches);
