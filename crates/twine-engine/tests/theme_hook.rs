//! The engine's theme hook: themes are applied before `Widget::init`, have the lowest
//! priority, and `set_theme` re-applies them with a single full-screen invalidation.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::{Color, Rect};
use twine_engine::{Engine, InvalidateReason, Obj, ThemeCx, ThemeHook, Widget, WidgetClass, WidgetCx};
use twine_style::{Part, PropId, Selector, StyleBuf, StyleProp};
use twine_testing::EngineHarness;
use twine_text::Font;

/// Gives every non-screen node a background color and counts its calls.
struct Toy {
    style: Rc<StyleBuf>,
    calls: Rc<Cell<u32>>,
}

impl Toy {
    fn new(c: Color) -> (Rc<Toy>, Rc<Cell<u32>>) {
        let calls = Rc::new(Cell::new(0));
        let t = Rc::new(Toy {
            style: Rc::new(StyleBuf::new().bg_color(c).radius(3)),
            calls: calls.clone(),
        });
        (t, calls)
    }
}

impl ThemeHook for Toy {
    fn apply(&self, cx: &mut ThemeCx<'_>, _class: &'static WidgetClass) {
        self.calls.set(self.calls.get() + 1);
        if cx.parent().is_some() {
            cx.add_style(Selector::MAIN, self.style.clone());
        }
    }
    fn font_normal(&self) -> &'static Font {
        &twine_assets::fonts::MONTSERRAT_14
    }
    fn name(&self) -> &'static str {
        "toy"
    }
}

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
    let (toy, _) = Toy::new(Color::RED);
    let mut h = EngineHarness::new(64, 48).theme(toy);
    let seen = Rc::new(Cell::new(None));
    let s = screen(h.engine());
    h.engine_mut().create(s, Box::new(Probe(seen.clone()))).unwrap();
    assert_eq!(seen.get(), Some(Color::RED));
}

#[test]
fn theme_styles_have_lowest_priority() {
    let (toy, _) = Toy::new(Color::RED);
    let mut h = EngineHarness::new(64, 48).theme(toy);
    let s = screen(h.engine());
    let e = h.engine_mut();
    let n = e.create(s, Box::new(Obj)).unwrap();
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::RED);
    e.add_style(n, Rc::new(StyleBuf::new().bg_color(Color::BLUE)), Selector::MAIN);
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::BLUE);
    e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(Color::GREEN));
    assert_eq!(e.style_color(n, Part::Main, PropId::BgColor), Color::GREEN);
    // Other properties of the theme still apply.
    assert_eq!(e.style_i32(n, Part::Main, PropId::Radius), 3);
    // The screen itself got no style from this theme.
    assert_eq!(e.style_i32(s, Part::Main, PropId::Radius), 0);
}

#[test]
fn set_theme_reapplies_and_invalidates_once() {
    let (red, _) = Toy::new(Color::RED);
    let (blue, calls) = Toy::new(Color::BLUE);
    let mut h = EngineHarness::new(64, 48).theme(red);
    let s = screen(h.engine());
    let (a, b) = {
        let e = h.engine_mut();
        let a = e.create(s, Box::new(Obj)).unwrap();
        e.set_pos(a, 2, 2);
        e.set_size(a, 20, 10);
        let b = e.create(a, Box::new(Obj)).unwrap();
        e.set_size(b, 5, 5);
        (a, b)
    };
    h.run_until_idle();
    let d = h.display();
    h.engine_mut().set_theme(d, blue);
    // Screen + two nodes (layers get no theme).
    assert_eq!(calls.get(), 3);
    for n in [a, b] {
        assert_eq!(
            h.engine().style_color(n, Part::Main, PropId::BgColor),
            Color::BLUE
        );
        let theme_entries = h
            .engine()
            .tree()
            .node(n)
            .unwrap()
            .styles()
            .entries()
            .iter()
            .filter(|e| e.kind == twine_style::EntryKind::Theme)
            .count();
        assert_eq!(theme_entries, 1, "the old theme's styles were removed");
    }
    h.update();
    let screen_area = Rect::from_xywh(0, 0, 64, 48);
    assert_eq!(h.invalidations(), &[(screen_area, InvalidateReason::StyleChange)]);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn nodes_without_theme_use_prop_defaults() {
    let mut h = EngineHarness::new(64, 48).no_theme();
    let s = screen(h.engine());
    let d = h.display();
    let e = h.engine_mut();
    let n = e.create(s, Box::new(Obj)).unwrap();
    for p in [PropId::BgColor, PropId::Radius, PropId::BgOpa] {
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
    let (toy, _) = Toy::new(Color::RED);
    let mut h = EngineHarness::new(64, 48).theme(toy);
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
    e.remove_theme(d);
    assert!(core::ptr::eq(
        e.style_font(n, Part::Main),
        &raw const twine_text::EMPTY_FONT
    ));
    assert_eq!(
        e.style_prop(n, Part::Main, PropId::Radius),
        PropId::Radius.meta().default
    );
}
