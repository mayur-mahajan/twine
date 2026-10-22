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
/// # Errors
/// [`Error::Decode`] for truncated input or an overflow of `output` by a block or more.
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
    let total = output.len();
    // The unread input and the unwritten output, consumed from the front.
    let mut inp = input;
    let mut out: &mut [u8] = output;
    while let Some((&c, rest)) = inp.split_first() {
        inp = rest;
        if c & 0x80 != 0 {
            // Literal packet: `c & 0x7F` blocks follow (saturating: an impossible length is
            // truncated input).
            let bytes = usize::from(c & 0x7F).saturating_mul(blk_size);
            let (src, rest) = inp
                .split_at_checked(bytes)
                .ok_or(Error::Decode("rle truncated"))?;
            inp = rest;
            let room = out.len();
            if bytes > room {
                // SAFE-ARITH: `bytes > room`.
                return overflow(src, out, bytes.wrapping_sub(room), blk_size, total);
            }
            // `bytes ≤ out.len()` (checked above): never `None`.
            let Some((dst, rest)) = core::mem::take(&mut out).split_at_mut_checked(bytes) else {
                return Err(Error::Decode("rle overflow"));
            };
            dst.copy_from_slice(src);
            out = rest;
        } else {
            let (blk, rest) = inp
                .split_at_checked(blk_size)
                .ok_or(Error::Decode("rle truncated"))?;
            inp = rest;
            // Whole blocks that fit, written in one pass. `blk_size != 0` (refused at the
            // top): the division's fallback is never taken.
            let fit = usize::from(c).min(out.len().checked_div(blk_size).unwrap_or(0));
            // SAFE-ARITH: `fit ≤ out.len() / blk_size`, so `fit · blk_size ≤ out.len()`.
            let Some((span, rest)) =
                core::mem::take(&mut out).split_at_mut_checked(fit.wrapping_mul(blk_size))
            else {
                return Err(Error::Decode("rle overflow"));
            };
            if let [b] = blk {
                span.fill(*b);
            } else {
                span.chunks_exact_mut(blk_size)
                    .for_each(|d| d.copy_from_slice(blk));
            }
            out = rest;
            if fit < usize::from(c) {
                // Here `out.len() < blk_size` (no further whole block fits).
                // SAFE-ARITH: `out.len() < blk_size`.
                let over = blk_size.wrapping_sub(out.len());
                return overflow(blk, out, over, blk_size, total);
            }
        }
    }
    // SAFE-ARITH: `out` is a tail of `output`.
    Ok(total.wrapping_sub(out.len()))
}

/// The last packet overflows `out` (the rest of the output) by `over` bytes: below one block,
/// the part of `src` that fits is written and the whole output counts as written; otherwise
/// the data is corrupt.
fn overflow(src: &[u8], out: &mut [u8], over: usize, blk_size: usize, total: usize) -> Result<usize, Error> {
    if over < blk_size {
        // `src` holds more than `out.len()` bytes (it overflows): never `None`.
        if let Some(fits) = src.get(..out.len()) {
            out.copy_from_slice(fits);
        }
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
    let data = if input.len().checked_rem(blk_size) == Some(0) {
        input
    } else {
        padded = input.to_vec();
        padded.resize(input.len().div_ceil(blk_size).saturating_mul(blk_size), 0);
        &padded[..]
    };
    let blocks: Vec<&[u8]> = data.chunks_exact(blk_size).collect();
    // Length of the run of equal blocks starting at `i` (at most 127).
    let run_at = |i: usize| -> usize {
        let Some((first, rest)) = blocks.get(i..).and_then(<[&[u8]]>::split_first) else {
            return 0;
        };
        // SAFE-ARITH: at most 126 more blocks are counted.
        rest.iter()
            .take(126)
            .take_while(|b| *b == first)
            .count()
            .wrapping_add(1)
    };
    let mut out = Vec::with_capacity((data.len() / 2).saturating_add(8));
    let mut i = 0;
    while i < blocks.len() {
        let r = run_at(i);
        if r >= 2 {
            // `r ≤ 127`: the cast is exact.
            out.push(r as u8);
            out.extend_from_slice(blocks.get(i).copied().unwrap_or_default());
            // SAFE-ARITH: the run lies inside `blocks`.
            i = i.wrapping_add(r);
            continue;
        }
        let start = i;
        // SAFE-ARITH: `start ≤ i < blocks.len()` in the loop.
        while i < blocks.len() && i.wrapping_sub(start) < 127 && (i == start || run_at(i) < 2) {
            i = i.wrapping_add(1);
        }
        let literal = blocks.get(start..i).unwrap_or_default();
        // `literal.len() ≤ 127`: the cast is exact.
        out.push(0x80 | literal.len() as u8);
        for b in literal {
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
