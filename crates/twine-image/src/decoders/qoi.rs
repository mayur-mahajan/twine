//! In-house QOI decoder (<https://qoiformat.org/qoi-specification.pdf>), plus an encoder
//! (feature `std`) for tools and tests.
//!
//! Decoding allocates nothing but the output buffer: the 64-entry color index lives on the
//! stack.

use alloc::vec::Vec;

use crate::decoder::{Decoder, log_decoded, out_header, prepare_out};
use crate::{Error, ImageHeader};

const MAGIC: &[u8; 4] = b"qoif";
const HEADER_LEN: usize = 14;
/// Largest accepted width or height.
pub const QOI_MAX_DIM: u32 = 16384;

const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xC0;
const OP_RGB: u8 = 0xFE;
const OP_RGBA: u8 = 0xFF;
const MASK_2: u8 = 0xC0;
#[cfg(feature = "std")]
const END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

#[inline(always)]
fn hash(p: [u8; 4]) -> usize {
    (usize::from(p[0]) * 3 + usize::from(p[1]) * 5 + usize::from(p[2]) * 7 + usize::from(p[3]) * 11) % 64
}

/// The QOI header: `(width, height, channels)`.
fn parse_header(bytes: &[u8]) -> Result<(u16, u16, u8), Error> {
    let h = bytes.get(..HEADER_LEN).ok_or(Error::InvalidHeader)?;
    if &h[..4] != MAGIC {
        return Err(Error::InvalidHeader);
    }
    let w = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
    let ht = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
    let channels = h[12];
    if w == 0 || ht == 0 || w > QOI_MAX_DIM || ht > QOI_MAX_DIM || !(3..=4).contains(&channels) || h[13] > 1 {
        return Err(Error::InvalidHeader);
    }
    Ok((w as u16, ht as u16, channels))
}

/// Decodes QOI images (`qoif` magic, 3 or 4 channels, up to 16384 × 16384).
#[derive(Clone, Copy, Debug, Default)]
pub struct QoiDecoder;

impl Decoder for QoiDecoder {
    fn name(&self) -> &'static str {
        "qoi"
    }

    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
        let (w, h, c) = parse_header(bytes).ok()?;
        Some(out_header(w, h, c == 4))
    }

    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        let (w, h, channels) = parse_header(bytes)?;
        let header = out_header(w, h, channels == 4);
        let n = usize::from(w)
            .checked_mul(usize::from(h))
            .and_then(|p| p.checked_mul(4))
            .ok_or(Error::InvalidHeader)?;
        prepare_out(out, n)?;
        decode_ops(&bytes[HEADER_LEN..], out)?;
        log_decoded("qoi", header, n);
        Ok(header)
    }
}

