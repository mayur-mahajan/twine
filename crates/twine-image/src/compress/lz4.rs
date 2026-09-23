//! LZ4 block decompression (feature `img-lz4`, via `lz4_flex` with safe decoding).

use crate::Error;

/// Decompresses an LZ4 block into `output` and returns the number of bytes written. Corrupt
/// input or a too small `output` is an error (never a panic).
///
/// ```
/// use twine_image::lz4_decompress;
/// let mut out = [0u8; 4];
/// // One literal-only sequence: token 0x40 (4 literals), "twin"
/// assert_eq!(lz4_decompress(&[0x40, b't', b'w', b'i', b'n'], &mut out), Ok(4));
/// assert_eq!(&out, b"twin");
/// ```
pub fn lz4_decompress(input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    lz4_flex::block::decompress_into(input, output).map_err(|_| Error::Decode("lz4 corrupt or too large"))
}
