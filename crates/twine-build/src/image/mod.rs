//! Images from PNG, JPEG, BMP, GIF or QOI files (feature `image`): a deterministic converter
//! that writes a `static` `twine::image::Image` in any [`ColorFormat`] as Rust source plus a
//! sidecar file holding the pixel data, optionally RLE- or LZ4-compressed.
//!
//! ```no_run
//! // build.rs
//! use twine_build::image::{ColorFormat, Compression, Image};
//!
//! Image::from_file("assets/logo.png")
//!     .format(ColorFormat::Rgb565)
//!     .compress(Compression::Rle)
//!     .name("LOGO")
//!     .build()
//!     .unwrap_or_else(|e| panic!("{e}"))
//!     .write_to_out_dir("logo.rs") // and logo.bin next to it
//!     .unwrap_or_else(|e| panic!("{e}"));
//! ```
//!
//! ```ignore
//! // src/main.rs
//! include!(concat!(env!("OUT_DIR"), "/logo.rs")); // pub static LOGO: twine::image::Image
//! ```
//!
//! Conversion: `A1`…`A8` keep the alpha channel, `L8` the luminance, `I1`…`I8` get a
//! deterministic median-cut palette (Floyd–Steinberg with [`dither`](Image::dither)),
//! `Rgb565*` are rounded or ordered-dithered; [`premultiply`](Image::premultiply) premultiplies
//! `Argb8888` and `Rgb565A8`; [`background`](Image::background) composites over an opaque
//! color first. The output depends only on the inputs (no timestamps), so regenerated files
//! diff cleanly; the first lines of the source record the equivalent `twine image` command,
//! the input's SHA-256 and a machine-readable summary (`twine image --info` prints it).

mod convert;
mod emit;
mod quantize;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub use twine_core::{Color, ColorFormat};
use twine_image::{ImageFlags, ImageHeader, rle_block_size, rle_compress};

pub use convert::{Converted, convert, flatten, luminance};
use emit::{DataRef, Emit, render, summary};
pub use quantize::{Rgba, map_to_palette, median_cut};

use crate::{
    AssetError, is_ident, is_path, is_visibility, out_dir, sha256_hex, static_name, write_if_changed,
};

/// How the pixel data is stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Compression {
    /// Uncompressed: drawn straight from flash.
    #[default]
    None,
    /// LVGL run-length encoding (decoded into the image cache when drawn).
    Rle,
    /// An LZ4 block (needs `twine-image` feature `img-lz4` at run time).
    Lz4,
}

