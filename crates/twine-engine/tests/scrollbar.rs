//! Scrollbars (LVGL `lv_obj_get_scrollbar_area`): modes, geometry, drawing with the
//! `Scrollbar` part styles, invalidation.

mod common;

use common::{list, style, white_screen};
use twine_core::{Color, Duration, Opa, Point, Rect};
use twine_engine::{Engine, NodeId, ScrollbarMode, State};
use twine_style::{BaseDir, Length, Part, Selector, StyleProp};
use twine_testing::EngineHarness;

/// Scrollbar styles like a theme would set: 6 px thick, 2 px from the edges, dark gray, and
/// blue while scrolled.
fn scrollbar_style(e: &mut Engine, id: NodeId) {
    let sb = Selector::part(Part::Scrollbar);
    for p in [
        StyleProp::Width(Length::Px(6)),
        StyleProp::BgColor(Color::hex(0x0055_5555)),
        StyleProp::BgOpa(Opa::COVER),
        StyleProp::Radius(3),
        StyleProp::PadRight(2),
        StyleProp::PadBottom(2),
        StyleProp::PadTop(2),
        StyleProp::PadLeft(2),
    ] {
        e.set_local_prop(id, sb, p);
    }
    e.set_local_prop(
        id,
        sb.with_state(State::SCROLLED),
        StyleProp::BgColor(Color::hex(0x0020_60FF)),
    );
}

/// A 160×140 screen with a 100×100 list at (20, 20): `rows` rows of 40 px, `wide` px wide.
fn scene(rows: usize, wide: i32) -> (EngineHarness, NodeId) {
    let mut ids = None;
    let mut h = EngineHarness::new(160, 140).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let (cont, rows) = list(e, s, Rect::from_xywh(20, 20, 100, 100), rows, 40);
        for r in rows {
            e.set_width(r, wide);
        }
        scrollbar_style(e, cont);
        e.update_layout();
        ids = Some(cont);
    });
    h.run_until_idle();
    (h, ids.unwrap())
}

#[test]
fn no_scrollbar_without_styles() {
    let mut ids = None;
    let mut h = EngineHarness::new(160, 140).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        ids = Some(list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40).0);
    });
    h.run_until_idle();
    assert_eq!(h.engine().scrollbar_areas(ids.unwrap()), (None, None));
}

#[test]
fn scrollbar_length_proportional() {
    let (mut h, cont) = scene(10, 100);
    // Track: 100 − 2 − 2 = 96 px; bar: 96 · 100 / 400 = 24 px; 72 px to travel over 300 px.
    let (hor, ver) = h.engine().scrollbar_areas(cont);
    assert_eq!(hor, None);
    assert_eq!(ver, Some(Rect::new(112, 22, 118, 46)));
    h.engine_mut().scroll_to_y(cont, 150, false);
    let ver = h.engine().scrollbar_areas(cont).1.unwrap();
    assert_eq!(ver, Rect::new(112, 22 + 36, 118, 22 + 36 + 24));
    h.engine_mut().scroll_to_y(cont, 300, false);
    let ver = h.engine().scrollbar_areas(cont).1.unwrap();
    assert_eq!(ver.y1, 120 - 2);
}

#[test]
fn scrollbar_min_length() {
    let (h, cont) = scene(100, 100);
    // 96 · 100 / 4000 = 2 px, raised to LVGL's minimum of 10 px at 160 dpi (8 px at 130 dpi).
    let ver = h.engine().scrollbar_areas(cont).1.unwrap();
    assert_eq!(ver.height(), 8);
}

#[test]
fn scrollbar_auto_vertical() {
    let (mut h, cont) = scene(10, 100);
    h.engine_mut().scroll_to_y(cont, 100, false);
    h.run_until_idle();
    h.assert_snapshot("scrollbar_auto_vertical");
}

#[test]
fn scrollbar_both_axes() {
    let (mut h, cont) = scene(10, 250);
    let (hor, ver) = h.engine().scrollbar_areas(cont);
    // The horizontal bar leaves room for the vertical one and vice versa.
    let (hor, ver) = (hor.unwrap(), ver.unwrap());
    assert_eq!((hor.y0, hor.y1), (120 - 2 - 6, 120 - 2));
    assert!(hor.x1 <= 120 - 2 - 6);
    assert!(ver.y1 <= 120 - 2 - 6);
    h.engine_mut().scroll_to(cont, 75, 150, false);
    h.run_until_idle();
    h.assert_snapshot("scrollbar_both_axes");
}

