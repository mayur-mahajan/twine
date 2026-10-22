//! In-house PNG decoder over `miniz_oxide` (zlib inflate).
//!
//! Supports every standard PNG: gray, gray + alpha, RGB, RGBA and indexed images at every
//! legal bit depth (1, 2, 4, 8, 16), Adam7 interlacing, palette transparency and `tRNS` color
//! keys. Ancillary chunks and CRCs are ignored. Malformed data returns an error, never panics.

use alloc::boxed::Box;
use alloc::vec::Vec;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::inflate_flags::{
    TINFL_FLAG_HAS_MORE_INPUT, TINFL_FLAG_PARSE_ZLIB_HEADER, TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
};
use miniz_oxide::inflate::core::{DecompressorOxide, decompress};

use crate::decoder::{Decoder, log_decoded, out_header, prepare_out};
use crate::{Error, ImageHeader};

/// Largest accepted width or height.
pub const PNG_MAX_DIM: u32 = 16384;

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Decodes PNG files.
#[derive(Clone, Copy, Debug, Default)]
pub struct PngDecoder;

/// A legal PNG bit depth, validated when the IHDR is parsed: the per-sample shifts of
/// [`sample`] and [`raw_sample`] are proven in range by this type, not checked per pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Depth {
    /// 1, 2 or 4 bits: several samples per byte. Holds the bit count.
    Sub(SubByte),
    /// 8 bits.
    Eight,
    /// 16 bits (big endian).
    Sixteen,
}

/// The bit counts of sub-byte samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum SubByte {
    One = 1,
    Two = 2,
    Four = 4,
}

impl Depth {
    /// The depth of IHDR value `bits`, `None` for an illegal depth (the image is rejected).
    fn new(bits: u8) -> Option<Depth> {
        Some(match bits {
            1 => Depth::Sub(SubByte::One),
            2 => Depth::Sub(SubByte::Two),
            4 => Depth::Sub(SubByte::Four),
            8 => Depth::Eight,
            16 => Depth::Sixteen,
            _ => return None,
        })
    }

    /// Bits per sample.
    fn bits(self) -> u8 {
        match self {
            Depth::Sub(d) => d as u8,
            Depth::Eight => 8,
            Depth::Sixteen => 16,
        }
    }
}

/// Sample `i` of a row of `d`-bit samples (MSB first), unscaled.
#[inline]
fn sub_byte(row: &[u8], d: SubByte, i: usize) -> u8 {
    let d = d as usize;
    // SAFE-ARITH: `i` is a sample index of a row (`< 2^16 · 4`) and `d ≤ 4`.
    let bit = i.wrapping_mul(d);
    // SHIFT: `d` is 1, 2 or 4 (`SubByte`) and divides 8, and `bit` is a multiple of `d`, so
    // `bit % 8 ≤ 8 − d`: the amount is `0..=7`, and `d ≤ 4` for the mask.
    // SAFE-ARITH: for the same reason `8 − d − bit % 8 ≥ 0`, and `2^d ≥ 2`.
    let v = row.get(bit / 8).copied().unwrap_or(0) >> 8usize.wrapping_sub(d).wrapping_sub(bit % 8);
    // SHIFT: `d ≤ 4`, as above.
    v & (1u8 << d).wrapping_sub(1)
}

/// IHDR contents.
#[derive(Clone, Copy, Debug)]
struct Ihdr {
    w: u32,
    h: u32,
    depth: Depth,
    color: u8,
    interlaced: bool,
}

impl Ihdr {
    fn channels(&self) -> usize {
        match self.color {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }

    fn has_alpha(&self) -> bool {
        matches!(self.color, 3 | 4 | 6)
    }

    /// Bits per pixel (at most `4 · 16 = 64`).
    fn bpp_bits(&self) -> usize {
        // SAFE-ARITH: at most 4 channels of at most 16 bits.
        self.channels().wrapping_mul(usize::from(self.depth.bits()))
    }

    /// Bytes per filtered row of `w ≤ PNG_MAX_DIM` pixels (without the filter byte).
    fn row_bytes(&self, w: usize) -> usize {
        // SAFE-ARITH: `w ≤ 2^14` and at most 64 bits per pixel.
        w.wrapping_mul(self.bpp_bits()).div_ceil(8)
    }
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..)?.first_chunk().map(|s| u32::from_be_bytes(*s))
}

