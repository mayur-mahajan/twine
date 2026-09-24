//! `SpanGroup`: LVGL's span flow (wrapping across span boundaries, mixed fonts on a common
//! baseline, indent, ellipsis, line limit), the modes' sizes, hit testing, precise
//! invalidation and the look.

mod common;

use common::{Mode, get, harness, with};
use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_20};
use twine_core::{Color, Point, Rect};
use twine_engine::{MeasureCx, NodeId};
use twine_style::{Align, PropId, StyleProp, TextDecor};
use twine_testing::EngineHarness;
use twine_text::TextLayout;
use twine_widgets::spangroup::{self, SpanGroup, SpanId, SpanLine, SpanMode, SpanOverflow};

fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(240, 160, mode);
    let screen = h.screen();
    let g = spangroup::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(g, Align::TopLeft, 10, 10);
    h.run_until_idle();
    (h, g)
}

fn add(h: &mut EngineHarness, g: NodeId, text: &str, props: &[StyleProp]) -> SpanId {
    with(h, g, |w: &mut SpanGroup, cx| {
        let id = w.add_span(cx);
        w.set_span_text(cx, id, text);
        for p in props {
            w.set_span_style(cx, id, *p);
        }
        id
    })
}

fn lines(h: &EngineHarness, g: NodeId) -> Vec<SpanLine> {
    get::<SpanGroup>(h, g).line_layout(&MeasureCx::new(h.engine(), g))
}

fn w14(s: &str) -> i32 {
    TextLayout::new(s, &MONTSERRAT_14).line_width(0..s.len())
}

fn w20(s: &str) -> i32 {
    TextLayout::new(s, &MONTSERRAT_20).line_width(0..s.len())
}

/// A `Break` group `width` px wide (content), no padding.
fn break_group(h: &mut EngineHarness, g: NodeId, width: i32) {
    with(h, g, |w: &mut SpanGroup, cx| w.set_mode(cx, SpanMode::Break));
    h.engine_mut().set_width(g, width);
    h.run_until_idle();
}

#[test]
fn span_defaults() {
    let (h, g) = scene(Mode::Light);
    let w = get::<SpanGroup>(&h, g);
    assert_eq!(h.engine().tree().node(g).unwrap().class().name, "spangroup");
    assert_eq!(w.span_count(), 0);
    assert_eq!(w.mode(&MeasureCx::new(h.engine(), g)), SpanMode::Expand);
    assert_eq!(
        (w.overflow(), w.indent(), w.max_lines()),
        (SpanOverflow::Clip, 0, -1)
    );
    assert_eq!(h.engine().coords(g).width(), 0, "empty: nothing to show");
}

#[test]
fn span_fallback_to_group_style() {
    let (mut h, g) = scene(Mode::Light);
    let a = add(&mut h, g, "abc", &[]);
    let b = add(&mut h, g, "def", &[StyleProp::TextColor(Color::RED)]);
    h.engine_mut()
        .set_local_prop(g, twine_style::Selector::MAIN, StyleProp::TextColor(Color::BLUE));
    let w = get::<SpanGroup>(&h, g);
    assert_eq!(
        w.span(a).unwrap().style().get(PropId::TextColor),
        None,
        "a uses the group's"
    );
    assert!(w.span(b).unwrap().style().get(PropId::TextColor).is_some());
    h.run_until_idle();
    // Pixels of "abc" are blue-ish, of "def" red-ish.
    let c = h.engine().content_area(g);
    let count = |h: &EngineHarness, x0: i32, x1: i32, f: &dyn Fn(Color) -> bool| {
        (x0..x1)
            .flat_map(|x| (c.y0..c.y1).map(move |y| (x, y)))
            .filter(|&(x, y)| f(h.pixel(x as u32, y as u32)))
            .count()
    };
    let wa = w14("abc");
    assert!(count(&h, c.x0, c.x0 + wa, &|p| p.b > 150 && p.r < 100) > 5);
    assert!(count(&h, c.x0 + wa + 1, c.x1, &|p| p.r > 150 && p.b < 100) > 5);
}

#[test]
fn spans_wrap_across_boundaries() {
    let (mut h, g) = scene(Mode::Light);
    let a = add(&mut h, g, "one two thr", &[]);
    let b = add(&mut h, g, "ee four", &[]);
    // Room for "one two three" but not for " four".
    let width = w14("one two three ") + 2;
    break_group(&mut h, g, width);
    let l = lines(&h, g);
    assert_eq!(l.len(), 2, "{l:?}");
    // The word "three" split across two spans stays on line 0 together.
    assert_eq!(l[0].pieces[0].0, a);
    assert_eq!(l[0].pieces[1].0, b);
    assert_eq!(l[0].pieces[1].1, 0..3, "\"ee \"");
    assert_eq!((l[1].pieces[0].0, l[1].pieces[0].1.clone()), (b, 3..7));
    // The height follows the lines.
    assert_eq!(h.engine().content_area(g).height(), 2 * 18);
}

