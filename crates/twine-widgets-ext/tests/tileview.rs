//! `Tileview`: tiles on a grid, swipes limited to each tile's directions, `set_tile` with and
//! without animation, the active tile after a scroll, and tiles following a resize.

mod common;

use common::{Mode, center, class, count_events, get, harness_with_group, with};
use twine_core::{Duration, Point};
use twine_engine::{EventCode, NodeId};
use twine_style::{Length, Sides};
use twine_testing::EngineHarness;
use twine_widgets_ext::tileview::{self, Tileview};

/// A 2 × 2 grid: (0,0) → right / down, (1,0) → left, (0,1) → up.
fn scene(mode: Mode) -> (EngineHarness, NodeId, [NodeId; 3]) {
    let mut h = harness_with_group(240, 200, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let tv = tileview::create(e, screen).unwrap();
    let t00 = tileview::add_tile(e, tv, 0, 0, Sides::RIGHT | Sides::BOTTOM).unwrap();
    let t10 = tileview::add_tile(e, tv, 1, 0, Sides::LEFT).unwrap();
    let t01 = tileview::add_tile(e, tv, 0, 1, Sides::TOP).unwrap();
    for (t, text) in [(t00, "Tile 0,0"), (t10, "Tile 1,0"), (t01, "Tile 0,1")] {
        let l = twine_widgets::label::create_with(e, t, text).unwrap();
        e.align(l, twine_style::Align::Center, 0, 0);
    }
    h.run_until_idle();
    (h, tv, [t00, t10, t01])
}

fn active(h: &EngineHarness, tv: NodeId) -> Option<NodeId> {
    get::<Tileview>(h, tv).tile_active()
}

fn swipe(h: &mut EngineHarness, tv: NodeId, dx: i32, dy: i32) {
    let c = center(h, tv);
    h.drag(c, Point::new(c.x + dx, c.y + dy), Duration::ms(150));
    h.run_until_idle();
}

#[test]
fn tileview_structure() {
    let (h, tv, tiles) = scene(Mode::Light);
    let e = h.engine();
    assert_eq!(class(&h, tv), "tileview");
    assert_eq!(class(&h, tiles[0]), "tileview_tile");
    let ca = e.content_area(tv);
    assert_eq!(e.coords(tiles[0]).size(), ca.size());
    assert_eq!(e.coords(tiles[1]).x0, ca.x0 + ca.width());
    assert_eq!(e.coords(tiles[2]).y0, ca.y0 + ca.height());
    assert_eq!(active(&h, tv), Some(tiles[0]));
    assert_eq!(e.scroll_dir(tv), Sides::RIGHT | Sides::BOTTOM);
}

#[test]
fn tileview_swipe_only_allowed_dirs() {
    let (mut h, tv, tiles) = scene(Mode::Light);
    // Swiping right (towards a tile on the left) is not allowed from (0,0): no scroll.
    h.run_until_idle();
    let before = h.engine().scroll_offset(tv);
    let c = center(&h, tv);
    h.press(c);
    for k in 1..=5 {
        h.advance(Duration::ms(30));
        h.move_to(Point::new(c.x + 20 * k, c.y));
    }
    assert_eq!(
        h.engine().scroll_offset(tv),
        before,
        "a forbidden swipe does not scroll"
    );
    h.release();
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(tv), before);
    h.assert_idle();
    // Left: to (1,0).
    swipe(&mut h, tv, -150, 0);
    assert_eq!(active(&h, tv), Some(tiles[1]));
    assert_eq!(h.engine().scroll_dir(tv), Sides::LEFT);
    // Up is not allowed from (1,0).
    let before = h.engine().scroll_offset(tv);
    swipe(&mut h, tv, 0, -120);
    assert_eq!(h.engine().scroll_offset(tv), before);
    assert_eq!(active(&h, tv), Some(tiles[1]));
    // Back right and down.
    swipe(&mut h, tv, 150, 0);
    assert_eq!(active(&h, tv), Some(tiles[0]));
    swipe(&mut h, tv, 0, -120);
    assert_eq!(active(&h, tv), Some(tiles[2]));
    h.assert_idle();
}

#[test]
fn tileview_forbidden_swipe_invalidates_nothing() {
    let (mut h, tv, _) = scene(Mode::Light);
    let c = center(&h, tv);
    h.press(c);
    h.advance(Duration::ms(30));
    let mut drawn = 0;
    for k in 1..=5 {
        h.move_to(Point::new(c.x + 20 * k, c.y));
        h.advance(Duration::ms(30));
        drawn += h.invalidations().len();
    }
    h.release();
    assert_eq!(drawn, 0, "no scroll, no redraw");
}

#[test]
fn tileview_set_tile_anim_then_idle() {
    let (mut h, tv, tiles) = scene(Mode::Light);
    let changes = count_events(&mut h, tv, EventCode::ValueChanged);
    with(&mut h, tv, |w: &mut Tileview, cx| w.set_tile(cx, tiles[1], true));
    assert!(h.engine().anim_count() > 0);
    let t = h.run_until_idle();
    assert!(t > Duration::ZERO);
    assert_eq!(
        h.engine().scroll_offset(tv).x,
        h.engine().content_area(tv).width()
    );
    assert_eq!(active(&h, tv), Some(tiles[1]));
    assert_eq!(
        changes.get(),
        0,
        "set_tile itself is silent; the scroll end finds the same tile"
    );
    h.assert_idle();
    with(&mut h, tv, |w: &mut Tileview, cx| {
        w.set_tile_by_index(cx, 0, 1, false);
    });
    assert_eq!(
        h.engine().scroll_offset(tv).y,
        h.engine().content_area(tv).height()
    );
    assert_eq!(active(&h, tv), Some(tiles[2]));
    // A missing tile is ignored.
    with(&mut h, tv, |w: &mut Tileview, cx| {
        w.set_tile_by_index(cx, 5, 5, false);
    });
    assert_eq!(active(&h, tv), Some(tiles[2]));
}

#[test]
fn tileview_active_after_scroll_end() {
    let (mut h, tv, tiles) = scene(Mode::Light);
    let changes = count_events(&mut h, tv, EventCode::ValueChanged);
    swipe(&mut h, tv, -150, 0);
    assert_eq!(active(&h, tv), Some(tiles[1]));
    assert_eq!(changes.get(), 1);
}

#[test]
fn tile_sizes_follow_viewport_resize() {
    let (mut h, tv, tiles) = scene(Mode::Light);
    h.engine_mut().set_size(tv, Length::Px(160), Length::Px(120));
    h.run_until_idle();
    let e = h.engine();
    let ca = e.content_area(tv);
    assert_eq!(e.coords(tiles[0]).size(), ca.size());
    assert_eq!(e.coords(tiles[1]).x0 - e.coords(tiles[0]).x0, ca.width());
}

#[test]
fn snapshot_tileview() {
    let (mut h, tv, _) = scene(Mode::Light);
    h.assert_snapshot("tileview_tile_0_0");
    let c = center(&h, tv);
    h.press(c);
    for k in 1..=4 {
        h.advance(Duration::ms(30));
        h.move_to(Point::new(c.x - 20 * k, c.y));
    }
    h.advance(Duration::ms(30));
    h.assert_panel_snapshot("tileview_mid_swipe");
    h.release();
    h.run_until_idle();
}
