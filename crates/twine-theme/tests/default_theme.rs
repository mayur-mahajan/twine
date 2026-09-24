//! `DefaultTheme`: LVGL's default theme values, DPI and display-size scaling, focus
//! outlines, theme chaining, and the look of a container (snapshots).
#![allow(clippy::unreadable_literal)] // colors as LVGL writes them (0xRRGGBB)

use std::rc::Rc;

use twine_core::{Color, Opa};
use twine_engine::{Engine, NodeId, Obj, ObjFlags, ThemeCx, ThemeHook, Widget, WidgetClass};
use twine_style::{Part, PropId, Selector, State, StyleBuf};
use twine_testing::EngineHarness;
use twine_text::Font;
use twine_theme::{DefaultTheme, DisplaySize, Palette, Theme, ThemeMode};

/// A stand-in for the button widget (the theme styles classes by name).
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

fn card_scene(h: EngineHarness) -> (EngineHarness, NodeId) {
    let mut h = h;
    let s = screen(h.engine());
    let e = h.engine_mut();
    let card = e.create(s, Box::new(Obj)).unwrap();
    e.set_size(card, 200, 120);
    e.align(card, twine_style::Align::Center, 0, 0);
    (h, card)
}

#[test]
fn screen_bg_matches_lvgl_light() {
    let h = EngineHarness::new(240, 160).theme(Rc::new(DefaultTheme::light()));
    let e = h.engine();
    let s = screen(e);
    // LIGHT_COLOR_SCR = lv_palette_lighten(LV_PALETTE_GREY, 4); LIGHT_COLOR_TEXT = darken(GREY, 4)
    assert_eq!(
        e.style_color(s, Part::Main, PropId::BgColor),
        Color::hex(0xF5F5F5)
    );
    assert_eq!(e.style_opa(s, Part::Main, PropId::BgOpa), Opa::COVER);
    assert_eq!(
        e.style_color(s, Part::Main, PropId::TextColor),
        Color::hex(0x212121)
    );
    assert!(core::ptr::eq(
        e.style_font(s, Part::Main),
        &raw const twine_assets::fonts::MONTSERRAT_14
    ));
}

#[test]
fn screen_bg_matches_lvgl_dark() {
    let h = EngineHarness::new(240, 160).theme(Rc::new(DefaultTheme::dark()));
    let e = h.engine();
    let s = screen(e);
    // DARK_COLOR_SCR = 0x15171A; DARK_COLOR_TEXT = lv_palette_lighten(GREY, 5)
    assert_eq!(
        e.style_color(s, Part::Main, PropId::BgColor),
        Color::hex(0x15171A)
    );
    assert_eq!(
        e.style_color(s, Part::Main, PropId::TextColor),
        Color::hex(0xFAFAFA)
    );
}

#[test]
fn card_matches_lvgl_small_display() {
    let (h, card) = card_scene(EngineHarness::new(240, 160));
    let e = h.engine();
    let m = Part::Main;
    // 240×160: DISP_SMALL at 130 dpi.
    assert_eq!(e.style_i32(card, m, PropId::Radius), 7); // RADIUS_DEFAULT = LV_DPX_CALC(130, 8)
    assert_eq!(e.style_i32(card, m, PropId::BorderWidth), 2); // BORDER_WIDTH = LV_DPX_CALC(130, 2)
    assert_eq!(e.style_color(card, m, PropId::BorderColor), Color::hex(0xE0E0E0));
    assert_eq!(e.style_color(card, m, PropId::BgColor), Color::WHITE);
    assert_eq!(e.style_i32(card, m, PropId::PadTop), 13); // PAD_DEF = LV_DPX_CALC(130, 16)
    assert_eq!(e.style_i32(card, m, PropId::PadRow), 8); // PAD_SMALL = LV_DPX_CALC(130, 10)
    assert!(e.style_prop(card, m, PropId::BorderPost).as_bool().unwrap());
    // Scrollbar: LV_DPX_CALC(130, 5) wide, grey at 40 %.
    assert_eq!(e.style_i32(card, Part::Scrollbar, PropId::Width), 4);
    assert_eq!(e.style_opa(card, Part::Scrollbar, PropId::BgOpa), Opa::P40);
    assert_eq!(
        e.style_color(card, Part::Scrollbar, PropId::BgColor),
        Palette::Grey.main()
    );
}

