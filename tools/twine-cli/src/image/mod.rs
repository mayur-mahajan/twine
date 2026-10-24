//! `twine image`: converts PNG, JPEG, BMP, GIF, QOI (anything the `image` crate reads) into a
//! `pub static <NAME>: Image` Rust file plus a `.bin` sidecar holding the pixel data, in any
//! `ColorFormat`, optionally RLE- or LZ4-compressed — the command-line form of
//! `twine_build::image::Image` (which build scripts use directly). `--bin` writes a raw
//! `.tbin` file instead (loaded at run time as `ImageSource::File`), `--info` prints the
//! summary of a converted image.
//!
//! The module also draws the repository's sample images ([`assets`], `cargo xtask
//! gen-assets`).

pub mod assets;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use twine_build::image::{Color, ColorFormat, Compression, FORMAT_NAMES, Image};

/// Result type of the command (errors are human-readable messages).
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `--compress`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum CompressArg {
    /// Uncompressed.
    #[default]
    None,
    /// LVGL run-length encoding.
    Rle,
    /// LZ4 block (needs `twine-image` feature `img-lz4` at run time).
    Lz4,
}

impl From<CompressArg> for Compression {
    fn from(c: CompressArg) -> Compression {
        match c {
            CompressArg::None => Compression::None,
            CompressArg::Rle => Compression::Rle,
            CompressArg::Lz4 => Compression::Lz4,
        }
    }
}

/// Parses a `--format` value (the names of `twine_build::image::FORMAT_NAMES`, any case).
///
/// ```
/// use twine_cli::image::parse_format;
/// use twine_core::ColorFormat;
///
/// assert_eq!(parse_format("RGB565"), Ok(ColorFormat::Rgb565));
/// assert!(parse_format("rgb999").is_err());
/// ```
///
/// # Errors
/// A message listing the valid names when `s` is not one.
pub fn parse_format(s: &str) -> std::result::Result<ColorFormat, String> {
    FORMAT_NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(s))
        .map(|&(_, f)| f)
        .ok_or_else(|| {
            let all: Vec<_> = FORMAT_NAMES.iter().map(|(n, _)| *n).collect();
            format!("unknown format {s:?} (expected one of {})", all.join(", "))
        })
}

/// Parses a `--background` value: `#RRGGBB` (hex digits in either case).
///
/// ```
/// use twine_cli::image::parse_background;
/// use twine_core::Color;
///
/// assert_eq!(parse_background("#FFffFF"), Ok(Color::WHITE));
/// assert_eq!(parse_background("#22B868"), Ok(Color::hex(0x22B868)));
/// assert!(parse_background("FFFFFF").is_err());
/// assert!(parse_background("#FFF").is_err());
/// ```
///
/// # Errors
/// A message when `s` is not `#` followed by exactly six hex digits.
pub fn parse_background(s: &str) -> std::result::Result<Color, String> {
    s.strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .map(Color::hex)
        .ok_or_else(|| format!("invalid background {s:?} (expected #RRGGBB, e.g. #FFFFFF)"))
}

/// Arguments of `twine image`.
#[derive(Debug, Clone, clap::Args)]
pub struct ImageArgs {
    /// Input image (PNG, JPEG, BMP, GIF, QOI).
    #[arg(long = "in", value_name = "FILE", required_unless_present = "info")]
    pub input: Option<PathBuf>,
    /// Output color format: l8, a1, a2, a4, a8, i1, i2, i4, i8, rgb565, rgb565-swapped,
    /// rgb565a8, rgb888, argb8888, xrgb8888, argb8888-premultiplied.
    #[arg(long, value_parser = parse_format, required_unless_present = "info")]
    pub format: Option<ColorFormat>,
    /// Compression of the pixel data.
    #[arg(long, value_enum, default_value = "none")]
    pub compress: CompressArg,
    /// Floyd–Steinberg dithering (indexed formats) or ordered dithering (RGB565 formats).
    #[arg(long)]
    pub dither: bool,
    /// Premultiply colors by alpha (argb8888, rgb565a8).
    #[arg(long)]
    pub premultiply: bool,
    /// Composite the image over this color (`#RRGGBB`) before converting: the result is opaque
    /// in every format (alpha 255), and formats without alpha (l8, rgb565, rgb888, xrgb8888, …)
    /// show this color instead of black where the source is transparent. Not allowed with
    /// the alpha-only formats a1, a2, a4, a8. Without it, alpha is kept as is.
    #[arg(long, value_name = "#RRGGBB", value_parser = parse_background)]
    pub background: Option<Color>,
    /// Name of the generated static (e.g. `LOGO`).
    #[arg(long, required_unless_present_any = ["info", "bin"])]
    pub name: Option<String>,
    /// Write one raw twine binary file (`.tbin`: 12-byte header + pixel data, loaded at run
    /// time as `ImageSource::File`) to `--out` instead of Rust source.
    #[arg(long)]
    pub bin: bool,
    /// Output `.rs` file; the pixel data goes to a `.bin` file next to it (with `--bin`: the
    /// `.tbin` file).
    #[arg(long, value_name = "FILE", required_unless_present = "info")]
    pub out: Option<PathBuf>,
    /// Path of the crate that defines `Image` in the generated code.
    #[arg(long, default_value = "twine_image")]
    pub crate_path: String,
    /// Print the header and sizes of a converted image (`.rs` or its `.bin`) instead.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["input", "format", "name", "out"])]
    pub info: Option<PathBuf>,
}

