//! The scroll model: extents (LVGL `lv_obj_get_scroll_top/bottom/left/right`), instant
//! `scroll_by` / `scroll_to` with LVGL's sign convention, invalidation limited to the
//! container, no relayout.

mod common;

use common::{EvLog, list, record, scroll_codes, style, white_screen};
use twine_core::{Point, Rect};
use twine_engine::{EventCode as C, InvalidateReason, NodeId, ObjFlags};
use twine_style::{BaseDir, StyleProp};
use twine_testing::EngineHarness;

/// A 200×200 screen with a 100×100 container at (20, 20) holding one child of `w`×`h` at the
/// start of its content area.
fn scene(w: i32, h: i32) -> (EngineHarness, NodeId, NodeId) {
    let mut ids = None;
    let mut hs = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let (cont, _) = list(e, s, Rect::from_xywh(20, 20, 100, 100), 0, 0);
        let child = twine_testing::scenes::child_box(e, cont, Rect::from_xywh(0, 0, w, h), &[]);
        e.update_layout();
        ids = Some((cont, child));
    });
    hs.run_until_idle();
    let (c, ch) = ids.unwrap();
    (hs, c, ch)
}

#[test]
fn extents_for_children_below_content() {
    let (h, cont, _) = scene(100, 300);
    let e = h.engine();
    assert_eq!(
        (
            e.scroll_top(cont),
            e.scroll_bottom(cont),
            e.scroll_left(cont),
            e.scroll_right(cont)
        ),
        (0, 200, 0, 0)
    );
}

#[test]
fn extents_zero_when_content_fits() {
    let (h, cont, _) = scene(50, 50);
    let e = h.engine();
    // LVGL values: the free space is negative room.
    assert_eq!(e.scroll_bottom(cont), -50);
    assert_eq!(e.scroll_right(cont), -50);
    assert_eq!((e.scroll_top(cont), e.scroll_left(cont)), (0, 0));
}

#[test]
fn extents_include_margins_and_padding() {
    let (mut h, cont, child) = scene(80, 200);
    let e = h.engine_mut();
    style(
        e,
        cont,
        &[
            StyleProp::PadLeft(10),
            StyleProp::PadTop(10),
            StyleProp::PadRight(10),
            StyleProp::PadBottom(10),
            StyleProp::BorderWidth(2),
        ],
    );
    style(e, child, &[StyleProp::MarginBottom(5), StyleProp::MarginRight(7)]);
    e.update_layout();
    // Child: x 32..112, y 32..232; container 20..120 with 12 px of padding + border.
    assert_eq!(e.coords(child), Rect::from_xywh(32, 32, 80, 200));
    assert_eq!(e.scroll_bottom(cont), 232 + 5 - (120 - 12));
    assert_eq!(e.scroll_right(cont), 112 + 7 - (120 - 12));
}

#[test]
fn scroll_by_moves_children_coords() {
    let (mut h, cont, child) = scene(100, 300);
    let e = h.engine_mut();
    let grand = twine_testing::scenes::child_box(e, child, Rect::from_xywh(5, 5, 10, 10), &[]);
    e.update_layout();
    let before = (e.coords(child), e.coords(grand));
    e.scroll_by(cont, 0, -30, false);
    assert_eq!(e.coords(child), before.0.translate(0, -30));
    assert_eq!(e.coords(grand), before.1.translate(0, -30));
    assert_eq!(
        e.coords(cont),
        Rect::from_xywh(20, 20, 100, 100),
        "the container stays"
    );
    assert_eq!((e.scroll_top(cont), e.scroll_bottom(cont)), (30, 170));
}

#[test]
fn scroll_by_sign_convention_matches_lvgl() {
    let (mut h, cont, child) = scene(100, 300);
    let e = h.engine_mut();
    // `lv_obj_scroll_by(obj, 0, -50)`: content up, scroll_y = 50.
    e.scroll_by(cont, 0, -50, false);
    assert_eq!(e.scroll_offset(cont), Point::new(0, 50));
    // `lv_obj_scroll_by(obj, 0, 10)`: content down by 10, scroll_y reduced by 10.
    let y = e.coords(child).y0;
    e.scroll_by(cont, 0, 10, false);
    assert_eq!(e.scroll_offset(cont), Point::new(0, 40));
    assert_eq!(e.coords(child).y0, y + 10);
    // `scroll_by` is unbounded (LVGL); `scroll_by_bounded` is not.
    e.scroll_by(cont, 0, 100, false);
    assert_eq!(e.scroll_offset(cont).y, -60);
    // Beyond the start: bounded, the content can only go back to the start.
    e.scroll_by_bounded(cont, 0, 100, false);
    assert_eq!(e.scroll_offset(cont).y, 0);
    e.scroll_by_bounded(cont, 0, -1000, false);
    assert_eq!(e.scroll_offset(cont).y, 200);
}

