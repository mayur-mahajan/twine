//! `Lvgl`: LVGL's default theme values, DPI and display-size scaling, focus outlines,
//! extension by a design system, and the look of a container (snapshots).
#![allow(clippy::unreadable_literal)] // colors as LVGL writes them (0xRRGGBB)

use twine_core::{Color, Opa};
use twine_engine::{
    ComputedRecipe, DesignSystem, Engine, NodeId, OBJ_CLASS, Obj, ObjFlags, RecipeRef, Theme, ThemeCx,
    ThemeHook, Widget, WidgetClass,
};
use twine_style::design::{self, ElementTable};
use twine_style::{Part, PropId, Selector, State, Style, style};
use twine_testing::EngineHarness;
use twine_theme::{DisplaySize, Lvgl, Palette, ThemeMode, lvgl};

/// A stand-in for the button widget: a class of its own, themed like a button (its base).
struct FakeButton;
static FAKE_BUTTON_CLASS: WidgetClass = WidgetClass::new("fake_button")
    .default_flags(ObjFlags::CLICKABLE)
    .base(&twine_widgets::button::BUTTON_CLASS);
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
    let h = EngineHarness::new(240, 160).theme(&lvgl::THEME);
    let e = h.engine();
    let s = screen(e);
    // LIGHT_COLOR_SCR = lv_palette_lighten(LV_PALETTE_GREY, 4); LIGHT_COLOR_TEXT = darken(GREY, 4)
    assert_eq!(
        e.style_color(s, Part::Main, PropId::BgColor),
        Color::hex(0xF5F5F5)
    );
    assert_eq!(e.style_opa(s, Part::Main, PropId::BgOpacity), Opa::COVER);
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
    let h = EngineHarness::new(240, 160)
        .theme(&lvgl::THEME)
        .mode(ThemeMode::Dark);
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
    assert_eq!(e.style_i32(card, m, PropId::PaddingTop), 13); // PAD_DEF = LV_DPX_CALC(130, 16)
    assert_eq!(e.style_i32(card, m, PropId::RowGap), 8); // PAD_SMALL = LV_DPX_CALC(130, 10)
    assert!(
        e.style_prop(card, m, PropId::BorderAboveChildren)
            .as_bool()
            .unwrap()
    );
    // Scrollbar: LV_DPX_CALC(130, 5) wide, grey at 40 %.
    assert_eq!(e.style_i32(card, Part::Scrollbar, PropId::Width), 4);
    assert_eq!(e.style_opa(card, Part::Scrollbar, PropId::BgOpacity), Opa::P40);
    assert_eq!(
        e.style_color(card, Part::Scrollbar, PropId::BgColor),
        Palette::Grey.main()
    );
}

#[test]
fn card_radius_scales_with_dpi() {
    static T130: Lvgl = Lvgl::new().dpi(130).display_size(DisplaySize::Large);
    static T260: Lvgl = Lvgl::new().dpi(260).display_size(DisplaySize::Large);
    for (t, dpi, r) in [(&T130, 130, 10), (&T260, 260, 20)] {
        let (h, card) = card_scene(EngineHarness::new(240, 160).theme(t));
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
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::PaddingLeft), 20);
    // 480×320 is DISP_MEDIUM: PAD_DEF = LV_DPX_CALC(130, 20) = 16.
    let (h, card) = card_scene(EngineHarness::new(480, 320));
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::PaddingLeft), 16);
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
    assert_eq!(e.style_i32(b, Part::Main, PropId::OutlineOffset), 2);
    assert_eq!(
        e.style_color(b, Part::Main, PropId::OutlineColor),
        Palette::Blue.main()
    );
    assert_eq!(e.style_opa(b, Part::Main, PropId::OutlineOpacity), Opa::P50);
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
    assert_eq!(e.style_opa(b, m, PropId::ShadowOpacity), Opa::P50);
    e.add_state(b, State::PRESSED);
    assert_eq!(e.style_opa(b, m, PropId::RecolorOpacity), Opa::from_raw(35));
    assert_eq!(e.style_i32(b, m, PropId::TransformWidth), 2);
    e.clear_state(b, State::PRESSED);
    e.add_state(b, State::CHECKED);
    assert_eq!(e.style_color(b, m, PropId::BgColor), Palette::Red.main());
    e.add_state(b, State::DISABLED);
    assert_eq!(e.style_color(b, m, PropId::Recolor), Color::hex(0xE0E0E0));
    assert_eq!(e.style_opa(b, m, PropId::RecolorOpacity), Opa::P50);
}

#[test]
fn dark_buttons_have_no_shadow() {
    let mut h = EngineHarness::new(240, 160)
        .theme(&lvgl::THEME)
        .mode(ThemeMode::Dark);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let b = e.create(s, Box::new(FakeButton)).unwrap();
    // LVGL sets no button shadow in dark mode: the `btn` style's shadow opacity is the
    // `SHADOW_OPACITY` design element, transparent in dark mode, so nothing is drawn and the
    // node has no extra draw area.
    assert_eq!(e.style_opa(b, Part::Main, PropId::ShadowOpacity), Opa::TRANSP);
    assert_eq!(e.compute_ext_draw(b), 0);
    e.set_theme_mode(e.default_display().unwrap(), ThemeMode::Light);
    assert_eq!(e.style_opa(b, Part::Main, PropId::ShadowOpacity), Opa::P50);
    assert!(e.compute_ext_draw(b) > 0);
}