#[test]
fn mixed_fonts_baseline_aligned() {
    let (mut h, g) = scene(Mode::Light);
    add(&mut h, g, "small ", &[]);
    add(&mut h, g, "BIG", &[StyleProp::TextFont(&MONTSERRAT_20)]);
    h.run_until_idle();
    let l = lines(&h, g);
    assert_eq!(l.len(), 1);
    assert_eq!(
        l[0].height,
        i32::from(MONTSERRAT_20.line_height),
        "the tallest span"
    );
    // The group is as tall as the big font's line (Expand mode).
    assert_eq!(
        h.engine().content_area(g).height(),
        i32::from(MONTSERRAT_20.line_height)
    );
    // Baselines: the small text is shifted down by the difference of the heights above
    // the baseline (LVGL: max_line_h - line_h - (max_base - base)).
    let small_top = l[0].height
        - i32::from(MONTSERRAT_14.line_height)
        - (i32::from(MONTSERRAT_20.base_line) - i32::from(MONTSERRAT_14.base_line));
    let above = |f: &twine_text::Font| i32::from(f.line_height) - i32::from(f.base_line);
    assert_eq!(
        small_top + above(&MONTSERRAT_14),
        above(&MONTSERRAT_20),
        "same baseline"
    );
}

#[test]
fn three_span_two_font_paragraph_layout_table() {
    // Hand-computed, 150 px wide: "The quick " (Montserrat 14, 76 px) + "BROWN fox "
    // (Montserrat 20, 124 px) + "jumps over the lazy dog" (14). LVGL's filling never
    // breaks a word that does not start the line:
    // - line 0: "The quick " (76); "BROWN" (89) does not fit in the remaining 74 px;
    // - line 1: "BROWN fox " (124) fits; "jumps" does not fit in the remaining 26 px;
    // - line 2: "jumps over the lazy " (146, the trailing space may overhang);
    // - line 3: "dog".
    type Row = (i32, i32, Vec<(SpanId, std::ops::Range<usize>, i32, i32)>);
    let (mut h, g) = scene(Mode::Light);
    let a = add(&mut h, g, "The quick ", &[]);
    let b = add(&mut h, g, "BROWN fox ", &[StyleProp::TextFont(&MONTSERRAT_20)]);
    let c = add(&mut h, g, "jumps over the lazy dog", &[]);
    break_group(&mut h, g, 150);
    assert_eq!(
        (w14("The quick "), w20("BROWN "), w20("BROWN fox ")),
        (76, 89, 124)
    );
    assert_eq!((w14("jumps over the lazy "), w14("dog")), (146, 29));
    let expected: Vec<Row> = vec![
        (0, 18, vec![(a, 0..10, 0, 76)]),
        (18, 26, vec![(b, 0..10, 0, 124)]),
        (44, 18, vec![(c, 0..20, 0, 146)]),
        (62, 18, vec![(c, 20..23, 0, 29)]),
    ];
    // Line heights: Montserrat 14 is 18 px, 20 is 26 px.
    assert_eq!((MONTSERRAT_14.line_height, MONTSERRAT_20.line_height), (18, 26));
    let got: Vec<Row> = lines(&h, g)
        .into_iter()
        .map(|l| (l.y, l.height, l.pieces))
        .collect();
    assert_eq!(got, expected);
    assert_eq!(h.engine().content_area(g).height(), 80);
    // Wider: "BROWN " joins line 0 (76 + 89 = 165 ≤ 170), "fox " and "jumps " share line 1.
    h.engine_mut().set_width(g, 170);
    h.run_until_idle();
    let l = lines(&h, g);
    assert_eq!(l[0].pieces, vec![(a, 0..10, 0, 76), (b, 0..6, 76, 89)]);
    assert_eq!((l[1].pieces[0].0, l[1].pieces[0].1.clone()), (b, 6..10));
    assert_eq!(l[1].pieces[1].0, c);
}

#[test]
fn indent_first_line() {
    let (mut h, g) = scene(Mode::Light);
    add(&mut h, g, "aaa bbb ccc ddd eee fff", &[]);
    with(&mut h, g, |w: &mut SpanGroup, cx| w.set_indent(cx, 20));
    break_group(&mut h, g, w14("aaa bbb ccc ") + 1);
    let l = lines(&h, g);
    assert_eq!(l[0].pieces[0].2, 20, "the first line starts at the indent");
    assert_eq!(
        l[0].pieces[0].1,
        0..8,
        "\"aaa bbb \": the indent leaves no room for ccc"
    );
    assert_eq!(l[1].pieces[0].2, 0);
    assert_eq!(l[1].pieces[0].1, 8..20);
}

#[test]
fn ellipsis_on_overflow() {
    let (mut h, g) = scene(Mode::Light);
    add(&mut h, g, "aaa bbb ccc ddd eee fff ggg", &[]);
    with(&mut h, g, |w: &mut SpanGroup, cx| {
        w.set_mode(cx, SpanMode::Fixed);
        w.set_overflow(cx, SpanOverflow::Ellipsis);
    });
    h.engine_mut().set_size(g, w14("aaa bbb ccc ") + 1, 18 * 2);
    h.run_until_idle();
    h.assert_region_snapshot("span_break_ellipsis", Rect::new(0, 0, 120, 60));
    // Clip: no dots, the second line ends normally.
    with(&mut h, g, |w: &mut SpanGroup, cx| {
        w.set_overflow(cx, SpanOverflow::Clip);
    });
    h.run_until_idle();
    assert_eq!(
        lines(&h, g).len(),
        3,
        "the layout is the same; only drawing differs"
    );
}

