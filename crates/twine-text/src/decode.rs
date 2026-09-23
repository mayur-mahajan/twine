//! Glyph bitmap decoding: bpp expansion and LVGL's RLE + XOR-prefilter compression.
//!
//! # Bit format
//!
//! All glyph bitmaps store coverage values of `bpp` bits (1, 2, 4 or 8), **MSB first**: bit 7
//! of the first byte is the first pixel's most significant bit. Values are mapped to A8 with
//! LVGL's opacity tables (1 bpp `{0, 255}`, 2 bpp `{0, 85, 170, 255}`, 4 bpp `v × 17`, 8 bpp
//! identity).
//!
//! **Plain** bitmaps are not row-padded: rows follow each other bit-continuously, so row `y` of
//! a `w`-pixel-wide glyph starts at bit `y × w × bpp` of the glyph's data.
//!
//! **Compressed** bitmaps (LVGL `lv_font_fmt_txt.c`, `rle_next`) are one bit stream per glyph
//! whose decoder state persists across rows:
//!
//! - *Single*: read a `bpp`-bit literal. If it is not the stream's first value and equals the
//!   previous value, switch to *Repeated* (repeat count 0).
//! - *Repeated*: read 1 bit. `1` repeats the previous value; after the 11th repeat bit a 6-bit
//!   count `N` follows: `N > 0` switches to *Counter* (the next `N − 1` values repeat, the `N`-th
//!   is a new literal), `N = 0` means the value is a new literal instead. `0` means a new
//!   literal follows (back to *Single*, without the equality check).
//! - *Counter*: repeat the previous value, and read a new literal when the count runs out.
//!
//! With the **XOR prefilter** each decoded row (in the `bpp` value domain, before the A8
//! mapping) is XOR-ed with the previous reconstructed row.
//!
//! Reads past the end of the data return 0 and mark the stream corrupt; decoding never panics.

/// Maps a `bpp`-bit coverage value to A8 (LVGL opacity tables).
#[inline]
#[must_use]
pub(crate) const fn to_a8(v: u8, bpp: u8) -> u8 {
    match bpp {
        1 => {
            if v & 1 == 0 {
                0
            } else {
                255
            }
        }
        2 => (v & 3) * 85,
        4 => (v & 15) * 17,
        _ => v,
    }
}

/// Reads `n` (1..=8) bits MSB-first at bit position `pos`; `None` past the end of `buf`.
#[inline]
fn get_bits(buf: &[u8], pos: usize, n: u8) -> Option<u8> {
    let n = u32::from(n);
    let byte = pos >> 3;
    let bit = (pos & 7) as u32;
    let hi = u32::from(*buf.get(byte)?);
    let v = if bit + n <= 8 {
        hi >> (8 - bit - n)
    } else {
        let lo = u32::from(*buf.get(byte + 1)?);
        ((hi << 8) | lo) >> (16 - bit - n)
    };
    Some((v & ((1 << n) - 1)) as u8)
}

/// Expands `w` values of `bpp` bits starting at bit `bit_offset` of `src` into A8 `out[..w]`.
///
/// Returns `false` (the missing values are written as 0) when `src` is too short.
pub(crate) fn expand_row(src: &[u8], bit_offset: usize, bpp: u8, w: usize, out: &mut [u8]) -> bool {
    let out = &mut out[..w];
    if bpp == 8 {
        let start = (bit_offset / 8).min(src.len());
        let n = (src.len() - start).min(w);
        out[..n].copy_from_slice(&src[start..start + n]);
        out[n..].fill(0);
        return n == w;
    }
    let mut ok = true;
    let mut pos = bit_offset;
    // Fast path per byte for aligned 1/2/4 bpp would be possible; values never straddle a
    // byte for these depths, so one byte read per value is enough.
    for o in out.iter_mut() {
        let byte = pos >> 3;
        let v = if let Some(&b) = src.get(byte) {
            (b >> (8 - (pos & 7) - usize::from(bpp))) & ((1 << bpp) - 1)
        } else {
            ok = false;
            0
        };
        *o = to_a8(v, bpp);
        pos += usize::from(bpp);
    }
    ok
}

/// Decoder state of [`Rle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Single,
    Repeated,
    Counter,
}

/// LVGL's RLE decoder (`rle_next`), line-by-line port. One instance per glyph.
#[derive(Debug)]
pub(crate) struct Rle<'a> {
    data: &'a [u8],
    bpp: u8,
    rdp: usize,
    prev: u8,
    cnt: u8,
    state: State,
    /// Set when a read went past the end of the data.
    pub corrupt: bool,
}

impl<'a> Rle<'a> {
    /// A decoder at the start of `data` (`rdp = 0`, `prev = 0`, `cnt = 0`, state Single).
    pub(crate) fn new(data: &'a [u8], bpp: u8) -> Self {
        Self {
            data,
            bpp,
            rdp: 0,
            prev: 0,
            cnt: 0,
            state: State::Single,
            corrupt: false,
        }
    }

