//! Fuzzes the GIF decoder and the animation player with arbitrary bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_image::Decoder;
use twine_image::decoders::gif::{GifDecoder, GifPlayer};

fuzz_target!(|data: &[u8]| {
    let mut out = Vec::new();
    let _ = GifDecoder.decode(data, &mut out);
    if let Ok(mut p) = GifPlayer::new(data) {
        for _ in 0..(p.frame_count() * 2).min(64) {
            let _ = p.advance();
        }
    }
});
