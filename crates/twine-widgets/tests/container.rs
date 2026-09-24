//! `Container` (the base object): LVGL flags, content sizing, the default theme's card look
//! and the draw descriptors built from styles.

mod common;

use common::{Mode, harness};
use twine_core::{Color, Opa, Point};
use twine_engine::{MeasureCx, OBJ_CLASS, ObjFlags};
use twine_render::BorderSide;
use twine_style::{Align, Part, Selector, State, StyleProp};
use twine_widgets::container::{self, CONTAINER_CLASS};
use twine_widgets::label;

#[test]
fn container_defaults_match_lvgl_flags() {
    let expected = ObjFlags::CLICKABLE
        | ObjFlags::CLICK_FOCUSABLE
        | ObjFlags::SCROLLABLE
        | ObjFlags::SCROLL_ELASTIC
        | ObjFlags::SCROLL_MOMENTUM
        | ObjFlags::SCROLL_WITH_ARROW
        | ObjFlags::SCROLL_CHAIN
        | ObjFlags::SNAPPABLE
        | ObjFlags::GESTURE_BUBBLE
        | ObjFlags::PRESS_LOCK
        | ObjFlags::SCROLL_ON_FOCUS;
    assert_eq!(CONTAINER_CLASS.default_flags, expected);
    assert!(core::ptr::eq(&raw const CONTAINER_CLASS, &raw const OBJ_CLASS));
    assert_eq!(CONTAINER_CLASS.name, "obj");
    assert_eq!(CONTAINER_CLASS.parts, &[Part::Main, Part::Scrollbar]);
    let mut h = harness(100, 80, Mode::Light);
    let screen = h.screen();
    let c = container::create(h.engine_mut(), screen).unwrap();
    assert_eq!(h.engine().tree().node(c).unwrap().flags(), expected);
    assert_eq!(
        h.engine().style_prop(c, Part::Main, twine_style::PropId::Width),
        twine_style::StyleValue::Length(twine_style::Length::Content)
    );
}

#[test]
fn container_content_size_wraps_children() {
    let mut h = harness(200, 160, Mode::Light);
    let screen = h.screen();
    let e = h.engine_mut();
    let c = container::create(e, screen).unwrap();
    let child = container::create(e, c).unwrap();
    e.set_pos(child, 5, 7);
    e.set_size(child, 50, 30);
    h.run_until_idle();
    // Card: padding LV_DPX_CALC(130, 16) = 13 and border 2 on every side (small display).
    let r = h.engine().coords(c);
    assert_eq!((r.width(), r.height()), (5 + 50 + 2 * 15, 7 + 30 + 2 * 15));
    h.assert_idle();
}

#[test]
fn container_press_and_release_then_idle() {
    let mut h = harness(200, 160, Mode::Light);
    let screen = h.screen();
    let c = container::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().set_size(c, 100, 60);
    h.run_until_idle();
    h.press(Point::new(20, 20));
    assert!(
        h.engine()
            .tree()
            .node(c)
            .unwrap()
            .state()
            .contains(State::PRESSED)
    );
    h.release();
    assert!(
        !h.engine()
            .tree()
            .node(c)
            .unwrap()
            .state()
            .contains(State::PRESSED)
    );
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn init_draw_rect_dsc_resolves_all_props() {
    let mut h = harness(100, 80, Mode::Light);
    let screen = h.screen();
    let c = container::create(h.engine_mut(), screen).unwrap();
    let m = Selector::MAIN;
    let e = h.engine_mut();
    for p in [
        StyleProp::BgColor(Color::hex(0x0001_0203)),
        StyleProp::BgOpa(Opa(201)),
        StyleProp::Radius(9),
        StyleProp::BorderColor(Color::hex(0x0004_0506)),
        StyleProp::BorderWidth(3),
        StyleProp::BorderOpa(Opa(202)),
        StyleProp::BorderSide(BorderSide::TOP | BorderSide::LEFT),
        StyleProp::BorderPost(false),
        StyleProp::OutlineColor(Color::hex(0x0007_0809)),
        StyleProp::OutlineWidth(4),
        StyleProp::OutlineOpa(Opa(203)),
        StyleProp::OutlinePad(5),
        StyleProp::ShadowWidth(6),
        StyleProp::ShadowOffsetX(7),
        StyleProp::ShadowOffsetY(8),
        StyleProp::ShadowSpread(10),
        StyleProp::ShadowColor(Color::hex(0x000A_0B0C)),
        StyleProp::ShadowOpa(Opa(204)),
    ] {
        e.set_local_prop(c, m, p);
    }
    let d = MeasureCx::new(h.engine(), c).rect_dsc(Part::Main).base;
    assert_eq!(d.bg_color, Color::hex(0x0001_0203));
    assert_eq!(d.bg_opa, Opa(201));
    assert_eq!(d.radius, 9);
    assert_eq!(d.border_color, Color::hex(0x0004_0506));
    assert_eq!(d.border_width, 3);
    assert_eq!(d.border_opa, Opa(202));
    assert_eq!(d.border_side, BorderSide::TOP | BorderSide::LEFT);
    assert!(!d.border_post);
    assert_eq!(d.outline_color, Color::hex(0x0007_0809));
    assert_eq!(d.outline_width, 4);
    assert_eq!(d.outline_opa, Opa(203));
    assert_eq!(d.outline_pad, 5);
    assert_eq!(d.shadow.width, 6);
    assert_eq!(d.shadow.ofs_x, 7);
    assert_eq!(d.shadow.ofs_y, 8);
    assert_eq!(d.shadow.spread, 10);
    assert_eq!(d.shadow.color, Color::hex(0x000A_0B0C));
    assert_eq!(d.shadow.opa, Opa(204));
    // The style `Recolor` applies to every color (LVGL 9.3+).
    h.engine_mut()
        .set_local_prop(c, m, StyleProp::Recolor(Color::BLACK));
    h.engine_mut()
        .set_local_prop(c, m, StyleProp::RecolorOpa(Opa::COVER));
    let d = MeasureCx::new(h.engine(), c).rect_dsc(Part::Main).base;
    assert_eq!(d.bg_color, Color::BLACK);
    assert_eq!(d.border_color, Color::BLACK);
}

#[test]
fn snapshot_container_card() {
    for m in Mode::ALL {
        let mut h = harness(240, 160, m);
        let screen = h.screen();
        let e = h.engine_mut();
        let c = container::create(e, screen).unwrap();
        e.set_size(c, 200, 120);
        e.align(c, Align::Center, 0, 0);
        let l = label::create_with(e, c, "A card").unwrap();
        e.align(l, Align::TopLeft, 0, 0);
        h.assert_snapshot(&format!("container_card_{}", m.suffix()));
    }
}
