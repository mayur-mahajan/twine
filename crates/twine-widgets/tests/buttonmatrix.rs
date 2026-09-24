//! `ButtonMatrix`: LVGL layout (relative widths, rows), per-button invalidation, hit testing,
//! checkable / one-checked buttons, long-press repeat, keypad and encoder navigation,
//! popovers and the default theme's look.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Mode, get, harness, with};
use twine_core::{Color, Duration, Point, Rect};
use twine_engine::{
    EventCode, EventFilter, EventParam, EventResult, GroupDef, Key, MeasureCx, NodeId, ObjFlags, State,
};
use twine_style::{Align, Part, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::buttonmatrix::{self, BUTTONMATRIX_CLASS, BtnCtrl, ButtonMatrix, MapSrc};

static MAP_3X4: [&str; 15] = [
    "1", "2", "3", "\n", "4", "5", "6", "\n", "7", "8", "9", "\n", "*", "0", "#",
];

/// A 200 × 120 matrix of `map` centered on a 240 × 160 screen, in the default group.
fn scene(mode: Mode, map: &'static [&'static str]) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 160, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let e = h.engine_mut();
    let m = buttonmatrix::create_with(e, screen, MapSrc::Static(map)).unwrap();
    e.set_size(m, 200, 120);
    e.align(m, Align::Center, 0, 0);
    h.run_until_idle();
    (h, m)
}

/// Records the value of every `ValueChanged`.
fn values(h: &mut EngineHarness, m: NodeId) -> Rc<RefCell<Vec<i32>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    h.engine_mut()
        .add_event_handler(m, EventFilter::Code(EventCode::ValueChanged), move |_, ev| {
            let EventParam::Value(v) = ev.param else {
                panic!("no value: {:?}", ev.param);
            };
            l.borrow_mut().push(v);
            EventResult::Continue
        });
    log
}

fn area(h: &EngineHarness, m: NodeId, i: u16) -> Rect {
    get::<ButtonMatrix>(h, m)
        .btn_area(&MeasureCx::new(h.engine(), m), i)
        .unwrap()
}

fn center(r: Rect) -> Point {
    Point::new((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2)
}

#[test]
fn btnm_defaults() {
    let mut h = harness(300, 200, Mode::Light);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    let screen = h.screen();
    let m = buttonmatrix::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let n = h.engine().tree().node(m).unwrap();
    assert_eq!(n.class().name, "buttonmatrix");
    assert_eq!(BUTTONMATRIX_CLASS.parts, &[Part::Main, Part::Items]);
    assert_eq!(BUTTONMATRIX_CLASS.group_def, GroupDef::True);
    assert_eq!(h.engine().group_of(m), Some(g));
    let c = h.engine().coords(m);
    assert_eq!((c.width(), c.height()), (260, 130), "LVGL 2 x 1 DPI_DEF");
    let w = get::<ButtonMatrix>(&h, m);
    assert_eq!(w.btn_count(), 5);
    assert_eq!(w.row_count(), 2);
    assert_eq!(w.btn_text(0), Some("Btn1"));
    assert_eq!(w.btn_text(4), Some("Btn5"));
    assert_eq!(w.selected_btn(), None);
    assert!(!w.one_checked());
    h.assert_idle();
}

#[test]
fn btnm_setters_same_value_no_invalidate() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_map(cx, MapSrc::Static(&MAP_3X4));
        w.set_ctrl_map(cx, &[BtnCtrl::empty(); 12]);
        w.set_one_checked(cx, false);
        w.set_selected_btn(cx, None);
        w.clear_btn_ctrl(cx, 3, BtnCtrl::CHECKED);
        w.set_btn_width(cx, 3, 1);
    });
    assert!(h.engine().invalidation_log().is_empty());
    // Adding a bit twice: the second call does nothing.
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 1, BtnCtrl::CHECKED);
    });
    h.run_until_idle();
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 1, BtnCtrl::CHECKED);
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn btnm_layout_widths_units() {
    static MAP: [&str; 3] = ["a", "b", "c"];
    let (mut h, m) = scene(Mode::Light, &MAP);
    // Units 1 : 2 : 3 of the content width minus two column gaps.
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_width(cx, 1, 2);
        w.set_btn_width(cx, 2, 3);
    });
    h.run_until_idle();
    let e = h.engine();
    let content = e.content_area(m);
    let pcol = e.style_i32(m, Part::Main, twine_style::PropId::PadColumn);
    let no_gap = content.width() - 2 * pcol;
    let (a, b, c) = (area(&h, m, 0), area(&h, m, 1), area(&h, m, 2));
    // LVGL: x1 = no_gap * units_before / 6 + i * pcol, x2 = no_gap * units_after / 6 + i * pcol - 1.
    assert_eq!(a.x0 - content.x0, 0);
    assert_eq!(a.x1 - content.x0, no_gap / 6);
    assert_eq!(b.x0 - content.x0, no_gap / 6 + pcol);
    assert_eq!(b.x1 - content.x0, no_gap * 3 / 6 + pcol);
    assert_eq!(c.x0 - content.x0, no_gap * 3 / 6 + 2 * pcol);
    assert_eq!(c.x1, content.x1, "the last button reaches the content edge");
    assert!(b.width() > a.width() && c.width() > b.width());
    // One row: the full content height.
    assert_eq!((a.y0, a.y1), (content.y0, content.y1));
}

