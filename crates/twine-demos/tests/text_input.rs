//! The `text_input` demo: the on-screen keyboard, the PC keypad and the encoder produce the
//! same text in the model; the preview follows the name; OK hides the keyboard; the form is
//! idle when no field is focused. Keyboard avoidance (R4.S05): the docked keyboard keeps the
//! edited field visible (also near the bottom and after a rotation), gives the area back when
//! closed, and moving it to another field lays nothing out.

use twine::core::{Duration, Rect, Rotation};
use twine::engine::NodeId;
use twine::prelude::*;
use twine::widgets::keyboard::Keyboard;
use twine::widgets::spangroup::SpanGroup;
use twine_demos::text_input::{Form, app};
use twine_testing::{TestUi, by_class, by_id};

/// A focused field blinks: let some frames pass instead of waiting for idle.
fn settle(t: &mut TestUi) {
    t.advance(Duration::ms(100));
}

fn keyboard(t: &TestUi) -> NodeId {
    t.find(by_class("keyboard")).id()
}

/// The index of the key labelled `key` on the keyboard's current map.
fn key_index(t: &TestUi, key: &str) -> u16 {
    let kb = keyboard(t);
    let e = t.engine();
    let m = e.widget::<Keyboard>(kb).unwrap().buttonmatrix();
    (0..m.btn_count())
        .find(|&i| m.btn_text(i) == Some(key))
        .unwrap_or_else(|| panic!("no key {key:?}"))
}

/// Taps the on-screen key labelled `key`.
fn tap_key(t: &mut TestUi, key: &str) {
    let kb = keyboard(t);
    let idx = key_index(t, key);
    let area = {
        let e = t.engine();
        let m = e.widget::<Keyboard>(kb).unwrap().buttonmatrix();
        m.btn_area(&MeasureCx::new(&e, kb), idx).unwrap()
    };
    t.tap(area.center());
    settle(t);
}

/// Types `key` with the encoder: turn to the key (the keyboard is being edited), click.
fn encoder_key(t: &mut TestUi, key: &str) {
    let kb = keyboard(t);
    let target = i32::from(key_index(t, key));
    let cur = {
        let e = t.engine();
        e.widget::<Keyboard>(kb)
            .unwrap()
            .buttonmatrix()
            .selected_btn()
            .map_or(0, i32::from)
    };
    let diff = i16::try_from(target - cur).unwrap();
    if diff != 0 {
        t.encoder(diff);
    }
    t.encoder_click();
    settle(t);
}

fn preview(t: &TestUi) -> String {
    let n = t.find(by_id("preview")).id();
    let e = t.engine();
    let g = e.widget::<SpanGroup>(n).unwrap();
    g.span_ids()
        .map(|id| g.span(id).unwrap().text().to_string())
        .collect()
}

#[test]
fn keyboard_keypad_and_encoder_type_the_same() {
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    let form = t.root_scope().expect_context::<Form>();

    // On-screen keyboard.
    t.find(by_id("name")).click();
    settle(&mut t);
    assert_eq!(form.editing.get_untracked(), Some(0));
    tap_key(&mut t, "a");
    tap_key(&mut t, "b");
    assert_eq!(form.name.get_untracked(), "ab");
    assert_eq!(preview(&t), "Hello, ab!");

    // PC keypad: the field has the input focus.
    form.name.set(String::new());
    settle(&mut t);
    t.type_text("ab");
    settle(&mut t);
    assert_eq!(form.name.get_untracked(), "ab");

    // Encoder: focus the keyboard, click to edit, turn to the key, click to type.
    form.name.set(String::new());
    settle(&mut t);
    let kb = keyboard(&t);
    t.engine_mut().focus(kb);
    settle(&mut t);
    t.encoder_click(); // edit mode
    settle(&mut t);
    encoder_key(&mut t, "a");
    encoder_key(&mut t, "b");
    assert_eq!(form.name.get_untracked(), "ab");
    assert_eq!(preview(&t), "Hello, ab!");
}

#[test]
fn ok_hides_the_keyboard_and_the_form_idles() {
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    t.assert_idle();
    let form = t.root_scope().expect_context::<Form>();
    t.find(by_id("notes")).click();
    settle(&mut t);
    assert_eq!(form.editing.get_untracked(), Some(2));
    tap_key(&mut t, "x");
    assert_eq!(form.notes.get_untracked(), "x");
    tap_key(&mut t, Symbol::Ok.as_str());
    assert!(t.find_all(by_class("keyboard")).is_empty());
    assert_eq!(form.editing.get_untracked(), None);
    // Move the input focus off the fields: nothing blinks, the form is idle.
    let clear = t.find(by_id("clear")).id();
    t.engine_mut().focus(clear);
    t.run_until_idle();
    t.assert_idle();
}

#[test]
fn spinbox_buttons_change_the_quantity() {
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    let form = t.root_scope().expect_context::<Form>();
    let page = t.find(by_id("page")).id();
    t.engine_mut().scroll_to_y(page, 200, false);
    t.run_until_idle();
    t.find(by_id("plus")).click();
    t.find(by_id("plus")).click();
    t.find(by_id("minus")).click();
    t.run_until_idle();
    assert_eq!(form.quantity.get_untracked(), 2);
    t.assert_idle();
}