#[test]
fn expand_mode_size() {
    let (mut h, g) = scene(Mode::Light);
    add(&mut h, g, "Hello, ", &[]);
    add(&mut h, g, "world", &[StyleProp::TextLetterSpace(2)]);
    h.run_until_idle();
    let w = get::<SpanGroup>(&h, g);
    let m = MeasureCx::new(h.engine(), g);
    let expected = w.expand_width(&m, 0);
    assert_eq!(h.engine().content_area(g).width(), expected);
    assert_eq!(h.engine().content_area(g).height(), 18);
    // LVGL `lv_spangroup_get_expand_width`: glyph advances plus letter spaces.
    let manual = w14("Hello, ") + "world".chars().count() as i32 * 2 - 2
        + TextLayout::new("world", &MONTSERRAT_14).line_width(0..5);
    assert!((expected - manual).abs() <= 2, "{expected} vs {manual}");
    assert_eq!(w.expand_width(&m, 10), 10, "limited");
}

#[test]
fn max_lines_limit() {
    let (mut h, g) = scene(Mode::Light);
    add(&mut h, g, "aaa bbb ccc ddd eee fff ggg hhh", &[]);
    break_group(&mut h, g, w14("aaa bbb ") + 1);
    let full = h.engine().content_area(g).height();
    with(&mut h, g, |w: &mut SpanGroup, cx| w.set_max_lines(cx, 2));
    h.run_until_idle();
    assert!(full > 2 * 18);
    assert_eq!(h.engine().content_area(g).height(), 2 * 18);
    let w = get::<SpanGroup>(&h, g);
    assert_eq!(w.expand_height(&MeasureCx::new(h.engine(), g), 1000), 18);
}

#[test]
fn span_by_point_hit() {
    let (mut h, g) = scene(Mode::Light);
    let a = add(&mut h, g, "left ", &[]);
    let b = add(&mut h, g, "right", &[StyleProp::TextFont(&MONTSERRAT_20)]);
    h.run_until_idle();
    let c = h.engine().content_area(g);
    let w = get::<SpanGroup>(&h, g);
    let m = MeasureCx::new(h.engine(), g);
    assert_eq!(w.span_by_point(&m, Point::new(c.x0 + 2, c.y0 + 10)), Some(a));
    assert_eq!(
        w.span_by_point(&m, Point::new(c.x0 + w14("left ") + 5, c.y0 + 10)),
        Some(b)
    );
    assert_eq!(w.span_by_point(&m, Point::new(c.x1 + 5, c.y0 + 10)), None);
}

#[test]
fn dynamic_span_text_updates_only_group_area() {
    let (mut h, g) = scene(Mode::Light);
    let screen = h.screen();
    let other = twine_widgets::label::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(other, Align::BottomRight, 0, 0);
    let a = add(&mut h, g, "Hello, ", &[]);
    let b = add(&mut h, g, "Ann", &[]);
    h.run_until_idle();
    let before = h.engine().coords(g);
    with(&mut h, g, |w: &mut SpanGroup, cx| w.set_span_text(cx, b, "Bob"));
    h.advance(twine_core::Duration::ms(40));
    let after = h.engine().coords(g);
    let area = before.union(&after).expand(4);
    for (r, _) in h.invalidations() {
        assert!(area.contains_rect(r), "{r:?} outside {area:?}");
    }
    // Same text again: nothing.
    h.run_until_idle();
    with(&mut h, g, |w: &mut SpanGroup, cx| {
        w.set_span_text(cx, b, "Bob");
        w.set_span_text(cx, a, "Hello, ");
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn snapshot_span_mixed() {
    for m in Mode::ALL {
        let (mut h, g) = scene(m);
        add(&mut h, g, "Twine ", &[]);
        add(
            &mut h,
            g,
            "rich ",
            &[
                StyleProp::TextFont(&MONTSERRAT_20),
                StyleProp::TextColor(twine_theme::Palette::Blue.main()),
            ],
        );
        add(&mut h, g, "text ", &[StyleProp::TextDecor(TextDecor::UNDERLINE)]);
        add(
            &mut h,
            g,
            "with spans that wrap over lines",
            &[StyleProp::TextColor(twine_theme::Palette::Red.main())],
        );
        break_group(&mut h, g, 200);
        h.assert_snapshot(&format!("span_mixed_{}", m.suffix()));
    }
}

#[test]
fn snapshot_span_indent() {
    let (mut h, g) = scene(Mode::Light);
    add(
        &mut h,
        g,
        "An indented first line, then the paragraph continues on the following lines.",
        &[],
    );
    with(&mut h, g, |w: &mut SpanGroup, cx| w.set_indent(cx, 24));
    break_group(&mut h, g, 200);
    h.assert_snapshot("span_indent");
}
