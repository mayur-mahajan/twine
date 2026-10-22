//! Software display rotation: [`rotate_buffer`] and [`rotate_area`].
//!
//! Panels without hardware rotation get each chunk rendered in logical orientation, rotated
//! into a scratch buffer with [`rotate_buffer`] and flushed to [`rotate_area`] of the panel.

use twine_core::{Rect, Rotation, Size};

use crate::RenderError;

/// Tile edge for cache-friendly rotation.
const TILE: usize = 8;

/// Rotates a `w × h` pixel block (`bytes_per_px` ∈ 1..=4) from `src` into `dst`. For 90° and
/// 270° the destination block is `h × w`. The pixel mapping is the one of
/// [`Rect::rotate_in`] (and [`rotate_area`]): 90° maps `(x, y)` to `(y, w − 1 − x)`.
///
/// # Errors
/// [`RenderError::UnsupportedFormat`] for `bytes_per_px` outside `1..=4`;
/// [`RenderError::InvalidArea`] for a stride shorter than a row; [`RenderError::BufferTooSmall`]
/// when `src` or `dst` cannot hold the block. An empty block is `Ok` and copies nothing.
///
/// ```
/// use twine_core::Rotation;
/// use twine_render::rotate_buffer;
///
/// // 3 × 2 block:  1 2 3      rotated 90°:  3 6
/// //               4 5 6                    2 5
/// //                                        1 4
/// let src = [1u8, 2, 3, 4, 5, 6];
/// let mut dst = [0u8; 6];
/// rotate_buffer(&src, 3, &mut dst, 2, 3, 2, Rotation::Deg90, 1).unwrap();
/// assert_eq!(dst, [3, 6, 2, 5, 1, 4]);
/// ```
#[allow(clippy::too_many_arguments)]
pub fn rotate_buffer(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    w: usize,
    h: usize,
    rotation: Rotation,
    bytes_per_px: usize,
) -> Result<(), RenderError> {
    let bpp = bytes_per_px;
    if !(1..=4).contains(&bpp) {
        return Err(RenderError::UnsupportedFormat(twine_core::ColorFormat::I1));
    }
    if w == 0 || h == 0 {
        return Ok(());
    }
    let (dw, dh) = if rotation.swaps_axes() { (h, w) } else { (w, h) };
    // Saturating sizes: an impossible size reads as "too small" below.
    let (row, drow) = (w.saturating_mul(bpp), dw.saturating_mul(bpp));
    if src_stride < row || dst_stride < drow {
        return Err(RenderError::InvalidArea);
    }
    // `w, h ≥ 1` (returned above).
    let need_src = src_stride.saturating_mul(h.saturating_sub(1)).saturating_add(row);
    if src.len() < need_src {
        return Err(RenderError::BufferTooSmall {
            needed: need_src,
            got: src.len(),
        });
    }
    let need_dst = dst_stride
        .saturating_mul(dh.saturating_sub(1))
        .saturating_add(drow);
    if dst.len() < need_dst {
        return Err(RenderError::BufferTooSmall {
            needed: need_dst,
            got: dst.len(),
        });
    }
    // From here every offset below is at most `need_src` / `need_dst`, which fit the buffers
    // (checked above), so the arithmetic is exact.
    if rotation == Rotation::Deg0 {
        for (d, s) in dst.chunks_mut(dst_stride).zip(src.chunks(src_stride)).take(h) {
            // SAFE-INDEX: every row holds `row` bytes (the size checks); never falls back.
            if let (Some(d), Some(s)) = (d.get_mut(..row), s.get(..row)) {
                d.copy_from_slice(s);
            }
        }
        return Ok(());
    }
    rotate_tiles(src, src_stride, dst, dst_stride, w, h, rotation, bpp);
    Ok(())
}

/// Copies the `B`-byte pixel at `s` of `srow` to `d` of `dst`: a fixed-size copy (a load and
/// a store), not a variable-length one. Out of range it copies nothing (never happens: the
/// caller checked the sizes).
#[inline(always)]
fn copy_px<const B: usize>(dst: &mut [u8], d: usize, srow: &[u8], s: usize) {
    let dp = dst.get_mut(d..).and_then(<[u8]>::first_chunk_mut::<B>);
    let sp = srow.get(s..).and_then(<[u8]>::first_chunk::<B>);
    if let (Some(dp), Some(sp)) = (dp, sp) {
        *dp = *sp;
    }
}