impl Compression {
    /// The command-line spelling (`none`, `rle`, `lz4`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Compression::None => "none",
            Compression::Rle => "rle",
            Compression::Lz4 => "lz4",
        }
    }

    /// Compresses `data` of `format` with this method.
    #[must_use]
    pub fn compress(self, data: &[u8], format: ColorFormat) -> Vec<u8> {
        match self {
            Compression::None => data.to_vec(),
            Compression::Rle => rle_compress(data, rle_block_size(format)),
            Compression::Lz4 => lz4_flex::block::compress(data),
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

/// The command-line name of `f` (`rgb565`, `argb8888-premultiplied`, …; see [`FORMAT_NAMES`]).
///
/// ```
/// use twine_build::image::{ColorFormat, format_name};
/// assert_eq!(format_name(ColorFormat::Rgb565Swapped), "rgb565-swapped");
/// ```
#[must_use]
pub fn format_name(f: ColorFormat) -> &'static str {
    FORMAT_NAMES.iter().find(|(_, g)| *g == f).map_or("?", |(n, _)| n)
}

/// The command-line spelling of `c` (`#RRGGBB`, upper case).
///
/// ```
/// use twine_build::image::{Color, color_name};
/// assert_eq!(color_name(Color::hex(0x22B868)), "#22B868");
/// ```
#[must_use]
pub fn color_name(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Input {
    File(PathBuf),
    Rgba { w: u32, h: u32, pixels: Vec<u8> },
}

/// An image to convert: a builder over the source image and the options. See the
/// [module docs](self).
///
/// A relative input path is read relative to [`root`](Image::root) — by default the current
/// directory, which in a build script is the package's directory — and is written as given
/// into the generated header.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct Image {
    input: Input,
    root: PathBuf,
    format: ColorFormat,
    compress: Compression,
    dither: bool,
    premultiply: bool,
    background: Option<Color>,
    name: Option<String>,
    visibility: String,
    crate_path: String,
}

impl Image {
    fn new(input: Input) -> Image {
        Image {
            input,
            root: PathBuf::new(),
            format: ColorFormat::Argb8888,
            compress: Compression::None,
            dither: false,
            premultiply: false,
            background: None,
            name: None,
            visibility: "pub".into(),
            crate_path: "twine::image".into(),
        }
    }

    /// An image converted from the file `path` (PNG, JPEG, BMP, GIF — first frame — or QOI,
    /// recognised by content), with the defaults: [`ColorFormat::Argb8888`], uncompressed, no
    /// dithering, straight alpha kept, `pub`, the static named after the file stem
    /// (`logo.png` → `LOGO`), paths through the `twine` facade (`twine::image`).
    ///
    /// ```
    /// use twine_build::image::Image;
    /// assert_eq!(Image::from_file("assets/power-on.png").static_name(), "POWER_ON");
    /// ```
    pub fn from_file(path: impl Into<PathBuf>) -> Image {
        Image::new(Input::File(path.into()))
    }

    /// An image converted from `w × h` straight-alpha RGBA pixels (`4 × w × h` bytes, row by
    /// row), e.g. drawn by the build script. Named `IMAGE` unless [`name`](Image::name)d.
    ///
    /// ```
    /// use twine_build::image::{ColorFormat, Image};
    ///
    /// let red = [255, 0, 0, 255].repeat(4);
    /// let g = Image::from_rgba(2, 2, red).format(ColorFormat::Rgb565).build().unwrap();
    /// assert_eq!(g.data, [0x00, 0xF8].repeat(4));
    /// ```
    pub fn from_rgba(w: u32, h: u32, pixels: Vec<u8>) -> Image {
        Image::new(Input::Rgba { w, h, pixels })
    }

    /// The directory a relative input path is read from (default: the current directory).
    pub fn root(mut self, dir: impl Into<PathBuf>) -> Image {
        self.root = dir.into();
        self
    }

    /// The color format of the pixel data (default [`ColorFormat::Argb8888`]).
    pub fn format(mut self, format: ColorFormat) -> Image {
        self.format = format;
        self
    }

    /// How the pixel data is stored (default [`Compression::None`]).
    pub fn compress(mut self, compression: Compression) -> Image {
        self.compress = compression;
        self
    }

    /// Floyd–Steinberg dithering (indexed formats) or 4 × 4 ordered dithering (RGB565
    /// formats); default off.
    pub fn dither(mut self, dither: bool) -> Image {
        self.dither = dither;
        self
    }

    /// Premultiplies the colors by alpha (`Argb8888`, `Rgb565A8`; sets
    /// `ImageFlags::PREMULTIPLIED`); default off.
    pub fn premultiply(mut self, premultiply: bool) -> Image {
        self.premultiply = premultiply;
        self
    }

    /// Composites the image over the opaque color `bg` before converting: the result is opaque
    /// in every format, and formats without alpha show `bg` instead of black where the source
    /// is transparent. Not allowed with the alpha-only formats `A1`…`A8` (checked by
    /// [`build`](Image::build)). Default: alpha kept as is.
    pub fn background(mut self, bg: Color) -> Image {
        self.background = Some(bg);
        self
    }

    /// The name of the generated static (a Rust identifier, checked by
    /// [`build`](Image::build)).
    pub fn name(mut self, name: &str) -> Image {
        self.name = Some(name.to_string());
        self
    }

    /// The visibility of the generated static (default `pub`; `""` for private,
    /// `pub(crate)`, …).
    pub fn visibility(mut self, visibility: &str) -> Image {
        self.visibility = visibility.to_string();
        self
    }

    /// The path through which the generated code names the image crate (default
    /// `twine::image`, for applications using the `twine` facade; `twine_image` for code that
    /// depends on `twine-image` directly).
    pub fn crate_path(mut self, path: &str) -> Image {
        self.crate_path = path.to_string();
        self
    }

    /// The name of the generated static: [`name`](Image::name), or the file stem.
    #[must_use]
    pub fn static_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| match &self.input {
            Input::File(p) => static_name(&[&p
                .file_stem()
                .map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned())]),
            Input::Rgba { .. } => "IMAGE".into(),
        })
    }

    /// Reads (for a file input) and converts the image.
    ///
    /// # Errors
    /// An unreadable or undecodable input file, an image wider or taller than 65 535 pixels,
    /// an RGBA buffer of the wrong size, premultiplied alpha for a format without both color and
    /// alpha, a background for an alpha-only format, a name that is not a Rust identifier, or
    /// an invalid crate path or visibility.
    pub fn build(&self) -> Result<GeneratedImage, AssetError> {
        let name = self.static_name();
        if !is_ident(&name) {
            return Err(format!("name `{name}` is not a Rust identifier").into());
        }
        if !is_path(&self.crate_path) {
            return Err(format!("crate path `{}` is not a Rust path", self.crate_path).into());
        }
        if !is_visibility(&self.visibility) {
            return Err(format!("`{}` is not a visibility", self.visibility).into());
        }
        let (decoded, input_sha256, inputs);
        let (rgba, w, h): (&[u8], u32, u32) = match &self.input {
            Input::File(p) => {
                let path = self.root.join(p);
                let bytes = std::fs::read(&path).map_err(|e| AssetError::file(&path, e))?;
                decoded = ::image::load_from_memory(&bytes)
                    .map_err(|e| AssetError::file(&path, e))?
                    .to_rgba8();
                input_sha256 = sha256_hex(&bytes);
                inputs = vec![path];
                (decoded.as_raw(), decoded.width(), decoded.height())
            }
            Input::Rgba { w, h, pixels } => {
                input_sha256 = sha256_hex(pixels);
                inputs = Vec::new();
                (pixels, *w, *h)
            }
        };
        let (w16, h16) = (
            u16::try_from(w).map_err(|_| AssetError::new("image too wide (max 65535)"))?,
            u16::try_from(h).map_err(|_| AssetError::new("image too tall (max 65535)"))?,
        );
        let flat;
        let rgba = match self.background {
            None => rgba,
            Some(_)
                if matches!(
                    self.format,
                    ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8
                ) =>
            {
                return Err(format!(
                    "a background needs a format with color (not {})",
                    format_name(self.format)
                )
                .into());
            }
            Some(bg) => {
                flat = flatten(rgba, bg);
                &flat[..]
            }
        };
        let Converted { mut header, data } =
            convert(rgba, w16, h16, self.format, self.dither, self.premultiply).map_err(AssetError::new)?;
        let stored = self.compress.compress(&data, self.format);
        if self.compress != Compression::None {
            header.flags |= ImageFlags::COMPRESSED;
        }
        Ok(GeneratedImage {
            header,
            data,
            stored,
            compression: self.compress,
            name,
            visibility: self.visibility.clone(),
            crate_path: self.crate_path.clone(),
            command: self.command_prefix(),
            input_sha256,
            inputs,
        })
    }

    /// The `twine image` command line up to `--name` / `--out` (completed by
    /// [`GeneratedImage::to_rust`]).
    fn command_prefix(&self) -> String {
        let input = match &self.input {
            Input::File(p) => p.display().to_string(),
            Input::Rgba { w, h, .. } => format!("<{w}×{h} RGBA pixels>"),
        };
        let mut s = format!("twine image --in {input} --format {}", format_name(self.format));
        if self.compress != Compression::None {
            let _ = write!(s, " --compress {}", self.compress.name());
        }
        if self.dither {
            s += " --dither";
        }
        if self.premultiply {
            s += " --premultiply";
        }
        if let Some(bg) = self.background {
            let _ = write!(s, " --background {}", color_name(bg));
        }
        s
    }
}

