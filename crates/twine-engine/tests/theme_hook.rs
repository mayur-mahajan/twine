//! The engine's theme hook: themes are attached when the display is created, applied before
//! `Widget::init`, have the lowest priority; a mode switch swaps the `'static` mode table and
//! keeps the display's metrics table.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::{Color, Rect, Size};
use twine_engine::{Engine, InvalidateReason, Obj, ThemeCx, ThemeHook, Widget, WidgetClass, WidgetCx};
use twine_style::design::{self, ElementTable, LengthElement};
use twine_style::{Length, Part, PropId, Selector, Style, StyleBuf, StyleProp, ThemeMode, style};
use twine_testing::EngineHarness;
use twine_text::Font;

/// Gives every non-screen node a background color and counts its calls.
struct Toy {
    style: &'static Style,
    calls: &'static AtomicU32,
}

impl ThemeHook for Toy {
    fn apply(&self, cx: &mut ThemeCx<'_>, _class: &'static WidgetClass) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if cx.parent().is_some() {
            cx.add_style(Selector::MAIN, self.style);
        }
    }
    fn font_normal(&self) -> &'static Font {
        &twine_assets::fonts::MONTSERRAT_14
    }
    fn name(&self) -> &'static str {
        "toy"
    }
}

static RED_STYLE: Style = style! { bg_color: Color::RED, radius: 3 };

/// Records the background color it sees in `init`.
struct Probe(Rc<Cell<Option<Color>>>);
static PROBE_CLASS: WidgetClass = WidgetClass::new("probe");
impl Widget for Probe {
    fn class(&self) -> &'static WidgetClass {
        &PROBE_CLASS
    }
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        self.0.set(Some(cx.style_color(Part::Main, PropId::BgColor)));
    }
}

fn screen(e: &Engine) -> twine_engine::NodeId {
    e.active_screen(e.default_display().unwrap()).unwrap()
}

#[test]
fn theme_applied_before_init() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static THEME: Toy = Toy {
        style: &RED_STYLE,
        calls: &CALLS,
    };
    let mut h = EngineHarness::new(64, 48).theme(&THEME);
    let seen = Rc::new(Cell::new(None));
    let s = screen(h.engine());
    h.engine_mut().create(s, Box::new(Probe(seen.clone()))).unwrap();
    assert_eq!(seen.get(), Some(Color::RED));
}

#[test]
fn theme_styles_have_lowest_priority() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static THEME: Toy = Toy {
        style: &RED_STYLE,
        calls: &CALLS,
    };
    let mut h = EngineHarness::new(64, 48).theme(&THEME);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let n = e.create(s, Box::new(Obj)).unwrap();
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::RED);
    e.add_style(n, Rc::new(StyleBuf::new().bg_color(Color::BLUE)), Selector::MAIN);
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::BLUE);
    e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(Color::GREEN.into()));
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::GREEN);
    // Other properties of the theme still apply.
    assert_eq!(e.style_i32(n, Part::Main, PropId::Radius), 3);
    // The screen itself got no style from this theme.
    assert_eq!(e.style_i32(s, Part::Main, PropId::Radius), 0);
}

/// R7.S04: the theme is attached when the display is created — its first screen is themed
/// (the layers get nothing) — and every node created afterwards is themed once.
#[test]
fn theme_attached_at_creation_themes_each_node_once() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static THEME: Toy = Toy {
        style: &RED_STYLE,
        calls: &CALLS,
    };
    let mut h = EngineHarness::new(64, 48).theme(&THEME);
    let d = h.display();
    assert_eq!(h.engine().theme(d).map(ThemeHook::name), Some("toy"));
    // The harness rebuilt its engine with the theme: count from here.
    let before = CALLS.load(Ordering::Relaxed);
    let s = screen(h.engine());
    let n = h.engine_mut().create(s, Box::new(Obj)).unwrap();
    h.engine_mut().create(n, Box::new(Obj)).unwrap();
    assert_eq!(CALLS.load(Ordering::Relaxed) - before, 2);
}