/// The rectangles of the keyboard and the page, and whether `field` is fully visible in the
/// page above the keyboard.
fn field_visible(t: &TestUi, field: &'static str) -> bool {
    let e = t.engine();
    let kb = e.coords(keyboard(t));
    let page = e.coords(t.find(by_id("page")).id());
    let f = e.coords(t.find(by_id(field)).id());
    f.y0 >= page.y0 && f.y1 <= page.y1 && f.y1 <= kb.y0
}

fn page_rect(t: &TestUi) -> Rect {
    t.engine().coords(t.find(by_id("page")).id())
}

#[test]
fn docked_keyboard_keeps_a_field_near_the_bottom_visible() {
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    let full = page_rect(&t);
    // The notes are below the fold once the keyboard takes the bottom half.
    t.find(by_id("notes")).click();
    settle(&mut t);
    let kb = t.engine().coords(keyboard(&t));
    assert_eq!((kb.y0, kb.y1, kb.width()), (120, 240, 320));
    // The page shrank to the area above the keyboard and scrolled the field into view.
    assert_eq!(page_rect(&t), Rect::new(full.x0, full.y0, full.x1, kb.y0));
    assert!(field_visible(&t, "notes"));
    let page = t.find(by_id("page")).id();
    assert!(t.engine().scroll_offset(page).y > 0);
    // Typing still goes to the field.
    tap_key(&mut t, "x");
    assert_eq!(t.root_scope().expect_context::<Form>().notes.get_untracked(), "x");
}

#[test]
fn closing_the_keyboard_restores_the_layout() {
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    let full = page_rect(&t);
    let name = t.engine().coords(t.find(by_id("name")).id());
    t.find(by_id("notes")).click();
    settle(&mut t);
    assert_ne!(page_rect(&t), full);
    tap_key(&mut t, Symbol::Ok.as_str());
    assert!(t.find_all(by_class("keyboard")).is_empty());
    settle(&mut t);
    // The page has its whole area again; the content was scrolled back as far as it fits.
    assert_eq!(page_rect(&t), full);
    let page = t.find(by_id("page")).id();
    let e = t.engine();
    assert!(e.scroll_bottom(page) >= 0, "scrolled beyond the end");
    let off = e.scroll_offset(page).y;
    assert_eq!(e.coords(t.find(by_id("name")).id()).y0, name.y0 - off);
}

#[test]
fn moving_the_keyboard_to_another_field_lays_nothing_out() {
    let mut t = TestUi::new(320, 240).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    let form = t.root_scope().expect_context::<Form>();
    t.find(by_id("name")).click();
    settle(&mut t);
    assert!(field_visible(&t, "name"));
    let kb = keyboard(&t);
    let passes = t.engine().layout_passes();
    // Another field: the same keyboard follows it, the field is scrolled into view, and no
    // layout pass runs (scrolling moves nodes without a layout).
    form.editing.set(Some(2));
    settle(&mut t);
    assert_eq!(keyboard(&t), kb, "the keyboard was rebuilt");
    let e = t.engine();
    let ta = e.widget::<Keyboard>(kb).unwrap().textarea();
    assert_eq!(ta, Some(t.find(by_id("notes")).id()));
    drop(e);
    assert!(field_visible(&t, "notes"));
    assert_eq!(t.engine().layout_passes(), passes, "focus move caused a layout");
    // And back by tapping the password field (partly visible above the keyboard).
    let page = t.find(by_id("page")).id();
    t.engine_mut().scroll_to_y(page, 0, false);
    settle(&mut t);
    let passes = t.engine().layout_passes();
    t.find(by_id("password")).click();
    settle(&mut t);
    assert_eq!(form.editing.get_untracked(), Some(1));
    assert!(field_visible(&t, "password"));
    assert_eq!(
        t.engine().layout_passes(),
        passes,
        "tap on another field caused a layout"
    );
}

#[test]
fn rotation_while_open_keeps_the_field_visible() {
    // A portrait panel turned to landscape while the keyboard is open: the page loses height.
    let mut t = TestUi::new(240, 320).app_config(twine_demos::config()).mount(app);
    t.run_until_idle();
    t.find(by_id("notes")).click();
    settle(&mut t);
    assert!(field_visible(&t, "notes"));
    let d = t.engine().default_display().unwrap();
    t.engine_mut().reserve_rotation(d).unwrap();
    t.engine_mut().set_rotation(d, Rotation::Deg90).unwrap();
    settle(&mut t);
    let kb = t.engine().coords(keyboard(&t));
    assert_eq!((kb.y0, kb.y1, kb.width()), (120, 240, 320));
    assert_eq!(page_rect(&t).y1, 120);
    assert!(field_visible(&t, "notes"));
    // Back to portrait.
    t.engine_mut().set_rotation(d, Rotation::Deg0).unwrap();
    settle(&mut t);
    assert_eq!(t.engine().coords(keyboard(&t)).y0, 200);
    assert!(field_visible(&t, "notes"));
}

#[test]
fn snapshots_keyboard_open() {
    for (name, mode) in [
        ("text_input_keyboard_open_light", ThemeMode::Light),
        ("text_input_keyboard_open_dark", ThemeMode::Dark),
    ] {
        let mut t = TestUi::new(320, 240).theme(&lvgl::THEME).mode(mode).mount(app);
        t.run_until_idle();
        let form = t.root_scope().expect_context::<Form>();
        form.name.set(String::from("Ada"));
        t.find(by_id("name")).click();
        t.advance(Duration::ms(100));
        // The cursor blinks (never idle): compare what the panel shows now.
        t.assert_panel_snapshot(name);
    }
}