/// The 90°/180°/270° walk of [`rotate_buffer`] for `bpp`-byte pixels (`1..=4`), in
/// `TILE × TILE` tiles (cache-friendly). The caller checked the strides and buffer sizes for
/// the `w × h` block (`w, h ≥ 1`). One copy of the walk serves every pixel size (flash); each
/// pixel is a fixed-size copy chosen by a branch that is the same for the whole call.
#[allow(clippy::too_many_arguments)]
fn rotate_tiles(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    w: usize,
    h: usize,
    rotation: Rotation,
    bpp: usize,
) {
    // Destination coordinates of source pixel (x, y).
    // SAFE-ARITH: `x < w` and `y < h`, so the mirrored coordinates are in range.
    let map = |x: usize, y: usize| -> (usize, usize) {
        let (mx, my) = (
            w.wrapping_sub(1).wrapping_sub(x),
            h.wrapping_sub(1).wrapping_sub(y),
        );
        match rotation {
            Rotation::Deg90 => (y, mx),
            Rotation::Deg180 => (mx, my),
            _ => (my, x),
        }
    };
    for ty in (0..h).step_by(TILE) {
        for tx in (0..w).step_by(TILE) {
            // SAFE-ARITH: `ty < h` and `tx < w`, so `+ TILE` cannot overflow.
            for y in ty..ty.wrapping_add(TILE).min(h) {
                // SAFE-ARITH: `y < h`: a row start inside the checked source size.
                let srow = src.get(y.wrapping_mul(src_stride)..).unwrap_or_default();
                for x in tx..tx.wrapping_add(TILE).min(w) {
                    let (dx, dy) = map(x, y);
                    // SAFE-ARITH: `dx < dw`, `dy < dh`, `x < w`: offsets inside the checked
                    // sizes.
                    let d = dy.wrapping_mul(dst_stride).wrapping_add(dx.wrapping_mul(bpp));
                    let sx = x.wrapping_mul(bpp);
                    // SAFE-INDEX: inside the checked sizes; nothing is ever skipped.
                    match bpp {
                        1 => copy_px::<1>(dst, d, srow, sx),
                        2 => copy_px::<2>(dst, d, srow, sx),
                        3 => copy_px::<3>(dst, d, srow, sx),
                        _ => copy_px::<4>(dst, d, srow, sx),
                    }
                }
            }
        }
    }
}

/// Where a logical `area` lands on a panel whose logical size is `panel` (the size before
/// rotation, i.e. as the application sees it) — the same mapping as [`Rect::rotate_in`].
///
/// ```
/// use twine_core::{Rect, Rotation, Size};
/// use twine_render::rotate_area;
/// let a = rotate_area(Rect::from_xywh(0, 0, 320, 40), Size::new(320, 240), Rotation::Deg90);
/// assert_eq!(a, Rect::new(0, 0, 40, 320));
/// ```
#[must_use]
pub fn rotate_area(area: Rect, panel: Size, rotation: Rotation) -> Rect {
    area.rotate_in(rotation, panel.w, panel.h)
}

