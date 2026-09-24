//! Keypad, encoder and button devices: navigation, clicks, long presses, edit mode.

mod common;

use common::{EvLog, clickable, input_codes, record, white_screen};
use twine_core::{Duration, Point, Rect};
use twine_engine::{
    Editable, EventCode as C, EventFilter, EventParam, EventResult, GroupDef, Key, NodeId, ObjFlags, State,
    Widget, WidgetClass,
};
use twine_testing::{EngineHarness, MockButton};

/// An editable widget (like a slider).
struct Knob;
static KNOB: WidgetClass = WidgetClass::new("knob")
    .default_flags(twine_engine::ObjFlags::CLICKABLE.union(twine_engine::ObjFlags::CLICK_FOCUSABLE))
    .group_def(GroupDef::True)
    .editable(Editable::True);
impl Widget for Knob {
    fn class(&self) -> &'static WidgetClass {
        &KNOB
    }
}

/// Three boxes in the default group (the middle one a `Knob` when `knob`), with logs.
fn setup(knob: bool) -> (EngineHarness, Vec<NodeId>, Vec<EvLog>) {
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(200, 100).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        e.set_default_group(Some(g));
        for i in 0..3 {
            let r = Rect::from_xywh(10 + i * 60, 10, 50, 50);
            let b = if knob && i == 1 {
                let k = e.create(s, Box::new(Knob)).unwrap();
                e.place(k, r);
                k
            } else {
                let b = clickable(e, s, r);
                // Like a button: not scrollable (the encoder would edit, i.e. scroll, it).
                e.set_flag(b, ObjFlags::SCROLLABLE, false);
                e.group_add(g, b);
                b
            };
            ids.push(b);
        }
    });
    let logs: Vec<EvLog> = ids
        .iter()
        .map(|id| {
            let l = EvLog::default();
            record(h.engine_mut(), *id, &l);
            l
        })
        .collect();
    h.run_until_idle();
    (h, ids, logs)
}

fn st(h: &EngineHarness, n: NodeId) -> State {
    h.engine().tree().node(n).unwrap().state()
}

fn keys(h: &mut EngineHarness, n: NodeId) -> std::rc::Rc<std::cell::RefCell<Vec<Key>>> {
    let keys = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let k = keys.clone();
    h.engine_mut()
        .add_event_handler(n, EventFilter::Code(C::Key), move |_, ev| {
            if let Some(key) = ev.key() {
                k.borrow_mut().push(key);
            }
            EventResult::Continue
        });
    keys
}

#[test]
fn tab_moves_focus_with_focus_key_state() {
    let (mut h, ids, _) = setup(false);
    let g = h.engine().default_group().unwrap();
    h.key(Key::Next);
    assert_eq!(h.engine().focused(g), Some(ids[1]));
    assert!(st(&h, ids[1]).contains(State::FOCUSED | State::FOCUS_KEY));
    h.key(Key::Prev);
    h.key(Key::Prev);
    assert_eq!(h.engine().focused(g), Some(ids[2]));
}

#[test]
fn keypad_focus_sets_focus_key() {
    let (mut h, ids, _) = setup(false);
    // Focus from the group API (no input) has no FOCUS_KEY, from the keypad it has.
    assert!(!st(&h, ids[0]).contains(State::FOCUS_KEY));
    h.key(Key::Next);
    h.key(Key::Prev);
    assert!(st(&h, ids[0]).contains(State::FOCUSED | State::FOCUS_KEY));
    // A pointer click focuses without it.
    h.tap(Point::new(80, 30));
    assert!(!st(&h, ids[1]).contains(State::FOCUS_KEY));
    assert!(!st(&h, ids[0]).contains(State::FOCUSED));
}

#[test]
fn enter_clicks_focused() {
    let (mut h, _, logs) = setup(false);
    h.key(Key::Enter);
    assert_eq!(
        input_codes(&logs[0]),
        [
            C::Key,
            C::Pressed,
            C::Released,
            C::ShortClicked,
            C::SingleClicked,
            C::Clicked
        ]
    );
}

#[test]
fn held_enter_long_press() {
    let (mut h, ids, logs) = setup(false);
    let t0 = h.now();
    h.key_state(Key::Enter, true);
    assert!(st(&h, ids[0]).contains(State::PRESSED));
    h.clock().set(t0 + Duration::ms(399));
    h.update();
    h.clock().set(t0 + Duration::ms(400));
    h.update();
    h.clock().set(t0 + Duration::ms(500));
    h.update();
    h.key_state(Key::Enter, false);
    let codes = input_codes(&logs[0]);
    assert!(
        codes.contains(&C::LongPressed) && codes.contains(&C::LongPressedRepeat),
        "{codes:?}"
    );
    assert!(!codes.contains(&C::ShortClicked));
    assert_eq!(codes.last(), Some(&C::Clicked));
    assert!(!st(&h, ids[0]).contains(State::PRESSED));
}

#[test]
fn held_arrow_repeats_key_event() {
    let (mut h, ids, _) = setup(false);
    let k = keys(&mut h, ids[0]);
    let t0 = h.now();
    h.key_state(Key::Right, true);
    for ms in [100, 399, 400, 499, 500, 600] {
        h.clock().set(t0 + Duration::ms(ms));
        h.update();
    }
    h.key_state(Key::Right, false);
    // Press, then a repeat every 100 ms after the long press time.
    assert_eq!(*k.borrow(), [Key::Right, Key::Right, Key::Right]);
}

