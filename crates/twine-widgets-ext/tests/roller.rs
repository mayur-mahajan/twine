//! `Roller`: LVGL's defaults and height from visible rows, dragging with snap, clicking a row,
//! infinite mode (virtual repetition, bounded offset), keypad / encoder control, drawing only
//! the visible rows, the snap animation's invalidation and the theme's look.

mod common;

use common::{Mode, center, class, get, harness, harness_with_group, values, with};
use twine_core::{Duration, Point};
use twine_engine::{EventCode, EventParam, Key, MeasureCx, NodeId};
use twine_style::{Align, Part, PropId};
use twine_testing::EngineHarness;
use twine_theme::Palette;
use twine_widgets_ext::roller::{self, ROLLER_CLASS, Roller, RollerMode};

const DAYS: &str = "Monday\nTuesday\nWednesday\nThursday\nFriday\nSaturday\nSunday";

fn scene(mode: Mode, rmode: RollerMode) -> (EngineHarness, NodeId) {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let other = twine_widgets::button::create(e, screen).unwrap();
    e.set_size(other, 10, 10);
    e.align(other, Align::BottomRight, 0, 0);
    let r = roller::create(e, screen).unwrap();
    e.align(r, Align::Center, 0, 0);
    with(&mut h, r, |w: &mut Roller, cx| {
        w.set_options_static(cx, DAYS, rmode);
        w.set_visible_row_count(cx, 3);
    });
    h.run_until_idle();
    (h, r)
}

fn sel(h: &EngineHarness, r: NodeId) -> u16 {
    get::<Roller>(h, r).selected()
}

/// Drags from `from` to `to` in 10 steps, holds still for a read period (no throw), releases.
fn drag_hold(h: &mut EngineHarness, from: Point, to: Point) {
    h.press(from);
    for k in 1..=10 {
        h.advance(Duration::ms(30));
        h.move_to(Point::new(
            from.x + (to.x - from.x) * k / 10,
            from.y + (to.y - from.y) * k / 10,
        ));
    }
    h.advance(Duration::ms(30));
    h.release();
}

/// The row unit (line height + line space of `Main`).
fn unit(h: &EngineHarness, r: NodeId) -> i32 {
    let m = MeasureCx::new(h.engine(), r);
    i32::from(m.font(Part::Main).line_height) + m.style_i32(Part::Main, PropId::LineSpacing)
}

#[test]
fn roller_defaults() {
    let mut h = harness(320, 240, Mode::Light);
    let screen = h.screen();
    let r = roller::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    assert_eq!(class(&h, r), "roller");
    assert_eq!(ROLLER_CLASS.editable, twine_engine::Editable::True);
    assert!(
        !ROLLER_CLASS
            .default_flags
            .contains(twine_engine::ObjFlags::SCROLLABLE)
    );
    let w = get::<Roller>(&h, r);
    assert_eq!(w.options(), roller::ROLLER_DEFAULT_OPTIONS);
    assert_eq!(
        (w.option_count(), w.selected(), w.mode()),
        (5, 0, RollerMode::Normal)
    );
    let c = h.engine().coords(r);
    assert_eq!(c.height(), 130, "LV_DPI_DEF");
    // Content wide: the widest option plus the card's padding and border.
    let m = MeasureCx::new(h.engine(), r);
    let text_w = twine_text::TextLayout::new(roller::ROLLER_DEFAULT_OPTIONS, m.font(Part::Main))
        .measure()
        .w;
    let pad = m.padding(Part::Main);
    let b = m.style_i32(Part::Main, PropId::BorderWidth);
    assert_eq!(c.width(), text_w + pad.left + pad.right + 2 * b);
    let mut s = String::new();
    w.selected_str(&mut s);
    assert_eq!(s, "Option 1");
    h.assert_idle();
}

#[test]
fn roller_theme_styles() {
    let (h, r) = scene(Mode::Light, RollerMode::Normal);
    let e = h.engine();
    assert_eq!(e.style_i32(r, Part::Main, PropId::AnimDuration), 200);
    assert_eq!(
        e.style_color(r, Part::Selected, PropId::BgColor),
        Palette::Blue.main()
    );
    assert_eq!(
        e.style_color(r, Part::Selected, PropId::TextColor),
        twine_core::Color::WHITE
    );
    assert_eq!(
        e.style_i32(r, Part::Main, PropId::LineSpacing),
        twine_style::dpx(20, 130)
    );
}