#[test]
fn btnm_row_split_on_newline() {
    static WITH_END: [&str; 4] = ["x", "\n", "y", ""];
    let (h, m) = scene(Mode::Light, &MAP_3X4);
    let w = get::<ButtonMatrix>(&h, m);
    assert_eq!(w.btn_count(), 12);
    assert_eq!(w.row_count(), 4);
    let e = h.engine();
    let content = e.content_area(m);
    let prow = e.style_i32(m, Part::Main, twine_style::PropId::PadRow);
    let rows: Vec<Rect> = [0, 3, 6, 9].iter().map(|&i| area(&h, m, i)).collect();
    for (r, a) in rows.iter().enumerate() {
        let r = r as i32;
        let no_gap = content.height() - 3 * prow;
        assert_eq!(a.y0 - content.y0, no_gap * r / 4 + r * prow);
        assert_eq!(a.y1 - content.y0, no_gap * (r + 1) / 4 + r * prow);
    }
    // Buttons of a row share the row.
    assert_eq!(area(&h, m, 4).y0, rows[1].y0);
    assert_eq!(w.btn_text(9), Some("*"));
    // A trailing "" (LVGL's terminator) is accepted.
    let m2 = ButtonMatrix::new(MapSrc::Static(&WITH_END));
    assert_eq!((m2.btn_count(), m2.row_count()), (2, 2));
    // Owned maps work the same.
    let owned = ButtonMatrix::new(MapSrc::Owned(vec!["p".into(), "\n".into(), "q".into()]));
    assert_eq!(owned.btn_text(1), Some("q"));
}

#[test]
fn btnm_themed_press_invalidates_only_button() {
    // The default theme styles `Items | PRESSED` (and `CHECKED`, `FOCUS_KEY`…): the node's
    // state changes must still redraw only the pressed button (`Items` is an item part).
    for mode in Mode::ALL {
        let (mut h, m) = scene(mode, &MAP_3X4);
        let b = area(&h, m, 4);
        let expected = get::<ButtonMatrix>(&h, m)
            .btn_invalidation_area(&MeasureCx::new(h.engine(), m), 4)
            .unwrap();
        let check = |h: &EngineHarness, what: &str| {
            let inv: Vec<Rect> = h
                .invalidations()
                .iter()
                .chain(h.engine().invalidation_log())
                .map(|(r, _)| *r)
                .collect();
            assert!(!inv.is_empty(), "{what}: nothing redrawn");
            for r in &inv {
                assert!(expected.contains_rect(r), "{what}: {r:?} outside {expected:?}");
            }
        };
        h.press(center(b));
        assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(4));
        check(&h, "press");
        assert_eq!(h.engine().transition_count(), 0, "items get no transitions");
        h.advance(Duration::ms(100));
        h.release();
        check(&h, "release");
        h.run_until_idle();
        // Another press of the same button: still only that button.
        h.press(center(b));
        check(&h, "second press");
        h.release();
        h.run_until_idle();
        h.assert_idle();
    }
}