/// A converted image: its header and pixel data, rendered as Rust source by
/// [`to_rust`](GeneratedImage::to_rust) and written with [`write`](GeneratedImage::write) or
/// [`write_to_out_dir`](GeneratedImage::write_to_out_dir).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedImage {
    /// Header of the pixel data (`COMPRESSED` set when compressed).
    pub header: ImageHeader,
    /// Uncompressed pixel data.
    pub data: Vec<u8>,
    /// The stored pixel data (the sidecar file's bytes: compressed or not).
    pub stored: Vec<u8>,
    /// The compression of [`stored`](GeneratedImage::stored).
    pub compression: Compression,
    name: String,
    visibility: String,
    crate_path: String,
    command: String,
    input_sha256: String,
    inputs: Vec<PathBuf>,
}

impl GeneratedImage {
    /// The file name of the sidecar holding the pixel data of a source written to `out`
    /// (`<out stem>.bin`).
    ///
    /// ```
    /// use twine_build::image::GeneratedImage;
    /// assert_eq!(GeneratedImage::sidecar_name("assets/logo.rs".as_ref()), "logo.bin");
    /// ```
    #[must_use]
    pub fn sidecar_name(out: &Path) -> String {
        let stem = out
            .file_stem()
            .map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned());
        format!("{stem}.bin")
    }

    fn command(&self, out: &Path) -> String {
        let mut s = format!("{} --name {} --out {}", self.command, self.name, out.display());
        if self.crate_path != "twine_image" {
            let _ = write!(s, " --crate-path {}", self.crate_path);
        }
        s
    }

    fn emit<'a>(&'a self, command: &'a str, data: DataRef<'a>) -> Emit<'a> {
        Emit {
            name: &self.name,
            visibility: &self.visibility,
            crate_path: &self.crate_path,
            command,
            input_sha256: &self.input_sha256,
            header: self.header,
            compress: self.compression,
            data,
            stored_len: self.stored.len(),
            data_len: self.data.len(),
        }
    }

    /// The Rust source of the image written to `out` (recorded in the header as given): the
    /// static includes its pixel data from the sidecar [`sidecar_name`](Self::sidecar_name)
    /// next to it.
    ///
    /// ```
    /// use twine_build::image::{ColorFormat, Image};
    ///
    /// let g = Image::from_rgba(1, 1, vec![0, 0, 0, 255]).format(ColorFormat::L8).name("DOT").build().unwrap();
    /// let src = g.to_rust("dot.rs".as_ref());
    /// assert!(src.contains("pub static DOT: twine::image::Image"));
    /// assert!(src.contains("include_bytes!(\"dot.bin\")"));
    /// ```
    #[must_use]
    pub fn to_rust(&self, out: &Path) -> String {
        let command = self.command(out);
        let bin = Self::sidecar_name(out);
        render(&self.emit(&command, DataRef::Sidecar(&bin)))
    }

    /// The Rust source with the pixel data inline (a byte-string literal) instead of a
    /// sidecar file: one self-contained item, as the `include_image!` macro expands to.
    #[must_use]
    pub fn to_rust_inline(&self) -> String {
        let command = format!("{} --name {}", self.command, self.name);
        render(&self.emit(&command, DataRef::Inline(&self.stored)))
    }

    /// The machine-readable summary line of the source (`// twine-image: format=… w=… …`).
    ///
    /// ```
    /// use twine_build::image::{ColorFormat, Image};
    ///
    /// let g = Image::from_rgba(2, 1, vec![0; 8]).format(ColorFormat::Rgb565).build().unwrap();
    /// assert_eq!(
    ///     g.summary(),
    ///     "// twine-image: format=RGB565 w=2 h=1 stride=4 flags=0x00 compression=none stored=4 size=4"
    /// );
    /// ```
    #[must_use]
    pub fn summary(&self) -> String {
        summary(&self.emit("", DataRef::Sidecar("")))
    }

    /// The raw twine binary file (`.tbin`: 12-byte header and the stored pixel data), loaded
    /// at run time as `ImageSource::File` (`twine image --bin`).
    #[must_use]
    pub fn to_tbin(&self) -> Vec<u8> {
        let compression = match self.compression {
            Compression::None => None,
            Compression::Rle => Some(twine_image::Compression::Rle),
            Compression::Lz4 => Some(twine_image::Compression::Lz4),
        };
        twine_image::decoders::tbin::encode_tbin(self.header, compression, &self.stored)
    }

    /// Writes the source to `out` and the pixel data to the sidecar next to it (each only when
    /// it changed; the parent directory is created).
    ///
    /// # Errors
    /// When a file cannot be written.
    pub fn write(&self, out: impl AsRef<Path>) -> Result<(), AssetError> {
        let out = out.as_ref();
        write_if_changed(out, self.to_rust(out).as_bytes())?;
        write_if_changed(&out.with_file_name(Self::sidecar_name(out)), &self.stored)
    }

    /// The build-script convenience: writes the source to `$OUT_DIR/<file_name>` and the
    /// sidecar next to it, prints `cargo:rerun-if-changed` for the input file, and returns the
    /// path of the source. Include it with `include!(concat!(env!("OUT_DIR"), "/<file_name>"))`.
    ///
    /// # Errors
    /// When `OUT_DIR` is not set (not in a build script) or a file cannot be written.
    pub fn write_to_out_dir(&self, file_name: &str) -> Result<PathBuf, AssetError> {
        let dir = out_dir("GeneratedImage::write_to_out_dir")?;
        let rel = Path::new(file_name);
        let path = dir.join(rel);
        write_if_changed(&path, self.to_rust(rel).as_bytes())?;
        write_if_changed(&path.with_file_name(Self::sidecar_name(rel)), &self.stored)?;
        for input in &self.inputs {
            println!("cargo:rerun-if-changed={}", input.display());
        }
        Ok(path)
    }

    /// Every file read (the input image, for a file input).
    #[must_use]
    pub fn inputs(&self) -> &[PathBuf] {
        &self.inputs
    }
}
