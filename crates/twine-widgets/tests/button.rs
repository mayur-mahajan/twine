//! `Button`: LVGL defaults, checkable toggling, disabled state, style transitions, keypad and
//! encoder activation, and the default theme's look in every state (light and dark).

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Color, Duration, Point};
use twine_engine::{EventCode, EventFilter, EventResult, GroupDef, Key, MeasureCx, NodeId, ObjFlags};
use twine_style::{Align, Length, Part, PropId, State, StyleValue};
use twine_testing::EngineHarness;
use twine_theme::Palette;
use twine_widgets::button::{self, BUTTON_CLASS, Button};
use twine_widgets::label;

/// A button with a centered label in the middle of a 160×80 screen.
fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(160, 80, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let b = button::create(e, screen).unwrap();
    e.align(b, Align::Center, 0, 0);
    let l = label::create_with(e, b, "Button").unwrap();
    e.align(l, Align::Center, 0, 0);
    (h, b)
}

fn count(h: &mut EngineHarness, b: NodeId, code: EventCode) -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0));
    let c = n.clone();
    h.engine_mut()
        .add_event_handler(b, EventFilter::Code(code), move |_, _| {
            c.set(c.get() + 1);
            EventResult::Continue
        });
    n
}

fn center(h: &EngineHarness, b: NodeId) -> Point {
    let c = h.engine().coords(b);
    Point::new((c.x0 + c.x1) / 2, (c.y0 + c.y1) / 2)
}

#[test]
fn button_defaults() {
    let (mut h, b) = scene(Mode::Light);
    let e = h.engine();
    let n = e.tree().node(b).unwrap();
    assert_eq!(n.class().name, "button");
    assert_eq!(BUTTON_CLASS.group_def, GroupDef::True);
    assert!(n.flags().contains(ObjFlags::CLICKABLE));
    assert!(n.flags().contains(ObjFlags::SCROLL_ON_FOCUS));
    assert!(
        !n.flags().contains(ObjFlags::SCROLLABLE),
        "LVGL: buttons do not scroll"
    );
    assert!(!n.flags().contains(ObjFlags::CHECKABLE));
    assert_eq!(
        e.style_prop(b, Part::Main, PropId::Width),
        StyleValue::Length(Length::Content)
    );
    assert!(!get::<Button>(&h, b).is_checkable(&MeasureCx::new(e, b)));
    h.run_until_idle();
    // Content size: the label plus the theme's paddings (PAD_DEF 13 / PAD_SMALL 8).
    let r = h.engine().coords(b);
    let lw = twine_text::TextLayout::new("Button", &twine_assets::fonts::MONTSERRAT_14).measure();
    assert_eq!((r.width(), r.height()), (lw.w + 26, lw.h + 16));
    h.assert_idle();
}