#[test]
fn btnm_press_invalidates_only_button() {
    // Local `Items` styles without a theme.
    let mut h = EngineHarness::new(240, 160).no_theme();
    let screen = h.screen();
    let e = h.engine_mut();
    let m = buttonmatrix::create_with(e, screen, MapSrc::Static(&MAP_3X4)).unwrap();
    e.set_size(m, 200, 120);
    e.align(m, Align::Center, 0, 0);
    e.set_local_prop(m, Selector::MAIN, StyleProp::PadRow(4));
    e.set_local_prop(m, Selector::MAIN, StyleProp::PadColumn(4));
    e.set_local_prop(m, Selector::part(Part::Items), StyleProp::BgColor(Color::BLUE));
    e.set_local_prop(
        m,
        Selector::part(Part::Items),
        StyleProp::BgOpa(twine_core::Opa::COVER),
    );
    h.run_until_idle();
    let b = area(&h, m, 4);
    h.press(center(b));
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(4));
    let expected = get::<ButtonMatrix>(&h, m)
        .btn_invalidation_area(&MeasureCx::new(h.engine(), m), 4)
        .unwrap();
    let inv: Vec<Rect> = h.engine().invalidation_log().iter().map(|(r, _)| *r).collect();
    assert!(!inv.is_empty());
    for r in &inv {
        assert!(expected.contains_rect(r), "{r:?} outside {expected:?}");
    }
    assert!(expected.contains_rect(&b.expand(4)), "the button plus the gaps");
    h.advance(Duration::ms(100));
    h.release();
    // Rendered in this update (a frame started) or still pending.
    let inv: Vec<Rect> = h
        .invalidations()
        .iter()
        .chain(h.engine().invalidation_log())
        .map(|(r, _)| *r)
        .collect();
    assert!(!inv.is_empty());
    for r in &inv {
        assert!(expected.contains_rect(r), "{r:?} outside {expected:?}");
    }
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn btnm_press_sends_value_changed() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    let log = values(&mut h, m);
    h.tap(center(area(&h, m, 7)));
    assert_eq!(*log.borrow(), vec![7], "on press (LVGL master)");
    // Pressing in the gap between two buttons still hits one (the gap is shared).
    let (a, b) = (area(&h, m, 0), area(&h, m, 1));
    h.tap(Point::new((a.x1 + b.x0) / 2, center(a).y));
    assert_eq!(log.borrow().len(), 2);
    // Sliding off the pressed button: nothing is selected any more.
    h.press(center(area(&h, m, 2)));
    h.move_to(center(area(&h, m, 5)));
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), None);
    h.release();
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn btnm_click_trig_on_release() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 0, BtnCtrl::CLICK_TRIG);
    });
    let log = values(&mut h, m);
    h.press(center(area(&h, m, 0)));
    assert!(log.borrow().is_empty());
    h.release();
    assert_eq!(*log.borrow(), vec![0]);
}

#[test]
fn btnm_hidden_button_not_hit() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 4, BtnCtrl::HIDDEN);
        w.set_btn_ctrl(cx, 5, BtnCtrl::DISABLED);
    });
    let log = values(&mut h, m);
    h.tap(center(area(&h, m, 4)));
    h.tap(center(area(&h, m, 5)));
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), None);
    // The hidden button still takes its place: its neighbours keep their size.
    assert_eq!(area(&h, m, 3).width(), area(&h, m, 0).width());
}

#[test]
fn btnm_one_checked_exclusive() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl_all(cx, BtnCtrl::CHECKABLE);
        w.set_btn_ctrl(cx, 1, BtnCtrl::CHECKED);
        w.set_btn_ctrl(cx, 2, BtnCtrl::CHECKED);
        w.set_one_checked(cx, true);
    });
    let checked = |h: &EngineHarness| -> Vec<u16> {
        let w = get::<ButtonMatrix>(h, m);
        (0..w.btn_count())
            .filter(|&i| w.has_btn_ctrl(i, BtnCtrl::CHECKED))
            .collect()
    };
    assert_eq!(checked(&h), vec![1], "enabling keeps the first checked one");
    h.tap(center(area(&h, m, 6)));
    assert_eq!(checked(&h), vec![6]);
    h.tap(center(area(&h, m, 6)));
    assert_eq!(
        checked(&h),
        vec![6],
        "one-checked: clicking the checked one keeps it"
    );
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 0, BtnCtrl::CHECKED);
    });
    assert_eq!(checked(&h), vec![0]);
    // Without one-checked a click toggles.
    with(&mut h, m, |w: &mut ButtonMatrix, cx| w.set_one_checked(cx, false));
    h.tap(center(area(&h, m, 0)));
    assert!(checked(&h).is_empty());
}

#[test]
fn btnm_long_press_repeat_values() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    let log = values(&mut h, m);
    h.press(center(area(&h, m, 3)));
    // Long press after 400 ms, then a repeat every 100 ms.
    h.advance(Duration::ms(720));
    h.release();
    let log = log.borrow();
    assert!(log.len() >= 3, "{log:?}");
    assert!(log.iter().all(|&v| v == 3));
}

#[test]
fn btnm_no_repeat() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 3, BtnCtrl::NO_REPEAT);
    });
    let log = values(&mut h, m);
    h.press(center(area(&h, m, 3)));
    h.advance(Duration::ms(720));
    h.release();
    assert_eq!(*log.borrow(), vec![3], "only the press");
}

