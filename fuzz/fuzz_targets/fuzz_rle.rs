//! Fuzzes LVGL RLE decompression: the first byte picks the block size (1–4) and output size.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&k, input)) = data.split_first() else { return };
    let blk = usize::from(k % 4) + 1;
    let mut out = vec![0u8; usize::from(k) * 16];
    let _ = twine_image::rle_decompress(input, &mut out, blk);
});
