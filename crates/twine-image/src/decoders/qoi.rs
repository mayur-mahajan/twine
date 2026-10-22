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
    let [r, g, b, a] = p.map(usize::from);
    // SAFE-ARITH: each channel is at most 255, so the weighted sum is at most `255 · 26`.
    r.wrapping_mul(3)
        .wrapping_add(g.wrapping_mul(5))
        .wrapping_add(b.wrapping_mul(7))
        .wrapping_add(a.wrapping_mul(11))
        & 63
}

/// The QOI header: `(width, height, channels)`.
fn parse_header(bytes: &[u8]) -> Result<(u16, u16, u8), Error> {
    let h: &[u8; HEADER_LEN] = bytes.first_chunk().ok_or(Error::InvalidHeader)?;
    let [
        m0,
        m1,
        m2,
        m3,
        w0,
        w1,
        w2,
        w3,
        h0,
        h1,
        h2,
        h3,
        channels,
        colorspace,
    ] = *h;
    if &[m0, m1, m2, m3] != MAGIC {
        return Err(Error::InvalidHeader);
    }
    let w = u32::from_be_bytes([w0, w1, w2, w3]);
    let ht = u32::from_be_bytes([h0, h1, h2, h3]);
    if w == 0
        || ht == 0
        || w > QOI_MAX_DIM
        || ht > QOI_MAX_DIM
        || !(3..=4).contains(&channels)
        || colorspace > 1
    {
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
        // `parse_header` read the header: the data follows it.
        decode_ops(bytes.get(HEADER_LEN..).unwrap_or_default(), out)?;
        log_decoded("qoi", header, n);
        Ok(header)
    }
}

/// Decodes the op stream into `out` (B, G, R, A per pixel).
fn decode_ops(data: &[u8], out: &mut [u8]) -> Result<(), Error> {
    let mut index = [[0u8; 4]; 64];
    let mut px = [0u8, 0, 0, 255]; // r, g, b, a
    let mut run = 0usize;
    let mut input = data.iter().copied();
    let mut byte = || input.next().ok_or(Error::Decode("qoi truncated"));
    for o in out.chunks_exact_mut(4) {
        if run > 0 {
            // SAFE-ARITH: `run > 0`.
            run = run.wrapping_sub(1);
        } else {
            let b1 = byte()?;
            let [r, g, b, a] = &mut px;
            match b1 {
                OP_RGB => {
                    (*r, *g, *b) = (byte()?, byte()?, byte()?);
                }
                OP_RGBA => {
                    (*r, *g, *b, *a) = (byte()?, byte()?, byte()?, byte()?);
                }
                // QOI channel arithmetic wraps by definition (the format's "diff" ops).
                _ => match b1 & MASK_2 {
                    // The index is the low 6 bits (`b1 & MASK_2 == 0`): always below 64.
                    OP_INDEX => px = index.get(usize::from(b1 & 0x3F)).copied().unwrap_or(px),
                    OP_DIFF => {
                        *r = r.wrapping_add(((b1 >> 4) & 3).wrapping_sub(2));
                        *g = g.wrapping_add(((b1 >> 2) & 3).wrapping_sub(2));
                        *b = b.wrapping_add((b1 & 3).wrapping_sub(2));
                    }
                    OP_LUMA => {
                        let b2 = byte()?;
                        let dg = (b1 & 0x3F).wrapping_sub(32);
                        *r = r.wrapping_add(dg.wrapping_add((b2 >> 4).wrapping_sub(8)));
                        *g = g.wrapping_add(dg);
                        *b = b.wrapping_add(dg.wrapping_add((b2 & 0x0F).wrapping_sub(8)));
                    }
                    _ => {
                        debug_assert_eq!(b1 & MASK_2, OP_RUN);
                        run = usize::from(b1 & 0x3F);
                    }
                },
            }
            // `hash` is below 64 (masked): the slot always exists.
            if let Some(slot) = index.get_mut(hash(px)) {
                *slot = px;
            }
        }
        let [r, g, b, a] = px;
        o.copy_from_slice(&[b, g, r, a]);
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
    let mut out = Vec::with_capacity(HEADER_LEN.saturating_add(rgba.len()).saturating_add(END.len()));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.push(channels);
    out.push(0);
    let mut index = [[0u8; 4]; 64];
    let mut prev = [0u8, 0, 0, 255];
    let mut run = 0u8;
    let n = (w as usize).saturating_mul(h as usize).min(rgba.len() / 4);
    for (i, c) in rgba.chunks_exact(4).take(n).enumerate() {
        let Some(&[r, g, b, a]) = c.first_chunk::<4>() else {
            continue;
        };
        let px = [r, g, b, if channels == 4 { a } else { 255 }];
        // SAFE-ARITH: `run < 62` before the increment, `run ≥ 1` when it is emitted, and
        // `i < n` (so `n ≥ 1`).
        if px == prev {
            run = run.wrapping_add(1);
            if run == 62 || i == n.wrapping_sub(1) {
                out.push(OP_RUN | run.wrapping_sub(1));
                run = 0;
            }
            continue;
        }
        if run > 0 {
            out.push(OP_RUN | run.wrapping_sub(1));
            run = 0;
        }
        let hi = hash(px);
        // `hi < 64` (masked): the slot always exists.
        let Some(slot) = index.get_mut(hi) else {
            continue;
        };
        if *slot == px {
            out.push(OP_INDEX | hi as u8);
        } else {
            *slot = px;
            if px[3] == prev[3] {
                let vr = px[0].wrapping_sub(prev[0]) as i8;
                let vg = px[1].wrapping_sub(prev[1]) as i8;
                let vb = px[2].wrapping_sub(prev[2]) as i8;
                let vg_r = vr.wrapping_sub(vg);
                let vg_b = vb.wrapping_sub(vg);
                // SAFE-ARITH: the offsets below map each checked range onto `0..` (`-2..2` +
                // 2, `-32..32` + 32, `-8..8` + 8).
                if (-2..2).contains(&vr) && (-2..2).contains(&vg) && (-2..2).contains(&vb) {
                    let [dr, dg, db] = [vr, vg, vb].map(|v| v.wrapping_add(2) as u8);
                    out.push(OP_DIFF | (dr << 4) | (dg << 2) | db);
                } else if (-8..8).contains(&vg_r) && (-32..32).contains(&vg) && (-8..8).contains(&vg_b) {
                    out.push(OP_LUMA | vg.wrapping_add(32) as u8);
                    out.push(((vg_r.wrapping_add(8) as u8) << 4) | vg_b.wrapping_add(8) as u8);
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
