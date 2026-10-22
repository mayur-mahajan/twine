//! In-house BMP decoder: 1, 4, 8 bpp paletted (uncompressed, RLE4, RLE8), 16 bpp
//! (RGB555 or bit fields), 24 bpp and 32 bpp (with optional alpha mask); bottom-up and
//! top-down rows; every DIB header version from `BITMAPCOREHEADER` to `BITMAPV5HEADER`.
//! Malformed data returns an error, never panics.

use alloc::vec::Vec;

use twine_core::color::unpack_bits;

use crate::decoder::{Decoder, log_decoded, out_header, prepare_out};
use crate::{Error, ImageHeader};

/// Largest accepted width or height.
pub const BMP_MAX_DIM: u32 = 16384;

/// Decodes BMP files (`BM` magic).
#[derive(Clone, Copy, Debug, Default)]
pub struct BmpDecoder;

/// The channels of a 16/32-bit pixel, validated when the header is parsed ([`Channel`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Channels {
    r: Channel,
    g: Channel,
    b: Channel,
    a: Channel,
}

/// One channel of a 16/32-bit pixel, derived from the header's bit mask once, at parse time
/// ([`Channel::from_mask`]): `shift < 32`, and `max = 2^bits − 1` with `bits` in `1..=16`
/// (`max == 0`: no such channel). The per-pixel [`get`](Self::get) only shifts by that
/// validated amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Channel {
    shift: u32,
    max: u32,
}

impl Channel {
    /// No channel (mask 0): reads as 255.
    const ABSENT: Channel = Channel { shift: 0, max: 0 };

    /// The channel that `mask` selects in a `bpp`-bit (16 or 32) pixel, or `None` (the header
    /// is rejected) when the mask has bits outside the pixel, is not one contiguous run of bits
    /// or is wider than 16 bits (user decision 2026-10-20: rejected rather than cut to its low
    /// 16 bits, like the other header checks).
    fn from_mask(mask: u32, bpp: u16) -> Option<Channel> {
        if mask == 0 {
            return Some(Self::ABSENT);
        }
        if bpp == 16 && mask > 0xFFFF {
            return None;
        }
        let shift = mask.trailing_zeros();
        // SHIFT: `shift < 32`: the trailing zeros of a non-zero `u32`.
        let run = mask >> shift;
        // Contiguous: `run` is `2^bits − 1` (`u32::MAX` wraps to 0).
        if run & run.wrapping_add(1) != 0 {
            return None;
        }
        let bits = run.count_ones();
        if bits > 16 {
            return None;
        }
        Some(Channel {
            shift,
            // SHIFT: `bits ≤ 16`, on a `u32`. SAFE-ARITH: `bits ≥ 1` (the mask is not 0).
            max: (1u32 << bits).wrapping_sub(1),
        })
    }

    /// The channel of pixel value `v`, scaled to 8 bits (255 without the channel).
    #[inline]
    fn get(self, v: u32) -> u8 {
        if self.max == 0 {
            return 255;
        }
        // SHIFT: `shift < 32`, validated by `from_mask` (the only constructor besides `ABSENT`).
        let c = (v >> self.shift) & self.max;
        // SAFE-ARITH: `c ≤ max < 2^16`, so `c · 255 + max / 2 < 2^24`. `max != 0` (returned
        // above): the division's fallback is never taken.
        (c.wrapping_mul(255)
            .wrapping_add(self.max / 2)
            .checked_div(self.max)
            .unwrap_or(255)) as u8
    }
}

/// Parsed headers.
#[derive(Clone, Copy, Debug)]
struct Info {
    w: u32,
    h: u32,
    top_down: bool,
    bpp: u16,
    compression: u32,
    channels: Channels,
    /// Byte range of the palette (4 bytes per entry; 3 for core headers).
    palette: (usize, usize),
    palette_entry: usize,
    data: usize,
}

fn le16(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..)?.first_chunk().map(|s| u16::from_le_bytes(*s))
}

fn le32(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..)?.first_chunk().map(|s| u32::from_le_bytes(*s))
}

const RGB: u32 = 0;
const RLE8: u32 = 1;
const RLE4: u32 = 2;
const BITFIELDS: u32 = 3;
const ALPHABITFIELDS: u32 = 6;

