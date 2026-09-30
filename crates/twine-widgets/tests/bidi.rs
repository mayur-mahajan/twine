//! Right-to-left text in widgets (features `bidi` and `arabic-shaping`): the inherited
//! `BaseDir` style reaches the text descriptors, labels align and reorder RTL text, flex rows
//! mirror, textarea cursors move logically, and Arabic labels are shaped.

mod common;

use common::{Mode, get, harness, with};
use twine_assets::fonts::DEJAVU_16_PERSIAN_HEBREW;
use twine_engine::{MeasureCx, NodeId};
use twine_style::{Align, BaseDir, FlexFlow, LayoutKind, Length, Part, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_text::TextDir;
use twine_widgets::label::{self, Label};
use twine_widgets::textarea::{self, Textarea};

fn rtl(h: &mut EngineHarness, id: NodeId) {
    let e = h.engine_mut();
    e.set_local_prop(id, Selector::MAIN, StyleProp::BaseDir(BaseDir::Rtl));
    e.set_local_prop(id, Selector::MAIN, StyleProp::Font(&DEJAVU_16_PERSIAN_HEBREW));
}

#[test]
fn base_dir_style_is_inherited_into_text_dsc() {
    let mut h = harness(160, 60, Mode::Light);
    let screen = h.screen();
    rtl(&mut h, screen);
    let l = label::create_with(h.engine_mut(), screen, "abc").unwrap();
    let d = MeasureCx::new(h.engine(), l).text_dsc(Part::Main);
    assert_eq!(d.base_dir, TextDir::Rtl);
    h.engine_mut()
        .set_local_prop(l, Selector::MAIN, StyleProp::BaseDir(BaseDir::Auto));
    let d = MeasureCx::new(h.engine(), l).text_dsc(Part::Main);
    assert_eq!(d.base_dir, TextDir::Auto);
}

#[test]
fn rtl_label_widget_snapshot() {
    for m in Mode::ALL {
        let mut h = harness(200, 80, m);
        let screen = h.screen();
        let l = label::create_with(h.engine_mut(), screen, "שלום 123 (abc)").unwrap();
        rtl(&mut h, l);
        let e = h.engine_mut();
        e.set_width(l, Length::Px(180));
        e.align(l, Align::Center, 0, 0);
        h.run_until_idle();
        h.assert_snapshot(&format!("label_rtl_{}", m.suffix()));
    }
}

#[test]
fn rtl_label_auto_align_is_right() {
    let mut h = harness(200, 60, Mode::Light);
    let screen = h.screen();
    let l = label::create_with(h.engine_mut(), screen, "שלום").unwrap();
    rtl(&mut h, l);
    h.engine_mut().set_width(l, Length::Px(180));
    h.run_until_idle();
    // The cursor position at the start of right-aligned RTL text is at the right edge.
    let w = get::<Label>(&h, l);
    let p = w.letter_pos(&MeasureCx::new(h.engine(), l), 0);
    assert!(p.x > 150, "{p:?}");
}

#[test]
fn rtl_flex_row_mirrored_snapshot() {
    for m in Mode::ALL {
        let mut h = harness(220, 100, m);
        let screen = h.screen();
        let e = h.engine_mut();
        let row = twine_widgets::container::create(e, screen).unwrap();
        e.set_size(row, 200, 80);
        e.align(row, Align::Center, 0, 0);
        e.set_local_prop(row, Selector::MAIN, StyleProp::Layout(LayoutKind::Flex));
        e.set_local_prop(row, Selector::MAIN, StyleProp::FlexFlow(FlexFlow::ROW));
        e.set_local_prop(row, Selector::MAIN, StyleProp::BaseDir(BaseDir::Rtl));
        let mut first = None;
        for t in ["1", "2", "3"] {
            let b = twine_widgets::button::create(e, row).unwrap();
            label::create_with(e, b, t).unwrap();
            first.get_or_insert(b);
        }
        h.run_until_idle();
        // The first child is at the right end.
        let c = h.engine().coords(first.unwrap());
        let r = h.engine().content_area(row);
        assert_eq!(c.x1, r.x1);
        h.assert_snapshot(&format!("flex_row_rtl_{}", m.suffix()));
    }
}

#[test]
fn textarea_cursor_moves_logically_in_rtl() {
    let mut h = harness(240, 100, Mode::Light);
    let screen = h.screen();
    let ta = textarea::create(h.engine_mut(), screen).unwrap();
    rtl(&mut h, ta);
    h.engine_mut().set_size(ta, 200, 60);
    with(&mut h, ta, |w: &mut Textarea, cx| {
        w.set_text(cx, "שלום");
        w.set_cursor_pos(cx, 0);
    });
    h.run_until_idle();
    let x0 = get::<Textarea>(&h, ta)
        .cursor_area(&MeasureCx::new(h.engine(), ta))
        .x0;
    with(&mut h, ta, |w: &mut Textarea, cx| w.cursor_right(cx));
    h.run_until_idle();
    let t = get::<Textarea>(&h, ta);
    assert_eq!(t.cursor_pos(), 1, "Right moves to the next logical character");
    let x1 = t.cursor_area(&MeasureCx::new(h.engine(), ta)).x0;
    assert!(x1 < x0, "…which is to the left in RTL text ({x1} vs {x0})");
}

#[test]
fn arabic_label_is_shaped_textarea_is_not() {
    let mut h = harness(200, 100, Mode::Light);
    let screen = h.screen();
    let l = label::create_with(h.engine_mut(), screen, "سلام").unwrap();
    let w = get::<Label>(&h, l);
    assert_eq!(w.text(), "سلام", "the logical text is kept");
    assert_eq!(
        w.shown_text(),
        "\u{FEB3}\u{FEFC}\u{FEE1}",
        "drawn with contextual forms"
    );
    with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "abc"));
    assert_eq!(get::<Label>(&h, l).shown_text(), "abc");
    let ta = textarea::create(h.engine_mut(), screen).unwrap();
    with(&mut h, ta, |w: &mut Textarea, cx| w.set_text(cx, "سلام"));
    let inner = h.engine().tree().child(ta, 0).unwrap();
    assert_eq!(get::<Label>(&h, inner).shown_text(), "سلام");
}
