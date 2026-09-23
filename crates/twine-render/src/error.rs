//! [`RenderError`]: errors of buffer construction and buffer utilities.

use twine_core::ColorFormat;

/// Errors reported when creating draw buffers or running buffer utilities.
///
/// Drawing itself never fails: invalid drawing arguments are logged and ignored.
///
/// ```
/// use twine_core::{ColorFormat, Rect};
/// use twine_render::{DrawBuf, RenderError};
///
/// let mut data = [0u8; 10];
/// let err = DrawBuf::new_packed(&mut data, ColorFormat::Rgb565, Rect::from_xywh(0, 0, 4, 4)).unwrap_err();
/// assert_eq!(err, RenderError::BufferTooSmall { needed: 32, got: 10 });
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RenderError {
    /// The byte slice is shorter than the described pixels.
    #[error("buffer too small: {needed} bytes needed, got {got}")]
    BufferTooSmall {
        /// Bytes required.
        needed: usize,
        /// Bytes provided.
        got: usize,
    },
    /// The color format cannot be used here.
    #[error("unsupported color format {0}")]
    UnsupportedFormat(ColorFormat),
    /// The area is empty, the stride is too small, or a sub-byte format is not byte-aligned.
    #[error("invalid area or stride")]
    InvalidArea,
}
