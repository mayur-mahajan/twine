//! The [`Decoder`] trait and the [`DecoderRegistry`] that tries decoders in order.

use alloc::vec::Vec;

use twine_core::ColorFormat;

use crate::{Error, ImageHeader};

/// Maximum number of decoders in a [`DecoderRegistry`].
pub const MAX_DECODERS: usize = 8;

/// Decodes one encoded image format.
///
/// Decoders produce `Argb8888` (straight alpha) or, for opaque sources, `Xrgb8888` pixels
/// with a packed stride (`4 · w`).
pub trait Decoder {
    /// Short name for logs, e.g. `"qoi"`.
    fn name(&self) -> &'static str;

    /// Reads only the header of `bytes`; `None` when the bytes are not this format (or the
    /// header is invalid).
    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader>;

    /// Decodes `bytes` into `out` (cleared first, reserved once; allocation failure is
    /// `Error::Decode("out of memory")`, never an abort).
    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error>;
}

impl core::fmt::Debug for dyn Decoder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Decoder({})", self.name())
    }
}

/// Clears `out` and reserves exactly `n` bytes (zero-filled).
#[cfg_attr(
    not(any(
        feature = "img-qoi",
        feature = "img-png",
        feature = "img-jpeg",
        feature = "img-bmp",
        feature = "img-gif"
    )),
    allow(dead_code)
)]
pub(crate) fn prepare_out(out: &mut Vec<u8>, n: usize) -> Result<(), Error> {
    out.clear();
    out.try_reserve_exact(n)
        .map_err(|_| Error::Decode("out of memory"))?;
    out.resize(n, 0);
    Ok(())
}

/// The output header of a decoder: `Argb8888` with alpha, else `Xrgb8888`.
#[cfg_attr(
    not(any(
        feature = "img-qoi",
        feature = "img-png",
        feature = "img-jpeg",
        feature = "img-bmp",
        feature = "img-gif"
    )),
    allow(dead_code)
)]
pub(crate) fn out_header(w: u16, h: u16, alpha: bool) -> ImageHeader {
    ImageHeader::new(
        if alpha {
            ColorFormat::Argb8888
        } else {
            ColorFormat::Xrgb8888
        },
        w,
        h,
    )
}

/// Logs a successful decode.
#[cfg_attr(
    not(any(
        feature = "img-qoi",
        feature = "img-png",
        feature = "img-jpeg",
        feature = "img-bmp",
        feature = "img-gif"
    )),
    allow(dead_code)
)]
pub(crate) fn log_decoded(name: &str, h: ImageHeader, len: usize) {
    twine_core::debug!(target: "twine::image", "decoded {} {}x{} in {} bytes", name, h.w, h.h, len);
    let _ = (name, h, len);
}

/// An ordered list of decoders; the first whose [`Decoder::probe`] accepts the bytes decodes
/// them.
///
/// ```
/// use twine_image::DecoderRegistry;
/// let reg = DecoderRegistry::with_defaults();
/// assert!(reg.probe(b"not an image").is_none());
/// # #[cfg(feature = "img-qoi")]
/// assert_eq!(reg.names().next(), Some("qoi"));
/// ```
#[derive(Debug, Default)]
pub struct DecoderRegistry {
    decoders: heapless::Vec<&'static dyn Decoder, MAX_DECODERS>,
}

impl DecoderRegistry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            decoders: heapless::Vec::new(),
        }
    }

    /// Every decoder enabled by features, in the order qoi, png, jpeg, bmp, gif.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        let all: &[&'static dyn Decoder] = &[
            #[cfg(feature = "img-qoi")]
            &crate::decoders::qoi::QoiDecoder,
            #[cfg(feature = "img-png")]
            &crate::decoders::png::PngDecoder,
            #[cfg(feature = "img-jpeg")]
            &crate::decoders::jpeg::JpegDecoder,
            #[cfg(feature = "img-bmp")]
            &crate::decoders::bmp::BmpDecoder,
            #[cfg(feature = "img-gif")]
            &crate::decoders::gif::GifDecoder,
        ];
        for d in all {
            // At most five built-in decoders: always fits.
            let _ = r.register(*d);
        }
        r
    }

    /// Appends a decoder (probed after the existing ones); [`Error::CacheFull`] when
    /// [`MAX_DECODERS`] are registered.
    pub fn register(&mut self, d: &'static dyn Decoder) -> Result<(), Error> {
        self.decoders.push(d).map_err(|_| Error::CacheFull)
    }

    /// Names of the registered decoders, in probe order.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.decoders.iter().map(|d| d.name())
    }

    /// The first decoder that recognises `bytes`, with the image header.
    #[must_use]
    pub fn probe(&self, bytes: &[u8]) -> Option<(&'static dyn Decoder, ImageHeader)> {
        self.decoders.iter().find_map(|d| d.probe(bytes).map(|h| (*d, h)))
    }

    /// Decodes `bytes` with the first decoder that recognises them; [`Error::InvalidHeader`]
    /// when none does.
    pub fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        if let Some((d, _)) = self.probe(bytes) {
            d.decode(bytes, out)
        } else {
            twine_core::warn!(target: "twine::image", "no decoder recognises the image data ({} bytes)", bytes.len());
            Err(Error::InvalidHeader)
        }
    }
}
