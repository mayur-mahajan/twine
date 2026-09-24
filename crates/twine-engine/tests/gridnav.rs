//! Grid navigation on a 3×3 grid and on uneven rows (LVGL `lv_gridnav` examples).

mod common;

use common::{EvLog, clickable, input_codes, record, white_screen};
use twine_core::Rect;
use twine_engine::{EventCode as C, GridnavCtrl, Key, NodeId, ObjFlags, State, gridnav};
use twine_testing::EngineHarness;

/// A container with children at `rects` (relative to the container at (0, 0)), in the default
/// group with gridnav `ctrl`.
fn setup(rects: &[Rect], ctrl: GridnavCtrl) -> (EngineHarness, NodeId, Vec<NodeId>) {
    let mut cont = None;
    let mut kids = Vec::new();
    let mut h = EngineHarness::new(240, 240).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let c = clickable(e, s, Rect::from_xywh(0, 0, 240, 240));
        for r in rects {
            kids.push(clickable(e, c, *r));
        }
        gridnav::gridnav_add(e, c, ctrl);
        let g = e.create_group().unwrap();
        e.set_default_group(Some(g));
        e.group_add(g, c);
        cont = Some(c);
    });
    h.run_until_idle();
    (h, cont.unwrap(), kids)
}

/// 3×3 grid of 60×60 cells with 20 px gaps.
fn grid3() -> Vec<Rect> {
    (0..9)
        .map(|i| Rect::from_xywh(10 + (i % 3) * 80, 10 + (i / 3) * 80, 60, 60))
        .collect()
}

fn focused(h: &EngineHarness, c: NodeId) -> Option<NodeId> {
    gridnav::gridnav_focused(h.engine(), c)
}

#[test]
fn gridnav_moves_right_left_in_row() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::empty());
    assert_eq!(focused(&h, c), Some(k[0]));
    h.key(Key::Right);
    assert_eq!(focused(&h, c), Some(k[1]));
    let s = h.engine().tree().node(k[1]).unwrap().state();
    assert!(s.contains(State::FOCUSED | State::FOCUS_KEY));
    assert!(
        !h.engine()
            .tree()
            .node(k[0])
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
    h.key(Key::Right);
    h.key(Key::Left);
    assert_eq!(focused(&h, c), Some(k[1]));
}

#[test]
fn gridnav_moves_down_to_overlapping_column() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::empty());
    h.key(Key::Right);
    h.key(Key::Down);
    assert_eq!(focused(&h, c), Some(k[4]));
    h.key(Key::Down);
    assert_eq!(focused(&h, c), Some(k[7]));
    // Uneven rows: a wide cell below two narrow ones; up from it goes to the nearest center.
    let rects = [
        Rect::from_xywh(10, 10, 60, 40),
        Rect::from_xywh(90, 10, 60, 40),
        Rect::from_xywh(10, 70, 140, 40),
        Rect::from_xywh(120, 130, 40, 40),
    ];
    let (mut h, c, k) = setup(&rects, GridnavCtrl::empty());
    h.key(Key::Right);
    h.key(Key::Down);
    assert_eq!(focused(&h, c), Some(k[2]));
    h.key(Key::Down);
    assert_eq!(focused(&h, c), Some(k[3]));
    h.key(Key::Up);
    assert_eq!(focused(&h, c), Some(k[2]));
    h.key(Key::Up);
    // Both upper cells are equally far from the wide cell's center: the first wins.
    assert_eq!(focused(&h, c), Some(k[0]));
}

#[test]
fn gridnav_rollover_wraps() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::ROLLOVER);
    h.key(Key::Right);
    h.key(Key::Right);
    h.key(Key::Right);
    assert_eq!(focused(&h, c), Some(k[3]), "next row, first item");
    h.key(Key::Left);
    assert_eq!(focused(&h, c), Some(k[2]), "previous row, last item");
    h.key(Key::Up);
    assert_eq!(focused(&h, c), Some(k[8]), "last row");
    h.key(Key::Down);
    assert_eq!(focused(&h, c), Some(k[2]), "first row");
    h.key(Key::Left);
    h.key(Key::Left);
    h.key(Key::Left);
    assert_eq!(focused(&h, c), Some(k[8]), "wraps to the last item");
}

#[test]
fn gridnav_horizontal_only_ignores_up_down() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::HORIZONTAL_MOVE_ONLY);
    h.key(Key::Down);
    assert_eq!(focused(&h, c), Some(k[0]));
    h.key(Key::Right);
    assert_eq!(focused(&h, c), Some(k[1]));
}

#[test]
fn gridnav_enter_clicks_child() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::empty());
    h.key(Key::Right);
    let log = EvLog::default();
    record(h.engine_mut(), k[1], &log);
    h.key(Key::Enter);
    let codes = input_codes(&log);
    assert!(
        codes.contains(&C::Pressed) && codes.contains(&C::Clicked),
        "{codes:?}"
    );
    assert_eq!(codes.first(), Some(&C::Key));
    assert!(
        !h.engine()
            .tree()
            .node(k[1])
            .unwrap()
            .state()
            .contains(State::PRESSED)
    );
    let _ = c;
}

#[test]
fn gridnav_skips_hidden_children() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::empty());
    h.engine_mut().set_flag(k[1], ObjFlags::HIDDEN, true);
    h.key(Key::Right);
    assert_eq!(focused(&h, c), Some(k[2]));
    gridnav::gridnav_set_focused(h.engine_mut(), c, Some(k[4]), false);
    assert_eq!(focused(&h, c), Some(k[4]));
    assert!(
        !h.engine()
            .tree()
            .node(k[2])
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
    gridnav::gridnav_remove(h.engine_mut(), c);
    assert_eq!(focused(&h, c), None);
}

#[test]
fn gridnav_edge_without_rollover_moves_group_focus() {
    let (mut h, c, k) = setup(&grid3(), GridnavCtrl::empty());
    let g = h.engine().default_group().unwrap();
    let s = h.screen();
    let other = clickable(h.engine_mut(), s, Rect::from_xywh(0, 0, 5, 5));
    h.engine_mut().group_add(g, other);
    h.key(Key::Left);
    assert_eq!(h.engine().focused(g), Some(other));
    assert!(
        !h.engine()
            .tree()
            .node(k[0])
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
    let _ = c;
}
