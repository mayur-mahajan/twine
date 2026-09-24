//! Criterion benchmarks of `twine-image`: QOI decoding and RLE decompression of a 64 × 64
//! image. Run with `cargo bench -p twine-image`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use twine_image::decoders::qoi::{QoiDecoder, qoi_encode};
use twine_image::{Decoder, rle_compress, rle_decompress};

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

fn qoi_decode(c: &mut Criterion) {
    let file = qoi_encode(&pixels(), 64, 64, 4);
    let mut out = Vec::new();
    c.bench_function("qoi_decode_64x64", |b| {
        b.iter(|| QoiDecoder.decode(black_box(&file), &mut out).unwrap());
    });
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
}

criterion_group!(benches, qoi_decode, rle);
criterion_main!(benches);
