//! In-house BMP decoder: 1, 4, 8 bpp paletted (uncompressed, RLE4, RLE8), 16 bpp
//! (RGB555 or bit fields), 24 bpp and 32 bpp (with optional alpha mask); bottom-up and
//! top-down rows; every DIB header version from `BITMAPCOREHEADER` to `BITMAPV5HEADER`.
//! Malformed data returns an error, never panics.

use alloc::vec::Vec;

use crate::decoder::{Decoder, log_decoded, out_header, prepare_out};
use crate::{Error, ImageHeader};

/// Largest accepted width or height.
pub const BMP_MAX_DIM: u32 = 16384;

/// Decodes BMP files (`BM` magic).
#[derive(Clone, Copy, Debug, Default)]
pub struct BmpDecoder;

/// Channel masks of 16/32-bit images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Masks {
    r: u32,
    g: u32,
    b: u32,
    a: u32,
}

/// Parsed headers.
#[derive(Clone, Copy, Debug)]
struct Info {
    w: u32,
    h: u32,
    top_down: bool,
    bpp: u16,
    compression: u32,
    masks: Masks,
    /// Byte range of the palette (4 bytes per entry; 3 for core headers).
    palette: (usize, usize),
    palette_entry: usize,
    data: usize,
}

fn le16(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn le32(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
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
    let masks = if matches!(compression, BITFIELDS | ALPHABITFIELDS) {
        // Masks follow a 40-byte header, or are part of V2+ headers (same offsets).
        Masks {
            r: le32(b, 54)?,
            g: le32(b, 58)?,
            b: le32(b, 62)?,
            a: if compression == ALPHABITFIELDS || dib >= 56 {
                le32(b, 66)?
            } else {
                0
            },
        }
    } else if bpp == 16 {
        Masks {
            r: 0x7C00,
            g: 0x03E0,
            b: 0x001F,
            a: 0,
        }
    } else {
        Masks {
            r: 0x00FF_0000,
            g: 0x0000_FF00,
            b: 0x0000_00FF,
            a: 0,
        }
    };
    let mut pal_start = 14 + dib;
    if dib == 40 && compression == BITFIELDS {
        pal_start += 12;
    } else if dib == 40 && compression == ALPHABITFIELDS {
        pal_start += 16;
    }
    let pal_end = if bpp <= 8 { data.min(b.len()) } else { pal_start };
    Some(Info {
        w,
        h,
        top_down,
        bpp,
        compression,
        masks,
        palette: (pal_start, pal_end.max(pal_start)),
        palette_entry: entry,
        data,
    })
}

/// Extracts the channel selected by `mask` from `v`, scaled to 8 bits (255 without a mask).
fn channel(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones().min(16);
    let max = (1u32 << bits) - 1;
    let c = ((v & mask) >> shift) & max;
    ((c * 255 + max / 2) / max) as u8
}

impl Info {
    fn has_alpha(&self) -> bool {
        self.masks.a != 0 && self.bpp == 32
    }

    fn palette_color(&self, b: &[u8], i: usize) -> [u8; 4] {
        let o = self.palette.0 + i * self.palette_entry;
        if o + 3 > self.palette.1 {
            return [0, 0, 0, 255];
        }
        match b.get(o..o + 3) {
            Some(p) => [p[0], p[1], p[2], 255],
            None => [0, 0, 0, 255],
        }
    }

    /// Output row of file row `r` (rows are stored bottom-up unless `top_down`).
    fn out_row(&self, r: usize) -> usize {
        if self.top_down { r } else { self.h as usize - 1 - r }
    }
}

fn decode_rgb(b: &[u8], info: &Info, out: &mut [u8]) -> Result<(), Error> {
    let (w, h) = (info.w as usize, info.h as usize);
    let stride = (w * usize::from(info.bpp)).div_ceil(32) * 4;
    let alpha = info.has_alpha();
    for r in 0..h {
        let s = info.data + r * stride;
        let row = b.get(s..s + stride).ok_or(Error::Decode("bmp truncated"))?;
        let y = info.out_row(r);
        for x in 0..w {
            let px = match info.bpp {
                1 | 4 | 8 => {
                    let d = usize::from(info.bpp);
                    let bit = x * d;
                    let i = (row[bit / 8] >> (8 - d - bit % 8)) & ((1u16 << d) - 1) as u8;
                    info.palette_color(b, usize::from(i))
                }
                24 => [row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 255],
                _ => {
                    let v = if info.bpp == 16 {
                        u32::from(u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]))
                    } else {
                        u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]])
                    };
                    let m = info.masks;
                    [
                        channel(v, m.b),
                        channel(v, m.g),
                        channel(v, m.r),
                        if alpha { channel(v, m.a) } else { 255 },
                    ]
                }
            };
            let o = (y * w + x) * 4;
            out[o..o + 4].copy_from_slice(&px);
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
        *p += 1;
        Ok(v)
    };
    let mut put = |x: usize, r: usize, i: u8| {
        if x < w && r < h {
            let o = (info.out_row(r) * w + x) * 4;
            out[o..o + 4].copy_from_slice(&info.palette_color(b, usize::from(i)));
        }
    };
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
                x += 1;
            }
            continue;
        }
        match v {
            0 => {
                x = 0;
                r += 1;
            }
            1 => return Ok(()),
            2 => {
                x += usize::from(next(&mut p)?);
                r += usize::from(next(&mut p)?);
            }
            n => {
                let n = usize::from(n);
                let bytes = if four { n.div_ceil(2) } else { n };
                for k in 0..n {
                    let byte = *b
                        .get(p + if four { k / 2 } else { k })
                        .ok_or(Error::Decode("bmp truncated"))?;
                    let i = if four {
                        if k % 2 == 0 { byte >> 4 } else { byte & 15 }
                    } else {
                        byte
                    };
                    put(x, r, i);
                    x += 1;
                }
                p += bytes + bytes % 2;
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
        let n = usize::from(header.w) * usize::from(header.h) * 4;
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

    #[test]
    fn channel_scaling() {
        assert_eq!(channel(0x1F, 0x1F), 255);
        assert_eq!(channel(0x10, 0x1F), 132);
        assert_eq!(channel(0xAB00, 0xFF00), 0xAB);
        assert_eq!(channel(0, 0), 255);
        assert_eq!(channel(u32::MAX, u32::MAX), 255);
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