fn parse_ihdr(bytes: &[u8]) -> Option<Ihdr> {
    if !bytes.starts_with(SIGNATURE) || bytes.get(12..16)? != b"IHDR" || be32(bytes, 8)? != 13 {
        return None;
    }
    let &[
        w0,
        w1,
        w2,
        w3,
        h0,
        h1,
        h2,
        h3,
        depth,
        color,
        compression,
        filter,
        interlace,
    ] = bytes.get(16..)?.first_chunk::<13>()?;
    let ihdr = Ihdr {
        w: u32::from_be_bytes([w0, w1, w2, w3]),
        h: u32::from_be_bytes([h0, h1, h2, h3]),
        depth: Depth::new(depth)?,
        color,
        interlaced: interlace == 1,
    };
    let depth_ok = match ihdr.color {
        0 => true,
        3 => ihdr.depth != Depth::Sixteen,
        2 | 4 | 6 => matches!(ihdr.depth, Depth::Eight | Depth::Sixteen),
        _ => false,
    };
    let ok = depth_ok
        && compression == 0
        && filter == 0
        && interlace <= 1
        && (1..=PNG_MAX_DIM).contains(&ihdr.w)
        && (1..=PNG_MAX_DIM).contains(&ihdr.h);
    ok.then_some(ihdr)
}

/// Adam7 passes: (x0, y0, dx, dy).
const ADAM7: [(usize, usize, usize, usize); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// Passes as (x0, y0, dx, dy, pass width, pass height); one full pass when not interlaced.
fn passes(ih: &Ihdr) -> impl Iterator<Item = (usize, usize, usize, usize, usize, usize)> + '_ {
    let (w, h) = (ih.w as usize, ih.h as usize);
    let list: &[(usize, usize, usize, usize)] = if ih.interlaced { &ADAM7 } else { &[(0, 0, 1, 1)] };
    list.iter().filter_map(move |&(x0, y0, dx, dy)| {
        // DIV: `dx, dy ≥ 1` in every entry of `ADAM7` and of the single pass.
        let pw = w.saturating_sub(x0).div_ceil(dx);
        // DIV: `dx, dy ≥ 1` (as above).
        let ph = h.saturating_sub(y0).div_ceil(dy);
        (pw > 0 && ph > 0).then_some((x0, y0, dx, dy, pw, ph))
    })
}

/// Palette (RGB triples) and transparency of the image.
#[derive(Default)]
struct Extra<'a> {
    plte: &'a [u8],
    trns: &'a [u8],
}

/// Walks the chunks after IHDR: returns PLTE/tRNS and calls `idat` for every IDAT payload.
fn chunks<'a>(
    bytes: &'a [u8],
    mut idat: impl FnMut(&'a [u8]) -> Result<(), Error>,
) -> Result<Extra<'a>, Error> {
    let mut extra = Extra::default();
    let mut pos = 8;
    let mut seen_idat = false;
    loop {
        // Offsets saturate: past the end of the file they read as truncated data.
        let len = be32(bytes, pos).ok_or(Error::Decode("png truncated"))? as usize;
        let kind = bytes
            .get(pos.saturating_add(4)..pos.saturating_add(8))
            .ok_or(Error::Decode("png truncated"))?;
        let data_start = pos.saturating_add(8);
        let data = bytes
            .get(data_start..data_start.saturating_add(len))
            .ok_or(Error::Decode("png truncated"))?;
        match kind {
            b"PLTE" => extra.plte = data,
            b"tRNS" => extra.trns = data,
            b"IDAT" => {
                seen_idat = true;
                idat(data)?;
            }
            b"IEND" => break,
            _ => {}
        }
        pos = pos.saturating_add(12).saturating_add(len);
    }
    if !seen_idat {
        return Err(Error::Decode("png without image data"));
    }
    Ok(extra)
}