#[test]
fn button_set_checkable_is_idempotent() {
    let (mut h, b) = scene(Mode::Light);
    h.run_until_idle();
    with(&mut h, b, |w: &mut Button, cx| w.set_checkable(cx, false));
    assert!(h.engine().invalidation_log().is_empty());
    with(&mut h, b, |w: &mut Button, cx| w.set_checkable(cx, true));
    assert!(h.engine().has_flag(b, ObjFlags::CHECKABLE));
    h.run_until_idle();
    with(&mut h, b, |w: &mut Button, cx| w.set_checkable(cx, true));
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn button_click_toggles_checked_when_checkable() {
    let (mut h, b) = scene(Mode::Light);
    with(&mut h, b, |w: &mut Button, cx| w.set_checkable(cx, true));
    let changed = count(&mut h, b, EventCode::ValueChanged);
    let clicked = count(&mut h, b, EventCode::Clicked);
    h.run_until_idle();
    let p = center(&h, b);
    h.tap(p);
    assert!(
        h.engine()
            .tree()
            .node(b)
            .unwrap()
            .state()
            .contains(State::CHECKED)
    );
    h.tap(p);
    assert!(
        !h.engine()
            .tree()
            .node(b)
            .unwrap()
            .state()
            .contains(State::CHECKED)
    );
    assert_eq!((changed.get(), clicked.get()), (2, 2));
    // A plain button does not toggle.
    with(&mut h, b, |w: &mut Button, cx| w.set_checkable(cx, false));
    h.tap(p);
    assert!(
        !h.engine()
            .tree()
            .node(b)
            .unwrap()
            .state()
            .contains(State::CHECKED)
    );
    assert_eq!(clicked.get(), 3);
}

#[test]
fn button_disabled_ignores_press() {
    let (mut h, b) = scene(Mode::Light);
    let clicked = count(&mut h, b, EventCode::Clicked);
    h.engine_mut().add_state(b, State::DISABLED);
    h.run_until_idle();
    let p = center(&h, b);
    h.press(p);
    assert!(
        !h.engine()
            .tree()
            .node(b)
            .unwrap()
            .state()
            .contains(State::PRESSED)
    );
    h.release();
    assert_eq!(clicked.get(), 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn button_press_triggers_transition() {
    let (mut h, b) = scene(Mode::Light);
    h.run_until_idle();
    let bg = |h: &EngineHarness| MeasureCx::new(h.engine(), b).rect_dsc(Part::Main).base.bg_color;
    let normal = bg(&h);
    assert_eq!(normal, Palette::Blue.main());
    let p = center(&h, b);
    h.press(p);
    assert!(h.engine().transition_count() > 0, "pressing starts transitions");
    h.advance(Duration::ms(48));
    let mid = bg(&h);
    h.advance(Duration::ms(100));
    let pressed = bg(&h);
    // LVGL `pressed`: black recolor at opacity 35.
    assert_eq!(pressed, Color::mix(Color::BLACK, normal, twine_core::Opa(35)));
    assert!(
        mid.b < normal.b && mid.b > pressed.b,
        "{normal:?} > {mid:?} > {pressed:?}"
    );
    h.release();
    h.run_until_idle();
    assert_eq!(bg(&h), normal);
    h.assert_idle();
}

#[test]
fn button_idle_after_click() {
    let (mut h, b) = scene(Mode::Light);
    h.run_until_idle();
    let p = center(&h, b);
    h.tap(p);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn enter_clicks_focused_button() {
    let mut h = harness(160, 80, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let (mut h, b) = {
        let screen = h.screen();
        let b = button::create(h.engine_mut(), screen).unwrap();
        (h, b)
    };
    assert_eq!(h.engine().group_of(b), Some(g), "buttons join the default group");
    let clicked = count(&mut h, b, EventCode::Clicked);
    let _ = h.keypad_input();
    h.run_until_idle();
    h.engine_mut().focus(b);
    h.key(Key::Enter);
    assert_eq!(clicked.get(), 1);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn encoder_press_clicks_button() {
    let mut h = harness(160, 80, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let b = button::create(h.engine_mut(), screen).unwrap();
    let clicked = count(&mut h, b, EventCode::Clicked);
    let _ = h.encoder_input();
    h.run_until_idle();
    h.engine_mut().focus(b);
    h.encoder_click();
    assert_eq!(clicked.get(), 1);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn snapshot_button_states() {
    let states: [(&str, State); 5] = [
        ("default", State::DEFAULT),
        ("pressed", State::PRESSED),
        ("checked", State::CHECKED),
        ("disabled", State::DISABLED),
        ("focused", State::FOCUSED.union(State::FOCUS_KEY)),
    ];
    for m in Mode::ALL {
        for (name, st) in states {
            let (mut h, b) = scene(m);
            h.run_until_idle();
            h.engine_mut().add_state(b, st);
            h.run_until_idle();
            h.assert_snapshot(&format!("button_{name}_{}", m.suffix()));
        }
    }
}