#[test]
fn nodes_without_theme_use_prop_defaults() {
    let mut h = EngineHarness::new(64, 48).no_theme();
    let s = screen(h.engine());
    let d = h.display();
    let e = h.engine_mut();
    let n = e.create(s, Box::new(Obj)).unwrap();
    for p in [PropId::BgColor, PropId::Radius, PropId::BgOpacity] {
        assert_eq!(e.style_prop(n, Part::Main, p), p.meta().default);
        assert_eq!(e.style_prop(s, Part::Main, p), p.meta().default);
    }
    assert!(e.theme(d).is_none());
    assert!(core::ptr::eq(
        e.default_font(d),
        &raw const twine_text::EMPTY_FONT
    ));
}

#[test]
fn default_font_comes_from_the_theme() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static THEME: Toy = Toy {
        style: &RED_STYLE,
        calls: &CALLS,
    };
    let mut h = EngineHarness::new(64, 48).theme(&THEME);
    let d = h.display();
    let s = screen(h.engine());
    let e = h.engine_mut();
    assert!(core::ptr::eq(
        e.default_font(d),
        &raw const twine_assets::fonts::MONTSERRAT_14
    ));
    let n = e.create(s, Box::new(Obj)).unwrap();
    assert!(core::ptr::eq(
        e.style_font(n, Part::Main),
        &raw const twine_assets::fonts::MONTSERRAT_14
    ));
}

/// Supplies design elements in two modes and a per-display metrics table, and counts
/// `apply` and `metrics` calls (R2.S02, R7.S04).
struct TwoModes {
    calls: &'static AtomicU32,
    metrics_calls: &'static AtomicU32,
}

static LIGHT: ElementTable = ElementTable::new()
    .color(design::SURFACE, Color::WHITE)
    .length(design::SPACE_M, Length::Px(4));
static DARK: ElementTable = ElementTable::new().color(design::SURFACE, Color::BLACK);
/// A theme-private length of the metrics table: 10 px at 160 DPI.
const GAP: LengthElement = LengthElement::private(0);

impl ThemeHook for TwoModes {
    fn apply(&self, _: &mut ThemeCx<'_>, _: &'static WidgetClass) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
    fn font_normal(&self) -> &'static Font {
        &twine_assets::fonts::MONTSERRAT_14
    }
    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        match mode {
            ThemeMode::Light => Some(&LIGHT),
            ThemeMode::Dark => Some(&DARK),
            ThemeMode::Night | ThemeMode::HighContrast => None,
        }
    }
    fn metrics(&self, dpi: u16, _: Size) -> Option<ElementTable> {
        self.metrics_calls.fetch_add(1, Ordering::Relaxed);
        Some(
            ElementTable::new()
                .with(GAP, Length::Px(twine_style::dpx(10, dpi)))
                .with(design::SPACE_M, Length::Px(99))
                .with(design::SPACE_S, Length::Px(7)),
        )
    }
    fn modes(&self) -> &'static [ThemeMode] {
        &[ThemeMode::Light, ThemeMode::Dark]
    }
}

/// R7.S04: the mode table wins over the metrics table, an element only the metrics table
/// defines comes from it, and it is computed once per display — a mode switch keeps it.
#[test]
fn metrics_table_is_looked_up_after_the_mode_table() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static METRICS: AtomicU32 = AtomicU32::new(0);
    static THEME: TwoModes = TwoModes {
        calls: &CALLS,
        metrics_calls: &METRICS,
    };
    let mut h = EngineHarness::new(64, 48).dpi(320).theme(&THEME);
    let d = h.display();
    let built = METRICS.load(Ordering::Relaxed);
    let e = h.engine_mut();
    assert_eq!(
        e.design_value(d, design::SPACE_M),
        Some(Length::Px(4)),
        "the mode table wins"
    );
    assert_eq!(
        e.design_value(d, design::SPACE_S),
        Some(Length::Px(7)),
        "the metrics table's"
    );
    assert_eq!(e.design_value(d, GAP), Some(Length::Px(20)), "scaled for 320 DPI");
    let s = screen(e);
    let n = e.create(s, Box::new(Obj)).unwrap();
    e.set_local_prop(n, Selector::MAIN, StyleProp::PaddingTop(GAP.into()));
    assert_eq!(e.style_i32(n, Part::Main, PropId::PaddingTop), 20);
    e.set_theme_mode(d, ThemeMode::Dark);
    assert_eq!(
        e.design_value(d, design::SPACE_M),
        Some(Length::Px(99)),
        "Dark has none"
    );
    assert_eq!(e.style_i32(n, Part::Main, PropId::PaddingTop), 20);
    assert_eq!(
        METRICS.load(Ordering::Relaxed),
        built,
        "not rebuilt by a mode switch"
    );
}

