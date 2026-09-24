//! Every buffer mode renders the `engine_boxes` scene pixel-identically, before and after the
//! player box moves.

use twine_core::{Duration, Rect};
use twine_hal::BufferSpec;
use twine_testing::EngineHarness;
use twine_testing::scenes::engine_boxes;

fn render(spec: BufferSpec) -> (Vec<u8>, Vec<u8>) {
    let mut player = None;
    let mut h = EngineHarness::new(320, 240)
        .no_theme()
        .buffers(spec)
        .mount_engine(|e| player = Some(engine_boxes(e).player));
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
    let reference = render(BufferSpec::PartialDouble { rows: 40 });
    for spec in [
        BufferSpec::PartialSingle { rows: 10 },
        BufferSpec::PartialDouble { rows: 7 },
        BufferSpec::Full,
        BufferSpec::Direct,
    ] {
        let got = render(spec);
        assert!(got.0 == reference.0, "{spec:?}: first frame differs");
        assert!(got.1 == reference.1, "{spec:?}: frame after moves differs");
    }
}
