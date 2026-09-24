//! Fuzzes LZ4 block decompression: the first byte picks the output size.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&k, input)) = data.split_first() else { return };
    let mut out = vec![0u8; usize::from(k) * 32];
    let _ = twine_image::lz4_decompress(input, &mut out);
});
