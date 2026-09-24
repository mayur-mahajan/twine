//! `Label`: defaults, text storage, content size, wrapping, hit testing and drawing.

mod common;

use common::{Mode, get, harness, with};
use proptest::prelude::*;
use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, Point};
use twine_engine::ObjFlags;
use twine_style::{Align, Part, PropId, Selector, StyleProp, TextAlign, TextDecor};
use twine_testing::EngineHarness;
use twine_text::{LongMode, TextLayout};
use twine_widgets::label::{self, LABEL_CLASS, Label, LabelText};

fn label_on(h: &mut EngineHarness, text: &'static str) -> twine_engine::NodeId {
    let screen = h.screen();
    label::create_with(h.engine_mut(), screen, text).unwrap()
}

#[test]
fn label_defaults_match_lvgl() {
    let mut h = harness(100, 40, Mode::Light);
    let screen = h.screen();
    let l = label::create(h.engine_mut(), screen).unwrap();
    let n = h.engine().tree().node(l).unwrap();
    assert_eq!(n.class().name, "label");
    assert!(
        !n.flags().contains(ObjFlags::CLICKABLE),
        "labels are not clickable (LVGL)"
    );
    assert!(n.flags().contains(ObjFlags::SCROLLABLE));
    assert_eq!(LABEL_CLASS.parts, &[Part::Main, Part::Scrollbar, Part::Selected]);
    let w = get::<Label>(&h, l);
    assert_eq!(w.text(), "Text"); // LV_LABEL_DEFAULT_TEXT
    assert_eq!(w.long_mode(), LongMode::Wrap);
    assert_eq!(w.max_lines(), 0);
    assert_eq!(w.selection(), None);
    assert!(matches!(w.text_storage(), LabelText::Static(_)));
    assert_eq!(n.widget().text(), Some("Text"));
    // The theme gives labels no styles: the text inherits the screen's color and font.
    assert_eq!(
        h.engine().style_color(l, Part::Main, PropId::TextColor),
        Color::hex(0x0021_2121)
    );
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn label_set_same_text_no_invalidate() {
    let mut h = harness(100, 40, Mode::Light);
    let l = label_on(&mut h, "Hello");
    h.run_until_idle();
    with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "Hello"));
    with(&mut h, l, |w: &mut Label, cx| w.set_long_mode(cx, LongMode::Wrap));
    with(&mut h, l, |w: &mut Label, cx| w.set_max_lines(cx, 0));
    with(&mut h, l, |w: &mut Label, cx| w.clear_selection(cx));
    with(&mut h, l, |w: &mut Label, cx| w.set_text_static(cx, "Hello"));
    assert!(h.engine().invalidation_log().is_empty());
    assert!(!h.engine().layout_pending());
    h.assert_idle();
}

#[test]
fn label_set_text_invalidates_own_area_and_relayouts() {
    let mut h = harness(100, 40, Mode::Light);
    let l = label_on(&mut h, "Hi");
    h.run_until_idle();
    let before = h.engine().coords(l);
    with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "Hello world"));
    assert!(h.engine().layout_pending());
    h.update();
    let after = h.engine().coords(l);
    assert!(after.width() > before.width());
    let ext = i32::from(h.engine().tree().node(l).unwrap().ext_draw());
    let bound = before.union(&after).expand(ext);
    for (r, _) in h.invalidations() {
        assert!(bound.contains_rect(r), "{r} outside {bound}");
    }
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn label_content_size_matches_measure() {
    let mut h = harness(200, 60, Mode::Light);
    let l = label_on(&mut h, "Hello, twine!");
    h.run_until_idle();
    let m = TextLayout::new("Hello, twine!", &MONTSERRAT_14).measure();
    let c = h.engine().coords(l);
    assert_eq!((c.width(), c.height()), (m.w, m.h));
    // Letter and line spacing are part of the measurement.
    h.engine_mut()
        .set_local_prop(l, Selector::MAIN, StyleProp::TextLetterSpace(2));
    h.run_until_idle();
    let mut lay = TextLayout::new("Hello, twine!", &MONTSERRAT_14);
    lay.letter_space = 2;
    assert_eq!(h.engine().coords(l).width(), lay.measure().w);
}

#[test]
fn label_wrap_at_fixed_width() {
    let mut h = harness(200, 100, Mode::Light);
    let l = label_on(&mut h, "The quick brown fox jumps over the lazy dog");
    h.engine_mut().set_width(l, 80);
    h.run_until_idle();
    let mut lay = TextLayout::new("The quick brown fox jumps over the lazy dog", &MONTSERRAT_14);
    lay.max_width = 80;
    let n = lay.line_count() as i32;
    assert!(n > 2);
    let c = h.engine().coords(l);
    assert_eq!(c.width(), 80);
    assert_eq!(c.height(), n * i32::from(MONTSERRAT_14.line_height));
    // max_lines limits the content height.
    with(&mut h, l, |w: &mut Label, cx| w.set_max_lines(cx, 2));
    h.run_until_idle();
    assert_eq!(
        h.engine().coords(l).height(),
        2 * i32::from(MONTSERRAT_14.line_height)
    );
}

