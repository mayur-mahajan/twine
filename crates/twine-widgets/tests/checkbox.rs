//! `Checkbox`: LVGL defaults, content size, text storage, toggling by click anywhere / keys /
//! encoder, and the look of the default theme (light, dark) and the mono theme (1 bit).

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{ColorFormat, Point};
use twine_engine::{EventCode, EventFilter, EventResult, GroupDef, Key, MeasureCx, NodeId, ObjFlags};
use twine_style::{Align, Part, PropId, State};
use twine_testing::EngineHarness;
use twine_text::TextLayout;
use twine_theme::MonoTheme;
use twine_widgets::checkbox::{self, CHECKBOX_CLASS, Checkbox};

fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(160, 50, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let c = checkbox::create_with(e, screen, "Remember me").unwrap();
    e.align(c, Align::Center, 0, 0);
    h.run_until_idle();
    (h, c)
}

fn checked(h: &EngineHarness, c: NodeId) -> bool {
    h.engine()
        .tree()
        .node(c)
        .unwrap()
        .state()
        .contains(State::CHECKED)
}

fn count(h: &mut EngineHarness, c: NodeId) -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0));
    let k = n.clone();
    h.engine_mut()
        .add_event_handler(c, EventFilter::Code(EventCode::ValueChanged), move |_, _| {
            k.set(k.get() + 1);
            EventResult::Continue
        });
    n
}

#[test]
fn checkbox_defaults() {
    let mut h = harness(160, 50, Mode::Light);
    let screen = h.screen();
    let c = checkbox::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let n = h.engine().tree().node(c).unwrap();
    assert_eq!(n.class().name, "checkbox");
    assert_eq!(CHECKBOX_CLASS.parts, &[Part::Main, Part::Indicator]);
    assert_eq!(CHECKBOX_CLASS.group_def, GroupDef::True);
    assert!(
        n.flags()
            .contains(ObjFlags::CHECKABLE | ObjFlags::CLICKABLE | ObjFlags::SCROLL_ON_FOCUS)
    );
    assert!(!n.flags().contains(ObjFlags::SCROLLABLE));
    assert_eq!(get::<Checkbox>(&h, c).text(), "Check box");
    assert!(!checked(&h, c));
    h.assert_idle();
}

#[test]
fn checkbox_content_size() {
    let (h, c) = scene(Mode::Light);
    let font = &twine_assets::fonts::MONTSERRAT_14;
    let font_h = i32::from(font.line_height);
    let text = TextLayout::new("Remember me", font).measure();
    // Default theme: marker padding dpx(3) = 2 at 130 dpi, `pad_gap` dpx(10) = 8.
    let e = h.engine();
    assert_eq!(e.style_i32(c, Part::Indicator, PropId::PadLeft), 2);
    assert_eq!(e.style_i32(c, Part::Main, PropId::PadColumn), 8);
    let marker = font_h + 4;
    let r = e.coords(c);
    assert_eq!((r.width(), r.height()), (marker + 8 + text.w, marker.max(text.h)));
    let m = get::<Checkbox>(&h, c).marker_area(&MeasureCx::new(e, c));
    assert_eq!((m.x0, m.y0, m.width(), m.height()), (r.x0, r.y0, marker, marker));
}

#[test]
fn checkbox_set_same_text_no_invalidate() {
    let (mut h, c) = scene(Mode::Light);
    with(&mut h, c, |w: &mut Checkbox, cx| {
        w.set_text(cx, "Remember me");
        w.set_text_static(cx, "Remember me");
        w.set_checked(cx, false);
    });
    assert!(h.engine().invalidation_log().is_empty());
    let before = h.engine().coords(c).width();
    with(&mut h, c, |w: &mut Checkbox, cx| {
        w.set_text(cx, "Remember me please");
    });
    h.run_until_idle();
    assert!(
        h.engine().coords(c).width() > before,
        "content size follows the text"
    );
    with(&mut h, c, |w: &mut Checkbox, cx| {
        w.set_text(cx, "Remember me please");
        w.set_checked(cx, true);
    });
    h.run_until_idle();
    with(&mut h, c, |w: &mut Checkbox, cx| w.set_checked(cx, true));
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn checkbox_text_query() {
    let (mut h, c) = scene(Mode::Light);
    let dyn_text = h
        .engine()
        .tree()
        .node(c)
        .unwrap()
        .widget()
        .text()
        .map(str::to_owned);
    assert_eq!(dyn_text.as_deref(), Some("Remember me"));
    with(&mut h, c, |w: &mut Checkbox, cx| w.set_text(cx, "Other"));
    assert_eq!(h.engine().tree().node(c).unwrap().widget().text(), Some("Other"));
}

#[test]
fn checkbox_click_anywhere_toggles() {
    let (mut h, c) = scene(Mode::Light);
    let changed = count(&mut h, c);
    let r = h.engine().coords(c);
    // On the text, far from the box.
    h.tap(Point::new(r.x1 - 3, r.center().y));
    assert!(checked(&h, c));
    // On the box.
    h.tap(Point::new(r.x0 + 3, r.y0 + 3));
    assert!(!checked(&h, c));
    assert_eq!(changed.get(), 2);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn checkbox_keys_toggle() {
    let (mut h, c) = scene(Mode::Light);
    let changed = count(&mut h, c);
    let _ = h.keypad_input();
    h.engine_mut().focus(c);
    h.run_until_idle();
    h.key(Key::Enter);
    assert!(checked(&h, c));
    h.key(Key::Left);
    assert!(!checked(&h, c));
    assert_eq!(changed.get(), 2);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn checkbox_encoder_click_toggles() {
    let (mut h, c) = scene(Mode::Light);
    let _ = h.encoder_input();
    h.engine_mut().focus(c);
    h.run_until_idle();
    h.encoder_click();
    assert!(checked(&h, c));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn snapshot_checkbox_states() {
    let states: [(&str, State); 5] = [
        ("unchecked", State::DEFAULT),
        ("checked", State::CHECKED),
        ("pressed", State::PRESSED),
        ("disabled", State::DISABLED.union(State::CHECKED)),
        ("focused", State::FOCUSED.union(State::FOCUS_KEY)),
    ];
    for m in Mode::ALL {
        for (name, st) in states {
            let (mut h, c) = scene(m);
            h.engine_mut().add_state(c, st);
            h.run_until_idle();
            h.assert_snapshot(&format!("checkbox_{name}_{}", m.suffix()));
        }
    }
}

#[test]
fn snapshot_checkbox_mono_i1() {
    let mut h = EngineHarness::new(128, 32)
        .format(ColorFormat::I1)
        .theme(Rc::new(MonoTheme::new(
            false,
            &twine_assets::fonts::MONTSERRAT_14,
        )));
    let screen = h.screen();
    let e = h.engine_mut();
    let a = checkbox::create_with(e, screen, "Off").unwrap();
    let b = checkbox::create_with(e, screen, "On").unwrap();
    e.align(a, Align::LeftMid, 4, 0);
    e.align(b, Align::RightMid, -4, 0);
    e.add_state(b, State::CHECKED);
    h.run_until_idle();
    // Only pure black and white pixels.
    let rgb = h.panel_rgb888();
    assert!(rgb.iter().all(|&v| v == 0 || v == 255));
    h.assert_snapshot("checkbox_mono_i1");
}
