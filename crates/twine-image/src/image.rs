//! The image data model: [`ImageHeader`], [`ImageFlags`], [`Compression`], [`ImageData`],
//! [`Image`] and [`ImageSource`].

use alloc::boxed::Box;
use core::fmt;

use twine_core::ColorFormat;
use twine_render::ImagePixels;

use crate::Error;

bitflags::bitflags! {
    /// Properties of an image's pixel data.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct ImageFlags: u8 {
        /// Color channels are premultiplied by alpha.
        const PREMULTIPLIED = 1;
        /// The data is compressed ([`ImageData::Compressed`]).
        const COMPRESSED = 2;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ImageFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "ImageFlags({=u8:#x})", self.bits());
    }
}

/// Size and layout of an image (see [`ColorFormat`] for the byte layout of every format).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::ImageHeader;
/// let h = ImageHeader::new(ColorFormat::I4, 5, 2);
/// assert_eq!(h.stride, 3);
/// assert_eq!(h.data_size(), 16 * 4 + 3 * 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ImageHeader {
    /// Pixel format.
    pub format: ColorFormat,
    /// Width in pixels.
    pub w: u16,
    /// Height in pixels.
    pub h: u16,
    /// Bytes per row of the main plane.
    pub stride: u16,
    /// Flags.
    pub flags: ImageFlags,
}

impl ImageHeader {
    /// A packed header (minimum stride, no flags).
    #[must_use]
    pub const fn new(format: ColorFormat, w: u16, h: u16) -> Self {
        Self {
            format,
            w,
            h,
            stride: format.min_stride(w),
            flags: ImageFlags::empty(),
        }
    }

    /// Bytes of uncompressed pixel data (palette + main plane + alpha plane).
    #[must_use]
    pub const fn data_size(&self) -> usize {
        self.format.data_size(self.w, self.h, self.stride)
    }

    /// Whether the stride can hold a row.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.stride >= self.format.min_stride(self.w)
    }
}

/// Compression method of [`ImageData::Compressed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Compression {
    /// LVGL 9 run-length encoding (blocks of one pixel; 1 byte for sub-byte and indexed formats).
    Rle,
    /// LZ4 block format (feature `img-lz4`).
    Lz4,
}

/// Where an image's bytes live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageData {
    /// Uncompressed bytes in flash/ROM.
    Static(&'static [u8]),
    /// Uncompressed bytes on the heap. (A `Box`, not an `Rc`, so that `Image` stays `Sync`
    /// and can be a `static`; share an owned image by wrapping the whole `Image` in an `Rc`.)
    Owned(Box<[u8]>),
    /// Compressed bytes; decompressed into the image cache before drawing.
    Compressed {
        /// Compression method.
        method: Compression,
        /// Compressed bytes.
        data: &'static [u8],
        /// Size of the decompressed data (equals [`ImageHeader::data_size`]).
        decompressed_size: u32,
    },
}

/// An image: header plus pixel data. Converted images from `twine image` are
/// `pub static NAME: Image`.
///
/// Layout of the uncompressed data: the palette (indexed formats, `palette_len` `Argb8888`
/// entries), then `h` rows of `stride` bytes, then (for `Rgb565A8`) the `w × h` alpha plane.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{Image, ImageHeader};
///
/// static PIXELS: [u8; 4] = [0x00, 0xF8, 0x1F, 0x00]; // red, blue (RGB565 LE)
/// static IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::Rgb565, 2, 1), &PIXELS);
/// assert!(IMG.validate().is_ok());
/// let px = IMG.pixels().unwrap();
/// assert_eq!(px.row(0), &PIXELS);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Header.
    pub header: ImageHeader,
    /// Pixel data.
    pub data: ImageData,
}

impl Image {
    /// An image over static, uncompressed bytes.
    #[must_use]
    pub const fn new_static(header: ImageHeader, data: &'static [u8]) -> Self {
        Self {
            header,
            data: ImageData::Static(data),
        }
    }

    /// An image over heap bytes.
    #[must_use]
    pub fn new_owned(header: ImageHeader, data: Box<[u8]>) -> Self {
        Self {
            header,
            data: ImageData::Owned(data),
        }
    }

