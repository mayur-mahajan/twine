//! The raw twine binary image file (`.tbin`, written by `twine image --bin`): pixels in any
//! [`ColorFormat`], stored as they are drawn, optionally RLE- or LZ4-compressed. Decoding is a
//! copy (or a decompression), so file images in the display's format cost no conversion.
//!
//! Layout (little endian), 12-byte header then the data:
//!
//! | Offset | Size | Field |
//! |-------:|-----:|-------|
//! | 0 | 4 | magic `b"TWIN"` |
//! | 4 | 1 | bits 0–3: version (1); bits 4–5: compression (0 none, 1 RLE, 2 LZ4); bit 6: premultiplied alpha |
//! | 5 | 1 | [`ColorFormat`] (`ColorFormat as u8`) |
//! | 6 | 2 | width |
//! | 8 | 2 | height |
//! | 10 | 2 | stride (bytes per row) |
//! | 12 | … | pixel data: [`ImageHeader::data_size`] bytes (palette first for indexed formats), or the compressed stream |

use alloc::vec::Vec;

use twine_core::ColorFormat;

use crate::decoder::prepare_out;
use crate::{Compression, Decoder, Error, ImageFlags, ImageHeader, rle_block_size, rle_decompress};

/// Magic bytes of a `.tbin` file.
pub const TBIN_MAGIC: [u8; 4] = *b"TWIN";
/// Size of the `.tbin` header.
pub const TBIN_HEADER_LEN: usize = 12;
/// Format version written by [`encode_tbin`].
pub const TBIN_VERSION: u8 = 1;

const PREMULTIPLIED_BIT: u8 = 0x40;

/// Decoder of `.tbin` files (always available: it only copies or decompresses).
#[derive(Debug, Clone, Copy, Default)]
pub struct BinDecoder;

/// The header and the compression of a `.tbin` file.
fn parse(bytes: &[u8]) -> Option<(ImageHeader, Option<Compression>)> {
    let h = bytes.get(..TBIN_HEADER_LEN)?;
    if h[..4] != TBIN_MAGIC || h[4] & 0x0F != TBIN_VERSION {
        return None;
    }
    let compression = match (h[4] >> 4) & 0x3 {
        0 => None,
        1 => Some(Compression::Rle),
        2 => Some(Compression::Lz4),
        _ => return None,
    };
    let format = ColorFormat::from_u8(h[5])?;
    let u16_at = |o: usize| u16::from_le_bytes([h[o], h[o + 1]]);
    let (w, hgt, stride) = (u16_at(6), u16_at(8), u16_at(10));
    if w == 0 || hgt == 0 || stride < format.min_stride(w) {
        return None;
    }
    let mut header = ImageHeader::new(format, w, hgt);
    header.stride = stride;
    if h[4] & PREMULTIPLIED_BIT != 0 {
        header.flags |= ImageFlags::PREMULTIPLIED;
    }
    Some((header, compression))
}

impl Decoder for BinDecoder {
    fn name(&self) -> &'static str {
        "tbin"
    }

    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
        parse(bytes).map(|(h, _)| h)
    }

    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        let (header, compression) = parse(bytes).ok_or(Error::InvalidHeader)?;
        let n = header.data_size();
        let data = &bytes[TBIN_HEADER_LEN..];
        prepare_out(out, n)?;
        let written = match compression {
            None => {
                let src = data.get(..n).ok_or(Error::SizeMismatch {
                    expected: n,
                    got: data.len(),
                })?;
                out.copy_from_slice(src);
                n
            }
            Some(Compression::Rle) => rle_decompress(data, out, rle_block_size(header.format))?,
            #[cfg(feature = "img-lz4")]
            Some(Compression::Lz4) => crate::lz4_decompress(data, out)?,
            #[cfg(not(feature = "img-lz4"))]
            Some(Compression::Lz4) => return Err(Error::Decode("LZ4 support not enabled (feature img-lz4)")),
        };
        if written < n {
            return Err(Error::SizeMismatch {
                expected: n,
                got: written,
            });
        }
        twine_core::debug!(target: "twine::image", "decoded tbin {}x{} {}", header.w, header.h, header.format.name());
        Ok(header)
    }
}

/// Writes a `.tbin` file: the header of `header` (its `PREMULTIPLIED` flag is kept) and
/// `stored`, the pixel data already compressed with `compression` (`None`: raw pixels,
/// [`ImageHeader::data_size`] bytes).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::decoders::tbin::{BinDecoder, encode_tbin};
/// use twine_image::{Decoder, ImageHeader};
///
/// let h = ImageHeader::new(ColorFormat::L8, 2, 2);
/// let file = encode_tbin(h, None, &[1, 2, 3, 4]);
/// let mut out = Vec::new();
/// assert_eq!(BinDecoder.decode(&file, &mut out).unwrap(), h);
/// assert_eq!(out, [1, 2, 3, 4]);
/// ```
#[must_use]
pub fn encode_tbin(header: ImageHeader, compression: Option<Compression>, stored: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(TBIN_HEADER_LEN + stored.len());
    v.extend_from_slice(&TBIN_MAGIC);
    let c = match compression {
        None => 0,
        Some(Compression::Rle) => 1,
        Some(Compression::Lz4) => 2,
    };
    let premul = if header.flags.contains(ImageFlags::PREMULTIPLIED) {
        PREMULTIPLIED_BIT
    } else {
        0
    };
    v.push(TBIN_VERSION | (c << 4) | premul);
    v.push(header.format as u8);
    v.extend_from_slice(&header.w.to_le_bytes());
    v.extend_from_slice(&header.h.to_le_bytes());
    v.extend_from_slice(&header.stride.to_le_bytes());
    v.extend_from_slice(stored);
    v
}