#[test]
fn roller_height_from_visible_rows() {
    for dpi in [130u16, 260] {
        let mut h = EngineHarness::new(320, 480).theme(twine_theme::DefaultTheme::builder().dpi(dpi).build());
        let screen = h.screen();
        let r = roller::create(h.engine_mut(), screen).unwrap();
        with(&mut h, r, |w: &mut Roller, cx| w.set_visible_row_count(cx, 4));
        h.run_until_idle();
        let m = MeasureCx::new(h.engine(), r);
        let font_h = i32::from(m.font(Part::Main).line_height);
        let ls = m.style_i32(Part::Main, PropId::LineSpacing);
        let b = m.style_i32(Part::Main, PropId::BorderWidth);
        assert_eq!(
            h.engine().coords(r).height(),
            (font_h + ls) * 4 + 2 * b,
            "{dpi} dpi"
        );
        assert_eq!(get::<Roller>(&h, r).visible_row_count(), 4);
        // Idempotent.
        with(&mut h, r, |w: &mut Roller, cx| w.set_visible_row_count(cx, 4));
        h.assert_idle();
    }
}

#[test]
fn roller_drag_and_snap_selects_nearest() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    let changes = values(&mut h, r);
    let u = unit(&h, r);
    let c = center(&h, r);
    // Drag up by two rows and hold (no throw).
    drag_hold(&mut h, c, Point::new(c.x, c.y - 2 * u));
    h.run_until_idle();
    assert_eq!(sel(&h, r), 2);
    assert_eq!(*changes.borrow(), vec![2]);
    // The column rests exactly on the selected row.
    let w = get::<Roller>(&h, r);
    let m = MeasureCx::new(h.engine(), r);
    let font_h = i32::from(m.font(Part::Main).line_height);
    assert_eq!(w.offset_y(), m.content_area().height() / 2 - font_h / 2 - 2 * u);
    // Dragging past the end clamps to the last option.
    drag_hold(&mut h, c, Point::new(c.x, c.y - 4 * u));
    h.run_until_idle();
    drag_hold(&mut h, c, Point::new(c.x, c.y - 4 * u));
    h.run_until_idle();
    assert_eq!(sel(&h, r), 6);
    // A flick throws further than the finger moved.
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    h.drag(c, Point::new(c.x, c.y - u), Duration::ms(90));
    h.run_until_idle();
    assert!(sel(&h, r) > 1, "thrown: {}", sel(&h, r));
    h.assert_idle();
}

#[test]
fn roller_click_row_selects() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    let changes = values(&mut h, r);
    let u = unit(&h, r);
    let c = center(&h, r);
    // The row below the band.
    h.tap(Point::new(c.x, c.y + u));
    h.run_until_idle();
    assert_eq!(sel(&h, r), 1);
    h.tap(Point::new(c.x, c.y + u));
    h.run_until_idle();
    assert_eq!(sel(&h, r), 2);
    // Tapping the selected row changes nothing.
    h.tap(c);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 2);
    assert_eq!(*changes.borrow(), vec![1, 2]);
    h.assert_idle();
}

#[test]
fn roller_infinite_wraps_and_normalizes() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Infinite);
    let pages = get::<Roller>(&h, r).page_count();
    assert!(pages >= 3 && pages % 2 == 1, "{pages}");
    let u = unit(&h, r);
    let c = center(&h, r);
    let start = get::<Roller>(&h, r).offset_y();
    // Up from the first option wraps to the last.
    let _ = h.keypad_input();
    h.engine_mut().focus(r);
    h.key(Key::Up);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 6);
    // Drag 1000 rows down in steps; the offset stays bounded (normalized to the middle page).
    let bound = i32::from(pages) * 7 * u;
    for _ in 0..250 {
        drag_hold(&mut h, c, Point::new(c.x, c.y + 4 * u));
        h.run_until_idle();
        let y = get::<Roller>(&h, r).offset_y();
        assert!((y - start).abs() <= bound, "offset {y} not bounded");
    }
    // 1 + 1000 rows up the column = option (6 - 1000) mod 7.
    assert_eq!(i32::from(sel(&h, r)), (6 - 1000_i32).rem_euclid(7));
    h.assert_idle();
}

#[test]
fn roller_set_mode_keeps_options_and_is_idempotent() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    assert_eq!(get::<Roller>(&h, r).page_count(), 1);
    with(&mut h, r, |w: &mut Roller, cx| {
        w.set_mode(cx, RollerMode::Infinite);
    });
    h.run_until_idle();
    let w = get::<Roller>(&h, r);
    assert_eq!(w.mode(), RollerMode::Infinite);
    assert!(w.page_count() >= 3);
    assert_eq!((w.option_count(), w.options()), (7, DAYS));
    let offset = w.offset_y();
    with(&mut h, r, |w: &mut Roller, cx| {
        w.set_mode(cx, RollerMode::Infinite);
    });
    assert_eq!(get::<Roller>(&h, r).offset_y(), offset);
    h.assert_idle();
}

#[test]
fn roller_infinite_no_string_duplication() {
    let (h, r) = scene(Mode::Light, RollerMode::Infinite);
    let w = get::<Roller>(&h, r);
    assert_eq!(w.options_storage().len(), DAYS.len());
    assert!(matches!(
        w.options_storage(),
        twine_widgets_ext::Options::Static(_)
    ));
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    let owned = "a\nb\nc\nd";
    with(&mut h, r, |w: &mut Roller, cx| {
        w.set_options(cx, owned, RollerMode::Infinite);
    });
    let w = get::<Roller>(&h, r);
    assert_eq!(w.options_storage().len(), owned.len());
    assert_eq!(w.option_count(), 4);
}

