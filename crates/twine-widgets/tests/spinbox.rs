//! `Spinbox`: LVGL's value formatting (leading zeros, sign, decimal point), digit stepping,
//! rollover and clamping, keypad / encoder / pointer control, events and the theme's look.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Duration, Point};
use twine_engine::{EventCode, EventFilter, EventParam, EventResult, Key, MeasureCx, NodeId, State};
use twine_style::{Align, Dir, Part, PropId};
use twine_testing::EngineHarness;
use twine_widgets::label::Label;
use twine_widgets::spinbox::{self, SPINBOX_CLASS, Spinbox, format_value};
use twine_widgets::textarea;

fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 100, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let b = twine_widgets::button::create(e, screen).unwrap();
    e.set_size(b, 10, 10);
    e.align(b, Align::BottomRight, 0, 0);
    let s = spinbox::create(e, screen).unwrap();
    e.align(s, Align::Center, 0, 0);
    h.run_until_idle();
    (h, s)
}

fn text(h: &EngineHarness, s: NodeId) -> String {
    textarea::text_of(h.engine(), s).unwrap().to_owned()
}

fn value(h: &EngineHarness, s: NodeId) -> i32 {
    get::<Spinbox>(h, s).value()
}

fn changes(h: &mut EngineHarness, s: NodeId) -> Rc<RefCell<Vec<i32>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    h.engine_mut()
        .add_event_handler(s, EventFilter::Code(EventCode::ValueChanged), move |_, ev| {
            let EventParam::Value(v) = ev.param else {
                panic!("no value: {:?}", ev.param);
            };
            l.borrow_mut().push(v);
            EventResult::Continue
        });
    log
}

#[test]
fn spinbox_defaults() {
    let (h, s) = scene(Mode::Light);
    assert_eq!(h.engine().tree().node(s).unwrap().class().name, "spinbox");
    assert_eq!(SPINBOX_CLASS.parts[3], Part::Cursor);
    let w = get::<Spinbox>(&h, s);
    assert_eq!(w.value(), 0);
    assert_eq!(w.range(), (-99_999, 99_999));
    assert_eq!((w.digit_count(), w.dec_point_pos(), w.step()), (5, 0, 1));
    assert!(!w.rollover());
    assert_eq!(w.digit_step_direction(), Dir::RIGHT);
    assert_eq!(text(&h, s), "+00000");
    // The cursor on the last digit, the width LVGL's `LV_DPI_DEF`, one line high.
    assert_eq!(w.textarea().cursor_pos(), 5);
    assert!(w.textarea().one_line());
    assert_eq!(h.engine().coords(s).width(), 130);
    let mut h = h;
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn spinbox_format_leading_zeros_sign_decimal() {
    let mut buf = [0u8; 16];
    for digits in 1u8..=10 {
        let max = if digits == 10 {
            i32::MAX
        } else {
            10i32.pow(u32::from(digits)) - 1
        };
        let v = max / 3; // e.g. 333, fewer digits than `digits` for small counts
        let n = v.to_string().len();
        let zeros = "0".repeat(usize::from(digits) - n);
        for signed in [false, true] {
            for sep in [0u8, 1] {
                if sep >= digits {
                    continue;
                }
                for neg in [false, true] {
                    if neg && !signed {
                        continue;
                    }
                    let value = if neg { -v } else { v };
                    let (txt, _) = format_value(&mut buf, value, digits, sep, signed, 1);
                    let mut expected = String::new();
                    if signed {
                        expected.push(if neg { '-' } else { '+' });
                    }
                    let body = format!("{zeros}{v}");
                    if sep == 0 {
                        expected.push_str(&body);
                    } else {
                        expected.push_str(&body[..usize::from(sep)]);
                        expected.push('.');
                        expected.push_str(&body[usize::from(sep)..]);
                    }
                    assert_eq!(
                        txt, expected,
                        "{digits} digits, sep {sep}, signed {signed}, {value}"
                    );
                }
            }
        }
    }
    // The cursor position of each step: on the digit, skipping the point and the sign.
    assert_eq!(format_value(&mut buf, 0, 5, 3, true, 1).1, 6);
    assert_eq!(format_value(&mut buf, 0, 5, 3, true, 100).1, 3);
    assert_eq!(format_value(&mut buf, 0, 5, 3, true, 1000).1, 2);
    assert_eq!(format_value(&mut buf, 0, 5, 0, false, 10000).1, 0);
}

#[test]
fn spinbox_step_digits() {
    let (mut h, s) = scene(Mode::Light);
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.set_step(cx, 100);
        w.increment(cx);
    });
    assert_eq!(value(&h, s), 100);
    assert_eq!(get::<Spinbox>(&h, s).textarea().cursor_pos(), 3, "+00|1|00");
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.step_next(cx);
        w.increment(cx);
        w.step_prev(cx);
        w.step_prev(cx);
        w.decrement(cx);
    });
    assert_eq!(get::<Spinbox>(&h, s).step(), 1000);
    // Crossing zero keeps the digits below the step (LVGL: 110 - 1000 = -110, like 3 - 10 = -3).
    assert_eq!(value(&h, s), -110);
    assert_eq!(text(&h, s), "-00110");
    with(&mut h, s, |w: &mut Spinbox, cx| w.increment(cx));
    assert_eq!(value(&h, s), 110);
    // step_prev stops at the range.
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.set_range(cx, 0, 500);
        w.set_step(cx, 100);
        w.step_prev(cx);
    });
    assert_eq!(get::<Spinbox>(&h, s).step(), 100, "1000 > 500");
    with(&mut h, s, |w: &mut Spinbox, cx| w.set_cursor_pos(cx, 0));
    assert_eq!(get::<Spinbox>(&h, s).step(), 1);
}

