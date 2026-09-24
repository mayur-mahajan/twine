//! Fuzzes the QoiDecoder decoder with arbitrary bytes: it must return, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_image::Decoder;
use twine_image::decoders::qoi::QoiDecoder;

fuzz_target!(|data: &[u8]| {
    let _ = QoiDecoder.probe(data);
    let mut out = Vec::new();
    let _ = QoiDecoder.decode(data, &mut out);
});
