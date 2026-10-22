//! Header-derived shift amounts of the decoders (R5.S09): bit depths, bit-field masks and LZW
//! code sizes are validated once, when the header is parsed, and an input with an out-of-range
//! value is rejected; the pixel loops then shift only by validated amounts. CI also runs this
//! file with `--release`, where an out-of-range shift would silently mask the amount instead of
//! panicking.
//!
//! Every rejected input here is also a seed of the decoder's fuzz target
//! (`fuzz/corpus/<target>/r5s09_*`, kept equal to these builders by
//! `fuzz_seeds_match_the_builders`; `TWINE_UPDATE_FUZZ_SEEDS=1` rewrites them).
//!
//! req: SAF-032

use std::path::PathBuf;

use twine_image::decoders::bmp::BmpDecoder;
use twine_image::decoders::gif::GifDecoder;
use twine_image::decoders::png::PngDecoder;
use twine_image::{Decoder, Error};

/// A 2 × 1 `BI_BITFIELDS` BMP of `bpp` (16 or 32) bits with the red, green and blue `masks`.
fn bmp_bitfields(bpp: u16, masks: [u32; 3]) -> Vec<u8> {
    let stride = (2 * u32::from(bpp)).div_ceil(32) * 4;
    let data = 14 + 40 + 12;
    let mut f = Vec::new();
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(data + stride).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&data.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&2i32.to_le_bytes());
    f.extend_from_slice(&1i32.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&bpp.to_le_bytes());
    f.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    f.extend_from_slice(&[0; 20]);
    for m in masks {
        f.extend_from_slice(&m.to_le_bytes());
    }
    f.extend(std::iter::repeat_n(0xFF, stride as usize));
    f
}

/// CRC-32 (ISO-HDLC) of `bytes`, as PNG chunks use.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A valid 2 × 1 8-bit gray PNG whose IHDR then declares `depth` and `color` (CRC fixed).
fn png_with(depth: u8, color: u8) -> Vec<u8> {
    let mut f = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut f, 2, 1);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0x40, 0xC0]).unwrap();
    }
    // Signature (8), IHDR length (4) and type (4), then w (4), h (4), depth, color, ….
    f[24] = depth;
    f[25] = color;
    let crc = crc32(&f[12..29]);
    f[29..33].copy_from_slice(&crc.to_be_bytes());
    f
}

/// A 1 × 1 GIF with a two-colour global table whose image declares LZW minimum code size
/// `min_code` (the data is the code stream of one pixel at minimum code size 2).
fn gif_with(min_code: u8) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"GIF89a");
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&[0x80, 0, 0]); // global table of 2 entries
    f.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
    f.push(0x2C);
    f.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0, 0]);
    f.push(min_code);
    // Codes clear (4), 0, end (5) at 3 bits, LSB first.
    f.extend_from_slice(&[2, 0x44, 0x01, 0]);
    f.push(0x3B);
    f
}

/// The rejected inputs: `(fuzz target, seed name, bytes)`.
fn rejected() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    vec![
        (
            "fuzz_bmp",
            "r5s09_mask_not_contiguous.bmp",
            bmp_bitfields(16, [0xF800, 0x07E0, 0b1011]),
        ),
        (
            "fuzz_bmp",
            "r5s09_mask_outside_16bpp.bmp",
            bmp_bitfields(16, [0x1_F800, 0x07E0, 0x001F]),
        ),
        (
            "fuzz_bmp",
            "r5s09_mask_split_32bpp.bmp",
            bmp_bitfields(32, [0x8000_0001, 0xFF00, 0xFF]),
        ),
        (
            "fuzz_bmp",
            "r5s09_channel_wider_than_16_bits.bmp",
            bmp_bitfields(32, [0x01FF_FF00, 0xC000_0000, 0xFF]),
        ),
        ("fuzz_png", "r5s09_gray_depth_3.png", png_with(3, 0)),
        ("fuzz_png", "r5s09_gray_depth_0.png", png_with(0, 0)),
        ("fuzz_png", "r5s09_gray_depth_32.png", png_with(32, 0)),
        ("fuzz_png", "r5s09_palette_depth_16.png", png_with(16, 3)),
        ("fuzz_png", "r5s09_rgb_depth_4.png", png_with(4, 2)),
        ("fuzz_gif", "r5s09_lzw_code_size_0.gif", gif_with(0)),
        ("fuzz_gif", "r5s09_lzw_code_size_12.gif", gif_with(12)),
        ("fuzz_gif", "r5s09_lzw_code_size_255.gif", gif_with(255)),
    ]
}