#[test]
fn roller_draw_visits_visible_rows_only() {
    // Many options: a draw visits only the rows in the roller (+ the band's second pass).
    let mut h = harness(320, 240, Mode::Light);
    let screen = h.screen();
    let r = roller::create(h.engine_mut(), screen).unwrap();
    let many: String = (0..500)
        .map(|i| format!("Row {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    with(&mut h, r, |w: &mut Roller, cx| {
        w.set_options(cx, &many, RollerMode::Infinite);
        w.set_visible_row_count(cx, 3);
    });
    h.render_full();
    let drawn = get::<Roller>(&h, r).rows_drawn();
    assert!(drawn > 0 && drawn <= 8, "rows drawn: {drawn}");
}

#[test]
fn roller_keypad_esc_restores() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    let changes = values(&mut h, r);
    let _ = h.keypad_input();
    h.engine_mut().focus(r);
    h.key(Key::Down);
    h.key(Key::Down);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 2);
    h.key(Key::Esc);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 0, "restored");
    assert!(changes.borrow().is_empty());
    h.key(Key::Down);
    h.key(Key::Enter);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 1);
    assert_eq!(*changes.borrow(), vec![1]);
    // Leaving without confirming restores too.
    h.key(Key::Down);
    h.key(Key::Next);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 1);
    h.assert_idle();
}

#[test]
fn roller_encoder_flow() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    let changes = values(&mut h, r);
    let g = h.engine().group_of(r).unwrap();
    let _ = h.encoder_input();
    h.engine_mut().focus(r);
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    h.encoder(3);
    h.run_until_idle();
    assert_eq!(sel(&h, r), 3);
    h.encoder_click();
    assert!(!h.engine().group_editing(g));
    assert_eq!(*changes.borrow(), vec![3]);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn roller_rotary_and_set_selected() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    h.engine_mut()
        .send_event(r, EventCode::Rotary, EventParam::Rotary(2));
    h.run_until_idle();
    assert_eq!(sel(&h, r), 2);
    with(&mut h, r, |w: &mut Roller, cx| w.set_selected(cx, 5, false));
    assert_eq!(sel(&h, r), 5);
    h.run_until_idle();
    // Idempotent.
    with(&mut h, r, |w: &mut Roller, cx| {
        w.set_selected(cx, 5, false);
        w.set_options_static(cx, DAYS, RollerMode::Normal);
    });
    h.assert_idle();
    with(&mut h, r, |w: &mut Roller, cx| w.set_selected(cx, 99, false));
    assert_eq!(sel(&h, r), 6, "clamped");
}

#[test]
fn roller_snap_anim_then_idle() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    with(&mut h, r, |w: &mut Roller, cx| w.set_selected(cx, 4, true));
    h.advance(Duration::ms(100));
    let mid = get::<Roller>(&h, r).offset_y();
    h.run_until_idle();
    let end = get::<Roller>(&h, r).offset_y();
    assert_ne!(mid, end, "animated");
    h.assert_idle();
}

#[test]
fn roller_anim_frame_invalidates_only_its_area() {
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    with(&mut h, r, |w: &mut Roller, cx| w.set_selected(cx, 4, true));
    let c = h.engine().coords(r);
    let b = h.engine().style_i32(r, Part::Main, PropId::BorderWidth);
    let inner = c.inset(twine_core::Insets::new(b, b, b, b));
    let mut seen = 0;
    for _ in 0..20 {
        h.advance(Duration::ms(10));
        for (a, reason) in h.invalidations() {
            seen += 1;
            assert!(inner.contains_rect(a), "{a:?} ({reason:?}) outside {inner:?}");
        }
    }
    assert!(seen > 0, "the animation redrew something");
    h.run_until_idle();
}

#[test]
fn snapshot_roller() {
    for m in Mode::ALL {
        for (rm, name) in [(RollerMode::Normal, "normal"), (RollerMode::Infinite, "infinite")] {
            let (mut h, r) = scene(m, rm);
            with(&mut h, r, |w: &mut Roller, cx| {
                w.set_selected(cx, if rm == RollerMode::Normal { 2 } else { 0 }, false);
            });
            h.run_until_idle();
            h.assert_snapshot(&format!("roller_{name}_{}", m.suffix()));
        }
    }
    let (mut h, r) = scene(Mode::Light, RollerMode::Normal);
    let c = center(&h, r);
    let u = unit(&h, r);
    h.press(c);
    h.advance(Duration::ms(30));
    h.move_to(Point::new(c.x, c.y - u / 4));
    h.advance(Duration::ms(30));
    h.move_to(Point::new(c.x, c.y - u / 2));
    h.advance(Duration::ms(40));
    h.assert_panel_snapshot("roller_mid_drag");
    h.release();
    h.run_until_idle();
}
