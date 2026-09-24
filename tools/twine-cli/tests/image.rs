//! `twine image`: every format round-trips through `twine-image` and the renderer, compression,
//! determinism and the palette quantizer.

use twine_cli::image::{CompressArg, Emit, FORMAT_NAMES, ImageOptions, generate_rgba, median_cut, summary};
use twine_core::{ColorFormat, Rect};
use twine_image::{Image, ImageData, ImageFlags, pixels_of};
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

fn opts(format: ColorFormat, compress: CompressArg) -> ImageOptions {
    ImageOptions {
        input: "test.png".into(),
        format,
        compress,
        dither: false,
        premultiply: false,
        name: "TEST".into(),
        out: "test.rs".into(),
        crate_path: "twine_image".into(),
    }
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
        let g = generate_rgba(&src, W, H, "0", &opts(f, CompressArg::None)).unwrap();
        let img = Image::new_owned(g.header, g.data.clone().into_boxed_slice());
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
        let plain = generate_rgba(&src, W, H, "0", &opts(f, CompressArg::None)).unwrap();
        for c in [CompressArg::Rle, CompressArg::Lz4] {
            let g = generate_rgba(&src, W, H, "0", &opts(f, c)).unwrap();
            assert!(g.header.flags.contains(ImageFlags::COMPRESSED));
            if f.bpp() >= 8 {
                assert!(g.bin.len() < plain.bin.len(), "{f} {c:?} compresses");
            }
            let img = Image {
                header: g.header,
                data: ImageData::Compressed {
                    method: if c == CompressArg::Rle {
                        twine_image::Compression::Rle
                    } else {
                        twine_image::Compression::Lz4
                    },
                    data: Box::leak(g.bin.clone().into_boxed_slice()),
                    decompressed_size: g.data.len() as u32,
                },
            };
            let mut out = vec![0u8; g.data.len()];
            let h = img.decompress_into(&mut out).unwrap();
            assert_eq!(out, plain.data, "{f} {c:?}");
            assert_eq!(h, plain.header);
            assert!(g.source.contains("ImageData::Compressed"));
        }
    }
}

#[test]
fn deterministic_output() {
    let src = pattern(200);
    for f in [ColorFormat::I8, ColorFormat::Rgb565, ColorFormat::A4] {
        let mut o = opts(f, CompressArg::Rle);
        o.dither = true;
        let a = generate_rgba(&src, W, H, "abc", &o).unwrap();
        let b = generate_rgba(&src, W, H, "abc", &o).unwrap();
        assert_eq!(a.source, b.source);
        assert_eq!(a.bin, b.bin);
    }
    let g = generate_rgba(&src, W, H, "abc", &opts(ColorFormat::Rgb565, CompressArg::None)).unwrap();
    assert!(g.source.contains("// Input SHA-256: abc"));
    assert!(g.source.contains("include_bytes!(\"test.bin\")"));
    assert!(g.source.contains("pub static TEST: twine_image::Image"));
    let e = Emit {
        name: "X",
        crate_path: "twine_image",
        command: "",
        input_sha256: "",
        header: g.header,
        compress: CompressArg::None,
        bin_name: "x.bin",
        stored_len: 2048,
        data_len: 2048,
    };
    assert_eq!(
        summary(&e),
        "// twine-image: format=RGB565 w=32 h=32 stride=64 flags=0x00 compression=none stored=2048 size=2048"
    );
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

#[test]
fn cli_writes_files_and_info_reads_them() {
    let dir = std::env::temp_dir().join(format!("twine-image-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("in.png");
    image::RgbaImage::from_raw(W, H, pattern(8))
        .unwrap()
        .save(&png)
        .unwrap();
    let out = dir.join("logo.rs");
    let bin = env!("CARGO_BIN_EXE_twine");
    let st = std::process::Command::new(bin)
        .args(["image", "--in"])
        .arg(&png)
        .args([
            "--format",
            "rgb565a8",
            "--compress",
            "rle",
            "--name",
            "LOGO",
            "--out",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    assert!(dir.join("logo.bin").exists());
    let info = std::process::Command::new(bin)
        .args(["image", "--info"])
        .arg(dir.join("logo.bin"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(text.contains("RGB565A8") && text.contains("rle"), "{text}");
    let bad = std::process::Command::new(bin)
        .args(["image", "--in"])
        .arg(&png)
        .args(["--format", "rgb999"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}