/// The image builder of parsed arguments (not for `--info`); a relative input path is read
/// from the working directory.
///
/// # Errors
/// A message naming the missing required option (`--in`, `--format`, `--name` without
/// `--bin`).
pub fn image_from_args(a: &ImageArgs) -> Result<Image> {
    let input = a.input.clone().ok_or("missing --in")?;
    let name = match (&a.name, a.bin) {
        (Some(n), _) => n.clone(),
        (None, true) => "IMAGE".to_string(),
        (None, false) => return Err("missing --name".into()),
    };
    let mut img = Image::from_file(input)
        .format(a.format.ok_or("missing --format")?)
        .compress(a.compress.into())
        .dither(a.dither)
        .premultiply(a.premultiply)
        .name(&name)
        .crate_path(&a.crate_path);
    if let Some(bg) = a.background {
        img = img.background(bg);
    }
    Ok(img)
}

/// `twine image --info`: the summary line of a converted image and its file sizes.
///
/// # Errors
/// A message when the file cannot be read or was not generated by `twine image`.
pub fn info(path: &Path) -> Result<String> {
    let rs = if path.extension().is_some_and(|e| e == "bin") {
        path.with_extension("rs")
    } else {
        path.to_path_buf()
    };
    let text = std::fs::read_to_string(&rs).map_err(|e| format!("{}: {e}", rs.display()))?;
    let line = text
        .lines()
        .find(|l| l.starts_with("// twine-image:"))
        .ok_or_else(|| format!("{}: not generated by `twine image`", rs.display()))?;
    let mut out = String::new();
    for kv in line.trim_start_matches("// twine-image:").split_whitespace() {
        if let Some((k, v)) = kv.split_once('=') {
            let _ = writeln!(out, "{k:>12}: {v}");
        }
    }
    let bin = rs.with_extension("bin");
    if let Ok(m) = std::fs::metadata(&bin) {
        let _ = writeln!(out, "{:>12}: {} ({} bytes)", "file", bin.display(), m.len());
    }
    Ok(out)
}

/// Runs `twine image`.
///
/// # Errors
/// The errors of [`image_from_args`], `Image::build` and [`info`], or an I/O error writing
/// the output.
pub fn run(args: &ImageArgs) -> Result<()> {
    if let Some(p) = &args.info {
        print!("{}", info(p)?);
        return Ok(());
    }
    let out = args.out.clone().ok_or("missing --out")?;
    let g = image_from_args(args)?.build()?;
    if args.bin {
        let file = g.to_tbin();
        if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&out, &file)?;
        println!(
            "image: wrote {} ({}×{} {}, {} bytes)",
            out.display(),
            g.header.w,
            g.header.h,
            g.header.format.name(),
            file.len()
        );
        return Ok(());
    }
    g.write(&out)?;
    println!(
        "image: wrote {} and {} ({}×{} {}, {} bytes)",
        out.display(),
        twine_build::image::GeneratedImage::sidecar_name(&out),
        g.header.w,
        g.header.h,
        g.header.format.name(),
        g.stored.len()
    );
    Ok(())
}
