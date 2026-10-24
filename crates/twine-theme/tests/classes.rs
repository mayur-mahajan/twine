//! Typed widget classes in themes (R2.S04): identity matching along `WidgetClass::base`,
//! design systems extending a built-in theme (R7.S04), public theme styles, the card class,
//! tuned element sets, and colors that follow every theme mode.

use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::{Color, Opa};
use twine_engine::{
    ComputedRecipe, DesignSystem, Engine, NodeId, OBJ_CLASS, ObjFlags, Recipe, RecipeRef, Theme, ThemeCx,
    ThemeHook, Widget, WidgetClass, recipe,
};
use twine_style::design::ElementTable;
use twine_style::{Part, PropId, Selector, State, Style, ThemeMode, design, style};
use twine_testing::EngineHarness;
use twine_theme::{FontScale, Lvgl, Mono, Palette, lvgl, mono, simple};
use twine_widgets::button::BUTTON_CLASS;
use twine_widgets::container::{CARD_CLASS, Card};

/// A custom widget themed like a button.
static KEY_CLASS: WidgetClass = WidgetClass::new("key")
    .default_flags(ObjFlags::CLICKABLE)
    .base(&BUTTON_CLASS);
/// A custom widget derived from the key.
static BIG_KEY_CLASS: WidgetClass = WidgetClass::new("big_key")
    .default_flags(ObjFlags::CLICKABLE)
    .base(&KEY_CLASS);
/// A class named like the button but not derived from it.
static NAMESAKE_CLASS: WidgetClass = WidgetClass::new("button").default_flags(ObjFlags::CLICKABLE);
/// A gauge styled only by an application recipe.
static GAUGE_CLASS: WidgetClass = WidgetClass::new("gauge").parts(&[Part::Main, Part::Indicator]);

struct Of(&'static WidgetClass);
impl Widget for Of {
    fn class(&self) -> &'static WidgetClass {
        self.0
    }
}

fn create(h: &mut EngineHarness, class: &'static WidgetClass) -> NodeId {
    let s = h.screen();
    h.engine_mut().create(s, Box::new(Of(class))).unwrap()
}

fn bg(e: &Engine, n: NodeId) -> Color {
    e.style_color(n, Part::Main, PropId::BgColor)
}

/// The built-in themes.
fn themes() -> [&'static dyn ThemeHook; 3] {
    [&lvgl::THEME, &mono::THEME, &simple::THEME]
}

#[test]
fn custom_class_with_button_base_gets_button_styling() {
    for theme in themes() {
        let mut h = EngineHarness::new(240, 160).theme(theme);
        let real = create(&mut h, &BUTTON_CLASS);
        let key = create(&mut h, &KEY_CLASS);
        let big = create(&mut h, &BIG_KEY_CLASS);
        let namesake = create(&mut h, &NAMESAKE_CLASS);
        let e = h.engine();
        for prop in [
            PropId::BgColor,
            PropId::Radius,
            PropId::PaddingLeft,
            PropId::ShadowWidth,
            PropId::BorderWidth,
        ] {
            let want = e.style_prop(real, Part::Main, prop);
            assert_eq!(
                e.style_prop(key, Part::Main, prop),
                want,
                "{} {prop:?}",
                theme.name()
            );
            assert_eq!(
                e.style_prop(big, Part::Main, prop),
                want,
                "{} {prop:?} (2 levels)",
                theme.name()
            );
        }
        // A class is not a button because of its name.
        assert_ne!(
            e.style_prop(namesake, Part::Main, PropId::BgOpacity),
            e.style_prop(real, Part::Main, PropId::BgOpacity),
            "{}",
            theme.name()
        );
    }
    // State styles come along: the default theme's pressed recolor.
    let mut h = EngineHarness::new(240, 160).theme(&lvgl::THEME);
    let key = create(&mut h, &KEY_CLASS);
    h.engine_mut().add_state(key, State::PRESSED);
    h.run_until_idle();
    assert_eq!(
        h.engine().style_opa(key, Part::Main, PropId::RecolorOpacity),
        Opa::from_raw(35)
    );
}

static RED: Style = style! { bg_color: Color::RED };
static GREEN: Style = style! { bg_color: Color::GREEN };
static KEY_CALLS: AtomicU32 = AtomicU32::new(0);