/// Converts a `w × h` block of 8-bit luminance (`src`, stride `w`) into packed 1-bit pixels
/// (`dst`, MSB first, stride `⌈w / 8⌉`): a pixel is 1 (white) when its luminance is ≥ 128, the
/// same threshold the renderer uses when blending into `I1` buffers. Padding bits of the last
/// byte of a row are 0.
///
/// Mono panels get their chunks rendered in `L8` (so anti-aliasing and blending see real
/// gray levels) and converted at the end of the chunk.
///
/// # Errors
/// [`RenderError::BufferTooSmall`] when `src` or `dst` cannot hold the block.
///
/// ```
/// use twine_render::convert_l8_to_i1;
/// let src = [255, 0, 200, 127, 128, 0, 0, 0, 9, 255];
/// let mut dst = [0u8; 2];
/// convert_l8_to_i1(&src, &mut dst, 10, 1).unwrap();
/// assert_eq!(dst, [0b1010_1000, 0b0100_0000]);
/// ```
pub fn convert_l8_to_i1(src: &[u8], dst: &mut [u8], w: usize, h: usize) -> Result<(), RenderError> {
    if w == 0 || h == 0 {
        return Ok(());
    }
    let dst_stride = w.div_ceil(8);
    let (need_src, need_dst) = (w.saturating_mul(h), dst_stride.saturating_mul(h));
    if src.len() < need_src {
        return Err(RenderError::BufferTooSmall {
            needed: need_src,
            got: src.len(),
        });
    }
    if dst.len() < need_dst {
        return Err(RenderError::BufferTooSmall {
            needed: need_dst,
            got: dst.len(),
        });
    }
    for (srow, drow) in src.chunks_exact(w).zip(dst.chunks_exact_mut(dst_stride)).take(h) {
        for (bits, byte) in srow.chunks(8).zip(drow.iter_mut()) {
            let mut b = 0u8;
            for (i, &l) in bits.iter().enumerate() {
                if l >= 128 {
                    // SHIFT: `i < 8` (`bits` is a chunk of at most 8).
                    b |= 0x80 >> i;
                }
            }
            *byte = b;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l8_to_i1_rows_and_errors() {
        let src = [0u8, 255, 255, 0, 128, 127, 255, 255, 1];
        let mut dst = [0xFFu8; 3];
        convert_l8_to_i1(&src, &mut dst, 3, 3).unwrap();
        assert_eq!(dst, [0b0110_0000, 0b0100_0000, 0b1100_0000]);
        assert!(convert_l8_to_i1(&src, &mut dst, 3, 4).is_err());
        assert!(convert_l8_to_i1(&src, &mut dst[..2], 3, 3).is_err());
        assert!(convert_l8_to_i1(&[], &mut [], 0, 5).is_ok());
    }

    #[test]
    fn rotate_90_small_known() {
        // 3 × 2: rows [1 2 3] [4 5 6]
        let expect: [(Rotation, [u8; 6]); 4] = [
            (Rotation::Deg0, [1, 2, 3, 4, 5, 6]),
            (Rotation::Deg90, [3, 6, 2, 5, 1, 4]),
            (Rotation::Deg180, [6, 5, 4, 3, 2, 1]),
            (Rotation::Deg270, [4, 1, 5, 2, 6, 3]),
        ];
        for bpp in 1..=4usize {
            let src: alloc::vec::Vec<u8> = (1..=6u8).flat_map(|v| core::iter::repeat_n(v, bpp)).collect();
            for (rot, e) in expect {
                let mut dst = alloc::vec![0u8; 6 * bpp];
                let ds = if rot.swaps_axes() { 2 } else { 3 };
                rotate_buffer(&src, 3 * bpp, &mut dst, ds * bpp, 3, 2, rot, bpp).unwrap();
                let got: alloc::vec::Vec<u8> = dst.chunks(bpp).map(|c| c[0]).collect();
                assert_eq!(got, e, "{rot:?} bpp {bpp}");
                assert!(dst.chunks(bpp).all(|c| c.iter().all(|&b| b == c[0])));
            }
        }
    }

    #[test]
    fn rotate_area_mapping() {
        let p = Size::new(320, 240);
        let a = Rect::from_xywh(10, 20, 30, 40);
        assert_eq!(rotate_area(a, p, Rotation::Deg0), a);
        assert_eq!(rotate_area(a, p, Rotation::Deg90), Rect::new(20, 280, 60, 310));
        assert_eq!(rotate_area(a, p, Rotation::Deg180), Rect::new(280, 180, 310, 220));
        assert_eq!(rotate_area(a, p, Rotation::Deg270), Rect::new(180, 10, 220, 40));
    }

    #[test]
    fn dst_too_small_errors() {
        let src = [0u8; 12];
        let mut dst = [0u8; 5];
        assert_eq!(
            rotate_buffer(&src, 3, &mut dst, 2, 3, 2, Rotation::Deg90, 1),
            Err(RenderError::BufferTooSmall { needed: 6, got: 5 })
        );
        let mut dst = [0u8; 12];
        assert_eq!(
            rotate_buffer(&src, 3, &mut dst, 1, 3, 2, Rotation::Deg90, 1),
            Err(RenderError::InvalidArea)
        );
        assert!(matches!(
            rotate_buffer(&src, 3, &mut dst, 3, 3, 2, Rotation::Deg0, 5),
            Err(RenderError::UnsupportedFormat(_))
        ));
    }
}
