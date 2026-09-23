//! Encoder helpers for font generators (feature `std`): bit packing, the XOR prefilter, the RLE
//! encoder (the exact inverse of the decoder) and [`FontData`], an owned font description that
//! a generator fills, writes out as Rust source, or turns into a live [`Font`] with
//! [`FontData::leak`].
//!
//! ```
//! use twine_text::encode::{rle_encode, rle_decode};
//! let values = [3u8, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 1];
//! let bytes = rle_encode(&values, 2);
//! assert!(bytes.len() < values.len() * 2 / 8 + 1);
//! assert_eq!(rle_decode(&bytes, 2, values.len()), values);
//! ```

use std::boxed::Box;
use std::vec;
use std::vec::Vec;

use crate::bitmap_font::{BitmapFont, BitmapFormat, Cmap, CmapKind, GlyphDsc, GlyphIdOfs, Kern, KernPairIds};
use crate::decode::Rle;
use crate::font::{Font, Subpx};

/// Writes values MSB-first into a growing byte vector.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    /// An empty writer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends the low `n` (≤ 8) bits of `v`, most significant first.
    pub fn write(&mut self, v: u8, n: u8) {
        for i in (0..n).rev() {
            let bit = (v >> i) & 1;
            if self.bits % 8 == 0 {
                self.bytes.push(0);
            }
            if bit == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    /// Number of bits written.
    #[must_use]
    pub fn bit_len(&self) -> usize {
        self.bits
    }

    /// The bytes, the last one zero-padded.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Packs `values` (each `< 1 << bpp`) MSB-first without padding between rows.
#[must_use]
pub fn pack_bits(values: &[u8], bpp: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    for &v in values {
        w.write(v, bpp);
    }
    w.finish()
}

/// XOR prefilter: every row (of `w` values) XOR-ed with the previous **original** row.
#[must_use]
pub fn prefilter(values: &[u8], w: usize) -> Vec<u8> {
    let mut out = values.to_vec();
    if w == 0 {
        return out;
    }
    for i in (w..values.len()).rev() {
        out[i] ^= values[i - w];
    }
    out
}

/// LVGL-compatible RLE encoding of `values` (each `< 1 << bpp`), the inverse of the decoder:
///
/// 1. *Single*: write a literal; if it equals the previous literal and is not the first value,
///    switch to *Repeated* with count 0.
/// 2. *Repeated*, next value `v`: equal to the previous → write `1`, count + 1; at count 11 a
///    6-bit `N = min(m + 1, 63)` follows (`m` = further equal values), `N − 1` repeats are
///    implicit, and the value after them (if any) is written as a literal without the
///    equality check. Different → write `0` and the literal `v` (again without the check).
///
/// The final byte is zero-padded.
#[must_use]
pub fn rle_encode(values: &[u8], bpp: u8) -> Vec<u8> {
    #[derive(PartialEq)]
    enum St {
        Single,
        Repeated,
    }
    let mut w = BitWriter::new();
    let mut st = St::Single;
    let mut prev = 0u8;
    let mut cnt = 0u32;
    let mut i = 0;
    while i < values.len() {
        let v = values[i];
        match st {
            St::Single => {
                let first = w.bit_len() == 0;
                w.write(v, bpp);
                if !first && v == prev {
                    st = St::Repeated;
                    cnt = 0;
                }
                prev = v;
                i += 1;
            }
            St::Repeated => {
                if v == prev {
                    w.write(1, 1);
                    cnt += 1;
                    i += 1;
                    if cnt == 11 {
                        let m = values[i..].iter().take_while(|&&x| x == prev).count();
                        let n = (m + 1).min(63);
                        w.write(n as u8, 6);
                        i += n - 1;
                        if i < values.len() {
                            prev = values[i];
                            w.write(prev, bpp);
                            i += 1;
                        }
                        st = St::Single;
                    }
                } else {
                    w.write(0, 1);
                    w.write(v, bpp);
                    prev = v;
                    i += 1;
                    st = St::Single;
                }
            }
        }
    }
    w.finish()
}

/// Decodes `n` RLE values (bpp domain) — the counterpart of [`rle_encode`] for tests and tools.
#[must_use]
pub fn rle_decode(bytes: &[u8], bpp: u8, n: usize) -> Vec<u8> {
    let mut r = Rle::new(bytes, bpp);
    let mut out = vec![0; n];
    r.decode_row(&mut out);
    out
}

/// Encodes one glyph's quantized values (`w` per row) in `format`.
#[must_use]
pub fn encode_glyph(values: &[u8], w: usize, bpp: u8, format: BitmapFormat) -> Vec<u8> {
    match format {
        BitmapFormat::Plain => pack_bits(values, bpp),
        BitmapFormat::Compressed => rle_encode(&prefilter(values, w), bpp),
        BitmapFormat::CompressedNoPrefilter => rle_encode(values, bpp),
    }
}

/// Owned counterpart of [`Cmap`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmapData {
    /// First code point.
    pub range_start: u32,
    /// Number of code points in the range.
    pub range_length: u16,
    /// Glyph id of the first code point.
    pub glyph_id_start: u16,
    /// Sorted code points relative to `range_start` (sparse kinds).
    pub unicode_list: Vec<u16>,
    /// `u8` offsets (`Format0Full`).
    pub ofs_u8: Vec<u8>,
    /// `u16` offsets (`SparseFull`).
    pub ofs_u16: Vec<u16>,
    /// Mapping kind.
    pub kind: CmapKind,
}

/// Owned kerning data (pair kerning only; values in 1/16 px before `kern_scale`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KernData {
    /// No kerning.
    #[default]
    None,
    /// Pairs sorted by `(left, right)` glyph id.
    Pairs {
        /// The pairs.
        ids: Vec<[u16; 2]>,
        /// One value per pair.
        values: Vec<i8>,
    },
}