fn decoder(target: &str) -> &'static dyn Decoder {
    match target {
        "fuzz_bmp" => &BmpDecoder,
        "fuzz_png" => &PngDecoder,
        _ => &GifDecoder,
    }
}

#[test]
fn the_builders_make_valid_files_with_legal_values() {
    let mut out = Vec::new();
    let h = BmpDecoder
        .decode(&bmp_bitfields(16, [0xF800, 0x07E0, 0x001F]), &mut out)
        .unwrap();
    assert_eq!((h.w, h.h), (2, 1));
    assert_eq!(&out[..4], [255, 255, 255, 255]);
    let h = BmpDecoder
        .decode(&bmp_bitfields(32, [0xFF_0000, 0xFF00, 0xFF]), &mut out)
        .unwrap();
    assert_eq!((h.w, h.h), (2, 1));
    let h = PngDecoder.decode(&png_with(8, 0), &mut out).unwrap();
    assert_eq!((h.w, h.h), (2, 1));
    assert_eq!(&out[..8], [0x40, 0x40, 0x40, 255, 0xC0, 0xC0, 0xC0, 255]);
    let h = GifDecoder.decode(&gif_with(2), &mut out).unwrap();
    assert_eq!((h.w, h.h), (1, 1));
    assert_eq!(&out[..4], [0, 0, 0, 255]);
}

#[test]
fn out_of_range_header_values_are_rejected() {
    for (target, name, bytes) in rejected() {
        let d = decoder(target);
        let mut out = Vec::new();
        let r = d.decode(&bytes, &mut out);
        assert!(
            matches!(
                r,
                Err(Error::InvalidHeader | Error::Decode(_) | Error::Unsupported(_))
            ),
            "{name}: {r:?}"
        );
        if target != "fuzz_gif" {
            // BMP and PNG validate their header when probing too (a GIF's LZW code size sits in
            // the frame, after the screen header the probe reads).
            assert_eq!(d.probe(&bytes), None, "{name}");
        }
    }
}

/// A 32-bit bit-field channel wider than 16 bits is rejected (user decision 2026-10-20);
/// a 16-bit channel at the top of the pixel is still accepted.
#[test]
fn channels_wider_than_16_bits_are_rejected() {
    let mut out = Vec::new();
    for mask in [0x0001_FFFF, 0xFFFF_FF00, u32::MAX] {
        let file = bmp_bitfields(32, [mask, 0, 0]);
        assert!(
            matches!(BmpDecoder.decode(&file, &mut out), Err(Error::InvalidHeader)),
            "mask {mask:#x}"
        );
        assert_eq!(BmpDecoder.probe(&file), None, "mask {mask:#x}");
    }
    let h = BmpDecoder
        .decode(&bmp_bitfields(32, [0xFFFF_0000, 0xFF00, 0xFF]), &mut out)
        .unwrap();
    assert_eq!((h.w, h.h), (2, 1));
    assert_eq!(&out[..4], [255, 255, 255, 255]);
}

/// Bit-field channels of every width: amounts derived from contiguous masks at every position.
#[test]
fn every_contiguous_mask_decodes() {
    for width in 1..=16u32 {
        for shift in 0..=16 - width {
            let m = ((1u32 << width) - 1) << shift;
            let mut out = Vec::new();
            let r = BmpDecoder.decode(&bmp_bitfields(16, [m, m, m]), &mut out);
            assert!(r.is_ok(), "mask {m:#x}: {r:?}");
            assert_eq!(out[0], 255, "mask {m:#x}");
        }
    }
}

#[test]
fn fuzz_seeds_match_the_builders() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus");
    let update = std::env::var_os("TWINE_UPDATE_FUZZ_SEEDS").is_some();
    for (target, name, bytes) in rejected() {
        let path = root.join(target).join(name);
        if update {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let seed = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            seed,
            bytes,
            "{} is stale (TWINE_UPDATE_FUZZ_SEEDS=1)",
            path.display()
        );
    }
}