#[test]
fn scrollbar_rtl() {
    let (mut h, cont) = scene(10, 100);
    style(h.engine_mut(), cont, &[StyleProp::BaseDir(BaseDir::Rtl)]);
    h.run_until_idle();
    let ver = h.engine().scrollbar_areas(cont).1.unwrap();
    assert_eq!((ver.x0, ver.x1), (22, 28), "on the left");
    h.assert_snapshot("scrollbar_rtl");
}

#[test]
fn scrollbar_off() {
    let (mut h, cont) = scene(10, 250);
    h.engine_mut().set_scrollbar_mode(cont, ScrollbarMode::Off);
    assert_eq!(h.engine().scrollbar_areas(cont), (None, None));
    h.run_until_idle();
    h.assert_snapshot("scrollbar_off");
}

#[test]
fn scrollbar_on_even_without_content() {
    let (mut h, cont) = scene(2, 100);
    assert_eq!(
        h.engine().scrollbar_areas(cont).1,
        None,
        "auto: nothing to scroll"
    );
    h.engine_mut().set_scrollbar_mode(cont, ScrollbarMode::On);
    let (hor, ver) = h.engine().scrollbar_areas(cont);
    // The whole track (LVGL), both axes of the default `Dir::ALL`.
    assert!(ver.is_some() && hor.is_some());
}

#[test]
fn scrollbar_active_only_while_scrolling() {
    let (mut h, cont) = scene(10, 100);
    h.engine_mut().set_scrollbar_mode(cont, ScrollbarMode::Active);
    h.run_until_idle();
    assert_eq!(h.engine().scrollbar_areas(cont), (None, None));
    h.press(Point::new(70, 50));
    h.clock().advance(Duration::ms(30));
    h.move_to(Point::new(70, 30));
    let ver = h.engine().scrollbar_areas(cont).1;
    assert!(ver.is_some(), "visible mid-drag");
    h.advance(Duration::ms(40));
    // Drawn in the `SCROLLED` state color.
    let v = ver.unwrap();
    assert!(is_blue(h.pixel(v.x0 as u32 + 3, v.y0 as u32 + 8)));
    h.release();
    h.run_until_idle();
    assert_eq!(
        h.engine().scrollbar_areas(cont),
        (None, None),
        "hidden after ScrollEnd"
    );
    let v = h.engine().scroll_offset(cont);
    // The area where the bar was is redrawn: no bar pixels remain.
    for y in 22..118 {
        let p = h.pixel(115, y);
        assert!(
            !is_blue(p) && !is_gray(p),
            "stale bar at y {y} (offset {v:?}): {p:?}"
        );
    }
}

/// The `SCROLLED` bar color (RGB565 rounding tolerated).
fn is_blue(c: Color) -> bool {
    c.r < 0x30 && c.g.abs_diff(0x60) < 8 && c.b > 0xF0
}

/// The idle bar color.
fn is_gray(c: Color) -> bool {
    c.r.abs_diff(0x55) < 8 && c.g.abs_diff(0x55) < 8 && c.b.abs_diff(0x55) < 8
}

#[test]
fn mode_change_invalidates() {
    let (mut h, cont) = scene(10, 100);
    h.engine_mut().set_scrollbar_mode(cont, ScrollbarMode::On);
    assert!(!h.engine().invalidation_log().is_empty());
    h.run_until_idle();
    h.engine_mut().set_scrollbar_mode(cont, ScrollbarMode::On);
    assert!(h.engine().invalidation_log().is_empty(), "unchanged: nothing");
}

#[test]
fn scrollbar_redrawn_when_content_grows() {
    let (mut h, cont) = scene(4, 100);
    // 160 px of content: a 60 px bar.
    let ver = h.engine().scrollbar_areas(cont).1.unwrap();
    assert_eq!(ver.height(), 60);
    assert!(is_gray(h.pixel(115, 22 + 50)));
    for i in 4..10 {
        twine_testing::scenes::child_box(
            h.engine_mut(),
            cont,
            Rect::from_xywh(0, i * 40, 100, 40),
            &[
                StyleProp::BgColor(Color::hex(0x0030_60C0)),
                StyleProp::BgOpa(Opa::COVER),
            ],
        );
    }
    h.run_until_idle();
    // 400 px of content: 24 px; the rest of the old bar is redrawn.
    assert_eq!(h.engine().scrollbar_areas(cont).1.unwrap().height(), 24);
    assert!(is_gray(h.pixel(115, 22 + 10)));
    assert!(!is_gray(h.pixel(115, 22 + 50)), "stale scrollbar pixels");
}