    fn bits(&mut self, n: u8) -> u8 {
        if let Some(v) = get_bits(self.data, self.rdp, n) {
            v
        } else {
            self.corrupt = true;
            0
        }
    }

    /// The next value (in the `bpp` domain).
    pub(crate) fn next_value(&mut self) -> u8 {
        let bpp = self.bpp;
        match self.state {
            State::Single => {
                let ret = self.bits(bpp);
                if self.rdp != 0 && self.prev == ret {
                    self.cnt = 0;
                    self.state = State::Repeated;
                }
                self.prev = ret;
                self.rdp += usize::from(bpp);
                ret
            }
            State::Repeated => {
                let v = self.bits(1);
                self.cnt = self.cnt.wrapping_add(1);
                self.rdp += 1;
                if v == 1 {
                    let mut ret = self.prev;
                    if self.cnt == 11 {
                        self.cnt = self.bits(6);
                        self.rdp += 6;
                        if self.cnt != 0 {
                            self.state = State::Counter;
                        } else {
                            ret = self.bits(bpp);
                            self.prev = ret;
                            self.rdp += usize::from(bpp);
                            self.state = State::Single;
                        }
                    }
                    ret
                } else {
                    let ret = self.bits(bpp);
                    self.prev = ret;
                    self.rdp += usize::from(bpp);
                    self.state = State::Single;
                    ret
                }
            }
            State::Counter => {
                let mut ret = self.prev;
                self.cnt -= 1;
                if self.cnt == 0 {
                    ret = self.bits(bpp);
                    self.prev = ret;
                    self.rdp += usize::from(bpp);
                    self.state = State::Single;
                }
                ret
            }
        }
    }

    /// Decodes `out.len()` values (bpp domain).
    pub(crate) fn decode_row(&mut self, out: &mut [u8]) {
        for o in out {
            *o = self.next_value();
        }
    }
}

