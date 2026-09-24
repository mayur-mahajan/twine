//! Fuzzes the PngDecoder decoder with arbitrary bytes: it must return, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_image::Decoder;
use twine_image::decoders::png::PngDecoder;

fuzz_target!(|data: &[u8]| {
    let _ = PngDecoder.probe(data);
    let mut out = Vec::new();
    let _ = PngDecoder.decode(data, &mut out);
});