#[test]
fn card_radius_scales_with_dpi() {
    for (dpi, r) in [(130, 10), (260, 20)] {
        let t = DefaultTheme::light()
            .with_dpi(dpi)
            .with_display_size(DisplaySize::Large);
        let (h, card) = card_scene(EngineHarness::new(240, 160).theme(Rc::new(t)));
        // RADIUS_DEFAULT = LV_DPX_CALC(dpi, 12) on large displays.
        assert_eq!(
            h.engine().style_i32(card, Part::Main, PropId::Radius),
            r,
            "dpi {dpi}"
        );
    }
}

#[test]
fn display_size_follows_resolution() {
    // 800×480 is DISP_LARGE: PAD_DEF = LV_DPX_CALC(130, 24) = 20.
    let (h, card) = card_scene(EngineHarness::new(800, 480));
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::PadLeft), 20);
    // 480×320 is DISP_MEDIUM: PAD_DEF = LV_DPX_CALC(130, 20) = 16.
    let (h, card) = card_scene(EngineHarness::new(480, 320));
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::PadLeft), 16);
}

#[test]
fn focus_key_shows_outline() {
    let mut h = EngineHarness::new(240, 160);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let b = e.create(s, Box::new(FakeButton)).unwrap();
    assert_eq!(e.style_i32(b, Part::Main, PropId::OutlineWidth), 0);
    e.add_state(b, State::FOCUSED | State::FOCUS_KEY);
    // outline_primary: OUTLINE_WIDTH = LV_DPX_CALC(130, 3), primary color at 50 %.
    assert_eq!(e.style_i32(b, Part::Main, PropId::OutlineWidth), 2);
    assert_eq!(e.style_i32(b, Part::Main, PropId::OutlinePad), 2);
    assert_eq!(
        e.style_color(b, Part::Main, PropId::OutlineColor),
        Palette::Blue.main()
    );
    assert_eq!(e.style_opa(b, Part::Main, PropId::OutlineOpa), Opa::P50);
}

#[test]
fn button_styles_match_lvgl() {
    let mut h = EngineHarness::new(240, 160);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let b = e.create(s, Box::new(FakeButton)).unwrap();
    let m = Part::Main;
    assert_eq!(e.style_color(b, m, PropId::BgColor), Palette::Blue.main());
    assert_eq!(e.style_color(b, m, PropId::TextColor), Color::WHITE);
    assert_eq!(e.style_i32(b, m, PropId::Radius), 7); // LV_DPX_CALC(130, 8) (small display)
    assert_eq!(e.style_i32(b, m, PropId::ShadowWidth), 2); // LV_DPX_CALC(130, 3)
    assert_eq!(e.style_i32(b, m, PropId::ShadowOffsetY), 2);
    assert_eq!(e.style_opa(b, m, PropId::ShadowOpa), Opa::P50);
    e.add_state(b, State::PRESSED);
    assert_eq!(e.style_opa(b, m, PropId::RecolorOpa), Opa(35));
    assert_eq!(e.style_i32(b, m, PropId::TransformWidth), 2);
    e.clear_state(b, State::PRESSED);
    e.add_state(b, State::CHECKED);
    assert_eq!(e.style_color(b, m, PropId::BgColor), Palette::Red.main());
    e.add_state(b, State::DISABLED);
    assert_eq!(e.style_color(b, m, PropId::Recolor), Color::hex(0xE0E0E0));
    assert_eq!(e.style_opa(b, m, PropId::RecolorOpa), Opa::P50);
}

