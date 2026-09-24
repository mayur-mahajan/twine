//! Fuzzes the Lottie loader with arbitrary bytes: it must return, never panic; compositions
//! that load must also render a few frames into a small buffer without panicking.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_core::{ColorFormat, Rect};
use twine_lottie::{LottiePlayer, load};
use twine_render::{DrawBuf, Painter, RenderCaches};

fuzz_target!(|data: &[u8]| {
    if let Ok(comp) = load(data) {
        let frames = [comp.ip, (comp.ip + comp.op) / 2.0, comp.op - 1.0];
        let mut player = LottiePlayer::new(comp);
        let mut caches = RenderCaches::default();
        let mut px = vec![0u8; 32 * 32 * 2];
        let area = Rect::from_xywh(0, 0, 32, 32);
        for f in frames {
            if let Ok(buf) = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, area) {
                let mut p = Painter::new(buf, &mut caches);
                player.render_frame(f, &mut p, area);
            }
        }
    }
});