/// Bytes of the widest PNG pixel (16-bit RGBA).
const MAX_BPP: usize = 8;

/// Reverses the PNG filter of one row in place. `prev` is the previous unfiltered row
/// (empty for the first row), `bpp` the bytes per complete pixel (`1..=8`).
///
/// The bytes are walked pixel by pixel (`chunks_mut(bpp)`); the left neighbours (`a`) and
/// upper-left neighbours (`c`) of the next pixel are the bytes just finished, kept in two
/// small arrays, so no byte is indexed.
fn unfilter(filter: u8, row: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), Error> {
    let bpp = bpp.clamp(1, MAX_BPP);
    // Upper neighbours (`b`): 0 past the end of `prev` (the first row has none).
    let mut ups = prev.iter().copied().chain(core::iter::repeat(0));
    let mut left = [0u8; MAX_BPP];
    let mut up_left = [0u8; MAX_BPP];
    match filter {
        0 => {}
        1 => {
            for px in row.chunks_mut(bpp) {
                for (v, l) in px.iter_mut().zip(left.iter_mut()) {
                    *v = v.wrapping_add(*l);
                    *l = *v;
                }
            }
        }
        2 => {
            for v in row.iter_mut() {
                *v = v.wrapping_add(ups.next().unwrap_or(0));
            }
        }
        3 => {
            for px in row.chunks_mut(bpp) {
                for (v, l) in px.iter_mut().zip(left.iter_mut()) {
                    *v = v.wrapping_add(u8::midpoint(*l, ups.next().unwrap_or(0)));
                    *l = *v;
                }
            }
        }
        4 => {
            for px in row.chunks_mut(bpp) {
                for ((v, l), ul) in px.iter_mut().zip(left.iter_mut()).zip(up_left.iter_mut()) {
                    let up = ups.next().unwrap_or(0);
                    let (a, b, c) = (i16::from(*l), i16::from(up), i16::from(*ul));
                    // SAFE-ARITH: `a, b, c ∈ 0..=255`, so `p ∈ −255..=510` and the distances
                    // are below 2^10.
                    let p = a.wrapping_add(b).wrapping_sub(c);
                    let (pa, pb, pc) = (
                        p.wrapping_sub(a).abs(),
                        p.wrapping_sub(b).abs(),
                        p.wrapping_sub(c).abs(),
                    );
                    let pred = if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    };
                    *v = v.wrapping_add(pred as u8);
                    *l = *v;
                    *ul = up;
                }
            }
        }
        _ => return Err(Error::Decode("png bad filter")),
    }
    Ok(())
}

/// Sample `i` of a row at `depth` bits, scaled to 8 bits (16-bit samples keep the high byte).
#[inline]
fn sample(row: &[u8], depth: Depth, i: usize) -> u8 {
    match depth {
        // SAFE-ARITH: `i` is a sample index of a row (`< 2^18`).
        Depth::Sixteen => row.get(i.wrapping_mul(2)).copied().unwrap_or(0),
        Depth::Eight => row.get(i).copied().unwrap_or(0),
        Depth::Sub(d) => {
            let v = sub_byte(row, d, i);
            // Scale 1/2/4-bit gray to 8 bits (indexed callers use `raw_sample`).
            // SHIFT: `d ≤ 4` (`SubByte`).
            // SAFE-ARITH: `v < 2^4`, so `v · 255 < 2^12`; `d ≥ 1`, so `2^d − 1 ≥ 1` and the
            // division's fallback is never taken.
            let max = (1u16 << d as u8).wrapping_sub(1);
            (u16::from(v).wrapping_mul(255).checked_div(max).unwrap_or(0)) as u8
        }
    }
}