    /// The uncompressed bytes (`None` for compressed data).
    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.data {
            ImageData::Static(d) => Some(d),
            ImageData::Owned(d) => Some(d),
            ImageData::Compressed { .. } => None,
        }
    }

    /// Checks that the data is consistent with the header.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_image::{Error, Image, ImageHeader};
    /// let img = Image::new_static(ImageHeader::new(ColorFormat::Rgb888, 2, 2), &[0; 10]);
    /// assert_eq!(img.validate(), Err(Error::SizeMismatch { expected: 12, got: 10 }));
    /// ```
    pub fn validate(&self) -> Result<(), Error> {
        let h = &self.header;
        if !h.is_valid() {
            return Err(Error::InvalidHeader);
        }
        let expected = h.data_size();
        let got = match &self.data {
            ImageData::Static(d) => d.len(),
            ImageData::Owned(d) => d.len(),
            ImageData::Compressed {
                decompressed_size, ..
            } => *decompressed_size as usize,
        };
        if got < expected {
            return Err(Error::SizeMismatch { expected, got });
        }
        Ok(())
    }

    /// The palette of an indexed image (`palette_len · 4` bytes; `None` for other formats,
    /// compressed or short data).
    #[must_use]
    pub fn palette(&self) -> Option<&[u8]> {
        let n = self.header.format.palette_len() * 4;
        if n == 0 {
            return None;
        }
        self.bytes()?.get(..n)
    }

    /// Drawable pixels (`None` for compressed images, which need the cache, and for invalid
    /// images).
    #[must_use]
    pub fn pixels(&self) -> Option<ImagePixels<'_>> {
        pixels_of(&self.header, self.bytes()?)
    }
}

/// Drawable pixels of uncompressed `data` laid out as described by `header`.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{ImageHeader, pixels_of};
/// let data = [0u8; 2 * 4 + 2]; // I1: 2-entry palette, then 2 rows of 1 byte
/// let px = pixels_of(&ImageHeader::new(ColorFormat::I1, 3, 2), &data).unwrap();
/// assert_eq!(px.palette.unwrap().len(), 8);
/// ```
#[must_use]
pub fn pixels_of<'a>(header: &ImageHeader, data: &'a [u8]) -> Option<ImagePixels<'a>> {
    let h = header;
    if !h.is_valid() || data.len() < h.data_size() {
        return None;
    }
    let pal_len = h.format.palette_len() * 4;
    let (palette, rest) = data.split_at(pal_len);
    ImagePixels::from_parts(
        h.format,
        h.w,
        h.h,
        h.stride,
        rest,
        (pal_len > 0).then_some(palette),
        None,
        h.flags.contains(ImageFlags::PREMULTIPLIED),
    )
    .ok()
}

/// Longest path of [`ImageSource::File`].
pub const MAX_PATH_LEN: usize = 64;

/// What an image widget shows.
///
/// ```
/// use twine_image::ImageSource;
/// let s = ImageSource::file("/sd/logo.qoi").unwrap();
/// assert!(matches!(s, ImageSource::File(ref p) if p.as_str() == "/sd/logo.qoi"));
/// assert!(ImageSource::file(&"x".repeat(65)).is_none());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// A converted image in flash.
    Static(&'static Image),
    /// Encoded file bytes (QOI, PNG, JPEG, BMP, GIF) decoded through the decoder registry.
    Encoded(&'static [u8]),
    /// A file path read through a file source.
    File(heapless::String<MAX_PATH_LEN>),
    /// A symbol drawn as text (with the symbol font).
    Symbol(&'static str),
    /// SVG bytes, rendered by the vector renderer when enabled.
    Svg(&'static [u8]),
}

impl ImageSource {
    /// A file source (`None` when `path` is longer than [`MAX_PATH_LEN`] bytes).
    #[must_use]
    pub fn file(path: &str) -> Option<Self> {
        heapless::String::try_from(path).ok().map(ImageSource::File)
    }
}

impl From<&'static Image> for ImageSource {
    /// A converted image in flash: `ImageSource::from(&GEAR)`.
    fn from(img: &'static Image) -> Self {
        ImageSource::Static(img)
    }
}

impl fmt::Display for ImageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageSource::Static(i) => write!(f, "static {}x{} {}", i.header.w, i.header.h, i.header.format),
            ImageSource::Encoded(b) => write!(f, "encoded ({} bytes)", b.len()),
            ImageSource::File(p) => write!(f, "file {p}"),
            ImageSource::Symbol(s) => write!(f, "symbol {s:?}"),
            ImageSource::Svg(b) => write!(f, "svg ({} bytes)", b.len()),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ImageSource {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            ImageSource::Static(i) => defmt::write!(f, "static {}x{}", i.header.w, i.header.h),
            ImageSource::Encoded(b) => defmt::write!(f, "encoded ({} bytes)", b.len()),
            ImageSource::File(p) => defmt::write!(f, "file {=str}", p.as_str()),
            ImageSource::Symbol(s) => defmt::write!(f, "symbol {=str}", s),
            ImageSource::Svg(b) => defmt::write!(f, "svg ({} bytes)", b.len()),
        }
    }
}