/// Decodes a compressed glyph of `w × h` values, calling `sink(y, a8_row)` per row.
///
/// `cur` and `prev` are scratch rows of at least `w` bytes. Returns `false` when the data is
/// corrupt (rows are still delivered, with zeros for missing data) or the scratch is too small.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_compressed(
    data: &[u8],
    bpp: u8,
    w: usize,
    h: usize,
    prefilter: bool,
    cur: &mut [u8],
    prev: &mut [u8],
    sink: &mut dyn FnMut(usize, &[u8]),
) -> bool {
    if cur.len() < w || prev.len() < w {
        return false;
    }
    let cur = &mut cur[..w];
    let prev = &mut prev[..w];
    let mut rle = Rle::new(data, bpp);
    for y in 0..h {
        rle.decode_row(cur);
        if prefilter {
            if y > 0 {
                for (c, p) in cur.iter_mut().zip(prev.iter()) {
                    *c ^= *p;
                }
            }
            prev.copy_from_slice(cur);
        }
        for c in cur.iter_mut() {
            *c = to_a8(*c, bpp);
        }
        sink(y, cur);
    }
    !rle.corrupt
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;

    fn decode_all(data: &[u8], bpp: u8, n: usize) -> (Vec<u8>, bool) {
        let mut r = Rle::new(data, bpp);
        let mut out = vec![0; n];
        r.decode_row(&mut out);
        (out, r.corrupt)
    }

    #[test]
    fn expand_row_all_bpp_known_values() {
        let mut out = [0u8; 8];
        assert!(expand_row(&[0b1010_0001], 0, 1, 8, &mut out));
        assert_eq!(out, [255, 0, 255, 0, 0, 0, 0, 255]);
        let mut out = [0u8; 4];
        assert!(expand_row(&[0b00_01_10_11], 0, 2, 4, &mut out));
        assert_eq!(out, [0, 85, 170, 255]);
        let mut out = [0u8; 2];
        assert!(expand_row(&[0x3F], 0, 4, 2, &mut out));
        assert_eq!(out, [51, 255]);
        let mut out = [0u8; 2];
        assert!(expand_row(&[7, 200], 0, 8, 2, &mut out));
        assert_eq!(out, [7, 200]);
    }

    #[test]
    fn plain_rows_are_bit_packed_without_padding() {
        // A 3×2 glyph at 1 bpp: rows `101` and `011` → bits 101011 then padding.
        let data = [0b1010_1100];
        let mut row = [0u8; 3];
        assert!(expand_row(&data, 0, 1, 3, &mut row));
        assert_eq!(row, [255, 0, 255]);
        assert!(expand_row(&data, 3, 1, 3, &mut row)); // row 1 starts at bit 1*3*1
        assert_eq!(row, [0, 255, 255]);
        // 4 bpp, 3 wide: row 1 starts in the middle of byte 1.
        let data = [0x12, 0x34, 0x56];
        assert!(expand_row(&data, 12, 4, 3, &mut row));
        assert_eq!(row, [4 * 17, 5 * 17, 6 * 17]);
    }

    #[test]
    fn expand_row_short_data_is_reported() {
        let mut out = [9u8; 4];
        assert!(!expand_row(&[0xFF], 4, 4, 4, &mut out));
        assert_eq!(out, [255, 0, 0, 0]);
        assert!(!expand_row(&[1], 0, 8, 4, &mut out));
        assert_eq!(out, [1, 0, 0, 0]);
    }

    #[test]
    fn rle_decodes_hand_encoded_vector() {
        // bpp 4, bits 0101 0101 1001 1100:
        //   Single  @0 : literal 0101 = 5 (first value: no repeat check), rdp 4
        //   Single  @4 : literal 0101 = 5 == prev → Repeated (cnt 0), rdp 8
        //   Repeated@8 : bit 1 → 5 (cnt 1), rdp 9
        //   Repeated@9 : bit 0 → literal @10 = 0111 = 7, Single, rdp 14
        let (v, corrupt) = decode_all(&[0x55, 0x9C], 4, 4);
        assert_eq!(v, [5, 5, 5, 7]);
        assert!(!corrupt);
    }

    #[test]
    fn rle_counter_path_decodes_long_runs() {
        // bpp 2: 19 values of 3, then 1.
        //   11 11     two literals; the second equals the first → Repeated      (2 values)
        //   1 × 11    repeat bits; the 11th is followed by a 6-bit count          (11 values)
        //   000111    N = 7 → Counter: 6 more repeats, the 7th value is a literal (6 values)
        //   01        that literal: 1
        let bits = "1111".to_string() + &"1".repeat(11) + "000111" + "01";
        let mut bytes = Vec::new();
        let mut padded = bits.clone();
        while padded.len() % 8 != 0 {
            padded.push('0');
        }
        for c in padded.as_bytes().chunks(8) {
            bytes.push(u8::from_str_radix(core::str::from_utf8(c).unwrap(), 2).unwrap());
        }
        let (v, corrupt) = decode_all(&bytes, 2, 20);
        assert!(!corrupt);
        assert_eq!(&v[..19], &[3u8; 19][..]);
        assert_eq!(v[19], 1);
    }

    #[test]
    fn rle_zero_count_reads_literal() {
        // bpp 4: literals 15, 15 (→ Repeated), 10 repeat bits (15), the 11th repeat bit
        // reads count 0, which makes that value the next literal (2): 12 × 15, then 2.
        let bits = "11111111".to_string() + &"1".repeat(11) + "000000" + "0010";
        let mut padded = bits;
        while padded.len() % 8 != 0 {
            padded.push('0');
        }
        let bytes: Vec<u8> = padded
            .as_bytes()
            .chunks(8)
            .map(|c| u8::from_str_radix(core::str::from_utf8(c).unwrap(), 2).unwrap())
            .collect();
        let (v, corrupt) = decode_all(&bytes, 4, 13);
        assert!(!corrupt);
        assert_eq!(&v[..12], &[15u8; 12][..]);
        assert_eq!(v[12], 2);
    }

    #[test]
    fn prefilter_xor_reconstructs_rows() {
        // 2×2 at 8 bpp: stored rows [1, 2] and the XOR difference [3, 3] → [2, 1]. The last
        // literal equals the previous one (→ Repeated) but no further value is read.
        let data = [1, 2, 3, 3];
        let (mut a, mut b) = ([0u8; 2], [0u8; 2]);
        let mut rows = Vec::new();
        let ok = decode_compressed(&data, 8, 2, 2, true, &mut a, &mut b, &mut |_, r| {
            rows.push(r.to_vec())
        });
        assert!(ok);
        assert_eq!(rows, vec![vec![1, 2], vec![2, 1]]);
        rows.clear();
        decode_compressed(&data, 8, 2, 2, false, &mut a, &mut b, &mut |_, r| {
            rows.push(r.to_vec())
        });
        assert_eq!(rows, vec![vec![1, 2], vec![3, 3]]);
    }

    #[test]
    fn corrupt_stream_is_flagged() {
        let (_, corrupt) = decode_all(&[0x12], 4, 5);
        assert!(corrupt);
        let (mut a, mut b) = ([0u8; 4], [0u8; 4]);
        assert!(!decode_compressed(
            &[0xFF],
            4,
            4,
            4,
            true,
            &mut a,
            &mut b,
            &mut |_, _| {}
        ));
        assert!(
            !decode_compressed(&[0xFF], 4, 8, 1, true, &mut a, &mut b, &mut |_, _| {}),
            "scratch too small"
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1000))]
        #[test]
        fn random_bytes_never_panic(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..64),
                                    bpp in proptest::sample::select(vec![1u8, 2, 4, 8]),
                                    w in 0usize..40, h in 0usize..40, pre in proptest::prelude::any::<bool>()) {
            let (mut a, mut b) = ([0u8; 64], [0u8; 64]);
            let _ = decode_compressed(&data, bpp, w, h, pre, &mut a, &mut b, &mut |_, r| assert_eq!(r.len(), w));
            let mut out = [0u8; 64];
            let _ = expand_row(&data, 3 * usize::from(bpp), bpp, w, &mut out);
        }
    }
}