/// The key: the LVGL button, red.
fn key(cx: &mut ThemeCx<'_>) {
    KEY_CALLS.fetch_add(1, Ordering::Relaxed);
    if let Some(r) = lvgl::THEME.recipe(&BUTTON_CLASS) {
        cx.add_recipe(r);
    }
    cx.add_style(Selector::MAIN, &RED);
}
/// The big key: the key, green.
fn big_key(cx: &mut ThemeCx<'_>) {
    key(cx);
    cx.add_style(Selector::MAIN, &GREEN);
}
static KEY: ComputedRecipe = ComputedRecipe::new("KEY", &KEY_CLASS, key);
static BIG_KEY: ComputedRecipe = ComputedRecipe::new("BIG_KEY", &BIG_KEY_CLASS, big_key);

/// Keys on the LVGL theme.
struct Keys;
impl DesignSystem for Keys {
    type Base = Lvgl;
    const NAME: &'static str = "keys";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = lvgl::MODES;
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        KEY.for_class(class).or_else(|| BIG_KEY.for_class(class))
    }
}
static KEYS: Theme<Keys> = Theme::new();

#[test]
fn design_system_recipes_style_custom_classes_over_the_base() {
    let mut h = EngineHarness::new(240, 160).theme(&KEYS);
    let before = KEY_CALLS.load(Ordering::Relaxed);
    let button = create(&mut h, &BUTTON_CLASS);
    let key = create(&mut h, &KEY_CLASS);
    let big = create(&mut h, &BIG_KEY_CLASS);
    let e = h.engine();
    // The base's styles stay (radius, shadow), the recipe wins where both set a value.
    assert_eq!(bg(e, key), Color::RED);
    assert_eq!(
        e.style_i32(key, Part::Main, PropId::ShadowWidth),
        e.style_i32(button, Part::Main, PropId::ShadowWidth)
    );
    assert_eq!(
        bg(e, button),
        Palette::Blue.main(),
        "the button itself is untouched"
    );
    // The most derived recipe is chosen; it builds on the key's.
    assert_eq!(bg(e, big), Color::GREEN);
    assert_eq!(KEY_CALLS.load(Ordering::Relaxed) - before, 2);
}

/// Mono buttons inverted, through a design system on the mono theme.
fn inverted_button(cx: &mut ThemeCx<'_>) {
    if let Some(r) = mono::THEME.recipe(&BUTTON_CLASS) {
        cx.add_recipe(r);
    }
    cx.add_style(Selector::MAIN, &mono::styles::INV);
}
static INVERTED_BUTTON: ComputedRecipe =
    ComputedRecipe::new("INVERTED_BUTTON", &BUTTON_CLASS, inverted_button);
struct InvertedMono;
impl DesignSystem for InvertedMono {
    type Base = Mono;
    const NAME: &'static str = "inverted_mono";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = mono::MODES;
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        INVERTED_BUTTON.for_class(class)
    }
}
static INVERTED: Theme<InvertedMono> = Theme::new();

#[test]
fn design_systems_restyle_built_in_classes_too() {
    let mut h = EngineHarness::new(128, 64).theme(&INVERTED);
    let b = create(&mut h, &BUTTON_CLASS);
    let key = create(&mut h, &KEY_CLASS);
    let e = h.engine();
    assert_eq!(bg(e, b), Color::BLACK, "inverted in light mode");
    assert_eq!(bg(e, key), Color::BLACK, "derived classes too");
}

recipe! {
    /// The gauge: the LVGL card with a primary indicator, from the theme's public styles.
    static GAUGE for GAUGE_CLASS {
        Selector::MAIN => { ..lvgl::styles::CARD },
        Part::Indicator => { ..lvgl::styles::BG_COLOR_PRIMARY },
    }
}
struct Gauges;
impl DesignSystem for Gauges {
    type Base = Lvgl;
    const NAME: &'static str = "gauges";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = lvgl::MODES;
    const RECIPES: &'static [&'static Recipe] = &[&GAUGE];
}
static GAUGES: Theme<Gauges> = Theme::new();

