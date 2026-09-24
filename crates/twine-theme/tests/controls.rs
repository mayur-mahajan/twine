//! The styles the default, simple and mono themes give LVGL's basic controls (bar, slider,
//! switch, checkbox, arc, spinner, LED, line), checked against `lv_theme_*.c`. Widgets are
//! stood in for by classes with the same names (themes style classes by name).
#![allow(clippy::unreadable_literal)]

use std::rc::Rc;

use twine_core::{Color, Opa};
use twine_engine::{Engine, NodeId, ObjFlags, ThemeHook, Widget, WidgetClass};
use twine_image::ImageSource;
use twine_style::{Part, PropId, RADIUS_CIRCLE, State};
use twine_testing::EngineHarness;
use twine_theme::{DefaultTheme, MonoTheme, Palette, SimpleTheme, default::colors};

macro_rules! fake {
    ($ty:ident, $class:ident, $name:literal) => {
        struct $ty;
        static $class: WidgetClass = WidgetClass::new($name).default_flags(ObjFlags::CLICKABLE);
        impl Widget for $ty {
            fn class(&self) -> &'static WidgetClass {
                &$class
            }
        }
    };
}

fake!(Bar, BAR, "bar");
fake!(Slider, SLIDER, "slider");
fake!(Switch, SWITCH, "switch");
fake!(Checkbox, CHECKBOX, "checkbox");
fake!(Arc, ARC, "arc");
fake!(Spinner, SPINNER, "spinner");
fake!(Led, LED, "led");
fake!(Line, LINE, "line");

fn node(theme: Rc<dyn ThemeHook>, w: Box<dyn Widget>) -> (EngineHarness, NodeId) {
    let mut h = EngineHarness::new(240, 160).theme(theme);
    let e: &mut Engine = h.engine_mut();
    let s = e.active_screen(e.default_display().unwrap()).unwrap();
    let n = e.create(s, w).unwrap();
    (h, n)
}

fn light() -> Rc<dyn ThemeHook> {
    Rc::new(DefaultTheme::light())
}

#[test]
fn default_bar_and_slider() {
    for w in [Box::new(Bar) as Box<dyn Widget>, Box::new(Slider)] {
        let (h, n) = node(light(), w);
        let e = h.engine();
        // bg_color_primary_muted + circle; indicator bg_color_primary + circle.
        assert_eq!(
            e.style_color(n, Part::Main, PropId::BgColor),
            Palette::Blue.main()
        );
        assert_eq!(e.style_opa(n, Part::Main, PropId::BgOpa), Opa::P20);
        assert_eq!(e.style_i32(n, Part::Main, PropId::Radius), RADIUS_CIRCLE);
        assert_eq!(e.style_opa(n, Part::Indicator, PropId::BgOpa), Opa::COVER);
        assert_eq!(e.style_i32(n, Part::Indicator, PropId::Radius), RADIUS_CIRCLE);
    }
    let (mut h, n) = node(light(), Box::new(Slider));
    // knob: primary, pad dpx(6) = 5 at 130 dpi, circle; grows by dpx(3) when pressed.
    assert_eq!(h.engine().style_i32(n, Part::Knob, PropId::PadLeft), 5);
    assert_eq!(
        h.engine().style_color(n, Part::Knob, PropId::BgColor),
        Palette::Blue.main()
    );
    h.engine_mut().add_state(n, State::PRESSED);
    h.run_until_idle();
    assert_eq!(h.engine().style_i32(n, Part::Knob, PropId::TransformWidth), 2);
    // Edited: the secondary outline.
    h.engine_mut().add_state(n, State::EDITED);
    assert_eq!(
        h.engine().style_color(n, Part::Main, PropId::OutlineColor),
        Palette::Red.main()
    );
}

#[test]
fn default_switch() {
    let (mut h, n) = node(light(), Box::new(Switch));
    let e = h.engine();
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), colors::LIGHT_GREY);
    assert_eq!(e.style_i32(n, Part::Main, PropId::AnimDuration), 120);
    // switch_knob: pad_all -dpx(4) = -3, white.
    assert_eq!(e.style_i32(n, Part::Knob, PropId::PadTop), -3);
    assert_eq!(e.style_color(n, Part::Knob, PropId::BgColor), Color::WHITE);
    assert_eq!(e.style_opa(n, Part::Indicator, PropId::BgOpa), Opa::TRANSP);
    h.engine_mut().add_state(n, State::CHECKED);
    h.run_until_idle();
    let e = h.engine();
    assert_eq!(e.style_opa(n, Part::Indicator, PropId::BgOpa), Opa::COVER);
    assert_eq!(
        e.style_color(n, Part::Indicator, PropId::BgColor),
        Palette::Blue.main()
    );
}

