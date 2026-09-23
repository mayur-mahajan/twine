//! The input-aware framebuffer runner (`SimApp::framebuffer_with_input`).

use std::cell::RefCell;
use std::rc::Rc;

use twine_core::Point;
use twine_hal::Key;
use twine_sim::{Headless, SimApp, SimConfig, SimFrame};

#[test]
fn frames_receive_keys_and_clicks_once() {
    let dir = std::env::temp_dir().join(format!("twine-sim-frame-input-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("t.twinescript");
    std::fs::write(
        &script,
        "wait 32\nkey Right\nkey Left\nwait 32\ntap 20 7\nwait 64\n",
    )
    .unwrap();
    let cfg = SimConfig::new(32, 16).headless(Some(Headless {
        frames: 1,
        script: Some(script),
        out_dir: dir.clone(),
    }));
    let seen: Rc<RefCell<Vec<SimFrame>>> = Rc::default();
    let log = seen.clone();
    let app = SimApp::framebuffer_with_input(cfg, move |_, _, frame| log.borrow_mut().push(frame.clone()));
    app.run_headless().unwrap();
    let frames = seen.borrow();
    let keys: Vec<(u32, Vec<Key>)> = frames
        .iter()
        .filter(|f| !f.keys.is_empty())
        .map(|f| (f.index, f.keys.clone()))
        .collect();
    assert_eq!(keys, vec![(2, vec![Key::Right, Key::Left])]);
    let clicks: Vec<Point> = frames.iter().filter_map(|f| f.clicked).collect();
    assert_eq!(clicks, vec![Point::new(20, 7)]);
    assert!(frames.iter().all(|f| f.pointer.is_some()));
    assert_eq!(frames[3].time.as_millis(), 48);
    let _ = std::fs::remove_dir_all(dir);
}
