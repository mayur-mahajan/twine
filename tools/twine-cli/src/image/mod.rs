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

use std::path::{Path, PathBuf};

use twine_core::ColorFormat;
use twine_image::{ImageFlags, ImageHeader, rle_block_size, rle_compress};

pub use convert::{Converted, convert, luminance};
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
    /// Name of the generated static (e.g. `LOGO`).
    #[arg(long, required_unless_present = "info")]
    pub name: Option<String>,
    /// Output `.rs` file; the pixel data goes to a `.bin` file next to it.
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
    /// Static name.
    pub name: String,
    /// Output `.rs` path.
    pub out: PathBuf,
    /// Crate path used in the generated code.
    pub crate_path: String,
}

impl ImageOptions {
    /// Options from command-line arguments.
    pub fn from_args(a: &ImageArgs) -> Result<Self> {
        fn need<T>(v: Option<T>, what: &str) -> std::result::Result<T, String> {
            v.ok_or_else(|| format!("missing --{what}"))
        }
        Ok(Self {
            input: need(a.input.clone(), "in")?,
            format: need(a.format, "format")?,
            compress: a.compress,
            dither: a.dither,
            premultiply: a.premultiply,
            name: need(a.name.clone(), "name")?,
            out: need(a.out.clone(), "out")?,
            crate_path: a.crate_path.clone(),
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
            s += &format!(" --compress {}", self.compress.name());
        }
        if self.dither {
            s += " --dither";
        }
        if self.premultiply {
            s += " --premultiply";
        }
        s += &format!(" --name {} --out {}", self.name, self.out.display());
        if self.crate_path != "twine_image" {
            s += &format!(" --crate-path {}", self.crate_path);
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
pub fn write(g: &Generated, out: &Path) -> Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, &g.source)?;
    std::fs::write(out.with_file_name(&g.bin_name), &g.bin)?;
    Ok(())
}

/// `twine image --info`: the summary line of a converted image and its file sizes.
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
            out += &format!("{k:>12}: {v}\n");
        }
    }
    let bin = rs.with_extension("bin");
    if let Ok(m) = std::fs::metadata(&bin) {
        out += &format!("{:>12}: {} ({} bytes)\n", "file", bin.display(), m.len());
    }
    Ok(out)
}

/// Runs `twine image`.
pub fn run(args: &ImageArgs) -> Result<()> {
    if let Some(p) = &args.info {
        print!("{}", info(p)?);
        return Ok(());
    }
    let opts = ImageOptions::from_args(args)?;
    let g = generate(&opts, Path::new("."))?;
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
