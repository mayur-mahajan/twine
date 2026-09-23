//! LVGL 9 run-length encoding (`lv_rle_decompress`).
//!
//! The stream is a sequence of packets. A control byte `c` with the top bit set is followed by
//! `(c & 0x7F)` literal blocks; otherwise one block follows that is repeated `c` times. A
//! block is `blk_size` bytes (see [`rle_block_size`]).

use alloc::vec::Vec;

use twine_core::ColorFormat;

use crate::Error;

/// RLE block size of a format: bytes per pixel, or 1 for sub-byte and indexed formats (whose
/// whole data, palette included, is compressed as one byte stream).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::rle_block_size;
/// assert_eq!(rle_block_size(ColorFormat::Rgb888), 3);
/// assert_eq!(rle_block_size(ColorFormat::I4), 1);
/// ```
#[must_use]
pub const fn rle_block_size(format: ColorFormat) -> usize {
    if format.is_indexed() || format.bpp() < 8 {
        1
    } else {
        format.bpp() as usize / 8
    }
}

/// Decompresses `input` into `output` and returns the number of bytes written.
///
/// Truncated input is an error. If the last packet would overflow `output` by less than one
/// block (padding of the final block, LVGL tolerance) the part that fits is written and
/// `Ok(output.len())` returned; a larger overflow is an error.
///
/// ```
/// use twine_image::rle_decompress;
/// // repeat [1, 2] three times, then two literal blocks [3, 4] [5, 6]
/// let packed = [3, 1, 2, 0x82, 3, 4, 5, 6];
/// let mut out = [0u8; 10];
/// assert_eq!(rle_decompress(&packed, &mut out, 2), Ok(10));
/// assert_eq!(out, [1, 2, 1, 2, 1, 2, 3, 4, 5, 6]);
/// ```
pub fn rle_decompress(input: &[u8], output: &mut [u8], blk_size: usize) -> Result<usize, Error> {
    if blk_size == 0 {
        return Err(Error::Decode("rle block size 0"));
    }
    let (mut rd, mut wr) = (0usize, 0usize);
    let total = output.len();
    while rd < input.len() {
        let c = input[rd];
        rd += 1;
        if c & 0x80 != 0 {
            let bytes = usize::from(c & 0x7F) * blk_size;
            let src = input.get(rd..rd + bytes).ok_or(Error::Decode("rle truncated"))?;
            rd += bytes;
            let room = output.len() - wr;
            if bytes > room {
                return overflow(&src[..room], &mut output[wr..], bytes - room, blk_size, total);
            }
            output[wr..wr + bytes].copy_from_slice(src);
            wr += bytes;
        } else {
            let blk = input.get(rd..rd + blk_size).ok_or(Error::Decode("rle truncated"))?;
            rd += blk_size;
            for _ in 0..c {
                let room = output.len() - wr;
                if blk_size > room {
                    return overflow(&blk[..room], &mut output[wr..], blk_size - room, blk_size, total);
                }
                output[wr..wr + blk_size].copy_from_slice(blk);
                wr += blk_size;
            }
        }
    }
    Ok(wr)
}

/// Handles a packet that does not fit: writes `fits` and accepts an overflow below one block.
fn overflow(fits: &[u8], out: &mut [u8], over: usize, blk_size: usize, total: usize) -> Result<usize, Error> {
    if over < blk_size {
        out[..fits.len()].copy_from_slice(fits);
        Ok(total)
    } else {
        Err(Error::Decode("rle overflow"))
    }
}

/// Compresses `input` (greedy: runs of two or more equal blocks become repeat packets of up
/// to 127 blocks, everything else literal packets of up to 127 blocks). A trailing partial
/// block is padded with zeros (which [`rle_decompress`] tolerates).
///
/// ```
/// use twine_image::{rle_compress, rle_decompress};
/// let data = [9u8, 9, 9, 9, 1, 2, 3];
/// let packed = rle_compress(&data, 1);
/// assert_eq!(packed, [4, 9, 0x83, 1, 2, 3]);
/// let mut out = [0u8; 7];
/// assert_eq!(rle_decompress(&packed, &mut out, 1), Ok(7));
/// ```
#[must_use]
pub fn rle_compress(input: &[u8], blk_size: usize) -> Vec<u8> {
    let blk_size = blk_size.max(1);
    let mut padded;
    let data = if input.len() % blk_size == 0 {
        input
    } else {
        padded = input.to_vec();
        padded.resize(input.len().div_ceil(blk_size) * blk_size, 0);
        &padded[..]
    };
    let blocks: Vec<&[u8]> = data.chunks_exact(blk_size).collect();
    let run_at = |i: usize| -> usize {
        let mut r = 1;
        while i + r < blocks.len() && r < 127 && blocks[i + r] == blocks[i] {
            r += 1;
        }
        r
    };
    let mut out = Vec::with_capacity(data.len() / 2 + 8);
    let mut i = 0;
    while i < blocks.len() {
        let r = run_at(i);
        if r >= 2 {
            out.push(r as u8);
            out.extend_from_slice(blocks[i]);
            i += r;
            continue;
        }
        let start = i;
        while i < blocks.len() && i - start < 127 && (i == start || run_at(i) < 2) {
            i += 1;
        }
        out.push(0x80 | (i - start) as u8);
        for b in &blocks[start..i] {
            out.extend_from_slice(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_block_sizes() {
        assert_eq!(rle_block_size(ColorFormat::A4), 1);
        assert_eq!(rle_block_size(ColorFormat::I8), 1);
        assert_eq!(rle_block_size(ColorFormat::L8), 1);
        assert_eq!(rle_block_size(ColorFormat::Rgb565A8), 2);
        assert_eq!(rle_block_size(ColorFormat::Argb8888), 4);
    }

    #[test]
    fn long_runs_split_at_127() {
        let data = [5u8; 300];
        let p = rle_compress(&data, 1);
        assert_eq!(p, [127, 5, 127, 5, 46, 5]);
        let lit: Vec<u8> = (0..=255).collect();
        let p = rle_compress(&lit, 1);
        assert_eq!(p[0], 0x80 | 127);
        let mut out = [0u8; 256];
        assert_eq!(rle_decompress(&p, &mut out, 1), Ok(256));
        assert_eq!(&out[..], &lit[..]);
    }
}
