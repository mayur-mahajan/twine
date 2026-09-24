//! The `text_input` demo: the on-screen keyboard, the PC keypad and the encoder produce the
//! same text in the model; the preview follows the name; OK hides the keyboard; the form is
//! idle when no field is focused.

use std::rc::Rc;

use twine::core::Duration;
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
    let mut t = TestUi::new(320, 240).mount(app);
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
    let mut t = TestUi::new(320, 240).mount(app);
    t.run_until_idle();
    t.assert_idle();
    let form = t.root_scope().expect_context::<Form>();
    t.find(by_id("notes")).click();
    settle(&mut t);
    assert_eq!(form.editing.get_untracked(), Some(2));
    tap_key(&mut t, "x");
    assert_eq!(form.notes.get_untracked(), "x");
    tap_key(&mut t, symbols::OK);
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
    let mut t = TestUi::new(320, 240).mount(app);
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

#[test]
fn snapshots_keyboard_open() {
    for (name, theme) in [
        ("text_input_keyboard_open_light", DefaultTheme::light()),
        ("text_input_keyboard_open_dark", DefaultTheme::dark()),
    ] {
        let mut t = TestUi::new(320, 240).theme(Rc::new(theme)).mount(app);
        t.run_until_idle();
        let form = t.root_scope().expect_context::<Form>();
        form.name.set(String::from("Ada"));
        t.find(by_id("name")).click();
        t.advance(Duration::ms(100));
        // The cursor blinks (never idle): compare what the panel shows now.
        t.assert_panel_snapshot(name);
    }
}