#[test]
fn spinbox_rollover() {
    let (mut h, s) = scene(Mode::Light);
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.set_range(cx, -10, 10);
        w.set_rollover(cx, true);
        w.set_value(cx, 10);
        w.increment(cx);
    });
    assert_eq!(value(&h, s), -10);
    with(&mut h, s, |w: &mut Spinbox, cx| w.decrement(cx));
    assert_eq!(value(&h, s), 10);
    // Not at the end yet: clamps first (LVGL).
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.set_value(cx, 8);
        w.set_step(cx, 10);
        w.increment(cx);
    });
    assert_eq!(value(&h, s), 10);
}

#[test]
fn spinbox_clamp_without_rollover() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.set_range(cx, 0, 20);
        w.set_value(cx, 50);
    });
    assert_eq!(value(&h, s), 20, "set_value clamps");
    assert_eq!(text(&h, s), "00020", "no sign without negative values");
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.increment(cx);
        w.set_value(cx, 0);
        w.decrement(cx);
    });
    assert_eq!(value(&h, s), 0);
    assert!(log.borrow().is_empty(), "no change, no event: {:?}", log.borrow());
    // The digit format shrinks the range.
    with(&mut h, s, |w: &mut Spinbox, cx| {
        w.set_range(cx, -5000, 5000);
        w.set_digit_format(cx, 3, 1);
    });
    assert_eq!(get::<Spinbox>(&h, s).range(), (-999, 999));
    assert_eq!(text(&h, s), "+0.00");
}

#[test]
fn spinbox_keypad() {
    let (mut h, s) = scene(Mode::Light);
    let log = changes(&mut h, s);
    let _ = h.keypad_input();
    h.engine_mut().focus(s);
    h.key(Key::Up);
    h.key(Key::Up);
    assert_eq!(value(&h, s), 2);
    h.key(Key::Left); // step 10
    h.key(Key::Up);
    assert_eq!(value(&h, s), 12);
    h.key(Key::Right); // step 1
    h.key(Key::Down);
    assert_eq!(value(&h, s), 11);
    assert_eq!(*log.borrow(), vec![1, 2, 12, 11]);
    // Characters are ignored.
    h.type_text("x");
    assert_eq!(text(&h, s), "+00011");
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn spinbox_encoder_edit() {
    let (mut h, s) = scene(Mode::Light);
    let g = h.engine().group_of(s).unwrap();
    let _ = h.encoder_input();
    h.engine_mut().focus(s);
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    h.encoder(3);
    assert_eq!(value(&h, s), 3, "turning increments in edit mode");
    h.encoder(-1);
    assert_eq!(value(&h, s), 2);
    // A click moves the step: from the last digit to the most significant one, then right.
    h.encoder_click();
    assert_eq!(get::<Spinbox>(&h, s).step(), 10_000);
    h.encoder_click();
    assert_eq!(get::<Spinbox>(&h, s).step(), 1000);
    h.encoder(1);
    assert_eq!(value(&h, s), 1002);
    h.run_until_idle();
}

#[test]
fn spinbox_click_selects_digit() {
    let (mut h, s) = scene(Mode::Light);
    h.run_until_idle();
    let label = get::<Spinbox>(&h, s).textarea().label();
    let lc = h.engine().content_area(label);
    let l = get::<Label>(&h, label);
    let x = l.letter_pos(&MeasureCx::new(h.engine(), label), 2).x; // "+0|0|000"
    h.tap(Point::new(lc.x0 + x + 2, lc.y0 + 5));
    assert_eq!(get::<Spinbox>(&h, s).step(), 1000);
    h.run_until_idle();
}

#[test]
fn spinbox_theme_cursor_highlight() {
    let (h, s) = scene(Mode::Light);
    // The digit highlight: the primary background on the cursor part, always.
    assert_eq!(
        h.engine().style_color(s, Part::Cursor, PropId::BgColor),
        twine_theme::Palette::Blue.main()
    );
    assert_eq!(
        h.engine().style_i32(s, Part::Cursor, PropId::AnimDuration),
        0,
        "no blink"
    );
}

#[test]
fn snapshot_spinbox() {
    for m in Mode::ALL {
        let (mut h, s) = scene(m);
        with(&mut h, s, |w: &mut Spinbox, cx| {
            w.set_digit_format(cx, 5, 3);
            w.set_value(cx, 1234);
            w.set_step(cx, 10);
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("spinbox_default_{}", m.suffix()));
        let _ = h.keypad_input();
        h.key(Key::Next);
        assert!(
            h.engine()
                .tree()
                .node(s)
                .unwrap()
                .state()
                .contains(State::FOCUS_KEY)
        );
        h.run_until_idle();
        h.assert_snapshot(&format!("spinbox_focused_{}", m.suffix()));
        h.advance(Duration::ms(10));
    }
}