#[test]
fn theme_styles_make_a_custom_widget_look_native() {
    let mut h = EngineHarness::new(240, 160).theme(&GAUGES);
    let gauge = create(&mut h, &GAUGE_CLASS);
    let card = create(&mut h, &OBJ_CLASS);
    let e = h.engine();
    for prop in [
        PropId::BgColor,
        PropId::BorderColor,
        PropId::BorderWidth,
        PropId::Radius,
        PropId::PaddingTop,
    ] {
        assert_eq!(
            e.style_prop(gauge, Part::Main, prop),
            e.style_prop(card, Part::Main, prop),
            "{prop:?}"
        );
    }
    assert_eq!(
        e.style_color(gauge, Part::Indicator, PropId::BgColor),
        Palette::Blue.main()
    );
    // The styles are `static` data with design elements.
    assert!(matches!(
        lvgl::styles::CARD.get(PropId::BgColor),
        Some(twine_style::StyleValue::Element(_))
    ));
}

static SQUARE_STYLE: Style = style! { radius: 0 };
/// Cards: the LVGL card, square.
fn square_card(cx: &mut ThemeCx<'_>) {
    if let Some(r) = lvgl::THEME.recipe(&CARD_CLASS) {
        cx.add_recipe(r);
    }
    cx.add_style(Selector::MAIN, &SQUARE_STYLE);
}
static SQUARE_CARD: ComputedRecipe = ComputedRecipe::new("SQUARE_CARD", &CARD_CLASS, square_card);
struct SquareCards;
impl DesignSystem for SquareCards {
    type Base = Lvgl;
    const NAME: &'static str = "square_cards";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = lvgl::MODES;
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        SQUARE_CARD.for_class(class)
    }
}
static SQUARE: Theme<SquareCards> = Theme::new();

#[test]
fn card_matches_the_themes_card() {
    for (theme, mode) in [
        (&lvgl::THEME as &'static dyn ThemeHook, ThemeMode::Light),
        (&lvgl::THEME, ThemeMode::Dark),
        (&mono::THEME, ThemeMode::Light),
        (&simple::THEME, ThemeMode::Light),
    ] {
        let mut h = EngineHarness::new(240, 160).theme(theme).mode(mode);
        let s = h.screen();
        let card = h.engine_mut().create(s, Box::new(Card)).unwrap();
        let obj = create(&mut h, &OBJ_CLASS);
        let e = h.engine();
        assert!(e.tree().node(card).unwrap().class().is(&CARD_CLASS));
        for prop in [
            PropId::BgColor,
            PropId::BgOpacity,
            PropId::BorderWidth,
            PropId::BorderColor,
            PropId::Radius,
            PropId::PaddingLeft,
            PropId::RowGap,
        ] {
            assert_eq!(
                e.style_prop(card, Part::Main, prop),
                e.style_prop(obj, Part::Main, prop),
                "{} {prop:?}",
                theme.name()
            );
        }
    }
    // An application restyles cards only, not plain containers.
    let mut h = EngineHarness::new(240, 160).theme(&SQUARE);
    let s = h.screen();
    let card = h.engine_mut().create(s, Box::new(Card)).unwrap();
    let obj = create(&mut h, &OBJ_CLASS);
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::Radius), 0);
    assert!(h.engine().style_i32(obj, Part::Main, PropId::Radius) > 0);
}

#[test]
fn a_theme_without_a_card_look_styles_cards_like_containers() {
    /// Knows only plain objects.
    struct ObjOnly(&'static Style);
    impl ThemeHook for ObjOnly {
        fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
            if class.lineage().any(|c| c.is(&OBJ_CLASS)) && cx.parent().is_some() {
                cx.add_style(Selector::MAIN, self.0);
            }
        }
        fn font_normal(&self) -> &'static twine_text::Font {
            &twine_assets::fonts::MONTSERRAT_14
        }
    }
    static OBJ_ONLY: ObjOnly = ObjOnly(&RED);
    let mut h = EngineHarness::new(64, 48).theme(&OBJ_ONLY);
    let s = h.screen();
    let card = h.engine_mut().create(s, Box::new(Card)).unwrap();
    assert_eq!(bg(h.engine(), card), Color::RED);
}

#[test]
fn mono_starts_in_each_mode() {
    for mode in ThemeMode::ALL {
        let mut h = EngineHarness::new(128, 64).theme(&mono::THEME).mode(mode);
        let d = h.engine().default_display().unwrap();
        assert_eq!(h.engine().theme_mode(d), mode);
        let b = create(&mut h, &BUTTON_CLASS);
        let want = mono::THEME.design(mode).unwrap().get(design::SURFACE).unwrap();
        assert_eq!(bg(h.engine(), b), want, "{mode:?}");
    }
}