impl KernData {
    /// Whether all pair ids fit `u8` (the compact [`KernPairIds::U8`] form).
    #[must_use]
    pub fn fits_u8(&self) -> bool {
        match self {
            KernData::None => true,
            KernData::Pairs { ids, .. } => ids.iter().all(|p| p[0] < 256 && p[1] < 256),
        }
    }
}

/// An owned font description, as produced by a font generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontData {
    /// Line height in px.
    pub line_height: i16,
    /// Baseline distance from the bottom of the line.
    pub base_line: i16,
    /// Underline position relative to the baseline.
    pub underline_position: i8,
    /// Underline thickness.
    pub underline_thickness: u8,
    /// Subpixel layout.
    pub subpx: Subpx,
    /// Bits per pixel.
    pub bpp: u8,
    /// Bitmap storage format.
    pub format: BitmapFormat,
    /// All glyph bitmaps.
    pub bitmap: Vec<u8>,
    /// Glyph descriptors (index 0 reserved).
    pub glyphs: Vec<GlyphDsc>,
    /// Cmaps.
    pub cmaps: Vec<CmapData>,
    /// Kerning.
    pub kern: KernData,
    /// Kerning scale (12.4).
    pub kern_scale: u16,
}

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

impl FontData {
    /// Builds a live `'static` [`Font`] from this description by leaking its memory. Meant for
    /// tools and tests (for example to compare generated fonts with the rasterizer).
    #[must_use]
    pub fn leak(&self) -> &'static Font {
        let cmaps: Vec<Cmap> = self
            .cmaps
            .iter()
            .map(|c| Cmap {
                range_start: c.range_start,
                range_length: c.range_length,
                glyph_id_start: c.glyph_id_start,
                unicode_list: leak(c.unicode_list.clone()),
                glyph_id_ofs_list: match c.kind {
                    CmapKind::Format0Full => GlyphIdOfs::U8(leak(c.ofs_u8.clone())),
                    CmapKind::SparseFull => GlyphIdOfs::U16(leak(c.ofs_u16.clone())),
                    _ => GlyphIdOfs::None,
                },
                kind: c.kind,
            })
            .collect();
        let kern = match &self.kern {
            KernData::None => Kern::None,
            KernData::Pairs { ids, values } => Kern::Pairs {
                glyph_ids: if self.kern.fits_u8() {
                    KernPairIds::U8(leak(ids.iter().map(|p| [p[0] as u8, p[1] as u8]).collect()))
                } else {
                    KernPairIds::U16(leak(ids.clone()))
                },
                values: leak(values.clone()),
            },
        };
        let provider: &'static BitmapFont = Box::leak(Box::new(BitmapFont {
            bpp: self.bpp,
            bitmap: leak(self.bitmap.clone()),
            glyphs: leak(self.glyphs.clone()),
            cmaps: leak(cmaps),
            kern,
            kern_scale: self.kern_scale,
            format: self.format,
        }));
        Box::leak(Box::new(Font {
            line_height: self.line_height,
            base_line: self.base_line,
            underline_position: self.underline_position,
            underline_thickness: self.underline_thickness,
            provider,
            fallback: None,
            subpx: self.subpx,
        }))
    }
}

/// Builds an in-memory font from the structures a generator produced (the same ones it writes
/// as Rust source), for tests that compare generated fonts with their source rasterizer.
#[must_use]
pub fn load_generated_for_test(data: &FontData) -> &'static Font {
    data.leak()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_writer_msb_first() {
        let mut w = BitWriter::new();
        w.write(0b101, 3);
        w.write(0b11111, 5);
        w.write(1, 1);
        assert_eq!(w.finish(), vec![0b1011_1111, 0b1000_0000]);
    }

    #[test]
    fn encoder_matches_hand_vector() {
        assert_eq!(rle_encode(&[5, 5, 5, 7], 4), vec![0x55, 0x9C]);
    }

    #[test]
    fn prefilter_is_inverted_by_decoder() {
        let v = [1u8, 2, 3, 3, 0, 1];
        let f = prefilter(&v, 2);
        assert_eq!(f, vec![1, 2, 2, 1, 3, 2]);
    }
}
