//! `Keyboard`: typing into a textarea, the special keys, mode switching, the number pad's
//! sign key, `Ready` / `Cancel`, a deleted textarea, popovers, encoder typing and the four
//! default layouts in both theme variants.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Duration, Point, Rect};
use twine_engine::{EventCode, EventFilter, EventResult, MeasureCx, NodeId, ObjFlags, State};
use twine_style::Align;
use twine_testing::{EngineHarness, capture_logs};
use twine_text::symbols;
use twine_widgets::buttonmatrix::BtnCtrl;
use twine_widgets::keyboard::{self, KEYBOARD_CLASS, Keyboard, KeyboardMode};
use twine_widgets::textarea::{self, Textarea};

/// A 320 × 240 screen with a textarea at the top and a keyboard attached to it.
fn scene(mode: Mode) -> (EngineHarness, NodeId, NodeId) {
    let mut h = harness(320, 240, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let ta = textarea::create(e, screen).unwrap();
    e.set_size(ta, 300, 60);
    e.align(ta, Align::TopMid, 0, 10);
    let kb = keyboard::create(e, screen).unwrap();
    with(&mut h, kb, |k: &mut Keyboard, cx| k.set_textarea(cx, Some(ta)));
    h.advance(Duration::ms(50));
    (h, ta, kb)
}

/// The center of the key labelled `text`.
fn key(h: &EngineHarness, kb: NodeId, text: &str) -> Point {
    let k = get::<Keyboard>(h, kb);
    let m = k.buttonmatrix();
    let i = (0..m.btn_count())
        .find(|&i| m.btn_text(i) == Some(text))
        .unwrap_or_else(|| panic!("no key {text:?}"));
    let r = m.btn_area(&MeasureCx::new(h.engine(), kb), i).unwrap();
    Point::new((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2)
}

fn tap_key(h: &mut EngineHarness, kb: NodeId, text: &str) {
    let p = key(h, kb, text);
    h.tap(p);
    h.advance(Duration::ms(20));
}

fn text(h: &EngineHarness, ta: NodeId) -> String {
    textarea::text_of(h.engine(), ta).unwrap().to_owned()
}

fn count(h: &mut EngineHarness, n: NodeId, code: EventCode) -> Rc<RefCell<u32>> {
    let c = Rc::new(RefCell::new(0));
    let c2 = c.clone();
    h.engine_mut()
        .add_event_handler(n, EventFilter::Code(code), move |_, _| {
            *c2.borrow_mut() += 1;
            EventResult::Continue
        });
    c
}

#[test]
fn kb_defaults() {
    let (h, ta, kb) = scene(Mode::Light);
    let n = h.engine().tree().node(kb).unwrap();
    assert_eq!(n.class().name, "keyboard");
    assert!(!KEYBOARD_CLASS.default_flags.contains(ObjFlags::CLICK_FOCUSABLE));
    // Full width, half height, at the bottom (LVGL).
    let c = h.engine().coords(kb);
    assert_eq!((c.x0, c.width(), c.height(), c.y1), (0, 320, 120, 240));
    let k = get::<Keyboard>(&h, kb);
    assert_eq!(k.mode(), KeyboardMode::TextLower);
    assert!(!k.popovers());
    assert_eq!(k.textarea(), Some(ta));
    assert_eq!(k.buttonmatrix().btn_count(), 40);
    assert_eq!(k.buttonmatrix().row_count(), 4);
    // Attaching focuses the textarea (its cursor shows).
    assert!(
        h.engine()
            .tree()
            .node(ta)
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
    // No popover flags while popovers are off.
    assert!(!k.buttonmatrix().has_btn_ctrl(1, BtnCtrl::POPOVER));
}

#[test]
fn kb_types_into_textarea() {
    let (mut h, ta, kb) = scene(Mode::Light);
    for k in ["h", "e", "y", " ", "."] {
        tap_key(&mut h, kb, k);
    }
    assert_eq!(text(&h, ta), "hey .");
    tap_key(&mut h, kb, symbols::LEFT);
    tap_key(&mut h, kb, "x");
    assert_eq!(text(&h, ta), "hey x.");
    tap_key(&mut h, kb, symbols::RIGHT);
    tap_key(&mut h, kb, symbols::NEW_LINE);
    assert_eq!(text(&h, ta), "hey x.\n");
}

#[test]
fn kb_backspace_deletes() {
    let (mut h, ta, kb) = scene(Mode::Light);
    with(&mut h, ta, |t: &mut Textarea, cx| t.set_text(cx, "abc"));
    tap_key(&mut h, kb, symbols::BACKSPACE);
    assert_eq!(text(&h, ta), "ab");
    // Backspace (a checked key without NO_REPEAT) repeats while held.
    let p = key(&h, kb, symbols::BACKSPACE);
    h.press(p);
    h.advance(Duration::ms(520));
    h.release();
    assert_eq!(text(&h, ta), "", "long press repeats backspace");
}

#[test]
fn kb_mode_switch_maps() {
    static MAP: [&str; 3] = ["A", "\n", "B"];
    static CTRL: [BtnCtrl; 2] = [BtnCtrl::empty(), BtnCtrl::CHECKED];
    let (mut h, _ta, kb) = scene(Mode::Light);
    tap_key(&mut h, kb, keyboard::MODE_TEXT_UPPER);
    let k = get::<Keyboard>(&h, kb);
    assert_eq!(k.mode(), KeyboardMode::TextUpper);
    assert_eq!(k.buttonmatrix().btn_text(1), Some("Q"));
    tap_key(&mut h, kb, keyboard::MODE_SPECIAL);
    let k = get::<Keyboard>(&h, kb);
    assert_eq!(k.mode(), KeyboardMode::Special);
    assert_eq!(k.buttonmatrix().btn_text(0), Some("1"));
    assert_eq!(k.buttonmatrix().btn_count(), 40);
    tap_key(&mut h, kb, keyboard::MODE_TEXT_LOWER);
    assert_eq!(get::<Keyboard>(&h, kb).mode(), KeyboardMode::TextLower);
    with(&mut h, kb, |k: &mut Keyboard, cx| {
        k.set_mode(cx, KeyboardMode::Number);
    });
    let m = get::<Keyboard>(&h, kb).buttonmatrix();
    assert_eq!((m.btn_count(), m.row_count()), (17, 4));
    // A custom map for a user mode.
    with(&mut h, kb, |k: &mut Keyboard, cx| {
        k.set_map(cx, KeyboardMode::User1, &MAP, &CTRL);
        k.set_mode(cx, KeyboardMode::User1);
    });
    let m = get::<Keyboard>(&h, kb).buttonmatrix();
    assert_eq!(m.btn_text(1), Some("B"));
    assert!(m.has_btn_ctrl(1, BtnCtrl::CHECKED));
}

#[test]
fn kb_number_mode_sign_toggle() {
    let (mut h, ta, kb) = scene(Mode::Light);
    with(&mut h, kb, |k: &mut Keyboard, cx| {
        k.set_mode(cx, KeyboardMode::Number);
    });
    h.advance(Duration::ms(20));
    tap_key(&mut h, kb, "4");
    tap_key(&mut h, kb, "2");
    tap_key(&mut h, kb, "+/-");
    assert_eq!(text(&h, ta), "-42");
    assert_eq!(
        get::<Textarea>(&h, ta).cursor_pos(),
        3,
        "the cursor stays after the digits"
    );
    tap_key(&mut h, kb, "+/-");
    assert_eq!(text(&h, ta), "+42");
    tap_key(&mut h, kb, "+/-");
    assert_eq!(text(&h, ta), "-42");
}

#[test]
fn kb_ok_sends_ready() {
    let (mut h, ta, kb) = scene(Mode::Light);
    let (on_kb, on_ta) = (
        count(&mut h, kb, EventCode::Ready),
        count(&mut h, ta, EventCode::Ready),
    );
    tap_key(&mut h, kb, symbols::OK);
    assert_eq!((*on_kb.borrow(), *on_ta.borrow()), (1, 1));
    // New line in a one-line textarea: Ready too.
    with(&mut h, ta, |t: &mut Textarea, cx| t.set_one_line(cx, true));
    tap_key(&mut h, kb, symbols::NEW_LINE);
    assert_eq!(*on_ta.borrow(), 2);
    assert_eq!(text(&h, ta), "");
}

#[test]
fn kb_close_sends_cancel() {
    let (mut h, ta, kb) = scene(Mode::Light);
    let (on_kb, on_ta) = (
        count(&mut h, kb, EventCode::Cancel),
        count(&mut h, ta, EventCode::Cancel),
    );
    tap_key(&mut h, kb, symbols::KEYBOARD);
    assert_eq!((*on_kb.borrow(), *on_ta.borrow()), (1, 1));
    tap_key(&mut h, kb, keyboard::MODE_TEXT_UPPER);
    tap_key(&mut h, kb, symbols::CLOSE);
    assert_eq!((*on_kb.borrow(), *on_ta.borrow()), (2, 2));
}

#[test]
fn kb_dead_textarea_warns_and_clears() {
    let (mut h, ta, kb) = scene(Mode::Light);
    h.engine_mut().delete(ta).unwrap();
    let p = key(&h, kb, "q");
    let ((), logs) = capture_logs(|| {
        h.tap(p);
    });
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("textarea")),
        "{logs:?}"
    );
    assert_eq!(get::<Keyboard>(&h, kb).textarea(), None);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn kb_popover_area_invalidated() {
    let (mut h, ta, kb) = scene(Mode::Light);
    with(&mut h, kb, |k: &mut Keyboard, cx| k.set_popovers(cx, true));
    h.advance(Duration::ms(20));
    assert!(
        get::<Keyboard>(&h, kb)
            .buttonmatrix()
            .has_btn_ctrl(1, BtnCtrl::POPOVER)
    );
    let q = key(&h, kb, "q");
    h.press(q);
    assert_eq!(text(&h, ta), "", "popover keys type on release");
    h.advance(Duration::ms(20));
    let p = get::<Keyboard>(&h, kb)
        .buttonmatrix()
        .popover_node()
        .expect("popover");
    let pc = h.engine().coords(p);
    assert!(pc.contains(Point::new(q.x, q.y)) && pc.y0 < h.engine().coords(kb).y0 + 2);
    h.release();
    // The popover's area is redrawn when it hides.
    let inv: Vec<Rect> = h
        .invalidations()
        .iter()
        .chain(h.engine().invalidation_log())
        .map(|(r, _)| *r)
        .collect();
    assert!(inv.iter().any(|r| r.contains_rect(&pc)), "{inv:?} vs {pc:?}");
    assert_eq!(text(&h, ta), "q");
    assert!(h.engine().has_flag(p, ObjFlags::HIDDEN));
    h.advance(Duration::ms(20));
}

#[test]
fn kb_encoder_types_char() {
    let (mut h, ta, kb) = scene(Mode::Light);
    let g = h.engine().group_of(kb).unwrap();
    let _ = h.encoder_input();
    h.engine_mut().focus(kb);
    h.encoder_click(); // edit mode, first key selected
    assert!(h.engine().group_editing(g));
    assert_eq!(get::<Keyboard>(&h, kb).buttonmatrix().selected_btn(), Some(0));
    h.encoder(2); // "w"
    h.encoder_click();
    assert_eq!(text(&h, ta), "w");
    h.advance(Duration::ms(20));
}

#[test]
fn snapshot_kb_layouts() {
    let modes = [
        ("lower", KeyboardMode::TextLower),
        ("upper", KeyboardMode::TextUpper),
        ("special", KeyboardMode::Special),
        ("number", KeyboardMode::Number),
    ];
    for m in Mode::ALL {
        for (name, km) in modes {
            let mut h = harness(320, 240, m);
            let screen = h.screen();
            let kb = keyboard::create(h.engine_mut(), screen).unwrap();
            with(&mut h, kb, |k: &mut Keyboard, cx| k.set_mode(cx, km));
            h.run_until_idle();
            h.assert_snapshot(&format!("kb_{name}_{}", m.suffix()));
        }
    }
}

#[test]
fn snapshot_kb_popover_pressed() {
    let (mut h, _ta, kb) = scene(Mode::Light);
    with(&mut h, kb, |k: &mut Keyboard, cx| k.set_popovers(cx, true));
    h.advance(Duration::ms(20));
    let p = key(&h, kb, "g");
    h.press(p);
    // The textarea cursor blinks: redraw everything and capture the panel at once.
    h.engine_mut().invalidate_all();
    h.advance(Duration::ms(40));
    h.assert_panel_snapshot("kb_popover_pressed");
    h.release();
}

#[test]
fn kb_delete_releases_textarea() {
    let (mut h, ta, kb) = scene(Mode::Light);
    // The group's focus is elsewhere: the textarea is `FOCUSED` only through the keyboard.
    let screen = h.screen();
    let other = twine_widgets::button::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().focus(other);
    with(&mut h, kb, |k: &mut Keyboard, cx| {
        k.set_textarea(cx, None);
        k.set_textarea(cx, Some(ta));
    });
    let blinking = |h: &EngineHarness| get::<Textarea>(h, ta).is_blinking(&MeasureCx::new(h.engine(), ta));
    assert!(
        h.engine()
            .tree()
            .node(ta)
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
    assert!(blinking(&h));
    h.engine_mut().delete(kb).unwrap();
    assert!(
        !h.engine()
            .tree()
            .node(ta)
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
    assert!(!blinking(&h), "the cursor stops blinking");
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn kb_delete_keeps_group_focused_textarea() {
    let (mut h, ta, kb) = scene(Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().group_add(g, ta);
    h.engine_mut().focus(ta);
    h.engine_mut().delete(kb).unwrap();
    assert!(
        h.engine()
            .tree()
            .node(ta)
            .unwrap()
            .state()
            .contains(State::FOCUSED)
    );
}
