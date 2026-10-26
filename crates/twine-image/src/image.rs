//! The image data model: [`ImageHeader`], [`ImageFlags`], [`Compression`], [`ImageData`],
//! [`Image`] and [`ImageSource`].

use alloc::boxed::Box;
use core::fmt;

use twine_core::{ColorFormat, DisplayScale, Symbol};
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
    /// # Errors
    /// [`Error::InvalidHeader`] for an inconsistent header; [`Error::SizeMismatch`] when the data
    /// is shorter than the header requires.
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
        let n = self.header.format.palette_len().saturating_mul(4);
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
    let pal_len = h.format.palette_len().saturating_mul(4);
    // `data_size()` includes the palette (checked above): never `None`.
    let (palette, rest) = data.split_at_checked(pal_len)?;
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
    /// Text drawn with the widget's font instead of an image: usually a built-in [`Symbol`]
    /// (`ImageSource::from(Symbol::Ok)` or [`ImageSource::symbol`]), or any `'static` text,
    /// e.g. a glyph of an application icon font.
    Symbol(&'static str),
    /// SVG bytes, rendered by the vector renderer when enabled.
    Svg(&'static [u8]),
    /// A converted image with renderings for scaled (hi-DPI) displays: the image widget draws
    /// the rendering for its display's scale ([`ImageSource::for_scale`]); everything else
    /// draws the 1× image.
    Scaled(&'static ScaledImage),
}

impl ImageSource {
    /// The source to draw on a display of `scale` (device pixels per logical pixel) and the
    /// scale its pixels were made for: a [`Scaled`](Self::Scaled) image's rendering
    /// ([`ScaledImage::pick`]), every other source itself at 1×. Draw the result zoomed by
    /// `scale / made-for scale` to keep the image's logical size.
    ///
    /// ```
    /// use twine_core::{ColorFormat, DisplayScale};
    /// use twine_image::{Image, ImageHeader, ImageSource, ScaledImage};
    ///
    /// static ICON: Image = Image::new_static(ImageHeader::new(ColorFormat::A8, 2, 2), &[0; 4]);
    /// static ICON_2X: Image = Image::new_static(ImageHeader::new(ColorFormat::A8, 4, 4), &[0; 16]);
    /// static SET: ScaledImage = ScaledImage::new(&ICON, &[(DisplayScale::X2, &ICON_2X)]);
    ///
    /// let (src, made_for) = ImageSource::Scaled(&SET).for_scale(DisplayScale::X2);
    /// assert_eq!((src, made_for), (ImageSource::Static(&ICON_2X), DisplayScale::X2));
    /// let plain = ImageSource::Static(&ICON);
    /// assert_eq!(plain.for_scale(DisplayScale::X2), (plain.clone(), DisplayScale::ONE));
    /// ```
    #[must_use]
    pub fn for_scale(&self, scale: DisplayScale) -> (ImageSource, DisplayScale) {
        match self {
            ImageSource::Scaled(s) => {
                let (img, made_for) = s.pick(scale);
                (ImageSource::Static(img), made_for)
            }
            other => (other.clone(), DisplayScale::ONE),
        }
    }

    /// A file source (`None` when `path` is longer than [`MAX_PATH_LEN`] bytes).
    #[must_use]
    pub fn file(path: &str) -> Option<Self> {
        heapless::String::try_from(path).ok().map(ImageSource::File)
    }

    /// A built-in symbol, drawn as text with the symbol font (`const`: usable in `static`s).
    ///
    /// ```
    /// use twine_image::{ImageSource, Symbol};
    /// static CHECK: ImageSource = ImageSource::symbol(Symbol::Ok);
    /// assert_eq!(CHECK, ImageSource::from(Symbol::Ok));
    /// assert_eq!(CHECK, ImageSource::Symbol("\u{F00C}"));
    /// ```
    #[must_use]
    #[inline]
    pub const fn symbol(s: Symbol) -> Self {
        ImageSource::Symbol(s.as_str())
    }
}

impl From<Symbol> for ImageSource {
    /// A built-in symbol: `ImageSource::from(Symbol::Close)` (no allocation).
    #[inline]
    fn from(s: Symbol) -> Self {
        ImageSource::symbol(s)
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
            ImageSource::Scaled(sc) => {
                let i = sc.base();
                write!(
                    f,
                    "scaled {}x{} {} (+{})",
                    i.header.w,
                    i.header.h,
                    i.header.format,
                    sc.variants().len()
                )
            }
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
            ImageSource::Scaled(sc) => {
                defmt::write!(f, "scaled {}x{}", sc.base().header.w, sc.base().header.h)
            }
        }
    }
}

/// A converted image with renderings for scaled (hi-DPI) displays: the 1× `base` and
/// `variants`, each the same picture rendered for a display scale (e.g. a 2× PNG exported at
/// twice the size). The image widget draws the smallest rendering made for at least its
/// display's scale (the largest one beyond that), zoomed to the image's logical size, so icons
/// stay sharp on Retina screens; on 1× displays and everywhere else (backgrounds, image
/// buttons) the base is drawn. Only the variants listed are linked.
///
/// The renderings are separate images (e.g. two `include_image!` statics of `logo.png` and
/// `logo@2x.png`); the asset generators do not make them from one source. An image widget with
/// `ImageAlign::Tile` repeats the chosen rendering's own pixels (tiles are not zoomed to the
/// display scale).
///
/// ```
/// use twine_core::{ColorFormat, DisplayScale};
/// use twine_image::{Image, ImageHeader, ImageSource, ScaledImage};
///
/// static LOGO: Image = Image::new_static(ImageHeader::new(ColorFormat::A8, 8, 8), &[0; 64]);
/// static LOGO_2X: Image = Image::new_static(ImageHeader::new(ColorFormat::A8, 16, 16), &[0; 256]);
/// static LOGO_SET: ScaledImage = ScaledImage::new(&LOGO, &[(DisplayScale::X2, &LOGO_2X)]);
/// static SRC: ImageSource = ImageSource::Scaled(&LOGO_SET);
///
/// assert!(core::ptr::eq(LOGO_SET.pick(DisplayScale::pct(150)).0, &LOGO_2X)); // ≥ 1.5×
/// assert!(core::ptr::eq(LOGO_SET.pick(DisplayScale::X3).0, &LOGO_2X)); // the largest
/// assert!(core::ptr::eq(LOGO_SET.pick(DisplayScale::ONE).0, &LOGO));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaledImage {
    base: &'static Image,
    variants: &'static [(DisplayScale, &'static Image)],
}

impl ScaledImage {
    /// `base` (1×) with `variants`, each rendered for a display scale.
    #[must_use]
    pub const fn new(base: &'static Image, variants: &'static [(DisplayScale, &'static Image)]) -> Self {
        Self { base, variants }
    }

    /// The 1× image.
    #[must_use]
    pub const fn base(&self) -> &'static Image {
        self.base
    }

    /// The renderings for scaled displays.
    #[must_use]
    pub const fn variants(&self) -> &'static [(DisplayScale, &'static Image)] {
        self.variants
    }

    /// The image to draw at `scale` and the scale it was made for: the smallest rendering
    /// made for at least `scale` (downscaling keeps detail), else the largest; the base
    /// counts as made for 1×.
    #[must_use]
    pub fn pick(&self, scale: DisplayScale) -> (&'static Image, DisplayScale) {
        let all = core::iter::once((DisplayScale::ONE, self.base)).chain(self.variants.iter().copied());
        let at_least = all.clone().filter(|(s, _)| *s >= scale).min_by_key(|(s, _)| *s);
        let (s, img) = at_least.unwrap_or_else(|| {
            all.max_by_key(|(s, _)| *s)
                .unwrap_or((DisplayScale::ONE, self.base))
        });
        (img, s)
    }
}