#[test]
fn set_theme_mode_swaps_the_table_and_invalidates_once() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static METRICS: AtomicU32 = AtomicU32::new(0);
    static THEME: TwoModes = TwoModes {
        calls: &CALLS,
        metrics_calls: &METRICS,
    };
    let mut h = EngineHarness::new(64, 48).theme(&THEME);
    let s = screen(h.engine());
    let n = {
        let e = h.engine_mut();
        let n = e.create(s, Box::new(Obj)).unwrap();
        e.set_size(n, 20, 10);
        e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(design::SURFACE.into()));
        e.set_local_prop(
            n,
            Selector::MAIN,
            StyleProp::BgOpacity(twine_core::Opa::COVER.into()),
        );
        n
    };
    h.run_until_idle();
    let d = h.display();
    assert_eq!(h.engine().cached_main(n).bg_color, Color::WHITE);
    let (applied, epoch) = (CALLS.load(Ordering::Relaxed), h.engine().design_epoch(d));
    h.engine_mut().set_theme_mode(d, ThemeMode::Dark);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        applied,
        "the theme's styles are not re-applied"
    );
    assert_ne!(h.engine().design_epoch(d), epoch);
    assert_eq!(h.engine().theme_mode(d), ThemeMode::Dark);
    // The cached hot properties were refreshed too.
    assert_eq!(h.engine().cached_main(n).bg_color, Color::BLACK);
    assert_eq!(h.engine().design_value(d, design::SURFACE), Some(Color::BLACK));
    h.update();
    assert_eq!(
        h.invalidations(),
        &[(Rect::from_xywh(0, 0, 64, 48), InvalidateReason::StyleChange)]
    );
    h.run_until_idle();
    // Idempotent: the current mode does nothing.
    let epoch = h.engine().design_epoch(d);
    h.engine_mut().set_theme_mode(d, ThemeMode::Dark);
    assert_eq!(h.engine().design_epoch(d), epoch);
    h.assert_idle();
    // R2.S03: an unsupported mode is the documented fallback — the display keeps its mode
    // and table, nothing is redrawn — plus a warning; never a panic.
    assert_eq!(h.engine().theme_modes(d), &[ThemeMode::Light, ThemeMode::Dark]);
    let ((), logs) = twine_testing::capture_logs(|| h.engine_mut().set_theme_mode(d, ThemeMode::Night));
    assert_eq!(h.engine().theme_mode(d), ThemeMode::Dark);
    assert_eq!(h.engine().design_epoch(d), epoch);
    assert_eq!(h.engine().cached_main(n).bg_color, Color::BLACK);
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("has no Night mode")),
        "{logs:?}"
    );
    h.assert_idle();
}

/// A display created without a theme refuses a mode switch: nothing changes, no panic.
#[test]
fn set_theme_mode_without_a_theme_is_refused() {
    let mut h = EngineHarness::new(64, 48).no_theme();
    let d = h.display();
    let s = screen(h.engine());
    let n = h.engine_mut().create(s, Box::new(Obj)).unwrap();
    h.engine_mut()
        .set_local_prop(n, Selector::MAIN, StyleProp::BgColor(design::SURFACE.into()));
    h.run_until_idle();
    h.engine_mut().set_theme_mode(d, ThemeMode::Dark);
    assert_eq!(h.engine().theme_mode(d), ThemeMode::Light);
    assert_eq!(h.engine().theme_modes(d), []);
    assert_eq!(
        h.engine().style_color(n, Part::Main, PropId::BgColor),
        Color::WHITE,
        "an element without a table gives the property's default"
    );
    h.assert_idle();
}

/// R7.S04: a start mode the theme does not list starts in its first mode, with a warning.
#[test]
fn unlisted_start_mode_falls_back_to_the_first_mode() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static METRICS: AtomicU32 = AtomicU32::new(0);
    static THEME: TwoModes = TwoModes {
        calls: &CALLS,
        metrics_calls: &METRICS,
    };
    let (h, logs) =
        twine_testing::capture_logs(|| EngineHarness::new(64, 48).theme(&THEME).mode(ThemeMode::Night));
    assert_eq!(h.engine().theme_mode(h.display()), ThemeMode::Light);
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("has no Night mode")),
        "{logs:?}"
    );
}