/// Raw sample `i` (palette index or unscaled gray, 16-bit as full value).
#[inline]
fn raw_sample(row: &[u8], depth: Depth, i: usize) -> u16 {
    match depth {
        // SAFE-ARITH: `i` is a sample index of a row (`< 2^18`).
        Depth::Sixteen => row
            .get(i.wrapping_mul(2)..)
            .and_then(<[u8]>::first_chunk)
            .map_or(0, |s| u16::from_be_bytes(*s)),
        Depth::Eight => u16::from(row.get(i).copied().unwrap_or(0)),
        Depth::Sub(d) => u16::from(sub_byte(row, d, i)),
    }
}

/// Converts pixel `x` of an unfiltered row to B, G, R, A.
fn pixel(ih: &Ihdr, ex: &Extra<'_>, row: &[u8], x: usize) -> [u8; 4] {
    let d = ih.depth;
    let n = ih.channels();
    // SAFE-ARITH (whole function): `x < PNG_MAX_DIM = 2^14` and `n ≤ 4`, so every sample index
    // is below 2^16; channel numbers `c ≤ 2` and palette indices `≤ 2^16`.
    let s = |c: usize| x.wrapping_mul(n).wrapping_add(c);
    let key = |c: usize| be_u16(ex.trns, c.wrapping_mul(2));
    match ih.color {
        0 => {
            let g = sample(row, d, x);
            let a = if ex.trns.len() >= 2 && key(0) == Some(raw_sample(row, d, x)) {
                0
            } else {
                255
            };
            [g, g, g, a]
        }
        2 => {
            let (r, g, b) = (sample(row, d, s(0)), sample(row, d, s(1)), sample(row, d, s(2)));
            let raw = [
                raw_sample(row, d, s(0)),
                raw_sample(row, d, s(1)),
                raw_sample(row, d, s(2)),
            ];
            let keyed = ex.trns.len() >= 6 && [key(0), key(1), key(2)] == raw.map(Some);
            [b, g, r, if keyed { 0 } else { 255 }]
        }
        3 => {
            let i = usize::from(raw_sample(row, d, x));
            let (r, g, b) = match ex.plte.get(i.wrapping_mul(3)..).and_then(<[u8]>::first_chunk) {
                Some(&[r, g, b]) => (r, g, b),
                None => (0, 0, 0),
            };
            [b, g, r, ex.trns.get(i).copied().unwrap_or(255)]
        }
        4 => {
            let g = sample(row, d, s(0));
            [g, g, g, sample(row, d, s(1))]
        }
        _ => [
            sample(row, d, s(2)),
            sample(row, d, s(1)),
            sample(row, d, s(0)),
            sample(row, d, s(3)),
        ],
    }
}

fn be_u16(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..)?.first_chunk().map(|s| u16::from_be_bytes(*s))
}