fn parse(b: &[u8]) -> Option<Info> {
    if !b.starts_with(b"BM") {
        return None;
    }
    let data = le32(b, 10)? as usize;
    let dib = le32(b, 14)? as usize;
    let (w, h, bpp, compression, entry) = if dib == 12 {
        (
            u32::from(le16(b, 18)?),
            i32::from(le16(b, 20)?),
            le16(b, 24)?,
            RGB,
            3,
        )
    } else if dib >= 40 {
        (le32(b, 18)?, le32(b, 22)? as i32, le16(b, 28)?, le32(b, 30)?, 4)
    } else {
        return None;
    };
    let top_down = h < 0;
    let h = h.unsigned_abs();
    if w == 0 || h == 0 || w > BMP_MAX_DIM || h > BMP_MAX_DIM {
        return None;
    }
    let ok = match compression {
        RGB => matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32),
        RLE8 => bpp == 8 && !top_down,
        RLE4 => bpp == 4 && !top_down,
        BITFIELDS | ALPHABITFIELDS => matches!(bpp, 16 | 32),
        _ => false,
    };
    if !ok {
        return None;
    }
    // Masks (red, green, blue, alpha) follow a 40-byte header, or are part of V2+ headers (same
    // offsets).
    let [r, g, bl, a] = if matches!(compression, BITFIELDS | ALPHABITFIELDS) {
        [
            le32(b, 54)?,
            le32(b, 58)?,
            le32(b, 62)?,
            if compression == ALPHABITFIELDS || dib >= 56 {
                le32(b, 66)?
            } else {
                0
            },
        ]
    } else if bpp == 16 {
        [0x7C00, 0x03E0, 0x001F, 0]
    } else {
        [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0]
    };
    // Paletted images have no channels; bit-field masks are validated here, once.
    let channel = |mask| {
        if bpp > 8 {
            Channel::from_mask(mask, bpp)
        } else {
            Some(Channel::ABSENT)
        }
    };
    let channels = Channels {
        r: channel(r)?,
        g: channel(g)?,
        b: channel(bl)?,
        a: channel(a)?,
    };
    // Saturating: an offset past the file reads as a missing palette.
    let mut pal_start = dib.saturating_add(14);
    if dib == 40 && compression == BITFIELDS {
        pal_start = pal_start.saturating_add(12);
    } else if dib == 40 && compression == ALPHABITFIELDS {
        pal_start = pal_start.saturating_add(16);
    }
    let pal_end = if bpp <= 8 { data.min(b.len()) } else { pal_start };
    Some(Info {
        w,
        h,
        top_down,
        bpp,
        compression,
        channels,
        palette: (pal_start, pal_end.max(pal_start)),
        palette_entry: entry,
        data,
    })
}

impl Info {
    fn has_alpha(&self) -> bool {
        self.channels.a != Channel::ABSENT && self.bpp == 32
    }

    fn palette_color(&self, b: &[u8], i: usize) -> [u8; 4] {
        // SAFE-ARITH: `i ≤ 255` and `palette_entry ≤ 4`.
        let o = self.palette.0.saturating_add(i.wrapping_mul(self.palette_entry));
        if o.saturating_add(3) > self.palette.1 {
            return [0, 0, 0, 255];
        }
        match b.get(o..).and_then(<[u8]>::first_chunk::<3>) {
            Some(&[b, g, r]) => [b, g, r, 255],
            None => [0, 0, 0, 255],
        }
    }

    /// Output row of file row `r < h` (rows are stored bottom-up unless `top_down`).
    fn out_row(&self, r: usize) -> usize {
        // SAFE-ARITH: `r < h` (the callers' loops and checks).
        if self.top_down {
            r
        } else {
            (self.h as usize).wrapping_sub(1).wrapping_sub(r)
        }
    }

    /// The output pixel (B, G, R, A) of a 16- or 32-bit value.
    #[inline]
    fn channels_px(&self, v: u32, alpha: bool) -> [u8; 4] {
        let c = self.channels;
        [
            c.b.get(v),
            c.g.get(v),
            c.r.get(v),
            if alpha { c.a.get(v) } else { 255 },
        ]
    }
}

