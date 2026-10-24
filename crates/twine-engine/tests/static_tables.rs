//! Const design element tables (R7.S02): a theme whose tables are `'static` data — built from
//! the typed sets, with application (`elements!`) and theme-private elements — resolves every
//! element kind through the engine, and switching modes or resolving allocates nothing.

use twine_anim::AnimSpec;
use twine_core::{Color, Duration, Opa};
use twine_engine::{Engine, NodeId, Obj, ThemeCx, ThemeHook, WidgetClass};
use twine_image::ImageSource;
use twine_style::design::{
    self, DesignSet, ElementTable, Elevation, ElevationLevel, FontSet, IconSet, LengthElement, MotionSet,
    Shape, Spacing, SystemColors,
};
use twine_style::{Length, Part, PropId, Radius, Selector, StyleProp, StyleValue, ThemeMode};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

static FAST: AnimSpec = AnimSpec::new(Duration::ms(90));
static CHECK: ImageSource = ImageSource::Symbol("v");

const fn set(fg: Color, bg: Color) -> DesignSet {
    DesignSet {
        colors: SystemColors::monochrome(fg, bg),
        spacing: Spacing {
            xs: Length::Px(1),
            s: Length::Px(2),
            m: Length::Px(4),
            l: Length::Px(8),
        },
        shape: Shape {
            s: Radius::Px(2),
            m: Radius::Px(4),
            l: Radius::Px(8),
            full: Radius::Circle,
        },
        elevation: Elevation {
            level_1: ElevationLevel {
                width: Length::Px(3),
                offset_y: Length::Px(2),
                spread: Length::Px(0),
            },
            ..Elevation::NONE
        },
        motion: MotionSet {
            fast: &FAST,
            ..MotionSet::INSTANT
        },
        icons: IconSet {
            check: Some(&CHECK),
            ..IconSet::NONE
        },
        fonts: FontSet::uniform(&twine_text::EMPTY_FONT),
    }
}

twine_style::elements! {
    /// The application's elements.
    struct App {
        /// A brand color.
        BRAND: Color,
        /// A gauge's track width.
        TRACK: Length,
    }
}

const APP_LIGHT: App = App {
    BRAND: Color::hex(0x0062_00EE),
    TRACK: Length::Px(6),
};
const APP_DARK: App = App {
    BRAND: Color::hex(0x00BB_86FC),
    TRACK: Length::Px(6),
};

/// A theme-private element (a size only the theme's own styles use).
const KNOB_INSET: LengthElement = LengthElement::private(0);
static PRIVATE: [Option<StyleValue>; 1] = [Some(StyleValue::Length(Length::Px(3)))];

static LIGHT: ElementTable = set(Color::BLACK, Color::WHITE)
    .to_table()
    .custom(&const { APP_LIGHT.slots() })
    .private(&PRIVATE);
static DARK: ElementTable = set(Color::WHITE, Color::BLACK)
    .to_table()
    .custom(&const { APP_DARK.slots() })
    .private(&PRIVATE);

/// A theme of const tables only.
struct Flash;
static FLASH: Flash = Flash;

impl ThemeHook for Flash {
    fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {}
    fn font_normal(&self) -> &'static twine_text::Font {
        &twine_text::EMPTY_FONT
    }
    fn modes(&self) -> &'static [ThemeMode] {
        &[ThemeMode::Light, ThemeMode::Dark]
    }
    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        match mode {
            ThemeMode::Light => Some(&LIGHT),
            ThemeMode::Dark => Some(&DARK),
            ThemeMode::Night | ThemeMode::HighContrast => None,
        }
    }
}

/// A node styled with an element of every kind.
fn styled(e: &mut Engine, screen: NodeId) -> NodeId {
    let n = e.create(screen, Box::new(Obj)).unwrap();
    for p in [
        StyleProp::BgColor(design::SURFACE.into()),
        StyleProp::BorderColor(App::BRAND.into()),
        StyleProp::PaddingTop(design::SPACE_M.into()),
        StyleProp::MarginTop(KNOB_INSET.into()),
        StyleProp::Radius(design::RADIUS_FULL.into()),
        StyleProp::BgOpacity(design::SCRIM_OPACITY.into()),
        StyleProp::Font(design::FONT_BODY.into()),
        StyleProp::Anim(design::MOTION_FAST.into()),
        StyleProp::BgImage(design::ICON_CHECK.into()),
    ] {
        e.set_local_prop(n, Selector::MAIN, p);
    }
    n
}

#[test]
fn every_kind_resolves_from_static_tables() {
    let mut h = EngineHarness::new(64, 48).theme(&FLASH);
    let screen = h.engine().active_screen(h.display()).unwrap();
    let n = styled(h.engine_mut(), screen);
    let e = h.engine();
    let prop = |p| e.style_prop(n, Part::Main, p);
    assert_eq!(prop(PropId::BgColor), StyleValue::Color(Color::WHITE));
    assert_eq!(
        prop(PropId::BorderColor),
        StyleValue::Color(Color::hex(0x0062_00EE))
    );
    assert_eq!(prop(PropId::PaddingTop), StyleValue::Length(Length::Px(4)));
    assert_eq!(prop(PropId::MarginTop), StyleValue::Length(Length::Px(3)));
    assert_eq!(prop(PropId::Radius).get::<Radius>(), Some(Radius::Circle));
    assert_eq!(prop(PropId::BgOpacity), StyleValue::Opa(Opa::COVER));
    assert_eq!(prop(PropId::Font), StyleValue::Font(&twine_text::EMPTY_FONT));
    assert_eq!(prop(PropId::Anim), StyleValue::AnimSpec(&FAST));
    assert_eq!(prop(PropId::BgImage), StyleValue::Image(&CHECK));
    // The engine reads the `'static` table, not a copy.
    assert!(
        e.design_table(h.display())
            .is_some_and(|t| core::ptr::eq(t, &raw const LIGHT))
    );
    assert_eq!(e.design_value(h.display(), App::TRACK), Some(Length::Px(6)));
    assert_eq!(
        e.design_value(h.display(), design::ELEVATION_1_WIDTH),
        Some(Length::Px(3))
    );
}

#[test]
fn static_tables_switch_and_resolve_without_allocating() {
    let mut h = EngineHarness::new(64, 48).theme(&FLASH);
    let d = h.display();
    let screen = h.engine().active_screen(d).unwrap();
    let n = styled(h.engine_mut(), screen);
    // Warm up: the first switch and draw size the engine's own buffers.
    h.engine_mut().set_theme_mode(d, ThemeMode::Dark);
    h.engine_mut().set_theme_mode(d, ThemeMode::Light);
    let ((), stats) = count_allocs(|| {
        for mode in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Dark] {
            h.engine_mut().set_theme_mode(d, mode);
            for p in [
                PropId::BgColor,
                PropId::BorderColor,
                PropId::Anim,
                PropId::BgImage,
                PropId::MarginTop,
            ] {
                let _ = h.engine().style_prop(n, Part::Main, p);
            }
        }
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
    assert_eq!(
        h.engine().style_prop(n, Part::Main, PropId::BorderColor),
        StyleValue::Color(Color::hex(0x00BB_86FC))
    );
    assert_eq!(h.engine().theme_mode(d), ThemeMode::Dark);
}
