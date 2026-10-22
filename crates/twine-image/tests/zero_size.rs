//! Zero-size images (R5.S10): a header that declares a width or height of 0 never reaches a
//! division (row strides, pixel indices, interlace passes, scaling) — each decoder rejects it,
//! in debug and release builds (CI also runs this file with `--release`), and the RLE
//! decompressor refuses a zero block size.
//!
//! Every input here is also a seed of the decoder's fuzz target (`fuzz/corpus/<target>/r5s10_*`,
//! kept equal to these builders by `fuzz_seeds_match_the_builders`; `TWINE_UPDATE_FUZZ_SEEDS=1`
//! rewrites them).
//!
//! req: SAF-033

use std::path::PathBuf;

use twine_image::decoders::bmp::BmpDecoder;
use twine_image::decoders::gif::GifDecoder;
use twine_image::decoders::png::PngDecoder;
use twine_image::decoders::qoi::QoiDecoder;
use twine_image::{Decoder, Error, rle_decompress};

/// A 24-bit BMP of `w` × `h` (one zero byte of pixel data per row, padded).
fn bmp(w: i32, h: i32) -> Vec<u8> {
    let stride = (w.unsigned_abs() * 3).div_ceil(4) * 4;
    let data = 14 + 40;
    let size = stride * h.unsigned_abs();
    let mut f = Vec::new();
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(data + size).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&data.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    f.extend(std::iter::repeat_n(0, size as usize));
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

/// A valid 1 × 1 gray PNG (interlaced with `adam7`) whose IHDR then declares `w` × `h` (CRC
/// fixed).
fn png(w: u32, h: u32, adam7: bool) -> Vec<u8> {
    let mut f = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut f, 1, 1);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0x80]).unwrap();
    }
    // Signature (8), IHDR length (4) and type (4), then w (4), h (4), depth, color,
    // compression, filter, interlace.
    f[16..20].copy_from_slice(&w.to_be_bytes());
    f[20..24].copy_from_slice(&h.to_be_bytes());
    f[28] = u8::from(adam7);
    let crc = crc32(&f[12..29]);
    f[29..33].copy_from_slice(&crc.to_be_bytes());
    f
}

/// A GIF with a `sw` × `sh` screen and one `fw` × `fh` frame (two-colour table, one pixel of
/// LZW data at minimum code size 2).
fn gif(sw: u16, sh: u16, fw: u16, fh: u16) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"GIF89a");
    f.extend_from_slice(&sw.to_le_bytes());
    f.extend_from_slice(&sh.to_le_bytes());
    f.extend_from_slice(&[0x80, 0, 0]);
    f.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
    f.push(0x2C);
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&fw.to_le_bytes());
    f.extend_from_slice(&fh.to_le_bytes());
    f.push(0);
    f.push(2);
    f.extend_from_slice(&[2, 0x44, 0x01, 0]);
    f.push(0x3B);
    f
}

/// A QOI header of `w` × `h` (RGBA) followed by the end marker.
fn qoi(w: u32, h: u32) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"qoif");
    f.extend_from_slice(&w.to_be_bytes());
    f.extend_from_slice(&h.to_be_bytes());
    f.extend_from_slice(&[4, 0]);
    f.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    f
}

/// The zero-size inputs: `(fuzz target, seed name, bytes)`.
fn zero_size() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    vec![
        ("fuzz_bmp", "r5s10_width_0.bmp", bmp(0, 2)),
        ("fuzz_bmp", "r5s10_height_0.bmp", bmp(2, 0)),
        ("fuzz_png", "r5s10_width_0.png", png(0, 1, false)),
        ("fuzz_png", "r5s10_height_0.png", png(1, 0, false)),
        ("fuzz_png", "r5s10_width_0_adam7.png", png(0, 1, true)),
        ("fuzz_gif", "r5s10_screen_0x0.gif", gif(0, 0, 1, 1)),
        ("fuzz_gif", "r5s10_frame_width_0.gif", gif(1, 1, 0, 1)),
        ("fuzz_gif", "r5s10_frame_height_0.gif", gif(1, 1, 1, 0)),
        ("fuzz_qoi", "r5s10_width_0.qoi", qoi(0, 1)),
        ("fuzz_qoi", "r5s10_height_0.qoi", qoi(1, 0)),
    ]
}

fn decoder(target: &str) -> &'static dyn Decoder {
    match target {
        "fuzz_bmp" => &BmpDecoder,
        "fuzz_png" => &PngDecoder,
        "fuzz_qoi" => &QoiDecoder,
        _ => &GifDecoder,
    }
}

#[test]
fn the_builders_make_valid_files_with_a_nonzero_size() {
    let mut out = Vec::new();
    let h = BmpDecoder.decode(&bmp(2, 2), &mut out).unwrap();
    assert_eq!((h.w, h.h), (2, 2));
    let h = PngDecoder.decode(&png(1, 1, false), &mut out).unwrap();
    assert_eq!((h.w, h.h), (1, 1));
    let h = GifDecoder.decode(&gif(1, 1, 1, 1), &mut out).unwrap();
    assert_eq!((h.w, h.h), (1, 1));
}

/// An image of zero width or height is rejected (`InvalidHeader`, also by the probe); a GIF
/// frame of zero size inside a non-empty screen draws nothing and the image is the screen.
/// Never a panic (a division by zero or an out-of-range index).
#[test]
fn zero_size_images_are_rejected() {
    for (target, name, bytes) in zero_size() {
        let d = decoder(target);
        let mut out = Vec::new();
        let r = d.decode(&bytes, &mut out);
        if name.starts_with("r5s10_frame_") {
            let h = r.unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!((h.w, h.h), (1, 1), "{name}");
        } else {
            assert!(matches!(r, Err(Error::InvalidHeader)), "{name}: {r:?}");
            assert_eq!(d.probe(&bytes), None, "{name}");
        }
    }
}

#[test]
fn rle_refuses_a_zero_block_size() {
    let mut out = [0u8; 4];
    assert!(rle_decompress(&[0x82, 1], &mut out, 0).is_err());
}

#[test]
fn fuzz_seeds_match_the_builders() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus");
    let update = std::env::var_os("TWINE_UPDATE_FUZZ_SEEDS").is_some();
    for (target, name, bytes) in zero_size() {
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
