//! `List`: a scrollable flex column with text headers and icon + text buttons, keyboard
//! navigation, momentum scrolling and the theme's look.

mod common;

use std::rc::Rc;

use common::{Mode, center, class, count_events, harness, harness_with_group, has_state};
use twine_core::{Duration, Point};
use twine_engine::{EventCode, Key, NodeId, State};
use twine_image::ImageSource;
use twine_style::{Align, FlexFlow, LayoutKind, Part, PropId};
use twine_testing::EngineHarness;
use twine_text::{LongMode, symbols};
use twine_widgets::label::Label;
use twine_widgets_ext::list;

fn scene(mode: Mode, n: usize) -> (EngineHarness, NodeId, Vec<NodeId>) {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let l = list::create(e, screen).unwrap();
    e.set_height(l, 220);
    e.align(l, Align::Center, 0, 0);
    list::add_text(e, l, "File").unwrap();
    let icons = [symbols::FILE, symbols::DIRECTORY, symbols::SAVE, symbols::CLOSE];
    let mut btns = Vec::new();
    for i in 0..n {
        let b = list::add_button(
            e,
            l,
            Some(ImageSource::Symbol(icons[i % icons.len()])),
            &format!("Item {i}"),
        )
        .unwrap();
        btns.push(b);
    }
    h.run_until_idle();
    (h, l, btns)
}

#[test]
fn list_is_scrollable_column() {
    let (h, l, btns) = scene(Mode::Light, 12);
    let e = h.engine();
    assert_eq!(class(&h, l), "list");
    assert_eq!(
        e.style_prop(l, Part::Main, PropId::Layout).get::<LayoutKind>(),
        Some(LayoutKind::Flex)
    );
    assert_eq!(
        e.style_prop(l, Part::Main, PropId::FlexFlow).get::<FlexFlow>(),
        Some(FlexFlow::Column)
    );
    assert_eq!(e.coords(l).width(), 195, "LV_DPI_DEF * 3 / 2");
    // Stacked without gaps (the theme's `pad_gap 0`), each as wide as the content area.
    let cw = e.content_area(l).width();
    for w in btns.windows(2) {
        assert_eq!(e.coords(w[0]).y1, e.coords(w[1]).y0);
    }
    assert_eq!(e.coords(btns[0]).width(), cw);
    assert!(e.scroll_bottom(l) > 0, "12 items overflow");
    let mut h = h;
    h.assert_idle();
}

#[test]
fn list_default_size() {
    let mut h = harness(400, 400, Mode::Light);
    let screen = h.screen();
    let l = list::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let c = h.engine().coords(l);
    assert_eq!((c.width(), c.height()), (195, 260));
}

#[test]
fn list_button_icon_and_text_layout() {
    let (h, _l, btns) = scene(Mode::Light, 2);
    let e = h.engine();
    let b = btns[0];
    assert_eq!(class(&h, b), "list_button");
    let kids: Vec<NodeId> = e.tree().children(b).collect();
    assert_eq!(kids.len(), 2);
    assert_eq!(class(&h, kids[0]), "image");
    assert_eq!(class(&h, kids[1]), "label");
    // A row: the icon, a column gap, then the label taking the rest.
    let (ic, lc) = (e.coords(kids[0]), e.coords(kids[1]));
    let gap = e.style_i32(b, Part::Main, PropId::PadColumn);
    assert_eq!(lc.x0, ic.x1 + gap);
    // Flex grow to the right padding (the bottom-only border takes no width: LVGL's
    // `lv_obj_get_style_space_right`).
    let pad_r = e.style_i32(b, Part::Main, PropId::PadRight);
    assert_eq!(lc.x1, e.coords(b).x1 - pad_r, "flex grow");
    assert_eq!(list::button_text(e, b), Some("Item 0"));
    let l = e.widget::<Label>(kids[1]).unwrap();
    assert_eq!(l.long_mode(), LongMode::ScrollCircular);
    // No icon: just the label.
    let mut h = h;
    let lst = h.engine().tree().parent(b).unwrap();
    let plain = list::add_button(h.engine_mut(), lst, None, "Plain").unwrap();
    assert_eq!(h.engine().tree().children(plain).count(), 1);
    assert!(list::set_button_text(h.engine_mut(), plain, "Renamed"));
    assert_eq!(list::button_text(h.engine(), plain), Some("Renamed"));
}