fn decode_rgb(b: &[u8], info: &Info, out: &mut [u8]) -> Result<(), Error> {
    let (w, h) = (info.w as usize, info.h as usize);
    let bpp = usize::from(info.bpp);
    // SAFE-ARITH: `w ≤ BMP_MAX_DIM = 2^14` and `bpp ≤ 32` (`parse`): `w · bpp < 2^20`.
    let stride = w.wrapping_mul(bpp).div_ceil(32).wrapping_mul(4);
    let alpha = info.has_alpha();
    // SAFE-ARITH: `w ≤ 2^14`.
    let out_stride = w.wrapping_mul(4);
    for r in 0..h {
        // SAFE-ARITH: `r < h ≤ 2^14` and `stride < 2^18`; the data offset is a header value
        // (saturating: a too large offset is a truncated file).
        let s = info.data.saturating_add(r.wrapping_mul(stride));
        let row = b
            .get(s..s.saturating_add(stride))
            .ok_or(Error::Decode("bmp truncated"))?;
        // The output holds `w × h` pixels (`decode`): the row always exists.
        let Some(orow) = out.chunks_exact_mut(out_stride.max(1)).nth(info.out_row(r)) else {
            continue;
        };
        let pixels = orow.chunks_exact_mut(4);
        match info.bpp {
            1 | 4 | 8 => {
                for (o, x) in pixels.zip(0..w) {
                    let i = unpack_bits(row, info.bpp as u8, x);
                    o.copy_from_slice(&info.palette_color(b, usize::from(i)));
                }
            }
            24 => {
                for (o, s) in pixels.zip(row.chunks_exact(3)) {
                    if let &[bl, g, r] = s {
                        o.copy_from_slice(&[bl, g, r, 255]);
                    }
                }
            }
            16 => {
                for (o, s) in pixels.zip(row.chunks_exact(2)) {
                    if let &[lo, hi] = s {
                        o.copy_from_slice(&info.channels_px(u32::from(u16::from_le_bytes([lo, hi])), alpha));
                    }
                }
            }
            _ => {
                for (o, s) in pixels.zip(row.chunks_exact(4)) {
                    if let Some(v) = s.first_chunk::<4>() {
                        o.copy_from_slice(&info.channels_px(u32::from_le_bytes(*v), alpha));
                    }
                }
            }
        }
    }
    Ok(())
}

fn decode_rle(b: &[u8], info: &Info, out: &mut [u8]) -> Result<(), Error> {
    let (w, h) = (info.w as usize, info.h as usize);
    let four = info.compression == RLE4;
    let mut p = info.data;
    let (mut x, mut r) = (0usize, 0usize);
    let next = |p: &mut usize| -> Result<u8, Error> {
        let v = *b.get(*p).ok_or(Error::Decode("bmp truncated"))?;
        // SAFE-ARITH: `*p < b.len()` (the byte exists).
        *p = p.wrapping_add(1);
        Ok(v)
    };
    let mut put = |x: usize, r: usize, i: u8| {
        if x < w && r < h {
            // SAFE-ARITH: `x < w`, `r < h`: inside the `w × h × 4` output.
            let o = info.out_row(r).wrapping_mul(w).wrapping_add(x).wrapping_mul(4);
            if let Some(px) = out.get_mut(o..o.wrapping_add(4)) {
                px.copy_from_slice(&info.palette_color(b, usize::from(i)));
            }
        }
    };
    // The positions only grow (by at most 255 per input byte); they saturate rather than wrap
    // (a position past the image draws nothing).
    loop {
        let n = next(&mut p)?;
        let v = next(&mut p)?;
        if n > 0 {
            for k in 0..usize::from(n) {
                let i = if four {
                    if k % 2 == 0 { v >> 4 } else { v & 15 }
                } else {
                    v
                };
                put(x, r, i);
                x = x.saturating_add(1);
            }
            continue;
        }
        match v {
            0 => {
                x = 0;
                r = r.saturating_add(1);
            }
            1 => return Ok(()),
            2 => {
                x = x.saturating_add(usize::from(next(&mut p)?));
                r = r.saturating_add(usize::from(next(&mut p)?));
            }
            n => {
                let n = usize::from(n);
                let bytes = if four { n.div_ceil(2) } else { n };
                for k in 0..n {
                    let byte = *b
                        .get(p.saturating_add(if four { k / 2 } else { k }))
                        .ok_or(Error::Decode("bmp truncated"))?;
                    let i = if four {
                        if k % 2 == 0 { byte >> 4 } else { byte & 15 }
                    } else {
                        byte
                    };
                    put(x, r, i);
                    x = x.saturating_add(1);
                }
                // Absolute runs are padded to an even length; `bytes ≤ 255`.
                p = p.saturating_add(bytes.wrapping_add(bytes % 2));
            }
        }
        if r >= h {
            return Ok(());
        }
    }
}

