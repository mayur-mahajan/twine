//! Headless engine apps: rendering, raw keys, buffer modes, software rotation and hotkeys.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use twine_core::{Color, Opa, Rotation};
use twine_engine::{Engine, NodeId, Obj};
use twine_hal::{BufferSpec, Key};
use twine_sim::{Headless, SimApp, SimConfig, run_engine_headless};
use twine_style::{Part, PropId, Selector, StyleProp};

fn out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("twine-sim-engine-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn scene(e: &mut Engine) -> NodeId {
    let screen = e.active_screen(e.default_display().unwrap()).unwrap();
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgColor(Color::WHITE.into()));
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    let b = e.create(screen, Box::new(Obj)).unwrap();
    e.set_pos(b, 4, 2);
    e.set_size(b, 10, 6);
    e.set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
    e.set_local_prop(b, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    b
}

fn headless(cfg: SimConfig, tag: &str, script: Option<&str>) -> (SimConfig, PathBuf) {
    let dir = out_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let script = script.map(|s| {
        let p = dir.join("s.twinescript");
        std::fs::write(&p, s).unwrap();
        p
    });
    let cfg = cfg.headless(Some(Headless {
        frames: 3,
        script,
        out_dir: dir.clone(),
    }));
    (cfg, dir)
}

fn png_pixel(path: &std::path::Path, x: u32, y: u32) -> [u8; 3] {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    let i = ((y * info.width + x) * 3) as usize;
    [buf[i], buf[i + 1], buf[i + 2]]
}

#[test]
fn engine_app_renders_headless() {
    for spec in [
        BufferSpec::PartialSingle { rows: 4 },
        BufferSpec::PartialDouble { rows: 8 },
        BufferSpec::Full,
        BufferSpec::Direct,
    ] {
        let (cfg, dir) = headless(
            SimConfig::new(32, 16).buffers(spec),
            &format!("{spec:?}").replace(' ', ""),
            None,
        );
        let report = run_engine_headless(cfg, |e| {
            scene(e);
        })
        .unwrap();
        assert_eq!(report.frames, 3);
        let f = dir.join("final.png");
        assert_eq!(png_pixel(&f, 5, 3), [255, 0, 0], "{spec:?}");
        assert_eq!(png_pixel(&f, 20, 10), [255, 255, 255], "{spec:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn raw_keys_reach_the_engine() {
    let presses = Rc::new(Cell::new(0));
    let seen = presses.clone();
    let node = Rc::new(Cell::new(None));
    let n2 = node.clone();
    let (cfg, dir) = headless(
        SimConfig::new(32, 16).on_raw_key(move |e, k| {
            if k == Key::Right {
                seen.set(seen.get() + 1);
                let b = n2.get().unwrap();
                // The style position (the coordinates are only set by the next layout).
                let x = e.style_i32(b, Part::Main, PropId::X);
                e.set_x(b, x + 4);
            }
        }),
        "keys",
        Some("key Right\nwait 20\nkey Right\nwait 20\nshot moved\n"),
    );
    let app = SimApp::engine(cfg, |e| node.set(Some(scene(e)))).unwrap();
    let report = app.run_headless().unwrap();
    assert!(report.frames > 1);
    assert_eq!(presses.get(), 2);
    // Moved by 8 px: x = 12..22.
    assert_eq!(png_pixel(&dir.join("moved.png"), 20, 3), [255, 0, 0]);
    assert_eq!(png_pixel(&dir.join("moved.png"), 5, 3), [255, 255, 255]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn software_rotation_shows_the_physical_panel() {
    let (cfg, dir) = headless(
        SimConfig::new(32, 16)
            .rotation(Rotation::Deg90)
            .hw_rotation(false),
        "rot",
        None,
    );
    run_engine_headless(cfg, |e| {
        scene(e);
    })
    .unwrap();
    let dec = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(dir.join("final.png")).unwrap(),
    ));
    let r = dec.read_info().unwrap();
    assert_eq!((r.info().width, r.info().height), (16, 32));
    // Logical (5, 3) → physical (3, 31 - 5).
    assert_eq!(png_pixel(&dir.join("final.png"), 3, 26), [255, 0, 0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn engine_hotkeys_toggle_overlays() {
    let (cfg, dir) = headless(
        SimConfig::new(200, 100),
        "hotkeys",
        Some("wait 16\nhotkey F2\nhotkey F2\nhotkey F3\nhotkey F4\nhotkey F8\nwait 1100\nshot overlays\n"),
    );
    let app = SimApp::engine(cfg, |e| {
        e.config_mut().default_font = Some(&twine_assets::fonts::MONTSERRAT_14);
        scene(e);
    })
    .unwrap();
    app.run_headless().unwrap();
    let shot = dir.join("overlays.png");
    // The bounds overlay outlines the box in magenta (F2 was toggled on and off again).
    assert_eq!(png_pixel(&shot, 4, 4), [255, 0, 255]);
    // The performance overlay darkens the bottom-right corner.
    let p = png_pixel(&shot, 190, 94);
    assert!(p[0] < 128 && p[1] < 128 && p[2] < 128, "{p:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// R2.S03: F12 switches the installed theme to its next mode (`Engine::set_theme_mode`),
/// cycling through the modes the theme supports.
#[test]
fn f12_cycles_the_theme_modes() {
    use twine_theme::{DefaultTheme, SimpleTheme};
    let script = "wait 16\nshot m0\nhotkey F12\nwait 32\nshot m1\nhotkey F12\nwait 32\nshot m2\n\
                  hotkey F12\nwait 32\nshot m3\nhotkey F12\nwait 32\nshot m4\n";
    let screen_bg = |dir: &PathBuf, shot: &str| png_pixel(&dir.join(format!("{shot}.png")), 0, 0);
    // The panel is RGB565: compare with the quantized color.
    let q = |hex: u32| {
        let c = Color::from_rgb565(Color::hex(hex).to_rgb565());
        [c.r, c.g, c.b]
    };
    // The default theme: light → dark → night → high contrast → light (screen backgrounds).
    let (cfg, dir) = headless(
        SimConfig::new(32, 16).theme(Rc::new(DefaultTheme::light())),
        "modes",
        Some(script),
    );
    SimApp::engine(cfg, |_| {}).unwrap().run_headless().unwrap();
    assert_eq!(screen_bg(&dir, "m0"), q(0x00F5_F5F5));
    assert_eq!(screen_bg(&dir, "m1"), q(0x0015_171A));
    assert_eq!(screen_bg(&dir, "m2"), q(0x0000_0000));
    assert_eq!(screen_bg(&dir, "m3"), q(0x0000_0000));
    assert_eq!(screen_bg(&dir, "m4"), q(0x00F5_F5F5));
    let _ = std::fs::remove_dir_all(dir);
    // The simple theme: light ↔ high contrast (white screen) only.
    let (cfg, dir) = headless(
        SimConfig::new(32, 16).theme(Rc::new(SimpleTheme::new())),
        "modes-simple",
        Some(script),
    );
    SimApp::engine(cfg, |_| {}).unwrap().run_headless().unwrap();
    assert_eq!(screen_bg(&dir, "m0"), q(0x00F5_F5F5));
    assert_eq!(screen_bg(&dir, "m1"), q(0x00FF_FFFF));
    assert_eq!(screen_bg(&dir, "m2"), q(0x00F5_F5F5));
    let _ = std::fs::remove_dir_all(dir);
}
