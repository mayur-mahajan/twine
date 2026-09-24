//! Page conversion for monochrome OLED controllers (SSD1306, SH1106).
//!
//! The engine renders `I1`: row-major, 1 bit per pixel, **most significant bit first**, each
//! row starting on a byte (`stride = ⌈w / 8⌉`). Page-based controllers store 8 vertically
//! stacked pixels per byte instead: byte `c` of page `p` holds the pixels `(c, 8p + k)` in bit
//! `k` (LSB = top). [`i1_to_page`] converts one page of an `I1` chunk into that layout;
//! [`page_to_i1`] is its inverse (tests, simulators).
//!
//! ```
//! use twine_drivers::mono::i1_to_page;
//!
//! // A 16 × 16 chunk with only pixel (3, 10) set: page 1, column 3, bit 2.
//! let mut chunk = [0u8; 2 * 16];
//! chunk[10 * 2] = 0b0001_0000;
//! let mut page = [0u8; 16];
//! i1_to_page(&chunk, 2, 16, 1, &mut page);
//! assert_eq!(page[3], 0b0000_0100);
//! assert_eq!(page.iter().filter(|b| **b != 0).count(), 1);
//! ```

/// Converts rows `8·page … 8·page + 7` of an `I1` chunk (`stride` bytes per row, `width`
/// pixels) into page layout in `out[..width]`.
///
/// Rows beyond the chunk read as 0. Only `out[..width.min(out.len())]` is written.
pub fn i1_to_page(src: &[u8], stride: usize, width: usize, page: usize, out: &mut [u8]) {
    let width = width.min(out.len());
    out[..width].fill(0);
    for k in 0..8 {
        let y = page * 8 + k;
        let Some(row) = src.get(y * stride..(y + 1) * stride) else {
            break;
        };
        // Walk whole source bytes: 8 pixels at a time.
        for (bx, &byte) in row.iter().enumerate() {
            if byte == 0 {
                continue;
            }
            for bit in 0..8 {
                let x = bx * 8 + bit;
                if x >= width {
                    break;
                }
                if byte & (0x80 >> bit) != 0 {
                    out[x] |= 1 << k;
                }
            }
        }
    }
}

/// Inverse of [`i1_to_page`]: writes page `page` (`width` bytes of `src`) back into rows
/// `8·page …` of an `I1` buffer with `stride` bytes per row.
pub fn page_to_i1(src: &[u8], stride: usize, width: usize, page: usize, out: &mut [u8]) {
    for k in 0..8 {
        let y = page * 8 + k;
        let Some(row) = out.get_mut(y * stride..(y + 1) * stride) else {
            break;
        };
        for (x, &b) in src.iter().take(width).enumerate() {
            let mask = 0x80u8 >> (x % 8);
            if b & (1 << k) != 0 {
                row[x / 8] |= mask;
            } else {
                row[x / 8] &= !mask;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use proptest::prelude::*;

    #[test]
    fn page_conversion_single_pixel() {
        // 128 × 16 chunk, pixel (3, 10).
        let stride = 16;
        let mut chunk = vec![0u8; stride * 16];
        twine_core::color::I1::set(&mut chunk[10 * stride..11 * stride], 3, true);
        let mut p0 = [0u8; 128];
        let mut p1 = [0u8; 128];
        i1_to_page(&chunk, stride, 128, 0, &mut p0);
        i1_to_page(&chunk, stride, 128, 1, &mut p1);
        assert!(p0.iter().all(|b| *b == 0));
        assert_eq!(p1[3], 1 << 2);
        assert_eq!(p1.iter().map(|b| b.count_ones()).sum::<u32>(), 1);
    }

    proptest! {
        #[test]
        fn page_conversion_full_buffer_roundtrip(
            wbytes in 1usize..=16,
            pages in 1usize..=8,
            seed in proptest::collection::vec(any::<u8>(), 16 * 64),
        ) {
            let stride = wbytes;
            let width = wbytes * 8;
            let h = pages * 8;
            let src: alloc::vec::Vec<u8> = seed[..stride * h].to_vec();
            let mut back = vec![0u8; stride * h];
            let mut page = [0u8; 128];
            for p in 0..pages {
                i1_to_page(&src, stride, width, p, &mut page);
                page_to_i1(&page, stride, width, p, &mut back);
            }
            prop_assert_eq!(back, src);
        }
    }

    #[test]
    fn short_chunk_and_narrow_width() {
        // 10 pixels wide (stride 2), only 3 rows.
        let chunk = [0xFF, 0xC0, 0x00, 0x00, 0x80, 0x40];
        let mut page = [0xAAu8; 12];
        i1_to_page(&chunk, 2, 10, 0, &mut page);
        assert_eq!(&page[..10], &[0b101, 1, 1, 1, 1, 1, 1, 1, 1, 0b101]);
        assert_eq!(&page[10..], &[0xAA, 0xAA]);
    }
}