#[test]
fn default_checkbox() {
    let (mut h, n) = node(light(), Box::new(Checkbox));
    let e = h.engine();
    assert_eq!(e.style_i32(n, Part::Main, PropId::PadColumn), 8); // pad_gap dpx(10)
    assert_eq!(e.style_i32(n, Part::Indicator, PropId::PadLeft), 2); // dpx(3)
    assert_eq!(e.style_i32(n, Part::Indicator, PropId::BorderWidth), 2); // BORDER_WIDTH
    assert_eq!(
        e.style_color(n, Part::Indicator, PropId::BorderColor),
        Palette::Blue.main()
    );
    assert_eq!(e.style_i32(n, Part::Indicator, PropId::Radius), 3); // RADIUS_DEFAULT / 2
    assert!(
        e.style_prop(n, Part::Indicator, PropId::BgImageSrc)
            .get::<&'static ImageSource>()
            .is_none()
    );
    h.engine_mut().add_state(n, State::CHECKED);
    let e = h.engine();
    let mark = e
        .style_prop(n, Part::Indicator, PropId::BgImageSrc)
        .get::<&'static ImageSource>();
    assert_eq!(mark, Some(&ImageSource::Symbol(twine_text::symbols::OK)));
    assert_eq!(
        e.style_color(n, Part::Indicator, PropId::BgColor),
        Palette::Blue.main()
    );
}

#[test]
fn default_arc_and_spinner() {
    for (w, knob) in [
        (Box::new(Arc) as Box<dyn Widget>, true),
        (Box::new(Spinner), false),
    ] {
        let (h, n) = node(light(), w);
        let e = h.engine();
        assert_eq!(e.style_i32(n, Part::Main, PropId::ArcWidth), 12); // dpx(15)
        assert_eq!(e.style_color(n, Part::Main, PropId::ArcColor), colors::LIGHT_GREY);
        assert_eq!(
            e.style_color(n, Part::Indicator, PropId::ArcColor),
            Palette::Blue.main()
        );
        assert_eq!(
            e.style_prop(n, Part::Indicator, PropId::ArcRounded).as_bool(),
            Some(true)
        );
        let knob_opa = e.style_opa(n, Part::Knob, PropId::BgOpa);
        assert_eq!(knob_opa == Opa::COVER, knob, "only the arc has a knob");
    }
}

#[test]
fn default_led_and_line() {
    let (h, n) = node(light(), Box::new(Led));
    let e = h.engine();
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::WHITE);
    assert_eq!(
        e.style_color(n, Part::Main, PropId::BgGradColor),
        Palette::Grey.main()
    );
    assert_eq!(e.style_i32(n, Part::Main, PropId::ShadowWidth), 12); // dpx(15)
    assert_eq!(e.style_i32(n, Part::Main, PropId::ShadowSpread), 4); // dpx(5)
    let (h, n) = node(Rc::new(DefaultTheme::dark()), Box::new(Line));
    let e = h.engine();
    assert_eq!(e.style_i32(n, Part::Main, PropId::LineWidth), 1);
    assert_eq!(e.style_color(n, Part::Main, PropId::LineColor), colors::DARK_TEXT);
}

#[test]
fn simple_theme_controls() {
    let t = || Rc::new(SimpleTheme::new()) as Rc<dyn ThemeHook>;
    let (h, n) = node(t(), Box::new(Slider));
    let e = h.engine();
    assert_eq!(
        e.style_color(n, Part::Main, PropId::BgColor),
        Palette::Grey.lighten(2)
    );
    assert_eq!(
        e.style_color(n, Part::Indicator, PropId::BgColor),
        Palette::Grey.main()
    );
    assert_eq!(
        e.style_color(n, Part::Knob, PropId::BgColor),
        Palette::Grey.darken(2)
    );
    let (h, n) = node(t(), Box::new(Arc));
    let e = h.engine();
    assert_eq!(e.style_i32(n, Part::Main, PropId::ArcWidth), 6); // arc_line
    assert_eq!(e.style_opa(n, Part::Main, PropId::BgOpa), Opa::TRANSP); // transp
    assert_eq!(e.style_i32(n, Part::Knob, PropId::PadLeft), 5); // arc_knob
    let (mut h, n) = node(t(), Box::new(Checkbox));
    h.engine_mut().add_state(n, State::CHECKED);
    assert_eq!(
        h.engine().style_color(n, Part::Indicator, PropId::BgColor),
        Palette::Grey.main()
    );
}

#[test]
fn mono_theme_controls() {
    let t = |dark| Rc::new(MonoTheme::new(dark, &twine_assets::fonts::MONTSERRAT_14)) as Rc<dyn ThemeHook>;
    let (h, n) = node(t(false), Box::new(Switch));
    let e = h.engine();
    // card + radius_circle + pad_zero; indicator inverted; knob card.
    assert_eq!(e.style_i32(n, Part::Main, PropId::BorderWidth), 1);
    assert_eq!(e.style_i32(n, Part::Main, PropId::PadLeft), 0);
    assert_eq!(e.style_color(n, Part::Indicator, PropId::BgColor), Color::BLACK);
    assert_eq!(e.style_color(n, Part::Knob, PropId::BgColor), Color::WHITE);
    let (h, n) = node(t(true), Box::new(Spinner));
    let e = h.engine();
    assert_eq!(e.style_i32(n, Part::Indicator, PropId::ArcWidth), 8); // SPINNER_WIDTH
    assert_eq!(e.style_color(n, Part::Indicator, PropId::ArcColor), Color::WHITE);
    let (mut h, n) = node(t(false), Box::new(Checkbox));
    h.engine_mut().add_state(n, State::CHECKED);
    let e = h.engine();
    assert_eq!(e.style_color(n, Part::Indicator, PropId::BgColor), Color::BLACK);
    assert!(
        e.style_prop(n, Part::Indicator, PropId::BgImageSrc)
            .get::<&'static ImageSource>()
            .is_some()
    );
}