#[test]
fn btnm_keypad_skips_disabled() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 0, BtnCtrl::DISABLED);
        w.set_btn_ctrl(cx, 1, BtnCtrl::HIDDEN);
    });
    let log = values(&mut h, m);
    let _ = h.keypad_input();
    // The matrix got the focus when it joined the group (no input device yet): nothing is
    // selected; the first arrow selects the first active button (LVGL).
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), None);
    h.key(Key::Right);
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(2));
    h.key(Key::Right);
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(3));
    h.key(Key::Left);
    h.key(Key::Left);
    // 2 → (1 hidden, 0 disabled) → wraps to the last one.
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(11));
    h.key(Key::Up);
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(8));
    h.key(Key::Down);
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(11));
    h.key(Key::Enter);
    assert_eq!(*log.borrow(), vec![11]);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn btnm_encoder_edit_navigation() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    let screen = h.screen();
    let _other = buttonmatrix::create(h.engine_mut(), screen).unwrap();
    let g = h.engine().group_of(m).unwrap();
    let log = values(&mut h, m);
    let _ = h.encoder_input();
    h.engine_mut().focus(m);
    h.run_until_idle();
    // Navigate mode: a click enters edit mode and selects the first button.
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(0));
    h.encoder(2);
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(2));
    h.encoder(-1);
    assert_eq!(get::<ButtonMatrix>(&h, m).selected_btn(), Some(1));
    // A click in edit mode presses the selected button.
    h.encoder_click();
    assert_eq!(*log.borrow(), vec![1]);
    // A long press leaves edit mode; turning then moves the focus away.
    h.encoder_button(true);
    h.advance(Duration::ms(500));
    h.encoder_button(false);
    assert!(!h.engine().group_editing(g));
    h.encoder(1);
    assert_ne!(h.engine().focused(g), Some(m));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn btnm_popover_on_top_layer() {
    let (mut h, m) = scene(Mode::Light, &MAP_3X4);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 4, BtnCtrl::POPOVER);
    });
    let log = values(&mut h, m);
    let b = area(&h, m, 4);
    h.press(center(b));
    assert!(log.borrow().is_empty(), "popover buttons trigger on release");
    let p = get::<ButtonMatrix>(&h, m).popover_node().expect("popover");
    let layer = h.engine().top_layer(h.display()).unwrap();
    assert_eq!(h.engine().tree().parent(p), Some(layer));
    assert!(!h.engine().has_flag(p, ObjFlags::HIDDEN));
    assert!(!h.engine().has_flag(p, ObjFlags::CLICKABLE));
    h.update();
    let pc = h.engine().coords(p);
    assert_eq!(pc, Rect::new(b.x0, b.y0 - b.height(), b.x1, b.y1));
    // The popover area was redrawn.
    let inv: Vec<Rect> = h.invalidations().iter().map(|(r, _)| *r).collect();
    assert!(
        inv.iter()
            .any(|r| r.contains_rect(&Rect::new(pc.x0, pc.y0, pc.x1, pc.y0 + 1)))
    );
    h.release();
    assert_eq!(*log.borrow(), vec![4]);
    assert!(h.engine().has_flag(p, ObjFlags::HIDDEN));
    // Deleting the matrix deletes the popover.
    h.engine_mut().delete(m).unwrap();
    assert!(!h.engine().tree().contains(p));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn btnm_idle_after_interaction() {
    let (mut h, m) = scene(Mode::Dark, &MAP_3X4);
    h.tap(center(area(&h, m, 5)));
    h.drag(center(area(&h, m, 0)), center(area(&h, m, 11)), Duration::ms(200));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn btnm_theme_styles() {
    let (h, m) = scene(Mode::Light, &MAP_3X4);
    let e = h.engine();
    // Items: the button look (grey bg with a shadow in light mode), Main: a card.
    let p = e.style_prop(m, Part::Items, twine_style::PropId::ShadowWidth);
    assert!(p.as_i32().unwrap_or(0) > 0);
    assert_eq!(
        e.style_color(m, Part::Main, twine_style::PropId::BgColor),
        twine_theme::default::colors::LIGHT_CARD
    );
}

#[test]
fn snapshot_btnm_3x4() {
    for mode in Mode::ALL {
        let (mut h, _m) = scene(mode, &MAP_3X4);
        h.assert_snapshot(&format!("btnm_3x4_{}", mode.suffix()));
    }
}

#[test]
fn snapshot_btnm_checked_disabled_mix() {
    static MAP: [&str; 8] = ["A", "B", "C", "\n", "D", "E", "F", "G"];
    let (mut h, m) = scene(Mode::Light, &MAP);
    with(&mut h, m, |w: &mut ButtonMatrix, cx| {
        w.set_btn_ctrl(cx, 0, BtnCtrl::CHECKED);
        w.set_btn_ctrl(cx, 1, BtnCtrl::DISABLED);
        w.set_btn_ctrl(cx, 2, BtnCtrl::HIDDEN);
        w.set_btn_width(cx, 3, 2);
        w.set_btn_ctrl(cx, 5, BtnCtrl::CHECKED | BtnCtrl::DISABLED);
        w.set_selected_btn(cx, Some(6));
    });
    h.engine_mut().add_state(m, State::FOCUSED | State::FOCUS_KEY);
    h.run_until_idle();
    h.assert_snapshot("btnm_checked_disabled_mix");
}
