//! R2.S02: design elements. Styles name theme values (`design::SURFACE`, application elements)
//! in every style API (`style!`, `StyleBuf`, view modifiers, part/state scopes); switching the
//! theme mode recolors them without rebuilding views or re-running bindings, allocates nothing
//! once the theme built its tables, and `ThemeHandle::mode()` / `get()` are tracked.

use std::cell::Cell;
use std::rc::Rc;

use twine_engine::NodeId;
use twine_style::design::ColorElement;
use twine_style::{StyleValue, ThemeMode};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id};
use twine_theme::default::colors;
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// An application element: the color of a "boost" zone, one value per mode.
const BOOST: ColorElement = ColorElement::custom(0);
const BOOST_LIGHT: Color = Color::hex(0x00E6_5100);
const BOOST_DARK: Color = Color::hex(0x00FF_B74D);

fn theme() -> Rc<dyn twine_engine::ThemeHook> {
    Rc::new(
        DefaultTheme::builder()
            .element(BOOST, BOOST_LIGHT)
            .element_in(ThemeMode::Light, BOOST, BOOST_LIGHT)
            .element_in(ThemeMode::Dark, BOOST, BOOST_DARK)
            .build(),
    )
}

static THEMED: Style = style! { bg: design::SURFACE, border: (2, design::OUTLINE) };

thread_local! {
    static BINDING_RUNS: Cell<u32> = const { Cell::new(0) };
}

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn color(t: &TestUi, id: &'static str, part: Part, p: PropId) -> Color {
    t.engine().style_color(node(t, id), part, p)
}

/// One view per style API, each using design elements.
fn app(cx: Scope) -> impl View {
    let alarm = cx.signal(false);
    cx.provide(alarm);
    column((
        container(()).size(20, 20).style(&THEMED).test_id("style_macro"),
        container(())
            .size(20, 20)
            .op(|cx, n| {
                let s = Rc::new(StyleBuf::new().bg(design::PRIMARY).radius(design::RADIUS_M));
                cx.engine().add_style(n, s, Selector::MAIN);
            })
            .test_id("style_buf"),
        container(())
            .size(20, 20)
            .bg(design::SURFACE_VARIANT)
            .border(1, BOOST)
            .padding(design::SPACE_S)
            .test_id("modifier"),
        // A closure property returning an element: it runs once; the theme does the rest.
        container(())
            .size(20, 20)
            .bg(move || {
                BINDING_RUNS.with(|r| r.set(r.get() + 1));
                design::BACKGROUND
            })
            .test_id("closure"),
        slider(cx.signal(50))
            .part(Part::Indicator, |s| {
                s.bg(move || if alarm.get() { design::DANGER } else { design::OK })
            })
            .on_state(State::custom::<0>(), |s| s.bg(design::SECONDARY))
            .test_id("slider"),
    ))
}

#[test]
fn light_to_dark_recolors_without_rebuilding_or_rebinding() {
    let mut t = TestUi::new(200, 200).theme(theme()).mount(app);
    t.run_until_idle();
    let ids: Vec<NodeId> = ["style_macro", "style_buf", "modifier", "closure", "slider"]
        .iter()
        .map(|id| node(&t, id))
        .collect();
    let runs = BINDING_RUNS.with(Cell::get);
    let m = Part::Main;
    assert_eq!(color(&t, "style_macro", m, PropId::BgColor), colors::LIGHT_CARD);
    assert_eq!(
        color(&t, "style_macro", m, PropId::BorderColor),
        colors::LIGHT_GREY
    );
    assert_eq!(color(&t, "style_buf", m, PropId::BgColor), Palette::Blue.main());
    assert_eq!(color(&t, "modifier", m, PropId::BgColor), colors::LIGHT_GREY);
    assert_eq!(color(&t, "modifier", m, PropId::BorderColor), BOOST_LIGHT);
    assert_eq!(color(&t, "closure", m, PropId::BgColor), colors::LIGHT_SCR);
    assert_eq!(
        color(&t, "slider", Part::Indicator, PropId::BgColor),
        Palette::Green.main()
    );
    // Spacing and radius elements resolve to the theme's pixel values.
    let dpi = t.engine().node_dpi(node(&t, "modifier"));
    let pad_small = twine_style::dpx(10, dpi); // a 200 px display is a small one
    assert_eq!(
        t.engine().style_i32(node(&t, "modifier"), m, PropId::PaddingTop),
        pad_small
    );
    assert_eq!(
        t.engine().style_i32(node(&t, "style_buf"), m, PropId::Radius),
        twine_style::dpx(8, dpi)
    );

    use_theme(t.root_scope()).set_mode(ThemeMode::Dark);
    t.run_until_idle();
    assert_eq!(
        t.engine().theme_mode(t.engine().default_display().unwrap()),
        ThemeMode::Dark
    );
    // The same nodes (nothing rebuilt), the closure did not run again, every color follows.
    let after: Vec<NodeId> = ["style_macro", "style_buf", "modifier", "closure", "slider"]
        .iter()
        .map(|id| node(&t, id))
        .collect();
    assert_eq!(ids, after);
    assert_eq!(BINDING_RUNS.with(Cell::get), runs);
    assert_eq!(color(&t, "style_macro", m, PropId::BgColor), colors::DARK_CARD);
    assert_eq!(
        color(&t, "style_macro", m, PropId::BorderColor),
        colors::DARK_GREY
    );
    assert_eq!(color(&t, "modifier", m, PropId::BgColor), colors::DARK_GREY);
    assert_eq!(color(&t, "modifier", m, PropId::BorderColor), BOOST_DARK);
    assert_eq!(color(&t, "closure", m, PropId::BgColor), colors::DARK_SCR);

    // A signal-driven element in a part scope still follows its signal...
    t.root_scope().expect_context::<Signal<bool>>().set(true);
    t.run_until_idle();
    assert_eq!(
        color(&t, "slider", Part::Indicator, PropId::BgColor),
        Palette::Red.main()
    );
    // ...and an element in a state scope applies in that state.
    let s = node(&t, "slider");
    t.engine_mut().add_state(s, State::custom::<0>());
    assert_eq!(color(&t, "slider", m, PropId::BgColor), Palette::Red.main());

    // Back to light.
    use_theme(t.root_scope()).set_mode(ThemeMode::Light);
    t.run_until_idle();
    assert_eq!(color(&t, "style_macro", m, PropId::BgColor), colors::LIGHT_CARD);
    assert_eq!(color(&t, "modifier", m, PropId::BorderColor), BOOST_LIGHT);
}

