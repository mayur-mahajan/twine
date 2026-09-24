//! Fuzzes the BmpDecoder decoder with arbitrary bytes: it must return, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_image::Decoder;
use twine_image::decoders::bmp::BmpDecoder;

fuzz_target!(|data: &[u8]| {
    let _ = BmpDecoder.probe(data);
    let mut out = Vec::new();
    let _ = BmpDecoder.decode(data, &mut out);
});
