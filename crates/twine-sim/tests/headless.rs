//! Headless runs of `show_framebuffer` programs (P02.S09).

use std::path::PathBuf;

use twine_core::ColorFormat;
use twine_hal::{InputData, InputDevice, Key, KeypadData};
use twine_sim::{Headless, SimApp, SimConfig, SimError, run_headless_framebuffer};

fn out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("twine-sim-headless-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn renders_frames_and_writes_final_png() {
    let dir = out_dir("frames");
    let cfg = SimConfig::new(32, 16).headless(Some(Headless {
        frames: 60,
        script: None,
        out_dir: dir.clone(),
    }));
    let report = run_headless_framebuffer(cfg, |fb, format, frame| {
        assert_eq!(format, ColorFormat::Rgb565);
        fb.fill(frame as u8);
    })
    .unwrap();
    assert_eq!(report.frames, 60);
    assert_eq!(report.sim_time.as_millis(), 59 * 16);
    assert!(dir.join("final.png").is_file());
    assert_eq!(report.shots, vec![dir.join("final.png")]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn script_drives_inputs_and_shots() {
    let dir = out_dir("script");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("t.twinescript");
    std::fs::write(
        &script,
        "wait 100\ntap 10 5\nkey Enter\nshot mid\nhotkey F9\nwait 40\nshot end\n",
    )
    .unwrap();
    let cfg = SimConfig::new(32, 16).headless(Some(Headless {
        frames: 1,
        script: Some(script),
        out_dir: dir.clone(),
    }));
    let app = SimApp::framebuffer(cfg, |fb, _, frame| fb.fill(frame as u8));
    let mut keypad = app.inputs().keypad;
    let report = app.run_headless().unwrap();
    // Last action at 100 + 50 (tap) + 40 = 190 ms → frame 12 (192 ms) is the last one.
    assert_eq!(report.frames, 13);
    for f in ["mid.png", "shot-0.png", "end.png", "final.png"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert_eq!(
        keypad.read(),
        InputData::Keypad(KeypadData {
            key: Key::Enter,
            pressed: true,
            more: true
        })
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn script_error_reports_file_and_line() {
    let dir = out_dir("error");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("bad.twinescript");
    std::fs::write(&script, "wait 10\nkey Nope\n").unwrap();
    let cfg = SimConfig::new(8, 8).headless(Some(Headless {
        frames: 1,
        script: Some(script.clone()),
        out_dir: dir.clone(),
    }));
    let err = run_headless_framebuffer(cfg, |_, _, _| {}).unwrap_err();
    assert!(matches!(err, SimError::Script { .. }));
    assert_eq!(
        err.to_string(),
        format!("{}:2: unknown key `Nope`", script.display())
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_output_is_deterministic() {
    let run = |tag: &str| {
        let dir = out_dir(tag);
        let cfg = SimConfig::new(16, 8)
            .format(ColorFormat::Rgb565Swapped)
            .headless(Some(Headless {
                frames: 5,
                script: None,
                out_dir: dir.clone(),
            }));
        run_headless_framebuffer(cfg, |fb, _, frame| {
            for (i, b) in fb.iter_mut().enumerate() {
                *b = (i as u32 * 7 + frame * 13) as u8;
            }
        })
        .unwrap();
        let bytes = std::fs::read(dir.join("final.png")).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        bytes
    };
    assert_eq!(run("det-a"), run("det-b"));
}