#[test]
fn theme_mode_and_get_are_tracked() {
    let mut t = TestUi::new(200, 100).theme(theme()).mount(|cx| {
        let theme = use_theme(cx);
        column((
            label(move || format!("{:?}", theme.mode())).test_id("mode"),
            label(move || match theme.get(BOOST) {
                Some(c) => c.to_string(),
                None => String::from("none"),
            })
            .test_id("boost"),
        ))
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("mode")).text(), "Light");
    assert_eq!(t.find(by_id("boost")).text(), BOOST_LIGHT.to_string());

    // Through the handle (queued between updates).
    use_theme(t.root_scope()).set_mode(ThemeMode::Dark);
    t.run_until_idle();
    assert_eq!(t.find(by_id("mode")).text(), "Dark");
    assert_eq!(t.find(by_id("boost")).text(), BOOST_DARK.to_string());

    // Through the engine directly: the `Ui` notices too.
    let d = t.engine().default_display().unwrap();
    t.engine_mut().set_theme_mode(d, ThemeMode::Light);
    t.run_until_idle();
    assert_eq!(t.find(by_id("mode")).text(), "Light");
    assert_eq!(use_theme(t.root_scope()).mode(), ThemeMode::Light);

    // A new theme (same mode) re-evaluates `get`.
    use_theme(t.root_scope()).set(DefaultTheme::builder().element(BOOST, Color::RED).build());
    t.run_until_idle();
    assert_eq!(t.find(by_id("boost")).text(), Color::RED.to_string());
}

#[test]
fn switching_modes_allocates_nothing_after_warm_up() {
    let mut t = TestUi::new(200, 200).theme(theme()).mount(app);
    t.run_until_idle();
    let handle = use_theme(t.root_scope());
    assert_eq!(handle.modes(), &ThemeMode::ALL);
    // Warm-up: the theme builds its table of each mode once.
    for mode in ThemeMode::ALL {
        handle.set_mode(mode);
        t.run_until_idle();
    }
    // R2.S03: every mode, night and high contrast included, and cycling with `next_in`.
    for mode in [
        ThemeMode::Dark,
        ThemeMode::Night,
        ThemeMode::HighContrast,
        ThemeMode::Light,
        ThemeMode::HighContrast,
        ThemeMode::Dark,
    ] {
        let ((), stats) = count_allocs(|| {
            handle.set_mode(mode);
            t.update();
        });
        assert_eq!(stats.allocs, 0, "switch to {mode:?}: {stats:?}");
        assert_eq!(stats.reallocs, 0, "switch to {mode:?}: {stats:?}");
    }
    let ((), stats) = count_allocs(|| {
        handle.set_mode(handle.mode().next_in(handle.modes()));
        t.update();
    });
    assert_eq!((stats.allocs, stats.reallocs), (0, 0), "cycle: {stats:?}");
    assert_eq!(handle.mode(), ThemeMode::Night);
    handle.set_mode(ThemeMode::Dark);
    t.run_until_idle();
    assert_eq!(
        t.engine()
            .style_color(node(&t, "style_macro"), Part::Main, PropId::BgColor),
        colors::DARK_CARD
    );
}

#[test]
fn unsupported_mode_is_ignored() {
    // The simple theme has no dark mode (light and high contrast only).
    let mut t = TestUi::new(100, 100)
        .theme(Rc::new(SimpleTheme::new()))
        .mount(|_| container(()).size(10, 10).bg(design::SURFACE).test_id("c"));
    t.run_until_idle();
    assert_eq!(
        use_theme(t.root_scope()).modes(),
        &[ThemeMode::Light, ThemeMode::HighContrast]
    );
    use_theme(t.root_scope()).set_mode(ThemeMode::Dark);
    t.run_until_idle();
    assert_eq!(use_theme(t.root_scope()).mode(), ThemeMode::Light);
    assert_eq!(
        t.engine().style_prop(node(&t, "c"), Part::Main, PropId::BgColor),
        StyleValue::Color(Color::WHITE)
    );
}
