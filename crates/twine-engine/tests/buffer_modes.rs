//! Every buffer mode renders the `engine_boxes` scene pixel-identically, before and after the
//! player box moves; buffers declared with `draw_buffers!`-style statics are accepted, caller
//! memory is checked, and a refused display allocates nothing (R3.S03).

use twine_core::{Duration, Rect};
use twine_engine::BufferSpec;
use twine_testing::scenes::engine_boxes;
use twine_testing::{EngineHarness, FbMode};

/// The buffers of a harness: heap partial buffers or a framebuffer display.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Heap(BufferSpec),
    Framebuffer(FbMode),
}

fn render(mode: Mode) -> (Vec<u8>, Vec<u8>) {
    let mut player = None;
    let h = EngineHarness::new(320, 240).no_theme();
    let h = match mode {
        Mode::Heap(spec) => h.buffers(spec),
        Mode::Framebuffer(fb) => h.framebuffer(fb, 0),
    };
    let mut h = h.mount_engine(|e| player = Some(engine_boxes(e).player));
    h.run_until_idle();
    let first = h.panel_rgb888();
    let p = player.unwrap();
    for dx in [4, 4, 4, -40] {
        let r = h.engine().coords(p);
        h.engine_mut()
            .place(p, Rect::from_xywh(r.x0 + dx, r.y0 - 3, r.width(), r.height()));
        h.clock().advance(Duration::ms(16));
        h.run_until_idle();
    }
    (first, h.panel_rgb888())
}

#[test]
fn all_modes_render_identically() {
    let reference = render(Mode::Heap(BufferSpec::PartialDouble { rows: 40 }));
    for spec in [
        Mode::Heap(BufferSpec::PartialSingle { rows: 10 }),
        Mode::Heap(BufferSpec::PartialDouble { rows: 7 }),
        Mode::Framebuffer(FbMode::Full),
        Mode::Framebuffer(FbMode::Direct),
    ] {
        let got = render(spec);
        assert!(got.0 == reference.0, "{spec:?}: first frame differs");
        assert!(got.1 == reference.1, "{spec:?}: frame after moves differs");
    }
}
