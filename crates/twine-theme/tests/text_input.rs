//! The text input branches of the three themes (button matrix, textarea, keyboard, spinbox,
//! a label inside a textarea) with LVGL's values. Stand-in widgets carry the class names
//! (the themes style classes by name).

use std::rc::Rc;

use twine_core::{Color, Opa};
use twine_engine::{Engine, NodeId, OBJ_FLAGS, ThemeHook, Widget, WidgetClass};
use twine_style::{BorderSide, Part, PropId, State};
use twine_testing::EngineHarness;
use twine_theme::{DefaultTheme, MonoTheme, Palette, SimpleTheme, dpx};

macro_rules! fake {
    ($ty:ident, $class:ident, $name:literal) => {
        struct $ty;
        static $class: WidgetClass = WidgetClass::new($name).default_flags(OBJ_FLAGS);
        impl Widget for $ty {
            fn class(&self) -> &'static WidgetClass {
                &$class
            }
        }
    };
}

fake!(FakeBtnm, BTNM, "buttonmatrix");
fake!(FakeTa, TA, "textarea");
fake!(FakeKb, KB, "keyboard");
fake!(FakeSpinbox, SPINBOX, "spinbox");
fake!(FakeLabel, LABEL, "label");

fn scene(theme: Rc<dyn ThemeHook>) -> (EngineHarness, [NodeId; 6]) {
    let mut h = EngineHarness::new(320, 240).theme(theme);
    let e: &mut Engine = h.engine_mut();
    let s = e.active_screen(e.default_display().unwrap()).unwrap();
    let btnm = e.create(s, Box::new(FakeBtnm)).unwrap();
    let ta = e.create(s, Box::new(FakeTa)).unwrap();
    let kb = e.create(s, Box::new(FakeKb)).unwrap();
    let sb = e.create(s, Box::new(FakeSpinbox)).unwrap();
    let ta_label = e.create(ta, Box::new(FakeLabel)).unwrap();
    let sb_label = e.create(sb, Box::new(FakeLabel)).unwrap();
    (h, [btnm, ta, kb, sb, ta_label, sb_label])
}

#[test]
fn default_theme_text_input_branches() {
    let (mut h, [btnm, ta, kb, sb, ta_label, sb_label]) = scene(Rc::new(DefaultTheme::light()));
    let e = h.engine();
    let dpi = 130;
    // Button matrix: a card with `btn` items, primary when checked.
    assert_eq!(e.style_color(btnm, Part::Main, PropId::BgColor), Color::WHITE);
    assert_eq!(e.style_i32(btnm, Part::Items, PropId::ShadowWidth), dpx(3, dpi));
    // Keyboard: screen-colored, flat white items with small radius on a small display.
    assert_eq!(
        e.style_color(kb, Part::Main, PropId::BgColor),
        Color::hex(0x00F5_F5F5)
    );
    assert_eq!(e.style_i32(kb, Part::Items, PropId::ShadowWidth), 0);
    assert_eq!(e.style_i32(kb, Part::Items, PropId::Radius), dpx(8, dpi) / 2);
    assert_eq!(e.style_color(kb, Part::Items, PropId::BgColor), Color::WHITE);
    assert_eq!(
        e.style_i32(kb, Part::Main, PropId::PadTop),
        dpx(2, dpi),
        "pad_tiny"
    );
    // Textarea: pad_small, the placeholder grey, the cursor only while focused.
    assert_eq!(e.style_i32(ta, Part::Main, PropId::PadTop), dpx(10, dpi));
    assert_eq!(
        e.style_color(ta, Part::CustomFirst, PropId::TextColor),
        Palette::Grey.lighten(1)
    );
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::BorderWidth), 0);
    h.engine_mut().add_state(ta, State::FOCUSED);
    let e = h.engine();
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::BorderWidth), dpx(2, dpi));
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::PadLeft), -dpx(1, dpi));
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::AnimDuration), 400);
    assert_eq!(
        e.style_prop(ta, Part::Cursor, PropId::BorderSide)
            .get::<BorderSide>(),
        Some(BorderSide::LEFT)
    );
    // Spinbox: the cursor is a primary background, always.
    assert_eq!(
        e.style_color(sb, Part::Cursor, PropId::BgColor),
        Palette::Blue.main()
    );
    assert_eq!(e.style_opa(sb, Part::Cursor, PropId::BgOpa), Opa::COVER);
    // Only a textarea's label gets the primary selection (LVGL's exact type check).
    assert_eq!(
        e.style_color(ta_label, Part::Selected, PropId::BgColor),
        Palette::Blue.main()
    );
    assert_ne!(
        e.style_color(sb_label, Part::Selected, PropId::BgColor),
        Palette::Blue.main()
    );
}

#[test]
fn simple_and_mono_text_input_branches() {
    let (h, [btnm, ta, kb, sb, ..]) = scene(Rc::new(SimpleTheme::new()));
    let e = h.engine();
    assert_eq!(e.style_color(btnm, Part::Main, PropId::BgColor), Color::WHITE);
    assert_eq!(
        e.style_color(btnm, Part::Items, PropId::BgColor),
        Palette::Grey.lighten(2)
    );
    assert_eq!(
        e.style_color(sb, Part::Cursor, PropId::BgColor),
        Palette::Grey.main()
    );
    assert_eq!(e.style_color(kb, Part::Items, PropId::BgColor), Color::WHITE);
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::BorderWidth), 0);

    let (mut h, [btnm, ta, _kb, sb, ..]) = scene(Rc::new(MonoTheme::new(
        false,
        &twine_assets::fonts::MONTSERRAT_14,
    )));
    let e = h.engine();
    assert_eq!(e.style_i32(btnm, Part::Items, PropId::BorderWidth), 1);
    assert_eq!(
        e.style_color(sb, Part::Cursor, PropId::BgColor),
        Color::BLACK,
        "inverted"
    );
    h.engine_mut().add_state(ta, State::FOCUSED);
    let e = h.engine();
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::BorderWidth), 2);
    assert_eq!(e.style_i32(ta, Part::Cursor, PropId::AnimDuration), 500);
    assert_eq!(
        e.style_i32(ta, Part::Main, PropId::OutlineWidth),
        1,
        "focus outline"
    );
}