/// Decodes the op stream into `out` (B, G, R, A per pixel).
fn decode_ops(data: &[u8], out: &mut [u8]) -> Result<(), Error> {
    let mut index = [[0u8; 4]; 64];
    let mut px = [0u8, 0, 0, 255]; // r, g, b, a
    let mut run = 0usize;
    let mut p = 0usize;
    let byte = |p: &mut usize| -> Result<u8, Error> {
        let b = *data.get(*p).ok_or(Error::Decode("qoi truncated"))?;
        *p += 1;
        Ok(b)
    };
    for o in out.chunks_exact_mut(4) {
        if run > 0 {
            run -= 1;
        } else {
            let b1 = byte(&mut p)?;
            match b1 {
                OP_RGB => {
                    px[0] = byte(&mut p)?;
                    px[1] = byte(&mut p)?;
                    px[2] = byte(&mut p)?;
                }
                OP_RGBA => {
                    px[0] = byte(&mut p)?;
                    px[1] = byte(&mut p)?;
                    px[2] = byte(&mut p)?;
                    px[3] = byte(&mut p)?;
                }
                _ => match b1 & MASK_2 {
                    OP_INDEX => px = index[usize::from(b1)],
                    OP_DIFF => {
                        px[0] = px[0].wrapping_add(((b1 >> 4) & 3).wrapping_sub(2));
                        px[1] = px[1].wrapping_add(((b1 >> 2) & 3).wrapping_sub(2));
                        px[2] = px[2].wrapping_add((b1 & 3).wrapping_sub(2));
                    }
                    OP_LUMA => {
                        let b2 = byte(&mut p)?;
                        let dg = (b1 & 0x3F).wrapping_sub(32);
                        px[0] = px[0].wrapping_add(dg.wrapping_add((b2 >> 4).wrapping_sub(8)));
                        px[1] = px[1].wrapping_add(dg);
                        px[2] = px[2].wrapping_add(dg.wrapping_add((b2 & 0x0F).wrapping_sub(8)));
                    }
                    _ => {
                        debug_assert_eq!(b1 & MASK_2, OP_RUN);
                        run = usize::from(b1 & 0x3F);
                    }
                },
            }
            index[hash(px)] = px;
        }
        o.copy_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    Ok(())
}

/// Encodes `w × h` pixels given as `R, G, B, A` bytes into a QOI file with `channels` (3 or 4;
/// with 3 the alpha bytes are ignored).
///
/// ```
/// use twine_image::decoders::qoi::{QoiDecoder, qoi_encode};
/// use twine_image::Decoder;
/// let rgba = [255, 0, 0, 255, 0, 0, 255, 128];
/// let file = qoi_encode(&rgba, 2, 1, 4);
/// let mut out = Vec::new();
/// QoiDecoder.decode(&file, &mut out).unwrap();
/// assert_eq!(out, [0, 0, 255, 255, 255, 0, 0, 128]); // B, G, R, A
/// ```
#[cfg(feature = "std")]
#[must_use]
pub fn qoi_encode(rgba: &[u8], w: u32, h: u32, channels: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + rgba.len() + END.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.push(channels);
    out.push(0);
    let mut index = [[0u8; 4]; 64];
    let mut prev = [0u8, 0, 0, 255];
    let mut run = 0u8;
    let n = (w as usize * h as usize).min(rgba.len() / 4);
    for (i, c) in rgba.chunks_exact(4).take(n).enumerate() {
        let px = [c[0], c[1], c[2], if channels == 4 { c[3] } else { 255 }];
        if px == prev {
            run += 1;
            if run == 62 || i == n - 1 {
                out.push(OP_RUN | (run - 1));
                run = 0;
            }
            continue;
        }
        if run > 0 {
            out.push(OP_RUN | (run - 1));
            run = 0;
        }
        let hi = hash(px);
        if index[hi] == px {
            out.push(OP_INDEX | hi as u8);
        } else {
            index[hi] = px;
            if px[3] == prev[3] {
                let vr = px[0].wrapping_sub(prev[0]) as i8;
                let vg = px[1].wrapping_sub(prev[1]) as i8;
                let vb = px[2].wrapping_sub(prev[2]) as i8;
                let vg_r = vr.wrapping_sub(vg);
                let vg_b = vb.wrapping_sub(vg);
                if (-2..2).contains(&vr) && (-2..2).contains(&vg) && (-2..2).contains(&vb) {
                    out.push(OP_DIFF | (((vr + 2) as u8) << 4) | (((vg + 2) as u8) << 2) | (vb + 2) as u8);
                } else if (-8..8).contains(&vg_r) && (-32..32).contains(&vg) && (-8..8).contains(&vg_b) {
                    out.push(OP_LUMA | (vg + 32) as u8);
                    out.push((((vg_r + 8) as u8) << 4) | (vg_b + 8) as u8);
                } else {
                    out.extend_from_slice(&[OP_RGB, px[0], px[1], px[2]]);
                }
            } else {
                out.extend_from_slice(&[OP_RGBA, px[0], px[1], px[2], px[3]]);
            }
        }
        prev = px;
    }
    out.extend_from_slice(&END);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_limits() {
        let mut h = [0u8; 14];
        h[..4].copy_from_slice(MAGIC);
        h[4..8].copy_from_slice(&16384u32.to_be_bytes());
        h[8..12].copy_from_slice(&1u32.to_be_bytes());
        h[12] = 4;
        assert!(parse_header(&h).is_ok());
        h[4..8].copy_from_slice(&16385u32.to_be_bytes());
        assert_eq!(parse_header(&h), Err(Error::InvalidHeader));
        h[4..8].copy_from_slice(&1u32.to_be_bytes());
        h[12] = 5;
        assert_eq!(parse_header(&h), Err(Error::InvalidHeader));
    }
}
