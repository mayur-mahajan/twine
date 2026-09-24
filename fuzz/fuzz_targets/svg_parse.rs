//! Fuzzes the SVG parser with arbitrary bytes: it must return, never panic; whatever parses
//! must also render into a small buffer without panicking.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_core::{ColorFormat, Rect};
use twine_render::{DrawBuf, Painter, RenderCaches};
use twine_vector::parse_svg;

fuzz_target!(|data: &[u8]| {
    if let Ok(doc) = parse_svg(data) {
        let mut caches = RenderCaches::default();
        let mut px = vec![0u8; 32 * 32 * 2];
        let area = Rect::from_xywh(0, 0, 32, 32);
        if let Ok(buf) = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, area) {
            let mut p = Painter::new(buf, &mut caches);
            doc.render(&mut p, area);
        }
    }
});