#[test]
fn simple_starts_in_its_modes_only() {
    /// The simple theme listing light and high contrast only.
    static TWO: twine_theme::Simple = twine_theme::Simple::new().modes(&[
        (ThemeMode::Light, &simple::LIGHT),
        (ThemeMode::HighContrast, &simple::HIGH_CONTRAST),
    ]);
    let h = EngineHarness::new(64, 48)
        .theme(&simple::THEME)
        .mode(ThemeMode::HighContrast);
    assert_eq!(h.engine().theme_mode(h.display()), ThemeMode::HighContrast);
    let h = EngineHarness::new(64, 48).theme(&TWO).mode(ThemeMode::Night);
    assert_eq!(
        h.engine().theme_mode(h.display()),
        ThemeMode::Light,
        "the first listed mode"
    );
}

/// A tuned element set with LVGL's own values is LVGL's table (same pixels); another primary
/// color changes them.
#[test]
fn tuned_sets_same_pixels() {
    static EXPLICIT_LIGHT: ElementTable = Lvgl::SET
        .primary(Palette::Blue.main())
        .secondary(Palette::Red.main())
        .fonts(FontScale::uniform(&twine_assets::fonts::MONTSERRAT_14))
        .table(ThemeMode::Light);
    static EXPLICIT: Lvgl = Lvgl::new().modes(&[(ThemeMode::Light, &EXPLICIT_LIGHT)]);
    static TEAL_LIGHT: ElementTable = Lvgl::SET
        .primary(Palette::Teal.main())
        .secondary(Palette::Amber.main())
        .table(ThemeMode::Light);
    static TEAL: Lvgl = Lvgl::new().modes(&[(ThemeMode::Light, &TEAL_LIGHT)]);
    let scene = |theme: &'static dyn ThemeHook, mode| {
        let mut h = EngineHarness::new(160, 100).theme(theme).mode(mode);
        let s = h.screen();
        let e = h.engine_mut();
        let c = e.create(s, Box::new(Card)).unwrap();
        e.set_size(c, 140, 80);
        let b = e.create(c, Box::new(Of(&BUTTON_CLASS))).unwrap();
        e.set_size(b, 60, 30);
        e.add_state(b, State::CHECKED);
        h.run_until_idle();
        h.panel_rgb888()
    };
    let explicit = scene(&EXPLICIT, ThemeMode::Light);
    assert_eq!(explicit, scene(&lvgl::THEME, ThemeMode::Light));
    assert_ne!(scene(&TEAL, ThemeMode::Light), explicit);
    assert_ne!(scene(&lvgl::THEME, ThemeMode::Dark), explicit);
}

#[test]
fn line_and_menu_greys_follow_every_mode() {
    let t = &lvgl::THEME;
    // The card's line color and the pressed menu row are the NEUTRAL element...
    assert_eq!(
        lvgl::styles::CARD.get(PropId::LineColor),
        Some(twine_style::StyleValue::Element(design::NEUTRAL.erase()))
    );
    assert_eq!(
        lvgl::styles::MENU_PRESSED.get(PropId::BgColor),
        Some(twine_style::StyleValue::Element(design::NEUTRAL.erase()))
    );
    // ...LVGL's grey in light and dark mode, the mode's own neutral otherwise.
    let neutral = |m| t.design(m).unwrap().get(design::NEUTRAL).unwrap();
    assert_eq!(neutral(ThemeMode::Light), Palette::Grey.main());
    assert_eq!(neutral(ThemeMode::Dark), Palette::Grey.main());
    assert_ne!(neutral(ThemeMode::Night), Palette::Grey.main());
    assert_ne!(neutral(ThemeMode::HighContrast), Palette::Grey.main());
    // Resolved on a node, it switches with the mode.
    let mut h = EngineHarness::new(240, 160).theme(&lvgl::THEME);
    let card = create(&mut h, &OBJ_CLASS);
    let d = h.engine().default_display().unwrap();
    for mode in ThemeMode::ALL {
        h.engine_mut().set_theme_mode(d, mode);
        assert_eq!(
            h.engine().style_color(card, Part::Main, PropId::LineColor),
            neutral(mode),
            "{mode:?}"
        );
    }
}
