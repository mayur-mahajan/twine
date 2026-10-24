//! `twine image`: the command writes the source, sidecar and `.tbin` files and reads them back
//! (`--info`), and the procedurally drawn logo (`cargo xtask gen-assets`). The conversion
//! itself is tested in `twine-build` (`tests/image.rs`).

use twine_core::ColorFormat;
use twine_image::ImageFlags;

const W: u32 = 32;
const H: u32 = 32;

/// A pattern with at most `colors` distinct RGBA values, translucent in the lower half.
fn pattern(colors: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let k = (x / 4 + (y / 4) * 8) % colors.max(1);
            let a = if y >= H / 2 { (255 - x * 6) as u8 } else { 255 };
            v.extend_from_slice(&[
                (k * 97 % 256) as u8,
                (k * 53 % 256) as u8,
                (255 - k * 31 % 256) as u8,
                a,
            ]);
        }
    }
    v
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
    let bad_bg = std::process::Command::new(bin)
        .args(["image", "--in"])
        .arg(&png)
        .args([
            "--format",
            "rgb565",
            "--background",
            "white",
            "--name",
            "LOGO",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!bad_bg.status.success());
    assert!(String::from_utf8_lossy(&bad_bg.stderr).contains("#RRGGBB"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `twine image --bin` writes a `.tbin` file; decoded by the registry (as the engine does for
/// `ImageSource::File`) it holds exactly the pixels of the static image of the same conversion.
#[test]
fn tbin_roundtrip_via_cli_output() {
    let dir = std::env::temp_dir().join(format!("twine-image-tbin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("in.png");
    image::RgbaImage::from_raw(W, H, pattern(8))
        .unwrap()
        .save(&png)
        .unwrap();
    let registry = twine_image::DecoderRegistry::with_defaults();
    for (format, compress) in [
        ("rgb565", "none"),
        ("argb8888", "rle"),
        ("rgb565a8", "lz4"),
        ("i4", "rle"),
    ] {
        let out = dir.join(format!("{format}-{compress}.tbin"));
        let st = std::process::Command::new(env!("CARGO_BIN_EXE_twine"))
            .args(["image", "--in"])
            .arg(&png)
            .args(["--format", format, "--compress", compress, "--bin", "--out"])
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success(), "{format} {compress}");
        let file = std::fs::read(&out).unwrap();
        assert_eq!(&file[..4], b"TWIN");
        let mut decoded = Vec::new();
        let header = registry.decode(&file, &mut decoded).unwrap();
        let method = match compress {
            "rle" => twine_build::image::Compression::Rle,
            "lz4" => twine_build::image::Compression::Lz4,
            _ => twine_build::image::Compression::None,
        };
        let cf: ColorFormat = twine_cli::image::parse_format(format).unwrap();
        let g = twine_build::image::Image::from_rgba(W, H, pattern(8))
            .format(cf)
            .compress(method)
            .build()
            .unwrap();
        let mut static_header = g.header;
        static_header.flags.remove(ImageFlags::COMPRESSED);
        assert_eq!(header, static_header, "{format} {compress}");
        assert_eq!(decoded, g.data, "{format} {compress}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The generated Twine logo (`cargo xtask gen-assets`): the Rope mark, deterministic, on a
/// transparent background, with the right strand on top at each of the plait's three crossings.
#[test]
fn logo_is_the_rope_mark() {
    let logo = twine_cli::image::assets::logo();
    assert_eq!(logo, twine_cli::image::assets::logo(), "deterministic");
    assert_eq!(logo.dimensions(), (64, 64));
    for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63), (32, 2), (32, 61)] {
        assert_eq!(logo.get_pixel(x, y).0[3], 0, "transparent at ({x}, {y})");
    }
    let (red, green, blue) = (
        [0xEC, 0x40, 0x48, 255],
        [0x22, 0xB8, 0x68, 255],
        [0x30, 0x70, 0xF0, 255],
    );
    // Crossing centers: red over green, blue over red, green over blue.
    for ((x, y), top) in [((15, 27), red), ((25, 36), blue), ((35, 27), green)] {
        assert_eq!(logo.get_pixel(x, y).0, top, "strand on top at ({x}, {y})");
    }
    // The rays on the right: blue up, green level, red down.
    for ((x, y), ray) in [((58, 13), blue), ((58, 32), green), ((58, 50), red)] {
        assert_eq!(logo.get_pixel(x, y).0, ray, "ray at ({x}, {y})");
    }
}
