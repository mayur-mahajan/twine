//! The basic controls under LVGL's simple and mono themes (the default theme is covered by
//! each widget's own snapshots), plus 1-bit output for the mono theme.

mod common;

use std::rc::Rc;

use common::with;
use twine_core::{ColorFormat, Point};
use twine_engine::{Engine, NodeId, State};
use twine_style::{FlexFlow, LayoutKind, Length};
use twine_testing::EngineHarness;
use twine_theme::{MonoTheme, SimpleTheme};
use twine_widgets::prelude::*;

static WAVE: [Point; 4] = [
    Point::new(0, 10),
    Point::new(20, 0),
    Point::new(40, 10),
    Point::new(60, 0),
];

/// Every basic control except the spinner (it never idles) in a wrapping row.
fn controls(h: &mut EngineHarness) {
    let screen = h.screen();
    let e: &mut Engine = h.engine_mut();
    let row = container::create(e, screen).unwrap();
    e.set_size(row, Length::Pct(100), Length::Pct(100));
    e.set_layout(row, LayoutKind::Flex);
    e.set_flex_flow(row, FlexFlow::RowWrap);
    let b = bar::create(e, row).unwrap();
    e.set_size(b, 90, 10);
    let s = slider::create(e, row).unwrap();
    e.set_size(s, 90, 8);
    let sw = switch::create(e, row).unwrap();
    let sw_on = switch::create(e, row).unwrap();
    let cb = checkbox::create_with(e, row, "Box").unwrap();
    let cb_on = checkbox::create_with(e, row, "On").unwrap();
    let a = arc::create(e, row).unwrap();
    e.set_size(a, 60, 60);
    let l = led::create(e, row).unwrap();
    let ln = line::create(e, row).unwrap();
    with(h, b, |w: &mut Bar, cx| w.set_value(cx, 60, false));
    with(h, s, |w: &mut Slider, cx| w.set_value(cx, 30, false));
    with(h, sw_on, |w: &mut Switch, cx| w.set_checked(cx, true, false));
    with(h, cb_on, |w: &mut Checkbox, cx| w.set_checked(cx, true));
    with(h, a, |w: &mut Arc, cx| w.set_value(cx, 40));
    with(h, ln, |w: &mut Line, cx| w.set_points_static(cx, &WAVE));
    let _: [NodeId; 3] = [sw, cb, l];
    h.engine_mut().add_state(s, State::FOCUSED | State::FOCUS_KEY);
}

#[test]
fn snapshot_simple_theme() {
    let mut h = EngineHarness::new(240, 200).theme(Rc::new(SimpleTheme::new()));
    controls(&mut h);
    h.run_until_idle();
    h.assert_snapshot("controls_simple");
}

#[test]
fn snapshot_mono_theme() {
    for dark in [false, true] {
        let mut h = EngineHarness::new(240, 200)
            .format(ColorFormat::I1)
            .theme(Rc::new(MonoTheme::new(dark, &twine_assets::fonts::MONTSERRAT_14)));
        controls(&mut h);
        h.run_until_idle();
        assert!(h.panel_rgb888().iter().all(|&v| v == 0 || v == 255));
        h.assert_snapshot(if dark {
            "controls_mono_dark"
        } else {
            "controls_mono_light"
        });
    }
}

static KEYS: [&str; 7] = ["1", "2", "3", "\n", "4", "5", "6"];

/// The text input widgets: a button matrix, a textarea, a spinbox, a span group and a
/// keyboard (nothing focused, so nothing blinks).
fn text_inputs(h: &mut EngineHarness) {
    use twine_widgets::buttonmatrix::{self, BtnCtrl, ButtonMatrix, MapSrc};
    use twine_widgets::spangroup::{self, SpanGroup};
    use twine_widgets::textarea::{self, Textarea};
    use twine_widgets::{keyboard, spinbox};
    let screen = h.screen();
    let e: &mut Engine = h.engine_mut();
    let m = buttonmatrix::create_with(e, screen, MapSrc::Static(&KEYS)).unwrap();
    e.set_size(m, 110, 60);
    e.align(m, twine_style::Align::TopLeft, 4, 4);
    let ta = textarea::create(e, screen).unwrap();
    e.set_size(ta, 110, 60);
    e.align(ta, twine_style::Align::TopRight, -4, 4);
    let sb = spinbox::create(e, screen).unwrap();
    e.align(sb, twine_style::Align::TopLeft, 4, 70);
    let sg = spangroup::create(e, screen).unwrap();
    e.align(sg, twine_style::Align::TopRight, -4, 76);
    let kb = keyboard::create(e, screen).unwrap();
    e.set_height(kb, 100);
    with(h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 1, BtnCtrl::CHECKED);
        w.set_btn_ctrl(cx, 4, BtnCtrl::DISABLED);
    });
    with(h, ta, |w: &mut Textarea, cx| w.set_text(cx, "Some text"));
    with(h, sg, |w: &mut SpanGroup, cx| {
        let a = w.add_span(cx);
        w.set_span_text(cx, a, "rich ");
        let b = w.add_span(cx);
        w.set_span_text(cx, b, "text");
        w.set_span_style(
            cx,
            b,
            twine_style::StyleProp::TextDecor(twine_style::TextDecor::UNDERLINE),
        );
    });
}

#[test]
fn snapshot_text_inputs_simple_theme() {
    let mut h = EngineHarness::new(240, 240).theme(Rc::new(SimpleTheme::new()));
    text_inputs(&mut h);
    h.run_until_idle();
    h.assert_snapshot("text_inputs_simple");
}

#[test]
fn snapshot_text_inputs_mono_theme() {
    for dark in [false, true] {
        let mut h = EngineHarness::new(240, 240)
            .format(ColorFormat::I1)
            .theme(Rc::new(MonoTheme::new(dark, &twine_assets::fonts::MONTSERRAT_14)));
        text_inputs(&mut h);
        h.run_until_idle();
        assert!(h.panel_rgb888().iter().all(|&v| v == 0 || v == 255));
        h.assert_snapshot(if dark {
            "text_inputs_mono_dark"
        } else {
            "text_inputs_mono_light"
        });
    }
}