impl Decoder for BmpDecoder {
    fn name(&self) -> &'static str {
        "bmp"
    }

    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
        let info = parse(bytes)?;
        Some(out_header(info.w as u16, info.h as u16, info.has_alpha()))
    }

    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        let info = parse(bytes).ok_or(Error::InvalidHeader)?;
        let header = out_header(info.w as u16, info.h as u16, info.has_alpha());
        // SAFE-ARITH: `w, h ≤ BMP_MAX_DIM = 2^14`: `n ≤ 2^30`.
        let n = usize::from(header.w)
            .wrapping_mul(usize::from(header.h))
            .wrapping_mul(4);
        prepare_out(out, n)?;
        if matches!(info.compression, RLE4 | RLE8) {
            // Pixels skipped by RLE deltas stay transparent black.
            decode_rle(bytes, &info, out)?;
        } else {
            decode_rgb(bytes, &info, out)?;
        }
        log_decoded("bmp", header, n);
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(v: u32, mask: u32) -> u8 {
        Channel::from_mask(mask, 32).unwrap().get(v)
    }

    #[test]
    fn channel_scaling() {
        assert_eq!(channel(0x1F, 0x1F), 255);
        assert_eq!(channel(0x10, 0x1F), 132);
        assert_eq!(channel(0xAB00, 0xFF00), 0xAB);
        assert_eq!(channel(0, 0), 255);
        assert_eq!(channel(0xFFFF_0000, 0xFFFF_0000), 255);
        assert_eq!(
            Channel::from_mask(0xFFFF_0000, 32),
            Some(Channel {
                shift: 16,
                max: 0xFFFF
            })
        );
        // Wider than 16 bits: rejected.
        assert_eq!(Channel::from_mask(u32::MAX, 32), None);
        assert_eq!(Channel::from_mask(0x1_FFFF, 32), None);
    }

    #[test]
    fn masks_are_validated_at_parse_time() {
        // Not contiguous, or outside a 16-bit pixel: rejected.
        assert_eq!(Channel::from_mask(0b1011, 32), None);
        assert_eq!(Channel::from_mask(0x1_0000, 16), None);
        assert_eq!(Channel::from_mask(0x8000_0001, 32), None);
        // The top bit alone, and every valid shift amount: in range.
        for shift in 0..32 {
            let c = Channel::from_mask(1 << shift, 32).unwrap();
            assert_eq!((c.shift, c.max), (shift, 1));
        }
    }

    /// A hand-built 3×2 RLE8 BMP: row 0 (bottom) = 2×index 1 + index 2, row 1 = absolute run.
    #[test]
    fn rle8_decodes() {
        let mut f = Vec::new();
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&[0; 8]);
        let pal = [0u8, 0, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0]; // black, red, green, blue
        f.extend_from_slice(&((14 + 40 + pal.len()) as u32).to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&3u32.to_le_bytes());
        f.extend_from_slice(&2i32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&8u16.to_le_bytes());
        f.extend_from_slice(&RLE8.to_le_bytes());
        f.extend_from_slice(&[0; 20]);
        f.extend_from_slice(&pal);
        f.extend_from_slice(&[2, 1, 1, 2, 0, 0, 0, 3, 3, 2, 1, 0, 0, 1]);
        let mut out = Vec::new();
        let h = BmpDecoder.decode(&f, &mut out).unwrap();
        assert_eq!((h.w, h.h), (3, 2));
        let red = [0, 0, 255, 255];
        let green = [0, 255, 0, 255];
        let blue = [255, 0, 0, 255];
        // Output row 1 is file row 0.
        assert_eq!(&out[12..24], [red, red, green].concat());
        assert_eq!(&out[..12], [blue, green, red].concat());
    }
}
