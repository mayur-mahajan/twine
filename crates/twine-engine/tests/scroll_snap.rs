//! Scroll snapping (LVGL `find_snap_point_x/y`, `lv_obj_update_snap`), `SCROLL_ONE` and the
//! snapped throw (`lv_indev_scroll_throw_predict`).

mod common;

use common::{style, white_screen};
use twine_core::{Color, Opa, Point, Rect};
use twine_engine::{NodeId, ObjFlags, ScrollSnap};
use twine_style::StyleProp;
use twine_testing::EngineHarness;

const COLORS: [u32; 8] = [
    0x00E0_4040,
    0x00E0_9040,
    0x00D0_D040,
    0x0040_C040,
    0x0040_C0C0,
    0x0040_60E0,
    0x0090_40E0,
    0x00E0_40A0,
];

/// A 240×140 screen with a 200×100 carousel at (20, 20) holding 8 cards of 80×80 every
/// 100 px (card `i` spans `100·i .. 100·i + 80` of the content), center snapping.
fn scene() -> (EngineHarness, NodeId, Vec<NodeId>) {
    let mut ids = None;
    let mut h = EngineHarness::new(240, 140).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let (cont, _) = common::list(e, s, Rect::from_xywh(20, 20, 200, 100), 0, 0);
        style(e, cont, &[StyleProp::BgColor(Color::hex(0x00DD_DDDD))]);
        let cards = (0..8)
            .map(|i| {
                twine_testing::scenes::child_box(
                    e,
                    cont,
                    Rect::from_xywh(i * 100, 10, 80, 80),
                    &[
                        StyleProp::BgColor(Color::hex(COLORS[i as usize])),
                        StyleProp::BgOpa(Opa::COVER),
                        StyleProp::Radius(8),
                    ],
                )
            })
            .collect::<Vec<_>>();
        e.set_scroll_snap_x(cont, ScrollSnap::Center);
        e.update_layout();
        ids = Some((cont, cards));
    });
    h.run_until_idle();
    let (c, cards) = ids.unwrap();
    (h, c, cards)
}

/// The offset that centers card `i` (its center `100·i + 40` at the content center 100).
fn centered(i: i32) -> i32 {
    100 * i - 60
}

/// Presses on the carousel background (below the cards' top) and moves by `dx` per read.
fn swipe(h: &mut EngineHarness, dx: i32, n: i32) {
    let y = 25;
    h.press(Point::new(120, y));
    for k in 1..=n {
        h.clock().advance(h.engine().config().read_period);
        h.move_to(Point::new(120 + k * dx, y));
    }
    h.release();
}

#[test]
fn center_snap_aligns_nearest_child() {
    let (mut h, cont, cards) = scene();
    h.engine_mut().scroll_to_x(cont, 120, false);
    h.engine_mut().update_snap(cont, false);
    assert_eq!(h.engine().scroll_offset(cont).x, centered(2));
    let c = h.engine().coords(cards[2]);
    assert_eq!(c.x0 + c.width() / 2, 20 + 100);
}

#[test]
fn start_snap_and_end_snap() {
    let (mut h, cont, cards) = scene();
    let e = h.engine_mut();
    e.set_scroll_snap_x(cont, ScrollSnap::Start);
    e.scroll_to_x(cont, 120, false);
    e.update_snap(cont, false);
    // Only snap points inside the container count: card 2 (at 100) is the nearest start.
    assert_eq!(e.scroll_offset(cont).x, 200);
    assert_eq!(e.coords(cards[2]).x0, 20);
    e.set_scroll_snap_x(cont, ScrollSnap::End);
    e.scroll_to_x(cont, 120, false);
    e.update_snap(cont, false);
    assert_eq!(e.scroll_offset(cont).x, 80);
    assert_eq!(e.coords(cards[2]).x1, 220);
}

#[test]
fn non_snappable_children_ignored() {
    let (mut h, cont, cards) = scene();
    let e = h.engine_mut();
    e.set_flag(cards[2], ObjFlags::SNAPPABLE, false);
    e.scroll_to_x(cont, 120, false);
    e.update_snap(cont, false);
    assert_eq!(e.scroll_offset(cont).x, centered(1));
}

#[test]
fn snapped_throw_matches_lvgl_prediction() {
    let (mut h, cont, _) = scene();
    swipe(&mut h, -20, 3);
    // Released after three reads of -20 px: the scroll started with the first (-20 px each):
    // offset 60. Throw vector = -20 + -13 (30 ms) + -7 (60 ms) = -40 (LVGL decay by age).
    let off = h.engine().scroll_offset(cont).x;
    assert_eq!(off, 60);
    // `lv_indev_scroll_throw_predict`: v, v·0.9, … until 0.
    let mut v = -40;
    let mut predicted = 0;
    while v != 0 {
        predicted += v;
        v = v * 90 / 100;
    }
    assert_eq!(predicted, -303);
    h.run_until_idle();
    // The predicted position (60 + 303 = 363) snaps to the nearest card: card 4 (340).
    let target = (0..8)
        .map(centered)
        .min_by_key(|c| (c - (off - predicted)).abs())
        .unwrap();
    assert_eq!(target, centered(4));
    assert_eq!(h.engine().scroll_offset(cont).x, target);
}

#[test]
fn scroll_one_moves_only_one_item_on_fast_throw() {
    let (mut h, cont, _) = scene();
    h.engine_mut().set_flag(cont, ObjFlags::SCROLL_ONE, true);
    h.engine_mut().scroll_to_x(cont, centered(1), false);
    swipe(&mut h, -40, 3);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).x, centered(2));
    // And back: one card to the left.
    swipe(&mut h, 40, 3);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).x, centered(1));
}

#[test]
fn snap_after_container_resize() {
    let (mut h, cont, _) = scene();
    h.engine_mut().scroll_to_x(cont, centered(1), false);
    h.engine_mut().set_width(cont, 240);
    h.update();
    // `SizeChanged` snaps again: card 1 (content center 140) moves to the new center 120.
    assert_eq!(h.engine().scroll_offset(cont).x, 100 + 40 - 120);
}

#[test]
fn carousel_center_snapped() {
    let (mut h, cont, _) = scene();
    h.engine_mut().scroll_to_x(cont, 130, false);
    h.engine_mut().update_snap(cont, true);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(cont).x, centered(2));
    h.assert_snapshot("carousel_center_snapped");
}