/// The button shadow is elevation level 1: LVGL's 3 dp wide, 3 dp down by default, and a
/// mode table that retunes `ELEVATION_1_WIDTH` / `ELEVATION_1_OFFSET_Y` retunes the button
/// with it (the style sets no spread, like LVGL).
#[test]
fn button_shadow_is_elevation_level_1() {
    use twine_style::Length;
    use twine_style::design::{Elevation, ElevationLevel};

    static RAISED: ElementTable = Lvgl::SET.table(ThemeMode::Light).elevation(Elevation {
        level_1: ElevationLevel {
            width: Length::Px(9),
            offset_y: Length::Px(5),
            spread: Length::Px(2),
        },
        ..Elevation::NONE
    });
    static RAISED_THEME: Lvgl = Lvgl::new().modes(&[(ThemeMode::Light, &RAISED)]);
    let mut h = EngineHarness::new(240, 160).theme(&lvgl::THEME);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let b = e.create(s, Box::new(FakeButton)).unwrap();
    let d = e.default_display().unwrap();
    let m = Part::Main;
    let level = |el| match e.design_value(d, el) {
        Some(Length::Px(px)) => px,
        other => panic!("ELEVATION_1 element: {other:?}"),
    };
    assert_eq!(
        e.style_i32(b, m, PropId::ShadowWidth),
        level(design::ELEVATION_1_WIDTH)
    );
    assert_eq!(
        e.style_i32(b, m, PropId::ShadowOffsetY),
        level(design::ELEVATION_1_OFFSET_Y)
    );
    assert_eq!(e.style_i32(b, m, PropId::ShadowWidth), 2); // LV_DPX_CALC(130, 3)
    assert_eq!(e.style_i32(b, m, PropId::ShadowSpread), 0);

    let mut h = EngineHarness::new(240, 160).theme(&RAISED_THEME);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let b = e.create(s, Box::new(FakeButton)).unwrap();
    assert_eq!(e.style_i32(b, m, PropId::ShadowWidth), 9);
    assert_eq!(e.style_i32(b, m, PropId::ShadowOffsetY), 5);
    assert_eq!(e.style_i32(b, m, PropId::ShadowSpread), 0);
}

static RED: Style = style! { bg_color: Color::RED };

/// Plain objects: the LVGL look (its recipe), red on top.
fn red_obj(cx: &mut ThemeCx<'_>) {
    if let Some(r) = lvgl::THEME.recipe(&OBJ_CLASS) {
        cx.add_recipe(r);
    }
    if cx.parent().is_some() {
        cx.add_style(Selector::MAIN, &RED);
    }
}
static RED_OBJ: ComputedRecipe = ComputedRecipe::new("RED_OBJ", &OBJ_CLASS, red_obj);

/// A design system extending the LVGL theme (LVGL's documented extension pattern): red plain
/// objects, every other class LVGL's.
struct RedCards;
impl DesignSystem for RedCards {
    type Base = Lvgl;
    const NAME: &'static str = "red_cards";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = lvgl::MODES;
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        RED_OBJ.for_class(class)
    }
}
static RED_CARDS: Theme<RedCards> = Theme::new();

#[test]
fn design_system_extends_the_lvgl_theme() {
    let (mut h, card) = card_scene(EngineHarness::new(240, 160).theme(&RED_CARDS));
    let s = screen(h.engine());
    let b = h.engine_mut().create(s, Box::new(FakeButton)).unwrap();
    let e = h.engine();
    // The design system's recipe wins where both set a property...
    assert_eq!(e.style_color(card, Part::Main, PropId::BgColor), Color::RED);
    // ...LVGL's other card properties remain (the recipe adds LVGL's first)...
    assert_eq!(e.style_i32(card, Part::Main, PropId::BorderWidth), 2);
    // ...the classes it does not restyle keep LVGL's look, and LVGL's metrics apply.
    assert_eq!(
        e.style_color(b, Part::Main, PropId::BgColor),
        Palette::Blue.main()
    );
    assert_eq!(e.style_i32(b, Part::Main, PropId::ShadowWidth), 2);
    assert_eq!(
        e.style_color(s, Part::Main, PropId::BgColor),
        Color::hex(0xF5F5F5),
        "screens: the base's"
    );
}

#[test]
fn theme_colors_from_a_tuned_set() {
    static TEAL_DARK: ElementTable = Lvgl::SET
        .primary(Palette::Teal.main())
        .secondary(Palette::Amber.main())
        .fonts(twine_theme::FontScale::uniform(
            &twine_assets::fonts::MONTSERRAT_14,
        ))
        .table(ThemeMode::Dark);
    static TEAL: Lvgl = Lvgl::new().modes(&[(ThemeMode::Dark, &TEAL_DARK)]);
    assert_eq!(ThemeHook::modes(&TEAL), &[ThemeMode::Dark]);
    let table = TEAL.design(ThemeMode::Dark).unwrap();
    assert_eq!(table.get(design::PRIMARY), Some(Palette::Teal.main()));
    assert_eq!(table.get(design::SECONDARY), Some(Palette::Amber.main()));
    assert_eq!(TEAL.name(), "lvgl");
    let (h, card) = card_scene(EngineHarness::new(240, 160).theme(&TEAL));
    assert_eq!(h.engine().theme_mode(h.display()), ThemeMode::Dark);
    assert_eq!(
        h.engine().style_color(card, Part::Main, PropId::BgColor),
        Color::hex(0x282B30)
    );
}

#[test]
fn snapshot_theme_default_container_light() {
    let (mut h, _) = card_scene(EngineHarness::new(240, 160));
    h.assert_snapshot("theme_default_container_light");
}

#[test]
fn snapshot_theme_default_container_dark() {
    let (mut h, _) = card_scene(
        EngineHarness::new(240, 160)
            .theme(&lvgl::THEME)
            .mode(ThemeMode::Dark),
    );
    h.assert_snapshot("theme_default_container_dark");
}
