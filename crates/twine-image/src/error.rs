//! [`Error`]: everything that can go wrong while validating, decompressing, decoding or caching
//! an image.

use twine_core::ColorFormat;

/// Errors of `twine-image`.
///
/// ```
/// use twine_image::Error;
/// let e = Error::SizeMismatch { expected: 8, got: 4 };
/// assert_eq!(e.to_string(), "image data size mismatch: expected 8 bytes, got 4");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The header is inconsistent (e.g. the stride cannot hold a row) or not recognised.
    #[error("invalid image header")]
    InvalidHeader,
    /// The data length does not match the header.
    #[error("image data size mismatch: expected {expected} bytes, got {got}")]
    SizeMismatch {
        /// Bytes the header requires.
        expected: usize,
        /// Bytes available.
        got: usize,
    },
    /// The color format cannot be used here.
    #[error("unsupported color format {0}")]
    Unsupported(ColorFormat),
    /// The kind of source cannot be resolved to pixels here (symbols, SVG).
    #[error("unsupported image source: {0}")]
    UnsupportedSource(&'static str),
    /// Encoded or compressed data is malformed, or decoding ran out of memory.
    #[error("decode error: {0}")]
    Decode(&'static str),
    /// The cache or registry has no room left.
    #[error("cache or registry full")]
    CacheFull,
    /// A file source could not be found (or no file system is available).
    #[error("image not found")]
    NotFound,
}