#[test]
fn list_text_scroll_circular_when_long() {
    let (mut h, l, _) = scene(Mode::Light, 1);
    let t = list::add_text(
        h.engine_mut(),
        l,
        "A very long section header that cannot fit the list",
    )
    .unwrap();
    h.advance(Duration::ms(100));
    assert_eq!(class(&h, t), "list_text");
    let w = h.engine().widget::<Label>(t).unwrap();
    assert_eq!(w.long_mode(), LongMode::ScrollCircular);
    let lc = h.engine().coords(t);
    assert_eq!(
        lc.height(),
        i32::from(h.engine().style_font(t, Part::Main).line_height)
    );
    // The text scrolls (an animation runs): it is not idle while shown.
    let x0 = h.engine().widget::<Label>(t).unwrap().scroll_offset().x;
    h.advance(Duration::ms(1000));
    let x1 = h.engine().widget::<Label>(t).unwrap().scroll_offset().x;
    assert_ne!(x0, x1);
}

#[test]
fn list_button_click() {
    let (mut h, _l, btns) = scene(Mode::Light, 3);
    let clicks = count_events(&mut h, btns[1], EventCode::Clicked);
    h.press(center(&h, btns[1]));
    assert!(has_state(&h, btns[1], State::PRESSED));
    h.release();
    assert_eq!(clicks.get(), 1);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn list_keypad_navigation_scrolls_into_view() {
    let (mut h, l, btns) = scene(Mode::Light, 12);
    let _ = h.keypad_input();
    let g = h.engine().default_group().unwrap();
    assert_eq!(
        h.engine().group_count(g),
        12,
        "list buttons join the default group"
    );
    h.engine_mut().focus(btns[0]);
    for _ in 0..11 {
        h.key(Key::Next);
    }
    h.run_until_idle();
    let last = btns[11];
    assert!(has_state(&h, last, State::FOCUS_KEY));
    let (lc, bc) = (h.engine().content_area(l), h.engine().coords(last));
    assert!(
        bc.y1 <= h.engine().coords(l).y1 && bc.y0 >= lc.y0 - 1,
        "{bc:?} in {lc:?}"
    );
    assert!(h.engine().scroll_offset(l).y > 0);
    // Focused buttons look primary (the theme's `FOCUS_KEY` style).
    assert_eq!(
        h.engine().style_color(last, Part::Main, PropId::BgColor),
        twine_theme::Palette::Blue.main()
    );
    h.assert_idle();
}

#[test]
fn list_200_items_momentum_then_idle() {
    let (mut h, l, _) = scene(Mode::Light, 200);
    let c = center(&h, l);
    h.drag(c, Point::new(c.x, c.y - 150), Duration::ms(100));
    let after_release = h.engine().scroll_offset(l).y;
    h.run_until_idle();
    let settled = h.engine().scroll_offset(l).y;
    assert!(settled > after_release, "momentum: {after_release} -> {settled}");
    h.assert_idle();
}

#[test]
fn list_theme_styles() {
    let (h, l, btns) = scene(Mode::Light, 1);
    let e = h.engine();
    let pad_def = twine_theme::dpx(16, 130);
    assert_eq!(e.style_i32(l, Part::Main, PropId::PadLeft), pad_def);
    assert_eq!(e.style_i32(l, Part::Main, PropId::PadTop), 0);
    assert_eq!(e.style_i32(l, Part::Main, PropId::PadRow), 0);
    let b = btns[0];
    assert_eq!(
        e.style_i32(b, Part::Main, PropId::BorderWidth),
        twine_theme::dpx(1, 130)
    );
    assert_eq!(
        e.style_prop(b, Part::Main, PropId::BorderSide)
            .get::<twine_render::BorderSide>(),
        Some(twine_render::BorderSide::BOTTOM)
    );
    let _ = Rc::new(());
}

#[test]
fn snapshot_list() {
    for m in Mode::ALL {
        let (mut h, _, _) = scene(m, 6);
        h.assert_snapshot(&format!("list_{}", m.suffix()));
    }
    let (mut h, _, btns) = scene(Mode::Light, 6);
    h.press(center(&h, btns[1]));
    h.advance(Duration::ms(200));
    h.assert_panel_snapshot("list_pressed_item");
    h.release();
    h.run_until_idle();
}
