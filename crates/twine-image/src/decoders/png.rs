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

/// IHDR contents.
#[derive(Clone, Copy, Debug)]
struct Ihdr {
    w: u32,
    h: u32,
    depth: u8,
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

    /// Bits per pixel.
    fn bpp_bits(&self) -> usize {
        self.channels() * usize::from(self.depth)
    }

    /// Bytes per filtered row of `w` pixels (without the filter byte).
    fn row_bytes(&self, w: usize) -> usize {
        (w * self.bpp_bits()).div_ceil(8)
    }
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn parse_ihdr(bytes: &[u8]) -> Option<Ihdr> {
    if !bytes.starts_with(SIGNATURE) || bytes.get(12..16)? != b"IHDR" || be32(bytes, 8)? != 13 {
        return None;
    }
    let d = bytes.get(16..29)?;
    let ihdr = Ihdr {
        w: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
        h: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
        depth: d[8],
        color: d[9],
        interlaced: d[12] == 1,
    };
    let depth_ok = match ihdr.color {
        0 => matches!(ihdr.depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(ihdr.depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(ihdr.depth, 8 | 16),
        _ => false,
    };
    let ok = depth_ok
        && d[10] == 0
        && d[11] == 0
        && d[12] <= 1
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
        let pw = w.saturating_sub(x0).div_ceil(dx);
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
        let len = be32(bytes, pos).ok_or(Error::Decode("png truncated"))? as usize;
        let kind = bytes
            .get(pos + 4..pos + 8)
            .ok_or(Error::Decode("png truncated"))?;
        let data = bytes
            .get(pos + 8..pos + 8 + len)
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
        pos += 12 + len;
    }
    if !seen_idat {
        return Err(Error::Decode("png without image data"));
    }
    Ok(extra)
}

/// Reverses the PNG filter of one row in place. `prev` is the previous unfiltered row
/// (empty for the first row), `bpp` the bytes per complete pixel (at least 1).
fn unfilter(filter: u8, row: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), Error> {
    let up = |i: usize| prev.get(i).copied().unwrap_or(0);
    match filter {
        0 => {}
        1 => {
            for i in bpp..row.len() {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for (i, v) in row.iter_mut().enumerate() {
                *v = v.wrapping_add(up(i));
            }
        }
        3 => {
            for i in 0..row.len() {
                let left = if i >= bpp { row[i - bpp] } else { 0 };
                row[i] = row[i].wrapping_add(u8::midpoint(left, up(i)));
            }
        }
        4 => {
            for i in 0..row.len() {
                let a = if i >= bpp { i16::from(row[i - bpp]) } else { 0 };
                let b = i16::from(up(i));
                let c = if i >= bpp { i16::from(up(i - bpp)) } else { 0 };
                let p = a + b - c;
                let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                let pred = if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                };
                row[i] = row[i].wrapping_add(pred as u8);
            }
        }
        _ => return Err(Error::Decode("png bad filter")),
    }
    Ok(())
}

/// Sample `i` of a row at `depth` bits, scaled to 8 bits (16-bit samples keep the high byte).
fn sample(row: &[u8], depth: u8, i: usize) -> u8 {
    match depth {
        16 => row.get(i * 2).copied().unwrap_or(0),
        8 => row.get(i).copied().unwrap_or(0),
        d => {
            let bit = i * usize::from(d);
            let v = row.get(bit / 8).copied().unwrap_or(0) >> (8 - usize::from(d) - bit % 8);
            let v = v & ((1u8 << d) - 1);
            // Scale 1/2/4-bit gray to 8 bits (indexed callers use `raw_sample`).
            (u16::from(v) * 255 / ((1u16 << d) - 1)) as u8
        }
    }
}

/// Raw sample `i` (palette index or unscaled gray, 16-bit as full value).
fn raw_sample(row: &[u8], depth: u8, i: usize) -> u16 {
    match depth {
        16 => row
            .get(i * 2..i * 2 + 2)
            .map_or(0, |s| u16::from_be_bytes([s[0], s[1]])),
        8 => u16::from(row.get(i).copied().unwrap_or(0)),
        d => {
            let bit = i * usize::from(d);
            let v = row.get(bit / 8).copied().unwrap_or(0) >> (8 - usize::from(d) - bit % 8);
            u16::from(v & ((1u8 << d) - 1))
        }
    }
}

/// Converts pixel `x` of an unfiltered row to B, G, R, A.
fn pixel(ih: &Ihdr, ex: &Extra<'_>, row: &[u8], x: usize) -> [u8; 4] {
    let d = ih.depth;
    let n = ih.channels();
    let key = |c: usize| be_u16(ex.trns, c * 2);
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
            let (r, g, b) = (
                sample(row, d, x * n),
                sample(row, d, x * n + 1),
                sample(row, d, x * n + 2),
            );
            let raw = [
                raw_sample(row, d, x * n),
                raw_sample(row, d, x * n + 1),
                raw_sample(row, d, x * n + 2),
            ];
            let keyed = ex.trns.len() >= 6 && [key(0), key(1), key(2)] == raw.map(Some);
            [b, g, r, if keyed { 0 } else { 255 }]
        }
        3 => {
            let i = usize::from(raw_sample(row, d, x));
            let (r, g, b) = match ex.plte.get(i * 3..i * 3 + 3) {
                Some(p) => (p[0], p[1], p[2]),
                None => (0, 0, 0),
            };
            [b, g, r, ex.trns.get(i).copied().unwrap_or(255)]
        }
        4 => {
            let g = sample(row, d, x * 2);
            [g, g, g, sample(row, d, x * 2 + 1)]
        }
        _ => [
            sample(row, d, x * 4 + 2),
            sample(row, d, x * 4 + 1),
            sample(row, d, x * 4),
            sample(row, d, x * 4 + 3),
        ],
    }
}

fn be_u16(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
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
        let raw_len: usize = passes(&ih).map(|(.., pw, ph)| (ih.row_bytes(pw) + 1) * ph).sum();
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
                written += n;
                input = &input[used..];
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
        let n = usize::from(header.w) * usize::from(header.h) * 4;
        prepare_out(out, n)?;
        let bpp = ih.bpp_bits().div_ceil(8).max(1);
        let w = ih.w as usize;
        let mut off = 0;
        for (x0, y0, dx, dy, pw, ph) in passes(&ih) {
            let rb = ih.row_bytes(pw);
            let mut prev_start: Option<usize> = None;
            for py in 0..ph {
                let start = off + py * (rb + 1);
                let filter = raw[start];
                let (before, rest) = raw.split_at_mut(start + 1);
                let row = &mut rest[..rb];
                let prev = prev_start.map_or(&[][..], |p| &before[p..p + rb]);
                unfilter(filter, row, prev, bpp)?;
                let y = y0 + py * dy;
                for px in 0..pw {
                    let x = x0 + px * dx;
                    let o = (y * w + x) * 4;
                    out[o..o + 4].copy_from_slice(&pixel(&ih, &extra, row, px));
                }
                prev_start = Some(start + 1);
            }
            off += ph * (rb + 1);
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
            depth: 8,
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
