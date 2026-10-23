//! `twine image`: converts PNG, JPEG, BMP, GIF, QOI (anything the `image` crate reads) into a
//! `pub static <NAME>: Image` Rust file plus a `.bin` sidecar holding the pixel data, in any
//! `ColorFormat`, optionally RLE- or LZ4-compressed.
//!
//! The output only depends on the inputs (deterministic median cut, no timestamps), so
//! regenerated files diff cleanly. The first lines of the `.rs` file record the command, the
//! input's SHA-256 and a machine-readable summary (`twine image --info` prints it).

pub mod assets;
mod convert;
mod emit;
mod quantize;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use twine_core::{Color, ColorFormat};
use twine_image::{ImageFlags, ImageHeader, rle_block_size, rle_compress};

pub use convert::{Converted, convert, flatten, luminance};
pub use emit::{Emit, render, summary, variant};
pub use quantize::{Rgba, map_to_palette, median_cut};

use crate::font::sha256_hex;

/// Result type of the converter (errors are human-readable messages).
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

impl CompressArg {
    /// Command-line spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            CompressArg::None => "none",
            CompressArg::Rle => "rle",
            CompressArg::Lz4 => "lz4",
        }
    }
}

/// Command-line names of the formats, in `ColorFormat::ALL` order.
pub const FORMAT_NAMES: [(&str, ColorFormat); 16] = [
    ("l8", ColorFormat::L8),
    ("a1", ColorFormat::A1),
    ("a2", ColorFormat::A2),
    ("a4", ColorFormat::A4),
    ("a8", ColorFormat::A8),
    ("i1", ColorFormat::I1),
    ("i2", ColorFormat::I2),
    ("i4", ColorFormat::I4),
    ("i8", ColorFormat::I8),
    ("rgb565", ColorFormat::Rgb565),
    ("rgb565-swapped", ColorFormat::Rgb565Swapped),
    ("rgb565a8", ColorFormat::Rgb565A8),
    ("rgb888", ColorFormat::Rgb888),
    ("argb8888", ColorFormat::Argb8888),
    ("xrgb8888", ColorFormat::Xrgb8888),
    ("argb8888-premultiplied", ColorFormat::Argb8888Premultiplied),
];

/// Parses a `--format` value.
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

/// The command-line name of `f`.
#[must_use]
pub fn format_name(f: ColorFormat) -> &'static str {
    FORMAT_NAMES.iter().find(|(_, g)| *g == f).map_or("?", |(n, _)| n)
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

/// The `--background` spelling of `c` (`#RRGGBB`, upper case).
#[must_use]
pub fn background_name(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
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

/// Converter options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOptions {
    /// Input image.
    pub input: PathBuf,
    /// Output format.
    pub format: ColorFormat,
    /// Compression.
    pub compress: CompressArg,
    /// Dithering.
    pub dither: bool,
    /// Premultiplied alpha.
    pub premultiply: bool,
    /// Composite over this color first (`--background`): every pixel becomes opaque. `None`
    /// keeps the source's alpha.
    pub background: Option<Color>,
    /// Static name.
    pub name: String,
    /// Output `.rs` path.
    pub out: PathBuf,
    /// Crate path used in the generated code.
    pub crate_path: String,
    /// Write a `.tbin` file instead of Rust source.
    pub bin: bool,
}

impl ImageOptions {
    /// Options from command-line arguments.
    ///
    /// # Errors
    /// A message naming the missing required option (`--name` or `--out` to derive it from, `--in`,
    /// `--format`).
    pub fn from_args(a: &ImageArgs) -> Result<Self> {
        fn need<T>(v: Option<T>, what: &str) -> std::result::Result<T, String> {
            v.ok_or_else(|| format!("missing --{what}"))
        }
        let name = match (&a.name, a.bin) {
            (Some(n), _) => n.clone(),
            (None, true) => "IMAGE".to_string(),
            (None, false) => need(None, "name")?,
        };
        Ok(Self {
            input: need(a.input.clone(), "in")?,
            format: need(a.format, "format")?,
            compress: a.compress,
            dither: a.dither,
            premultiply: a.premultiply,
            background: a.background,
            name,
            out: need(a.out.clone(), "out")?,
            crate_path: a.crate_path.clone(),
            bin: a.bin,
        })
    }

    /// The equivalent command line (recorded in the output).
    #[must_use]
    pub fn command(&self) -> String {
        let mut s = format!(
            "twine image --in {} --format {}",
            self.input.display(),
            format_name(self.format)
        );
        if self.compress != CompressArg::None {
            let _ = write!(s, " --compress {}", self.compress.name());
        }
        if self.dither {
            s += " --dither";
        }
        if self.premultiply {
            s += " --premultiply";
        }
        if let Some(bg) = self.background {
            let _ = write!(s, " --background {}", background_name(bg));
        }
        if self.bin {
            let _ = write!(s, " --bin --out {}", self.out.display());
            return s;
        }
        let _ = write!(s, " --name {} --out {}", self.name, self.out.display());
        if self.crate_path != "twine_image" {
            let _ = write!(s, " --crate-path {}", self.crate_path);
        }
        s
    }