#[test]
fn label_clip_mode_is_single_line() {
    let mut h = harness(200, 60, Mode::Light);
    let l = label_on(&mut h, "one two three four five");
    h.engine_mut().set_width(l, 60);
    with(&mut h, l, |w: &mut Label, cx| w.set_long_mode(cx, LongMode::Clip));
    h.run_until_idle();
    assert_eq!(
        h.engine().coords(l).height(),
        i32::from(MONTSERRAT_14.line_height)
    );
}

#[test]
fn label_selection_is_clamped_and_idempotent() {
    let mut h = harness(100, 40, Mode::Light);
    let l = label_on(&mut h, "héllo");
    h.run_until_idle();
    // Byte 2 is inside 'é' (bytes 1..3): rounded down to 1.
    with(&mut h, l, |w: &mut Label, cx| w.set_selection(cx, 50, 2));
    assert_eq!(get::<Label>(&h, l).selection(), Some((1, 6)));
    h.run_until_idle();
    with(&mut h, l, |w: &mut Label, cx| w.set_selection(cx, 1, 6));
    assert!(h.engine().invalidation_log().is_empty());
    with(&mut h, l, |w: &mut Label, cx| w.set_selection(cx, 3, 3));
    assert_eq!(get::<Label>(&h, l).selection(), None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn letter_pos_letter_on_roundtrip(text in "[a-zA-Z0-9.,]{1,24}( [a-zA-Z0-9.,]{1,12}){0,4}") {
        let text: &'static str = Box::leak(text.into_boxed_str());
        let mut h = harness(200, 120, Mode::Light);
        let l = label_on(&mut h, text);
        h.engine_mut().set_width(l, 90);
        h.run_until_idle();
        let e = h.engine();
        let cx = twine_engine::MeasureCx::new(e, l);
        let w = get::<Label>(&h, l);
        for (i, c) in text.char_indices() {
            if c == ' ' {
                continue; // a trailing space of a wrapped line has no own position
            }
            let p = w.letter_pos(&cx, i);
            prop_assert_eq!(w.letter_on(&cx, p), i, "index {} at {:?}", i, p);
        }
    }
}

#[test]
fn letter_pos_of_second_line() {
    let mut h = harness(200, 80, Mode::Light);
    let l = label_on(&mut h, "ab\ncd");
    h.run_until_idle();
    let cx = twine_engine::MeasureCx::new(h.engine(), l);
    let w = get::<Label>(&h, l);
    assert_eq!(w.letter_pos(&cx, 0), Point::ZERO);
    let p = w.letter_pos(&cx, 3);
    assert_eq!(p, Point::new(0, i32::from(MONTSERRAT_14.line_height)));
    assert_eq!(w.letter_on(&cx, Point::new(1, p.y + 2)), 3);
}

fn snapshot_scene(mode: Mode, f: impl FnOnce(&mut EngineHarness, twine_engine::NodeId)) -> EngineHarness {
    let mut h = harness(160, 80, mode);
    let l = label_on(&mut h, "Twine label");
    h.engine_mut().align(l, Align::Center, 0, 0);
    f(&mut h, l);
    h
}

#[test]
fn snapshot_label_basic() {
    for m in Mode::ALL {
        let mut h = snapshot_scene(m, |_, _| {});
        h.assert_snapshot(&format!("label_basic_{}", m.suffix()));
    }
}

#[test]
fn snapshot_label_center_aligned() {
    for m in Mode::ALL {
        let mut h = snapshot_scene(m, |h, l| {
            with(h, l, |w: &mut Label, cx| {
                w.set_text(cx, "Centered\nmultiline text\nin a label");
            });
            let e = h.engine_mut();
            e.set_width(l, 140);
            e.set_local_prop(l, Selector::MAIN, StyleProp::TextAlign(TextAlign::Center));
        });
        h.assert_snapshot(&format!("label_center_aligned_{}", m.suffix()));
    }
}

#[test]
fn snapshot_label_selection() {
    for m in Mode::ALL {
        let mut h = snapshot_scene(m, |h, l| {
            let sel = Selector::part(Part::Selected);
            let e = h.engine_mut();
            e.set_local_prop(l, sel, StyleProp::BgColor(twine_theme::Palette::Blue.main()));
            e.set_local_prop(l, sel, StyleProp::TextColor(Color::WHITE));
            with(h, l, |w: &mut Label, cx| w.set_selection(cx, 6, 11));
        });
        h.assert_snapshot(&format!("label_selection_{}", m.suffix()));
    }
}

#[test]
fn snapshot_label_multiline_underline() {
    for m in Mode::ALL {
        let mut h = snapshot_scene(m, |h, l| {
            with(h, l, |w: &mut Label, cx| {
                w.set_text(cx, "Underlined text that wraps onto several lines");
            });
            let e = h.engine_mut();
            e.set_width(l, 120);
            e.set_local_prop(l, Selector::MAIN, StyleProp::TextDecor(TextDecor::UNDERLINE));
            e.set_local_prop(l, Selector::MAIN, StyleProp::TextLineSpace(4));
        });
        h.assert_snapshot(&format!("label_multiline_underline_{}", m.suffix()));
    }
}