#[test]
fn dark_buttons_have_no_shadow() {
    let mut h = EngineHarness::new(240, 160).theme(Rc::new(DefaultTheme::dark()));
    let s = screen(h.engine());
    let e = h.engine_mut();
    let b = e.create(s, Box::new(FakeButton)).unwrap();
    // The `btn` style sets no shadow in dark mode (width stays 0).
    assert_eq!(e.style_i32(b, Part::Main, PropId::ShadowWidth), 0);
    assert_eq!(e.style_i32(b, Part::Main, PropId::ShadowOffsetY), 0);
}

/// A theme that makes every container red and counts its calls.
struct RedCards(Rc<StyleBuf>);
impl ThemeHook for RedCards {
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if class.name == "obj" && cx.parent().is_some() {
            cx.add_style(Selector::MAIN, self.0.clone());
        }
    }
    fn font_normal(&self) -> &'static Font {
        &twine_assets::fonts::MONTSERRAT_14
    }
}
impl Theme for RedCards {
    fn font_small(&self) -> &'static Font {
        self.font_normal()
    }
    fn font_large(&self) -> &'static Font {
        self.font_normal()
    }
    fn color_primary(&self) -> Color {
        Color::RED
    }
    fn color_secondary(&self) -> Color {
        Color::RED
    }
}

#[test]
fn parent_theme_applied_first() {
    let parent = Rc::new(RedCards(Rc::new(
        StyleBuf::new().bg_color(Color::RED).shadow_width(9),
    )));
    let t = DefaultTheme::light().with_parent(parent);
    let (h, card) = card_scene(EngineHarness::new(240, 160).theme(Rc::new(t)));
    let e = h.engine();
    // The child theme (default) wins where both set a property...
    assert_eq!(e.style_color(card, Part::Main, PropId::BgColor), Color::WHITE);
    // ...and the parent's other properties remain.
    assert_eq!(e.style_i32(card, Part::Main, PropId::ShadowWidth), 9);

    // A theme extending the default one: the child's styles win.
    let child = RedChild(
        Rc::new(DefaultTheme::light()),
        Rc::new(StyleBuf::new().bg_color(Color::RED)),
    );
    let (h, card) = card_scene(EngineHarness::new(240, 160).theme(Rc::new(child)));
    assert_eq!(
        h.engine().style_color(card, Part::Main, PropId::BgColor),
        Color::RED
    );
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::BorderWidth), 2);
}

/// A user theme on top of the default theme (LVGL's documented extension pattern).
struct RedChild(Rc<DefaultTheme>, Rc<StyleBuf>);
impl ThemeHook for RedChild {
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        self.0.apply(cx, class);
        if class.name == "obj" && cx.parent().is_some() {
            cx.add_style(Selector::MAIN, self.1.clone());
        }
    }
    fn font_normal(&self) -> &'static Font {
        self.0.font_normal()
    }
}

#[test]
fn theme_mode_and_colors() {
    let t = DefaultTheme::new(
        Palette::Teal,
        Palette::Amber,
        ThemeMode::Dark,
        &twine_assets::fonts::MONTSERRAT_14,
    );
    assert_eq!(t.mode(), ThemeMode::Dark);
    assert_eq!(t.color_primary(), Palette::Teal.main());
    assert_eq!(t.color_secondary(), Palette::Amber.main());
    assert_eq!(t.name(), "default-dark");
    // Style sets are cached per (dpi, size).
    let a = t.styles(130, twine_core::Size::new(240, 160));
    let b = t.styles(130, twine_core::Size::new(240, 160));
    assert!(Rc::ptr_eq(&a, &b));
    let c = t.styles(130, twine_core::Size::new(800, 480));
    assert!(!Rc::ptr_eq(&a, &c));
}

#[test]
fn snapshot_theme_default_container_light() {
    let (mut h, _) = card_scene(EngineHarness::new(240, 160));
    h.assert_snapshot("theme_default_container_light");
}

#[test]
fn snapshot_theme_default_container_dark() {
    let (mut h, _) = card_scene(EngineHarness::new(240, 160).theme(Rc::new(DefaultTheme::dark())));
    h.assert_snapshot("theme_default_container_dark");
}
