//! Keyboard and encoder navigation with scrolling: scroll-on-focus, arrow-key scrolling and
//! gridnav (LVGL `lv_obj_event` `FOCUSED`/`KEY`, `lv_gridnav`).

mod common;

use common::{list, white_screen};
use twine_core::Rect;
use twine_engine::gridnav::{GridnavCtrl, gridnav_add, gridnav_focused};
use twine_engine::{Key, NodeId, ObjFlags};
use twine_testing::EngineHarness;

/// A 200×200 screen with a 100×100 list at (20, 20) of 10 rows of 40 px. With `rows_in_group`
/// the rows are the members of the default group, else the list is.
fn scene(rows_in_group: bool) -> (EngineHarness, NodeId, Vec<NodeId>) {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        e.set_default_group(Some(g));
        let (cont, rows) = list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40);
        if rows_in_group {
            for &r in &rows {
                e.group_add(g, r);
            }
        } else {
            e.group_add(g, cont);
        }
        ids = Some((cont, rows));
    });
    h.keypad_input();
    h.run_until_idle();
    let (c, r) = ids.unwrap();
    (h, c, r)
}

#[test]
fn focus_next_scrolls_item_into_view() {
    let (mut h, cont, rows) = scene(true);
    for _ in 0..5 {
        h.key(Key::Next);
    }
    h.run_until_idle();
    let g = h.engine().default_group().unwrap();
    assert_eq!(h.engine().focused(g), Some(rows[5]));
    // Row 5 (200..240 of the content) is aligned to the bottom edge.
    assert_eq!(h.engine().scroll_offset(cont).y, 140);
    // Back to the first row: scrolled up again.
    h.engine_mut().focus(rows[0]);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 0);
}

#[test]
fn scroll_on_focus_flag_off_does_not_scroll() {
    let (mut h, cont, rows) = scene(true);
    for &r in &rows {
        h.engine_mut().set_flag(r, ObjFlags::SCROLL_ON_FOCUS, false);
    }
    for _ in 0..5 {
        h.key(Key::Next);
    }
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).y, 0);
}

#[test]
fn arrow_key_scrolls_focused_container() {
    let (mut h, cont, _) = scene(false);
    h.key(Key::Down);
    // A quarter of the height, without animation (LVGL).
    assert_eq!(h.engine().scroll_offset(cont).y, 25);
    h.key(Key::Right); // cannot scroll horizontally: scrolls vertically
    assert_eq!(h.engine().scroll_offset(cont).y, 50);
    h.key(Key::Up);
    assert_eq!(h.engine().scroll_offset(cont).y, 25);
    for _ in 0..20 {
        h.key(Key::Down);
    }
    assert_eq!(h.engine().scroll_offset(cont).y, 300, "bounded");
    // Without `SCROLL_WITH_ARROW` nothing happens.
    h.engine_mut().set_flag(cont, ObjFlags::SCROLL_WITH_ARROW, false);
    h.key(Key::Up);
    assert_eq!(h.engine().scroll_offset(cont).y, 300);
}

#[test]
fn encoder_scrolls_scrollable_node_in_edit_mode() {
    let (mut h, cont, _) = scene(false);
    // A scrollable node counts as editable for the encoder: a click enters edit mode, then
    // the rotation scrolls it.
    let g = h.engine().default_group().unwrap();
    h.encoder_input();
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    h.encoder(2);
    assert_eq!(h.engine().scroll_offset(cont).y, 50);
}

/// A gridnav container 100×100 at (20, 20) with a 2-column grid of 40×40 boxes (8 rows,
/// 50 px apart), member of the default group.
fn grid_scene(ctrl: GridnavCtrl) -> (EngineHarness, NodeId, Vec<NodeId>) {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        e.set_default_group(Some(g));
        let (cont, _) = list(e, s, Rect::from_xywh(20, 20, 100, 100), 0, 0);
        let boxes = (0..16)
            .map(|i| {
                let r = Rect::from_xywh((i % 2) * 50, (i / 2) * 50, 40, 40);
                twine_testing::scenes::child_box(e, cont, r, &[])
            })
            .collect::<Vec<_>>();
        e.update_layout();
        gridnav_add(e, cont, ctrl);
        e.group_add(g, cont);
        ids = Some((cont, boxes));
    });
    h.keypad_input();
    h.run_until_idle();
    let (c, b) = ids.unwrap();
    (h, c, b)
}

#[test]
fn gridnav_scrolls_to_focused_child() {
    let (mut h, cont, boxes) = grid_scene(GridnavCtrl::empty());
    assert_eq!(gridnav_focused(h.engine(), cont), Some(boxes[0]));
    for _ in 0..4 {
        h.key(Key::Down);
    }
    h.run_until_idle();
    assert_eq!(gridnav_focused(h.engine(), cont), Some(boxes[8]));
    // Box 8 spans 200..240 of the content: at the bottom edge.
    assert_eq!(h.engine().scroll_offset(cont).y, 140);
    let c = h.engine().coords(boxes[8]);
    assert!(Rect::from_xywh(20, 20, 100, 100).contains_rect(&c));
}

#[test]
fn gridnav_scroll_first_scrolls_before_moving() {
    let (mut h, cont, boxes) = grid_scene(GridnavCtrl::SCROLL_FIRST);
    // Box 0 gets content to scroll: 3 × its height.
    twine_testing::scenes::child_box(h.engine_mut(), boxes[0], Rect::from_xywh(0, 0, 40, 120), &[]);
    h.engine_mut().update_layout();
    h.key(Key::Down);
    h.run_until_idle();
    // The focused box scrolled by a quarter of its height instead of moving the focus.
    assert_eq!(gridnav_focused(h.engine(), cont), Some(boxes[0]));
    assert_eq!(h.engine().scroll_offset(boxes[0]).y, 10);
    for _ in 0..10 {
        h.key(Key::Down);
        h.run_until_idle(); // each step animates from the current offset (LVGL)
    }
    // At its end the focus moves on.
    assert_eq!(h.engine().scroll_offset(boxes[0]).y, 80);
    assert_ne!(gridnav_focused(h.engine(), cont), Some(boxes[0]));
}
