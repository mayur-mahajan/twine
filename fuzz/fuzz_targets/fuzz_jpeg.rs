//! Fuzzes the JpegDecoder decoder with arbitrary bytes: it must return, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_image::Decoder;
use twine_image::decoders::jpeg::JpegDecoder;

fuzz_target!(|data: &[u8]| {
    let _ = JpegDecoder.probe(data);
    let mut out = Vec::new();
    let _ = JpegDecoder.decode(data, &mut out);
});