impl Decoder for PngDecoder {
    fn name(&self) -> &'static str {
        "png"
    }

    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
        let ih = parse_ihdr(bytes)?;
        Some(out_header(ih.w as u16, ih.h as u16, ih.has_alpha()))
    }

    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        let ih = parse_ihdr(bytes).ok_or(Error::InvalidHeader)?;
        let header = out_header(ih.w as u16, ih.h as u16, ih.has_alpha());
        // Saturating: an impossible size fails the allocation below.
        let raw_len = passes(&ih).fold(0usize, |sum, (.., pw, ph)| {
            sum.saturating_add(ih.row_bytes(pw).saturating_add(1).saturating_mul(ph))
        });
        let mut raw = Vec::new();
        prepare_out(&mut raw, raw_len)?;
        // Inflate every IDAT into `raw`.
        let mut inflater = Box::<DecompressorOxide>::default();
        let mut written = 0usize;
        let mut done = false;
        let mut pending: Option<&[u8]> = None;
        let mut feed = |data: Option<&[u8]>, more: bool| -> Result<(), Error> {
            let Some(mut input) = data else { return Ok(()) };
            let mut flags = TINFL_FLAG_PARSE_ZLIB_HEADER | TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
            if more {
                flags |= TINFL_FLAG_HAS_MORE_INPUT;
            }
            while !done {
                let (status, used, n) = decompress(&mut inflater, input, &mut raw, written, flags);
                written = written.saturating_add(n);
                input = input.get(used..).unwrap_or_default();
                match status {
                    // `HasMoreOutput`: extra data after the image, ignored.
                    TINFLStatus::Done | TINFLStatus::HasMoreOutput => done = true,
                    TINFLStatus::NeedsMoreInput if more => return Ok(()),
                    TINFLStatus::NeedsMoreInput => return Err(Error::Decode("png truncated")),
                    _ => return Err(Error::Decode("png inflate error")),
                }
            }
            Ok(())
        };
        let extra = chunks(bytes, |d| {
            // Keep one chunk back so the last one is fed without `HAS_MORE_INPUT`.
            let r = feed(pending, true);
            pending = Some(d);
            r
        })?;
        feed(pending, false)?;
        if written < raw_len {
            return Err(Error::Decode("png image data too short"));
        }
        // SAFE-ARITH: `w, h ≤ PNG_MAX_DIM = 2^14`: `n ≤ 2^30`.
        let n = usize::from(header.w)
            .wrapping_mul(usize::from(header.h))
            .wrapping_mul(4);
        prepare_out(out, n)?;
        let bpp = ih.bpp_bits().div_ceil(8).max(1);
        let w = ih.w as usize;
        // The passes' rows, one after the other in `raw` (`written ≥ raw_len`).
        let mut rows = raw.get_mut(..raw_len).unwrap_or_default();
        for (x0, y0, dx, dy, pw, ph) in passes(&ih) {
            let rb = ih.row_bytes(pw);
            let mut prev: &[u8] = &[];
            // SAFE-ARITH (the pass loop): `x0 + px · dx < w` and `y0 + py · dy < h` (Adam7
            // pass geometry, `passes`), so the output offset is below `n`.
            for py in 0..ph {
                let Some((&mut filter, rest)) = core::mem::take(&mut rows).split_first_mut() else {
                    return Err(Error::Decode("png image data too short"));
                };
                let Some((row, rest)) = rest.split_at_mut_checked(rb) else {
                    return Err(Error::Decode("png image data too short"));
                };
                rows = rest;
                unfilter(filter, row, prev, bpp)?;
                let y = y0.wrapping_add(py.wrapping_mul(dy));
                for px in 0..pw {
                    let x = x0.wrapping_add(px.wrapping_mul(dx));
                    let o = y.wrapping_mul(w).wrapping_add(x).wrapping_mul(4);
                    if let Some(dst) = out.get_mut(o..o.wrapping_add(4)) {
                        dst.copy_from_slice(&pixel(&ih, &extra, row, px));
                    }
                }
                prev = row;
            }
        }
        log_decoded("png", header, n);
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adam7_passes_cover_every_pixel_once() {
        let ih = Ihdr {
            w: 13,
            h: 9,
            depth: Depth::Eight,
            color: 0,
            interlaced: true,
        };
        let mut seen = [[0u8; 13]; 9];
        for (x0, y0, dx, dy, pw, ph) in passes(&ih) {
            for py in 0..ph {
                for px in 0..pw {
                    seen[y0 + py * dy][x0 + px * dx] += 1;
                }
            }
        }
        assert!(seen.iter().flatten().all(|&v| v == 1));
    }

    #[test]
    fn paeth_and_average_filters() {
        let prev = [10u8, 20, 30, 40];
        let mut row = [1u8, 1, 1, 1];
        unfilter(3, &mut row, &prev, 1).unwrap();
        assert_eq!(row, [6, 14, 23, 32]);
        let mut row = [0u8, 0, 0, 0];
        unfilter(4, &mut row, &prev, 2).unwrap();
        assert_eq!(row, [10, 20, 30, 40]);
        assert!(unfilter(5, &mut row, &prev, 1).is_err());
    }
}
