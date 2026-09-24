//! `Label` long modes: `Dots`, `Scroll`, `ScrollCircular` (LVGL `lv_label_refr_text`).

mod common;

use common::{Mode, get, harness, with};
use twine_assets::fonts::MONTSERRAT_14;
use twine_core::Duration;
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_text::{LongMode, TextLayout};
use twine_widgets::label::{self, Label};

const LONG: &str = "A rather long label text that does not fit";

/// A 100 px wide, one line high label in `mode`.
fn long_label(mode: LongMode) -> (EngineHarness, twine_engine::NodeId) {
    let mut h = harness(160, 60, Mode::Light);
    let screen = h.screen();
    let l = label::create_with(h.engine_mut(), screen, LONG).unwrap();
    let e = h.engine_mut();
    e.set_size(l, 100, i32::from(MONTSERRAT_14.line_height));
    e.align(l, Align::Center, 0, 0);
    with(&mut h, l, |w: &mut Label, cx| w.set_long_mode(cx, mode));
    (h, l)
}

fn text_width(s: &str) -> i32 {
    TextLayout::new(s, &MONTSERRAT_14).measure().w
}

#[test]
fn dots_truncate_to_fit() {
    let (mut h, l) = long_label(LongMode::Dots);
    h.run_until_idle();
    let w = get::<Label>(&h, l);
    let end = w.dots_end().expect("text is cut");
    let shown = format!("{}...", &LONG[..end]);
    assert!(shown.ends_with("..."));
    assert!(
        text_width(&shown) <= 100,
        "{shown:?} is {} px",
        text_width(&shown)
    );
    // One more character would not fit.
    let next = LONG[end..].chars().next().unwrap();
    let longer = format!("{}{}...", &LONG[..end], next);
    assert!(next == ' ' || text_width(&longer) > 100);
    h.assert_idle();
}

#[test]
fn dots_do_not_modify_text() {
    let (mut h, l) = long_label(LongMode::Dots);
    h.run_until_idle();
    assert_eq!(get::<Label>(&h, l).text(), LONG);
    assert_eq!(h.engine().tree().node(l).unwrap().widget().text(), Some(LONG));
    // When it fits again, no dots.
    h.engine_mut().set_width(l, 400);
    h.run_until_idle();
    assert_eq!(get::<Label>(&h, l).dots_end(), None);
}

#[test]
fn scroll_starts_anim_only_when_overflowing() {
    let (mut h, l) = long_label(LongMode::Scroll);
    h.update();
    h.advance(Duration::ms(200));
    assert_eq!(h.engine().anims_of(l).count(), 1);
    let (mut h2, l2) = long_label(LongMode::Scroll);
    with(&mut h2, l2, |w: &mut Label, cx| w.set_text(cx, "Short"));
    h2.run_until_idle();
    assert_eq!(h2.engine().anims_of(l2).count(), 0);
    h2.assert_idle();
}

#[test]
fn scroll_offset_follows_speed() {
    let (mut h, l) = long_label(LongMode::Scroll);
    h.update(); // layout, the animation starts
    let t0 = h.now();
    let x0 = get::<Label>(&h, l).scroll_offset().x;
    assert_eq!(x0, 0);
    h.advance(Duration::ms(500));
    let dt = h.now().saturating_duration_since(t0).as_millis() as i32;
    let x = get::<Label>(&h, l).scroll_offset().x;
    // 40 px/s to the left.
    let expected = -(40 * dt / 1000);
    assert!(
        (x - expected).abs() <= 1,
        "offset {x}, expected {expected} after {dt} ms"
    );
}

#[test]
fn scroll_invalidates_only_own_rect() {
    let (mut h, l) = long_label(LongMode::Scroll);
    h.update();
    h.advance(Duration::ms(100));
    let n = h.engine().tree().node(l).unwrap();
    let own = n.coords().expand(i32::from(n.ext_draw()));
    // At 40 px/s not every 16 ms frame moves the text by a whole pixel.
    let mut seen = 0;
    for _ in 0..20 {
        h.advance(Duration::ms(16));
        // Animation writes are rendered in the same update: check what was flushed.
        seen += h.flushes().len();
        for f in h.flushes() {
            assert!(own.contains_rect(&f.area), "{} outside {own}", f.area);
        }
    }
    assert!(seen >= 5, "{seen} flushes");
}

#[test]
fn circular_scroll_wraps() {
    let (mut h, l) = long_label(LongMode::ScrollCircular);
    h.update();
    let full = text_width(LONG) + 3 * MONTSERRAT_14.advance_px(' ', Some(' '));
    // One cycle takes `full / 40` s; afterwards the offset starts again near 0.
    let cycle = Duration::ms(u64::try_from(full * 1000 / 40).unwrap());
    let mut min = 0;
    let steps = cycle.as_millis() / 16 + 20;
    let mut wrapped = false;
    let mut prev = 0;
    for _ in 0..steps {
        h.advance(Duration::ms(16));
        let x = get::<Label>(&h, l).scroll_offset().x;
        min = min.min(x);
        if x > prev + 10 {
            wrapped = true;
        }
        prev = x;
    }
    assert!(min < -full + 20, "reached {min}, cycle {full}");
    assert!(wrapped, "the offset jumps back to the start");
}

#[test]
fn mode_change_deletes_anim() {
    let (mut h, l) = long_label(LongMode::Scroll);
    h.update();
    h.advance(Duration::ms(300));
    assert!(get::<Label>(&h, l).scroll_offset().x < 0);
    with(&mut h, l, |w: &mut Label, cx| w.set_long_mode(cx, LongMode::Clip));
    assert_eq!(h.engine().anims_of(l).count(), 0);
    assert_eq!(get::<Label>(&h, l).scroll_offset().x, 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn idle_when_not_overflowing() {
    for mode in [
        LongMode::Scroll,
        LongMode::ScrollCircular,
        LongMode::Dots,
        LongMode::Clip,
    ] {
        let (mut h, l) = long_label(mode);
        with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "Fits"));
        h.run_until_idle();
        h.assert_idle();
    }
}

#[test]
fn snapshot_label_dots() {
    let (mut h, _) = long_label(LongMode::Dots);
    h.assert_snapshot("label_dots");
}

#[test]
fn snapshot_label_scroll_t0() {
    let (mut h, _) = long_label(LongMode::Scroll);
    h.update();
    h.assert_panel_snapshot("label_scroll_t0");
}

#[test]
fn snapshot_label_scroll_t1500() {
    let (mut h, _) = long_label(LongMode::Scroll);
    h.update();
    h.advance(Duration::ms(1500));
    h.assert_panel_snapshot("label_scroll_t1500");
}

#[test]
fn snapshot_label_circular_t800() {
    let (mut h, _) = long_label(LongMode::ScrollCircular);
    h.update();
    h.advance(Duration::ms(800));
    h.assert_panel_snapshot("label_circular_t800");
}