#[test]
fn esc_sends_cancel() {
    let (mut h, _, logs) = setup(false);
    h.key(Key::Esc);
    assert_eq!(input_codes(&logs[0]), [C::Key, C::Cancel]);
}

#[test]
fn char_key_sent_as_key_event() {
    let (mut h, ids, _) = setup(false);
    let k = keys(&mut h, ids[0]);
    h.type_text("hi!");
    assert_eq!(*k.borrow(), [Key::Char('h'), Key::Char('i'), Key::Char('!')]);
}

#[test]
fn encoder_rotation_moves_focus() {
    let (mut h, ids, _) = setup(false);
    let g = h.engine().default_group().unwrap();
    h.encoder(2);
    assert_eq!(h.engine().focused(g), Some(ids[2]));
    assert!(st(&h, ids[2]).contains(State::FOCUS_KEY));
    h.encoder(-1);
    assert_eq!(h.engine().focused(g), Some(ids[1]));
}

#[test]
fn encoder_click_on_non_editable_clicks() {
    let (mut h, _, logs) = setup(false);
    h.encoder_click();
    assert_eq!(
        input_codes(&logs[0]),
        [
            C::Pressed,
            C::Released,
            C::ShortClicked,
            C::SingleClicked,
            C::Clicked
        ]
    );
}

#[test]
fn encoder_click_on_editable_enters_edit_mode() {
    let (mut h, ids, logs) = setup(true);
    let g = h.engine().default_group().unwrap();
    h.encoder(1);
    assert_eq!(h.engine().focused(g), Some(ids[1]));
    logs[1].borrow_mut().clear();
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    assert!(st(&h, ids[1]).contains(State::EDITED));
    assert!(!input_codes(&logs[1]).contains(&C::Clicked));
}

#[test]
fn encoder_rotation_in_edit_mode_sends_left_right_and_rotary() {
    let (mut h, ids, logs) = setup(true);
    let g = h.engine().default_group().unwrap();
    h.encoder(1);
    h.encoder_click();
    let k = keys(&mut h, ids[1]);
    let rot = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let r = rot.clone();
    h.engine_mut()
        .add_event_handler(ids[1], EventFilter::Code(C::Rotary), move |_, ev| {
            if let EventParam::Rotary(d) = ev.param {
                r.borrow_mut().push(d);
            }
            EventResult::Continue
        });
    h.encoder(2);
    h.encoder(-1);
    assert_eq!(h.engine().focused(g), Some(ids[1]), "focus stays in edit mode");
    assert_eq!(*k.borrow(), [Key::Right, Key::Right, Key::Left]);
    assert_eq!(*rot.borrow(), [2, -1]);
    // A click in edit mode clicks and sends Enter.
    logs[1].borrow_mut().clear();
    k.borrow_mut().clear();
    h.encoder_click();
    assert!(input_codes(&logs[1]).contains(&C::Clicked));
    assert_eq!(*k.borrow(), [Key::Enter]);
    assert!(h.engine().group_editing(g));
}

#[test]
fn encoder_long_press_leaves_edit_mode() {
    let (mut h, ids, _) = setup(true);
    let g = h.engine().default_group().unwrap();
    h.encoder(1);
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    let t0 = h.now();
    h.encoder_button(true);
    h.clock().set(t0 + Duration::ms(400));
    h.update();
    assert!(!h.engine().group_editing(g));
    assert!(!st(&h, ids[1]).contains(State::PRESSED | State::EDITED));
    h.encoder_button(false);
    assert!(
        !h.engine().group_editing(g),
        "the release of the long press does nothing"
    );
    // Rotation navigates again.
    h.encoder(1);
    assert_eq!(h.engine().focused(g), Some(ids[2]));
}

#[test]
fn single_editable_in_group_stays_editing() {
    let mut k = None;
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        e.set_default_group(Some(g));
        let n = e.create(s, Box::new(Knob)).unwrap();
        e.place(n, Rect::from_xywh(10, 10, 50, 50));
        k = Some(n);
    });
    h.run_until_idle();
    let g = h.engine().default_group().unwrap();
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    let t0 = h.now();
    h.encoder_button(true);
    h.clock().set(t0 + Duration::ms(500));
    h.update();
    h.encoder_button(false);
    assert!(
        h.engine().group_editing(g),
        "nowhere to navigate: stays in edit mode"
    );
    assert!(st(&h, k.unwrap()).contains(State::EDITED));
}

#[test]
fn button_device_clicks_mapped_point() {
    static POINTS: [Point; 2] = [Point::new(20, 20), Point::new(140, 20)];
    let (mut h, _, logs) = setup(false);
    let b = MockButton::new();
    let d = h.display();
    let id = h.engine_mut().add_input(b.clone(), d).unwrap();
    h.engine_mut().set_button_points(id, &POINTS);
    b.press(1);
    h.update();
    b.release();
    h.clock().advance(Duration::ms(30));
    h.update();
    let codes = input_codes(&logs[2]);
    assert!(
        codes.contains(&C::Pressed) && codes.contains(&C::Clicked),
        "{codes:?}"
    );
    assert!(!input_codes(&logs[0]).contains(&C::Clicked));
}
