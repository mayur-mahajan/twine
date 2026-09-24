//! Decompression of [`ImageData::Compressed`](crate::ImageData::Compressed): LVGL RLE and LZ4.

#[cfg(feature = "img-lz4")]
mod lz4;
mod rle;

#[cfg(feature = "img-lz4")]
pub use lz4::lz4_decompress;
pub use rle::rle_compress;
pub use rle::{rle_block_size, rle_decompress};

use crate::{Compression, Error, Image, ImageData, ImageFlags, ImageHeader};

impl Image {
    /// Decompresses (or copies) the pixel data into `out` (at least
    /// [`ImageHeader::data_size`] bytes) and returns the header of the result (without
    /// [`ImageFlags::COMPRESSED`]).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_image::{Compression, Image, ImageData, ImageHeader, rle_compress};
    ///
    /// let raw = [7u8; 64];
    /// let packed: &'static [u8] = Box::leak(rle_compress(&raw, 1).into_boxed_slice());
    /// let img = Image {
    ///     header: ImageHeader::new(ColorFormat::L8, 8, 8),
    ///     data: ImageData::Compressed { method: Compression::Rle, data: packed, decompressed_size: 64 },
    /// };
    /// let mut out = [0u8; 64];
    /// img.decompress_into(&mut out).unwrap();
    /// assert_eq!(out, raw);
    /// ```
    pub fn decompress_into(&self, out: &mut [u8]) -> Result<ImageHeader, Error> {
        let mut header = self.header;
        header.flags.remove(ImageFlags::COMPRESSED);
        let expected = header.data_size();
        if out.len() < expected {
            return Err(Error::SizeMismatch {
                expected,
                got: out.len(),
            });
        }
        let out = &mut out[..expected];
        let written = match &self.data {
            ImageData::Static(d) => copy(d, out)?,
            ImageData::Owned(d) => copy(d, out)?,
            ImageData::Compressed { method, data, .. } => match method {
                Compression::Rle => rle_decompress(data, out, rle_block_size(header.format))?,
                #[cfg(feature = "img-lz4")]
                Compression::Lz4 => lz4_decompress(data, out)?,
                #[cfg(not(feature = "img-lz4"))]
                Compression::Lz4 => return Err(Error::Decode("LZ4 support not enabled (feature img-lz4)")),
            },
        };
        if written < expected {
            return Err(Error::SizeMismatch {
                expected,
                got: written,
            });
        }
        Ok(header)
    }
}

fn copy(src: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let s = src.get(..out.len()).ok_or(Error::SizeMismatch {
        expected: out.len(),
        got: src.len(),
    })?;
    out.copy_from_slice(s);
    Ok(out.len())
}
