//! Typed units in styles (R1.S05): density-independent lengths resolve with the DPI of the
//! node's display, pixel lengths do not, and `Radius::Circle` draws exactly like the
//! renderer's circle marker.

use twine_core::{Color, Opa};
use twine_engine::{NodeId, Obj};
use twine_render::RADIUS_CIRCLE;
use twine_style::{Length, Part, PropId, Radius, Selector, StyleProp, StyleValue};
use twine_testing::EngineHarness;

/// A 100 × 60 child of the screen with `props`, laid out on a panel of `dpi`.
fn node_at(dpi: u16, props: &[StyleProp]) -> (EngineHarness, NodeId) {
    let mut h = EngineHarness::new(200, 120).no_theme().dpi(dpi);
    let screen = h.screen();
    let e = h.engine_mut();
    let n = e.create(screen, Box::new(Obj)).unwrap();
    for p in props {
        e.set_local_prop(n, Selector::MAIN, *p);
    }
    h.run_until_idle();
    (h, n)
}

#[test]
fn dp_resolves_with_the_display_dpi() {
    let props = [
        StyleProp::Width(Length::dp(50)),
        StyleProp::Height(Length::Px(30)),
        StyleProp::PaddingLeft(Length::dp(10)),
        StyleProp::BorderWidth(Length::dp(1)),
        StyleProp::Radius(Radius::Dp(4)),
        StyleProp::RowGap(Length::dp(6)),
    ];
    let (lo, a) = node_at(160, &props);
    let (hi, b) = node_at(320, &props);
    let px = |h: &EngineHarness, n, p| h.engine().style_i32(n, Part::Main, p);
    // 160 DPI is the reference: 1 dp = 1 px; 320 DPI doubles every Dp value.
    assert_eq!(px(&lo, a, PropId::PaddingLeft), 10);
    assert_eq!(px(&hi, b, PropId::PaddingLeft), 20);
    assert_eq!(px(&lo, a, PropId::BorderWidth), 1);
    assert_eq!(px(&hi, b, PropId::BorderWidth), 2);
    assert_eq!(px(&lo, a, PropId::Radius), 4);
    assert_eq!(px(&hi, b, PropId::Radius), 8);
    assert_eq!(px(&lo, a, PropId::RowGap), 6);
    assert_eq!(px(&hi, b, PropId::RowGap), 12);
    // Resolved values are pixels, never `Dp`.
    assert_eq!(
        hi.engine().style_prop(b, Part::Main, PropId::PaddingLeft),
        StyleValue::Length(Length::Px(20))
    );
    // Layout uses the converted sizes; `Px` stays as is.
    assert_eq!(lo.engine().coords(a).width(), 50);
    assert_eq!(hi.engine().coords(b).width(), 100);
    assert_eq!(lo.engine().coords(a).height(), 30);
    assert_eq!(hi.engine().coords(b).height(), 30);
    // The engine's DPI lookup (one display: a direct read).
    assert_eq!(lo.engine().node_dpi(a), 160);
    assert_eq!(hi.engine().node_dpi(b), 320);
}

#[test]
fn dp_rounds_like_lvgl_dpx() {
    // 130 DPI (the default): LVGL `LV_DPX_CALC` rounding, at least 1 px for a positive value.
    let (h, n) = node_at(
        130,
        &[
            StyleProp::PaddingTop(Length::dp(12)),
            StyleProp::PaddingBottom(Length::dp(1)),
        ],
    );
    assert_eq!(h.engine().style_i32(n, Part::Main, PropId::PaddingTop), 10);
    assert_eq!(h.engine().style_i32(n, Part::Main, PropId::PaddingBottom), 1);
}

#[test]
fn px_lengths_do_not_depend_on_dpi() {
    let props = [
        StyleProp::PaddingLeft(Length::Px(10)),
        StyleProp::Width(Length::Px(50)),
    ];
    let (lo, a) = node_at(160, &props);
    let (hi, b) = node_at(320, &props);
    assert_eq!(lo.engine().style_i32(a, Part::Main, PropId::PaddingLeft), 10);
    assert_eq!(hi.engine().style_i32(b, Part::Main, PropId::PaddingLeft), 10);
    assert_eq!(lo.engine().coords(a).width(), hi.engine().coords(b).width());
}

#[test]
fn radius_circle_draws_like_the_circle_marker() {
    let fill = |r: Radius| {
        node_at(
            160,
            &[
                StyleProp::Width(Length::Px(40)),
                StyleProp::Height(Length::Px(40)),
                StyleProp::BgColor(Color::RED),
                StyleProp::BgOpacity(Opa::COVER),
                StyleProp::Radius(r),
            ],
        )
    };
    let (circle, n) = fill(Radius::Circle);
    // The draw descriptor carries the renderer's marker, exactly as before `Radius` existed.
    let rs = circle.engine().rect_dsc(n, Part::Main, Opa::COVER);
    assert_eq!(rs.dsc().radius, RADIUS_CIRCLE);
    assert_eq!(
        circle.engine().style_i32(n, Part::Main, PropId::Radius),
        RADIUS_CIRCLE
    );
    assert_eq!(
        circle
            .engine()
            .style_prop(n, Part::Main, PropId::Radius)
            .get::<Radius>(),
        Some(Radius::Circle)
    );
    // Same pixels as the largest radius the renderer can draw on a 40 × 40 box.
    let (half, _) = fill(Radius::Px(20));
    assert_eq!(circle.panel_rgb888(), half.panel_rgb888());
    // And not the same as a square box (the circle really is round).
    let (square, _) = fill(Radius::Px(0));
    assert_ne!(circle.panel_rgb888(), square.panel_rgb888());
}
