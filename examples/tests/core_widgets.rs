//! The `core_widgets` showcase run headless through the simulator with its smoke script; the
//! shot after tapping the checkable button is compared with the snapshot.

use std::path::PathBuf;
use std::rc::Rc;

use twine_examples::core_widgets::{H, W, build, engine_config};
use twine_sim::{Headless, SimConfig, run_engine_headless};
use twine_testing::png_io::read_rgb_png;
use twine_testing::{Tolerance, assert_rgb_snapshot, snapshot_config};
use twine_theme::DefaultTheme;

#[test]
fn core_widgets_headless_matches_snapshot() {
    let out = std::env::temp_dir().join(format!("twine-core-widgets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/core_widgets.twinescript");
    let cfg = SimConfig::new(W, H)
        .engine_config(engine_config())
        .theme_toggle(Rc::new(DefaultTheme::light()), Rc::new(DefaultTheme::dark()))
        .headless(Some(Headless {
            frames: 1,
            script: Some(script),
            out_dir: out.clone(),
        }));
    let report = run_engine_headless(cfg, |e| {
        build(e);
    })
    .expect("headless run");
    assert!(report.frames > 30);
    let img = read_rgb_png(&out.join("core_widgets_after_tap.png")).expect("the script's shot");
    assert_rgb_snapshot(
        &snapshot_config!(),
        "core_widgets_after_tap",
        img.width,
        img.height,
        &img.data,
        Tolerance::default(),
    );
    let _ = std::fs::remove_dir_all(out);
}
