//! Software rotation glue: rotates a rendered chunk into the flush buffer.

use twine_core::Rotation;

/// Rotates a `w × h` chunk of `bpp`-byte pixels (rows packed) from `src` into `dst` (rows
/// packed in the rotated layout). Errors (impossible with the sizes the refresher allocates)
/// are logged.
pub(crate) fn rotate_chunk(src: &[u8], dst: &mut [u8], w: usize, h: usize, bpp: usize, rotation: Rotation) {
    let dst_w = if rotation.swaps_axes() { h } else { w };
    if let Err(e) = twine_render::rotate_buffer(src, w * bpp, dst, dst_w * bpp, w, h, rotation, bpp) {
        twine_core::error!(target: "twine::refresh", "software rotation failed: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_packed_chunk() {
        // 3 × 2 L8 → 2 × 3 after 90°.
        let src = [1u8, 2, 3, 4, 5, 6];
        let mut dst = [0u8; 6];
        rotate_chunk(&src, &mut dst, 3, 2, 1, Rotation::Deg90);
        assert_eq!(dst, [3, 6, 2, 5, 1, 4]);
    }
}
