//! Typed widget classes in themes (R2.S04): identity matching along `WidgetClass::base`,
//! application class registrations, public theme styles, the card class, builders, and colors
//! that follow every theme mode.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::{Color, Opa, Size};
use twine_engine::{Engine, NodeId, OBJ_CLASS, ObjFlags, ThemeHook, Widget, WidgetClass};
use twine_style::{Part, PropId, Selector, State, StyleBuf, ThemeMode, design};
use twine_testing::EngineHarness;
use twine_theme::{DefaultTheme, FontScale, MonoTheme, Palette, SimpleTheme};
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
/// A gauge styled only by an application registration.
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

#[test]
fn custom_class_with_button_base_gets_button_styling() {
    for theme in [
        Rc::new(DefaultTheme::light()) as Rc<dyn ThemeHook>,
        Rc::new(MonoTheme::builder().build()),
        Rc::new(SimpleTheme::new()),
    ] {
        let mut h = EngineHarness::new(240, 160).theme(theme.clone());
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
    let mut h = EngineHarness::new(240, 160).theme(DefaultTheme::light());
    let key = create(&mut h, &KEY_CLASS);
    h.engine_mut().add_state(key, State::PRESSED);
    h.run_until_idle();
    assert_eq!(
        h.engine().style_opa(key, Part::Main, PropId::RecolorOpacity),
        Opa::from_raw(35)
    );
}

#[test]
fn app_registered_class_style_is_applied_and_overrides_the_base() {
    let calls = Rc::new(Cell::new(0));
    let c = calls.clone();
    let theme = DefaultTheme::builder()
        .class(&KEY_CLASS, move |cx| {
            c.set(c.get() + 1);
            assert!(cx.class().is_a(&KEY_CLASS));
            cx.add_style(Selector::MAIN, Rc::new(StyleBuf::new().bg_color(Color::RED)));
        })
        .class(&BIG_KEY_CLASS, |cx| {
            cx.add_style(Selector::MAIN, Rc::new(StyleBuf::new().bg_color(Color::GREEN)));
        })
        .build();
    let mut h = EngineHarness::new(240, 160).theme(theme);
    let button = create(&mut h, &BUTTON_CLASS);
    let key = create(&mut h, &KEY_CLASS);
    let big = create(&mut h, &BIG_KEY_CLASS);
    let e = h.engine();
    // The base's styles stay (radius, shadow), the registration wins where both set a value.
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
    // The most derived registration runs last and wins; the key's runs for the big key too.
    assert_eq!(bg(e, big), Color::GREEN);
    assert_eq!(calls.get(), 2);
}

#[test]
fn registrations_style_built_in_classes_and_screens_too() {
    let theme = MonoTheme::builder()
        .class(&BUTTON_CLASS, |cx| {
            let inv = cx.styles().inv.clone();
            cx.add_style(Selector::MAIN, inv);
        })
        .build();
    let mut h = EngineHarness::new(128, 64).theme(theme);
    let b = create(&mut h, &BUTTON_CLASS);
    let key = create(&mut h, &KEY_CLASS);
    let e = h.engine();
    assert_eq!(bg(e, b), Color::BLACK, "inverted in light mode");
    assert_eq!(bg(e, key), Color::BLACK, "derived classes too");
}

#[test]
fn theme_styles_make_a_custom_widget_look_native() {
    let theme = DefaultTheme::builder()
        .class(&GAUGE_CLASS, |cx| {
            let s = cx.styles();
            let (card, primary) = (s.card.clone(), s.bg_color_primary.clone());
            cx.add_style(Selector::MAIN, card);
            cx.add_style(Selector::part(Part::Indicator), primary);
        })
        .build();
    let mut h = EngineHarness::new(240, 160).theme(theme);
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
    // The same style set is public on the theme itself.
    let t = DefaultTheme::light();
    let s = t.styles(130, Size::new(240, 160));
    assert!(Rc::ptr_eq(&s, &t.styles(130, Size::new(240, 160))));
    assert!(s.card.get(PropId::BgColor).is_some());
}

#[test]
fn card_matches_the_themes_card() {
    for theme in [
        Rc::new(DefaultTheme::light()) as Rc<dyn ThemeHook>,
        Rc::new(DefaultTheme::dark()),
        Rc::new(MonoTheme::builder().build()),
        Rc::new(SimpleTheme::new()),
    ] {
        let mut h = EngineHarness::new(240, 160).theme(theme.clone());
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
    let theme = DefaultTheme::builder()
        .class(&CARD_CLASS, |cx| {
            cx.add_style(Selector::MAIN, Rc::new(StyleBuf::new().radius(0)));
        })
        .build();
    let mut h = EngineHarness::new(240, 160).theme(theme);
    let s = h.screen();
    let card = h.engine_mut().create(s, Box::new(Card)).unwrap();
    let obj = create(&mut h, &OBJ_CLASS);
    assert_eq!(h.engine().style_i32(card, Part::Main, PropId::Radius), 0);
    assert!(h.engine().style_i32(obj, Part::Main, PropId::Radius) > 0);
}

#[test]
fn a_theme_without_a_card_look_styles_cards_like_containers() {
    /// Knows only plain objects.
    struct ObjOnly(Rc<StyleBuf>);
    impl ThemeHook for ObjOnly {
        fn apply(&self, cx: &mut twine_engine::ThemeCx<'_>, class: &'static WidgetClass) {
            if class.lineage().any(|c| c.is(&OBJ_CLASS)) && cx.parent().is_some() {
                cx.add_style(Selector::MAIN, self.0.clone());
            }
        }
        fn font_normal(&self) -> &'static twine_text::Font {
            &twine_assets::fonts::MONTSERRAT_14
        }
    }
    let mut h = EngineHarness::new(64, 48).theme(ObjOnly(Rc::new(StyleBuf::new().bg_color(Color::RED))));
    let s = h.screen();
    let card = h.engine_mut().create(s, Box::new(Card)).unwrap();
    assert_eq!(bg(h.engine(), card), Color::RED);
}

#[test]
fn mono_starts_in_each_mode() {
    let size = Size::new(128, 64);
    for mode in ThemeMode::ALL {
        let t = MonoTheme::builder().mode(mode).build();
        assert_eq!(ThemeHook::mode(&t), mode);
        let mut h = EngineHarness::new(128, 64).theme(t);
        let d = h.engine().default_display().unwrap();
        assert_eq!(h.engine().theme_mode(d), mode);
        let b = create(&mut h, &BUTTON_CLASS);
        let want = MonoTheme::builder()
            .build()
            .design(mode, 130, size)
            .unwrap()
            .get(design::SURFACE)
            .unwrap();
        assert_eq!(bg(h.engine(), b), want, "{mode:?}");
    }
}

#[test]
fn simple_starts_in_its_modes_only() {
    assert_eq!(
        ThemeHook::mode(&SimpleTheme::builder().mode(ThemeMode::HighContrast).build()),
        ThemeMode::HighContrast
    );
    assert_eq!(
        ThemeHook::mode(&SimpleTheme::builder().mode(ThemeMode::Night).build()),
        ThemeMode::Light
    );
}

/// The builders produce exactly the themes of the old positional constructors (same pixels).
#[test]
fn builder_equivalence_same_pixels() {
    let f = &twine_assets::fonts::MONTSERRAT_14;
    let scene = |theme: Rc<dyn ThemeHook>| {
        let mut h = EngineHarness::new(160, 100).theme(theme);
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
    let explicit = scene(Rc::new(
        DefaultTheme::builder()
            .primary(Palette::Blue)
            .secondary(Palette::Red)
            .mode(ThemeMode::Light)
            .fonts(FontScale::uniform(f))
            .build(),
    ));
    assert_eq!(explicit, scene(Rc::new(DefaultTheme::light())));
    assert_eq!(
        scene(Rc::new(DefaultTheme::builder().mode(ThemeMode::Dark).build())),
        scene(Rc::new(DefaultTheme::dark()))
    );
    let teal = scene(Rc::new(
        DefaultTheme::builder()
            .primary(Palette::Teal)
            .secondary(Palette::Amber)
            .build(),
    ));
    let teal_colors = scene(Rc::new(
        DefaultTheme::builder()
            .primary(Palette::Teal.main())
            .secondary(Color::from(Palette::Amber))
            .build(),
    ));
    assert_eq!(teal, teal_colors);
    assert_ne!(teal, explicit);
}

#[test]
fn line_and_menu_greys_follow_every_mode() {
    let t = DefaultTheme::light();
    let size = Size::new(240, 160);
    let s = t.styles(130, size);
    // The card's line color and the pressed menu row are the NEUTRAL element...
    assert_eq!(
        s.card.get(PropId::LineColor),
        Some(twine_style::StyleValue::Element(design::NEUTRAL.erase()))
    );
    assert_eq!(
        s.menu_pressed.get(PropId::BgColor),
        Some(twine_style::StyleValue::Element(design::NEUTRAL.erase()))
    );
    // ...LVGL's grey in light and dark mode, the mode's own neutral otherwise.
    let neutral = |m| t.design(m, 130, size).unwrap().get(design::NEUTRAL).unwrap();
    assert_eq!(neutral(ThemeMode::Light), Palette::Grey.main());
    assert_eq!(neutral(ThemeMode::Dark), Palette::Grey.main());
    assert_ne!(neutral(ThemeMode::Night), Palette::Grey.main());
    assert_ne!(neutral(ThemeMode::HighContrast), Palette::Grey.main());
    // Resolved on a node, it switches with the mode.
    let mut h = EngineHarness::new(240, 160).theme(DefaultTheme::light());
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
