//! `SimpleTheme`, `MonoTheme` and the harness's default theme.
#![allow(clippy::unreadable_literal)] // colors as LVGL writes them (0xRRGGBB)

use std::rc::Rc;

use twine_core::{Color, ColorFormat};
use twine_engine::{Engine, NodeId, Obj, ObjFlags, Widget, WidgetClass};
use twine_style::{Part, PropId, State, StyleValue};
use twine_testing::EngineHarness;
use twine_theme::{MonoTheme, Palette, SimpleTheme};

struct FakeButton;
static FAKE_BUTTON_CLASS: WidgetClass = WidgetClass::new("button").default_flags(ObjFlags::CLICKABLE);
impl Widget for FakeButton {
    fn class(&self) -> &'static WidgetClass {
        &FAKE_BUTTON_CLASS
    }
}

fn screen(e: &Engine) -> NodeId {
    e.active_screen(e.default_display().unwrap()).unwrap()
}

/// A card and a button on a 240×160 screen.
fn scene(h: EngineHarness) -> (EngineHarness, NodeId, NodeId) {
    let mut h = h;
    let s = screen(h.engine());
    let e = h.engine_mut();
    let card = e.create(s, Box::new(Obj)).unwrap();
    e.set_size(card, 200, 120);
    e.align(card, twine_style::Align::Center, 0, 0);
    let b = e.create(card, Box::new(FakeButton)).unwrap();
    e.set_size(b, 80, 30);
    e.align(b, twine_style::Align::Center, 0, 0);
    (h, card, b)
}

#[test]
fn simple_theme_screen_matches_lvgl() {
    let (h, card, b) = scene(EngineHarness::new(240, 160).theme(Rc::new(SimpleTheme::new())));
    let e = h.engine();
    let s = screen(e);
    // COLOR_SCR = lv_palette_lighten(GREY, 4), text COLOR_DIM = lv_palette_darken(GREY, 2)
    assert_eq!(
        e.style_color(s, Part::Main, PropId::BgColor),
        Color::hex(0xF5F5F5)
    );
    assert_eq!(
        e.style_color(s, Part::Main, PropId::TextColor),
        Palette::Grey.darken(2)
    );
    // Containers are white, buttons COLOR_DARK = lv_palette_main(GREY).
    assert_eq!(e.style_color(card, Part::Main, PropId::BgColor), Color::WHITE);
    assert_eq!(
        e.style_color(b, Part::Main, PropId::BgColor),
        Palette::Grey.main()
    );
    assert_eq!(e.style_i32(card, Part::Scrollbar, PropId::Width), 2);
}

#[test]
fn mono_theme_only_uses_pure_colors() {
    for dark in [false, true] {
        let t = Rc::new(MonoTheme::new(dark, &twine_assets::fonts::MONTSERRAT_14));
        let (mut h, card, b) = scene(EngineHarness::new(240, 160).theme(t));
        let s = screen(h.engine());
        let states = [
            State::DEFAULT,
            State::PRESSED,
            State::CHECKED,
            State::DISABLED,
            State::FOCUSED | State::FOCUS_KEY,
            State::FOCUSED | State::EDITED,
        ];
        for st in states {
            for n in [s, card, b] {
                let e = h.engine_mut();
                e.set_state(n, State::ANY, false);
                e.add_state(n, st);
                for part in [Part::Main, Part::Scrollbar] {
                    for p in &twine_style::PropId::ALL {
                        if let StyleValue::Color(c) = e.style_prop(n, part, *p) {
                            assert!(
                                c == Color::BLACK || c == Color::WHITE,
                                "{p:?} of {n} in {st:?}: {c:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn harness_defaults_to_default_light() {
    let h = EngineHarness::new(64, 48);
    let d = h.display();
    let t = h.engine().theme(d).expect("a theme");
    assert_eq!(t.name(), "default-light");
    let s = screen(h.engine());
    assert_eq!(
        h.engine().style_color(s, Part::Main, PropId::BgColor),
        Color::hex(0xF5F5F5)
    );
}

#[test]
fn snapshot_theme_simple_container() {
    let (mut h, _, _) = scene(EngineHarness::new(240, 160).theme(Rc::new(SimpleTheme::new())));
    h.assert_snapshot("theme_simple_container");
}

#[test]
fn snapshot_theme_mono_container() {
    let t = Rc::new(MonoTheme::new(false, &twine_assets::fonts::MONTSERRAT_14));
    let (mut h, _, b) = scene(EngineHarness::new(240, 160).format(ColorFormat::I1).theme(t));
    h.engine_mut().add_state(b, State::CHECKED);
    h.assert_snapshot("theme_mono_container");
}
