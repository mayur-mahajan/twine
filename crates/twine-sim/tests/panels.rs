//! Multi-panel apps (`SimApp::panels`, `run_panels_headless`): panel layout, input routing to
//! the panel under the pointer or the focused panel, independent redraw and stepping.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use twine_core::Rect;
use twine_hal::Key;
use twine_reactive::{Runtime, Signal};
use twine_sim::{Headless, PanelLayout, SimApp, SimConfig, SimDisplay, SimPanel, run_panels_headless};
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 32;

fn out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("twine-sim-panels-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn headless(dir: &Path, script: &str) -> SimConfig {
    let path = dir.join("t.twinescript");
    std::fs::write(&path, script).unwrap();
    SimConfig::new(0, 0).headless(Some(Headless {
        frames: 1,
        script: Some(path),
        out_dir: dir.to_path_buf(),
    }))
}

/// A panel with one full-size button counting its clicks, recording the keys its hook saw.
fn button_panel(clicks: Rc<Cell<u32>>, keys: Rc<RefCell<Vec<Key>>>) -> SimPanel {
    SimPanel::new(W, H, Runtime::current_thread(), move |_| {
        button(label("tap"))
            .size(i32::from(W), i32::from(H))
            .on_click(move || clicks.set(clicks.get() + 1))
    })
    .on_frame(move |cx| keys.borrow_mut().extend_from_slice(cx.keys()))
}

#[test]
fn panels_are_laid_out_in_one_window() {
    let panels = vec![
        SimPanel::new(W, H, Runtime::current_thread(), |_| label("a")),
        SimPanel::new(40, 20, Runtime::current_thread(), |_| label("b")),
    ];
    let app = SimApp::panels(SimConfig::new(1, 1), panels, PanelLayout::Column).unwrap();
    let (rects, size) = PanelLayout::Column.arrange(&[(W, H), (40, 20)]);
    assert_eq!(rects[1], Rect::new(4, 40, 44, 60));
    assert_eq!(app.window_size(), size);
    assert_eq!(app.panel_count(), 2);
    for i in 0..2 {
        let e = app.panel_engine(i).unwrap();
        let d = e.driver::<SimDisplay>(e.default_display().unwrap()).unwrap();
        assert_eq!(
            d.panel_size(),
            [(W, H), (40, 20)][i],
            "each panel has its own display"
        );
    }
    assert_eq!(app.focused_panel(), Some(0), "the first panel has the focus");
    assert!(app.panel_ui(1).is_some());
}

#[test]
fn pointer_goes_to_the_panel_under_it_and_keys_to_the_focused_one() {
    let dir = out_dir("routing");
    let (ca, cb) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (ka, kb) = (
        Rc::new(RefCell::new(Vec::new())),
        Rc::new(RefCell::new(Vec::new())),
    );
    let panels = vec![
        button_panel(ca.clone(), ka.clone()),
        button_panel(cb.clone(), kb.clone()),
    ];
    // Panel 0 at x 4..68, panel 1 at x 72..136 (y 4..36). A key before any click goes to the
    // first panel; a tap on the second focuses it, and the next key goes there.
    let cfg = headless(
        &dir,
        "wait 50\nkey 1\nwait 50\ntap 100 20\nwait 100\nkey 2\nwait 50\ntap 70 20\nwait 50\n",
    );
    run_panels_headless(cfg, panels).unwrap();
    assert_eq!(
        (ca.get(), cb.get()),
        (0, 1),
        "the tap reached the panel under the pointer"
    );
    assert_eq!(*ka.borrow(), [Key::Char('1')]);
    assert_eq!(*kb.borrow(), [Key::Char('2')]);
    let _ = std::fs::remove_dir_all(dir);
}

// (panel, simulator ms, flushes, heartbeat) seen by the panels' hooks.
type Seen = Rc<RefCell<Vec<(usize, u64, u64, u32)>>>;
fn record(seen: &Seen, cx: &mut twine_sim::PanelCx<'_>) {
    let (i, ms) = (cx.index(), cx.now().as_millis());
    let e = cx.engine();
    let flushes = e
        .driver::<SimDisplay>(e.default_display().unwrap())
        .unwrap()
        .flush_count();
    seen.borrow_mut().push((i, ms, flushes, e.heartbeat()));
}

#[test]
fn panels_redraw_and_step_independently() {
    let dir = out_dir("redraw");
    let value: Rc<Cell<Option<Signal<u32>>>> = Rc::new(Cell::new(None));
    let v = value.clone();
    let seen: Seen = Rc::default();
    let (s1, s2) = (seen.clone(), seen.clone());
    let theme = || AppConfig::new().theme(&lvgl::THEME);
    let panels = vec![
        SimPanel::new(W, H, Runtime::current_thread(), move |cx| {
            let n = cx.signal(0u32);
            v.set(Some(n));
            label(text!("{}", n.get()))
        })
        .app_config(theme())
        .on_frame(move |cx| {
            record(&s1, cx);
            // From 160 ms to 320 ms panel 0 changes at every frame.
            if (160..320).contains(&cx.now().as_millis()) {
                if let Some(n) = value.get() {
                    n.update(|n| *n += 1);
                }
            }
        }),
        SimPanel::new(W, H, Runtime::current_thread(), |_| label("still"))
            .app_config(theme())
            .on_frame(move |cx| {
                record(&s2, cx);
                // From 400 ms on panel 1 stalls.
                if cx.now().as_millis() >= 400 {
                    cx.set_running(false);
                }
            }),
    ];
    let cfg = SimConfig::new(0, 0).headless(Some(Headless {
        frames: 40,
        script: None,
        out_dir: dir.clone(),
    }));
    run_panels_headless(cfg, panels).unwrap();
    let seen = seen.borrow();
    // The last record of panel `i` at or before `ms`: (flushes, heartbeat).
    let at = |i: usize, ms: u64| {
        seen.iter()
            .rev()
            .find(|s| s.0 == i && s.1 <= ms)
            .map(|s| (s.2, s.3))
            .unwrap()
    };
    assert!(
        at(0, 352).0 >= at(0, 144).0 + 5,
        "panel 0 redrew while it changed"
    );
    assert_eq!(at(1, 352).0, at(1, 144).0, "panel 1 did not redraw meanwhile");
    // Panel 1 stalled: its heartbeat stands still while panel 0's keeps counting.
    assert_eq!(at(1, 620).1, at(1, 432).1, "stalled panel");
    assert!(at(0, 620).1 > at(0, 432).1, "the other panel keeps updating");
    let _ = std::fs::remove_dir_all(dir);
}
