//! Widgets on a scaled display (R7.S04d): the LVGL theme's metrics and the widgets' own
//! logical sizes (line points) resolve to `scale ×` their 1× device pixels. Runs with and
//! without the engine's `display-scale` feature: without it the 2× panel is laid out at 1×.

use twine_core::{DisplayScale, Point};
use twine_engine::NodeId;
use twine_style::{Part, PropId};
use twine_testing::EngineHarness;
use twine_widgets::{button, line};

/// The scale the engine lays a `requested` display out at (1× without `display-scale`).
fn effective(h: &EngineHarness) -> i32 {
    i32::from(h.engine().display_scale(h.display()).raw_256() / 256)
}

/// A themed button and a line on a 160 × 120 logical screen at `scale`.
fn scene(scale: DisplayScale) -> (EngineHarness, NodeId, NodeId) {
    static ZIGZAG: [Point; 3] = [Point::new(0, 0), Point::new(30, 12), Point::new(50, 4)];
    let side = |v: i32| u16::try_from(scale.to_device(v)).unwrap();
    let mut h = EngineHarness::new(side(160), side(120)).display_scale(scale);
    let screen = h.screen();
    let e = h.engine_mut();
    let b = button::create(e, screen).unwrap();
    let l = line::create(e, screen).unwrap();
    e.with_widget_mut(l, |w: &mut line::Line, cx| w.set_points_static(cx, &ZIGZAG));
    h.run_until_idle();
    (h, b, l)
}

#[test]
fn theme_metrics_and_widget_sizes_scale_once() {
    let metrics = |h: &EngineHarness, b: NodeId| {
        [
            PropId::PaddingTop,
            PropId::PaddingLeft,
            PropId::Radius,
            PropId::ShadowWidth,
        ]
        .map(|p| h.engine().style_i32(b, Part::Main, p))
    };
    let (one, b1, l1) = scene(DisplayScale::ONE);
    let (two, b2, l2) = scene(DisplayScale::X2);
    let k = effective(&two);
    assert_eq!(metrics(&two, b2), metrics(&one, b1).map(|v| v * k));
    let size = |h: &EngineHarness, n: NodeId| {
        let c = h.engine().coords(n);
        (c.width(), c.height())
    };
    let (w, h) = size(&one, l1);
    assert_eq!((w, h), (50, 12));
    assert_eq!(size(&two, l2), (w * k, h * k));
}

/// A themed scene at 2× (golden image `display_scale_2x`): the LVGL theme's metrics, a
/// `ScaledFont` label drawn from its 2× rendering, a checkbox and a slider, all at twice their
/// 1× size. Compared only when the engine lays out at 2× (`display-scale`).
#[test]
fn themed_scene_at_2x() {
    use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_28};
    use twine_style::{Align, Selector, StyleProp};
    use twine_text::{Font, ScaledFont};
    use twine_widgets::{checkbox, label, slider};

    static BODY: Font = {
        static SET: ScaledFont = ScaledFont::new(&MONTSERRAT_14, &[(DisplayScale::X2, &MONTSERRAT_28)]);
        SET.font()
    };
    let mut h = EngineHarness::new(320, 240).display_scale(DisplayScale::X2);
    if effective(&h) != 2 {
        return; // laid out at 1× without the feature: nothing to compare
    }
    let screen = h.screen();
    let e = h.engine_mut();
    let b = button::create(e, screen).unwrap();
    e.align(b, Align::TopLeft, 8, 8);
    let l = label::create_with(e, b, "Play").unwrap();
    e.set_local_prop(
        l,
        Selector::MAIN,
        StyleProp::Font(twine_style::design::DesignValue::Fixed(&BODY)),
    );
    let c = checkbox::create(e, screen).unwrap();
    e.align(c, Align::TopLeft, 8, 56);
    let s = slider::create(e, screen).unwrap();
    e.set_width(s, 120);
    e.align(s, Align::TopLeft, 16, 96);
    h.run_until_idle();
    h.assert_snapshot("display_scale_2x");
}