#[test]
fn scroll_invalidates_container_only() {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        // Partly outside the screen: the invalidation is clipped by it.
        ids = Some(list(e, s, Rect::from_xywh(150, 20, 100, 100), 10, 40).0);
    });
    h.run_until_idle();
    let cont = ids.unwrap();
    let log = EvLog::default();
    record(h.engine_mut(), cont, &log);
    h.engine_mut().scroll_by(cont, 0, -25, false);
    assert_eq!(
        h.engine().invalidation_log(),
        &[(Rect::new(150, 20, 200, 120), InvalidateReason::Scroll)]
    );
    assert_eq!(scroll_codes(&log), [C::ScrollBegin, C::ScrollEnd]);
    assert_eq!(log.borrow().iter().filter(|(c, _)| *c == C::Scroll).count(), 1);
    h.clock().advance(h.engine().config().refr_period);
    h.update();
    let areas: Vec<Rect> = h.flushes().iter().map(|f| f.area).collect();
    assert!(!areas.is_empty());
    for a in areas {
        assert!(Rect::new(150, 20, 200, 120).contains_rect(&a), "{a:?}");
    }
}

#[test]
fn scroll_does_not_relayout() {
    let (mut h, cont, _) = scene(100, 300);
    let e = h.engine_mut();
    assert!(!e.layout_pending());
    e.scroll_by(cont, 0, -40, false);
    e.scroll_to(cont, 0, 120, false);
    assert!(!e.layout_pending(), "scrolling must not schedule a layout");
    let stats = e.layout_stats();
    h.update();
    assert_eq!(h.engine().layout_stats(), stats);
}

#[test]
fn scroll_to_same_position_is_noop() {
    let (mut h, cont, _) = scene(100, 300);
    h.engine_mut().scroll_to(cont, 0, 60, false);
    h.run_until_idle();
    let log = EvLog::default();
    record(h.engine_mut(), cont, &log);
    h.engine_mut().scroll_to(cont, 0, 60, false);
    assert!(h.engine().invalidation_log().is_empty());
    assert!(log.borrow().is_empty());
    h.assert_idle();
}

#[test]
fn relayout_preserves_scroll_offset() {
    let (mut h, cont, child) = scene(100, 300);
    let e = h.engine_mut();
    e.scroll_to_y(cont, 50, false);
    e.set_height(child, 400);
    h.update();
    let e = h.engine();
    assert_eq!(e.scroll_offset(cont).y, 50);
    assert_eq!(e.coords(child), Rect::from_xywh(20, 20 - 50, 100, 400));
    assert_eq!(e.scroll_bottom(cont), 400 - 100 - 50);
}

#[test]
fn deleting_children_readjusts_scroll() {
    let mut ids = None;
    let mut h = EngineHarness::new(200, 200).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        ids = Some(list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40));
    });
    h.run_until_idle();
    let (cont, rows) = ids.unwrap();
    h.engine_mut().scroll_to_y(cont, 300, false);
    assert_eq!(h.engine().scroll_bottom(cont), 0);
    for &r in &rows[6..] {
        h.engine_mut().delete(r).unwrap();
    }
    h.update();
    // Content scrolled in beyond its end is scrolled back (LVGL `lv_obj_readjust_scroll`).
    assert_eq!(h.engine().scroll_offset(cont).y, 140);
    assert_eq!(h.engine().scroll_bottom(cont), 0);
}

#[test]
fn rtl_extents_swap_left_right() {
    let (mut h, cont, child) = scene(300, 50);
    assert_eq!(
        (h.engine().scroll_left(cont), h.engine().scroll_right(cont)),
        (0, 200)
    );
    let e = h.engine_mut();
    style(e, cont, &[StyleProp::BaseDir(BaseDir::Rtl)]);
    e.update_layout();
    // RTL children start at the right edge; the content to scroll to is on the left.
    assert_eq!(e.coords(child).x1, 120);
    assert_eq!((e.scroll_left(cont), e.scroll_right(cont)), (200, 0));
    e.scroll_by_bounded(cont, 500, 0, false);
    assert_eq!(e.scroll_offset(cont).x, -200);
    assert_eq!((e.scroll_left(cont), e.scroll_right(cont)), (0, 200));
    assert_eq!(e.coords(child).x0, 20);
}

#[test]
fn floating_and_hidden_children_do_not_count() {
    let (mut h, cont, _) = scene(100, 100);
    let e = h.engine_mut();
    let far = twine_testing::scenes::child_box(e, cont, Rect::from_xywh(0, 500, 10, 10), &[]);
    e.update_layout();
    assert_eq!(e.scroll_bottom(cont), 500 + 10 - 100);
    e.set_flag(far, ObjFlags::FLOATING, true);
    e.update_layout();
    assert_eq!(e.scroll_bottom(cont), 0);
    e.set_flag(far, ObjFlags::FLOATING, false);
    e.set_flag(far, ObjFlags::HIDDEN, true);
    e.update_layout();
    assert_eq!(e.scroll_bottom(cont), 0);
    // A floating child does not move with the scroll.
    e.set_flag(far, ObjFlags::HIDDEN, false);
    e.set_flag(far, ObjFlags::FLOATING, true);
    e.update_layout();
    let before = e.coords(far);
    e.scroll_by(cont, 0, 0, false);
    e.set_height(cont, 50);
    e.update_layout();
    e.scroll_by(cont, 0, -20, false);
    assert_eq!(e.coords(far), before);
}