    /// File name of the sidecar (`<out stem>.bin`).
    #[must_use]
    pub fn bin_name(&self) -> String {
        let stem = self
            .out
            .file_stem()
            .map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned());
        format!("{stem}.bin")
    }
}

/// A conversion result.
#[derive(Debug, Clone)]
pub struct Generated {
    /// Header of the pixel data (`COMPRESSED` set when compressed).
    pub header: ImageHeader,
    /// Uncompressed pixel data.
    pub data: Vec<u8>,
    /// Bytes of the `.bin` sidecar (compressed or not).
    pub bin: Vec<u8>,
    /// File name of the sidecar.
    pub bin_name: String,
    /// The `.rs` source.
    pub source: String,
}

/// Compresses `data` of `format`.
#[must_use]
pub fn compress(data: &[u8], format: ColorFormat, method: CompressArg) -> Vec<u8> {
    match method {
        CompressArg::None => data.to_vec(),
        CompressArg::Rle => rle_compress(data, rle_block_size(format)),
        CompressArg::Lz4 => lz4_flex::block::compress(data),
    }
}

/// Converts in-memory RGBA pixels (the part of [`generate`] after decoding the input).
///
/// With [`ImageOptions::background`], the pixels are first composited over that color
/// ([`flatten`]).
///
/// # Errors
/// A message when the image is wider or taller than 65 535 pixels, when a background is given
/// for an alpha-only format (A1…A8: flattening would leave a fully opaque mask), or when it
/// cannot be converted to the requested format.
pub fn generate_rgba(
    rgba: &[u8],
    w: u32,
    h: u32,
    input_sha256: &str,
    opts: &ImageOptions,
) -> Result<Generated> {
    let (w16, h16) = (
        u16::try_from(w).map_err(|_| "image too wide (max 65535)")?,
        u16::try_from(h).map_err(|_| "image too tall (max 65535)")?,
    );
    let flat;
    let rgba = match opts.background {
        None => rgba,
        Some(_)
            if matches!(
                opts.format,
                ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8
            ) =>
        {
            return Err(format!(
                "--background needs a format with color (not {})",
                format_name(opts.format)
            )
            .into());
        }
        Some(bg) => {
            flat = flatten(rgba, bg);
            &flat[..]
        }
    };
    let Converted { mut header, data } = convert(rgba, w16, h16, opts.format, opts.dither, opts.premultiply)?;
    let bin = compress(&data, opts.format, opts.compress);
    if opts.compress != CompressArg::None {
        header.flags |= ImageFlags::COMPRESSED;
    }
    let bin_name = opts.bin_name();
    let command = opts.command();
    let source = render(&Emit {
        name: &opts.name,
        crate_path: &opts.crate_path,
        command: &command,
        input_sha256,
        header,
        compress: opts.compress,
        bin_name: &bin_name,
        stored_len: bin.len(),
        data_len: data.len(),
    });
    Ok(Generated {
        header,
        data,
        bin,
        bin_name,
        source,
    })
}

/// Reads and converts `opts.input` (relative paths are resolved against `root`).
///
/// # Errors
/// A message when the input cannot be read or decoded, and the errors of [`generate_rgba`].
pub fn generate(opts: &ImageOptions, root: &Path) -> Result<Generated> {
    let path = root.join(&opts.input);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let rgba = img.to_rgba8();
    generate_rgba(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
        &sha256_hex(&bytes),
        opts,
    )
}

/// Writes the `.rs` file and its sidecar.
///
/// # Errors
/// An I/O error creating the output directory or writing either file.
pub fn write(g: &Generated, out: &Path) -> Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, &g.source)?;
    std::fs::write(out.with_file_name(&g.bin_name), &g.bin)?;
    Ok(())
}

/// The `.tbin` file of a conversion (`twine image --bin`).
#[must_use]
pub fn tbin(g: &Generated, method: CompressArg) -> Vec<u8> {
    let compression = match method {
        CompressArg::None => None,
        CompressArg::Rle => Some(twine_image::Compression::Rle),
        CompressArg::Lz4 => Some(twine_image::Compression::Lz4),
    };
    twine_image::decoders::tbin::encode_tbin(g.header, compression, &g.bin)
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
/// The errors of [`ImageOptions::from_args`], [`generate`], [`write`](fn@write) and [`info`].
pub fn run(args: &ImageArgs) -> Result<()> {
    if let Some(p) = &args.info {
        print!("{}", info(p)?);
        return Ok(());
    }
    let opts = ImageOptions::from_args(args)?;
    let g = generate(&opts, Path::new("."))?;
    if opts.bin {
        let file = tbin(&g, opts.compress);
        if let Some(dir) = opts.out.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&opts.out, &file)?;
        println!(
            "image: wrote {} ({}×{} {}, {} bytes)",
            opts.out.display(),
            g.header.w,
            g.header.h,
            g.header.format.name(),
            file.len()
        );
        return Ok(());
    }
    write(&g, &opts.out)?;
    println!(
        "image: wrote {} and {} ({}×{} {}, {} bytes)",
        opts.out.display(),
        g.bin_name,
        g.header.w,
        g.header.h,
        g.header.format.name(),
        g.bin.len()
    );
    Ok(())
}
