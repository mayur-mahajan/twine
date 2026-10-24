//! Criterion benchmarks of the widgets' draw and event paths (RGB565, 320 × 240, default light
//! theme, blocking in-memory panel): full redraws of a keyboard (button matrix) with its
//! textarea, of the value widgets (bar, slider, arc, switch, checkbox, LED, line) with a label
//! paragraph and a span group, and of the selectors (roller, open dropdown list, list of
//! buttons); typing into a textarea (insert + delete, cursor math), and one drag frame on a
//! slider and on an arc (value ↔ position mapping).
//!
//! Run with `cargo bench -p twine-bench --bench widgets`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use criterion::{Criterion, criterion_group, criterion_main};
use twine_core::{Duration, Point, Rect};
use twine_engine::{InvalidateReason, NodeId};
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_theme::lvgl;
use twine_widgets::prelude::*;
use twine_widgets_ext::{dropdown, list, roller};

const PARAGRAPH: &str = "Twine draws widgets with integer arithmetic only. This paragraph wraps \
over several lines so that the label's line breaking and glyph loops run on every redraw.";

fn harness() -> EngineHarness {
    EngineHarness::new(320, 240).theme(&lvgl::THEME)
}

fn full_redraw(c: &mut Criterion, name: &str, h: &mut EngineHarness) {
    let d = h.display();
    c.bench_function(name, |b| {
        b.iter(|| {
            h.engine_mut()
                .invalidate_area(d, Rect::new(0, 0, 320, 240), InvalidateReason::Explicit);
            h.clock().advance(Duration::ms(16));
            h.update()
        });
    });
}

/// A textarea with two lines of text and a keyboard attached to it.
fn keyboard_scene() -> (EngineHarness, NodeId) {
    let mut h = harness();
    let screen = h.screen();
    let e = h.engine_mut();
    let ta = textarea::create(e, screen).unwrap();
    e.set_size(ta, 300, 60);
    e.align(ta, Align::TopMid, 0, 10);
    let kb = keyboard::create(e, screen).unwrap();
    e.with_widget_mut(kb, |k: &mut Keyboard, cx| k.set_textarea(cx, Some(ta)));
    e.with_widget_mut(ta, |t: &mut Textarea, cx| {
        t.set_text(cx, "Hello, keyboard!\nA second line of text");
    });
    // The cursor blinks: the engine never idles.
    h.advance(Duration::ms(100));
    (h, ta)
}

fn widgets_keyboard_redraw(c: &mut Criterion) {
    let (mut h, _) = keyboard_scene();
    full_redraw(c, "widgets_keyboard_textarea_redraw", &mut h);
}

fn widgets_textarea_type(c: &mut Criterion) {
    let (mut h, ta) = keyboard_scene();
    c.bench_function("widgets_textarea_insert_delete", |b| {
        b.iter(|| {
            h.engine_mut().with_widget_mut(ta, |t: &mut Textarea, cx| {
                t.cursor_left(cx);
                t.add_char(cx, 'x');
                t.delete_char(cx);
                t.cursor_right(cx);
            });
            h.clock().advance(Duration::ms(16));
            h.update()
        });
    });
}

/// The value widgets, a label paragraph and a span group.
fn controls_scene() -> (EngineHarness, NodeId, NodeId) {
    let mut h = harness();
    let screen = h.screen();
    let e = h.engine_mut();
    let b = bar::create(e, screen).unwrap();
    e.set_pos(b, 10, 10);
    e.set_size(b, 140, 12);
    e.with_widget_mut(b, |w: &mut Bar, cx| w.set_value(cx, 40, false));
    let s = slider::create(e, screen).unwrap();
    e.set_pos(s, 20, 40);
    e.set_size(s, 120, 10);
    e.with_widget_mut(s, |w: &mut Slider, cx| w.set_value(cx, 70, false));
    let a = arc::create(e, screen).unwrap();
    e.set_pos(a, 170, 10);
    e.set_size(a, 100, 100);
    e.with_widget_mut(a, |w: &mut Arc, cx| w.set_value(cx, 60));
    let sw = switch::create(e, screen).unwrap();
    e.set_pos(sw, 10, 70);
    let cb = checkbox::create(e, screen).unwrap();
    e.set_pos(cb, 70, 70);
    e.with_widget_mut(cb, |w: &mut Checkbox, cx| w.set_text(cx, "Check"));
    let l = led::create(e, screen).unwrap();
    e.set_pos(l, 280, 120);
    let ln = line::create(e, screen).unwrap();
    e.with_widget_mut(ln, |w: &mut Line, cx| {
        w.set_points(
            cx,
            &[
                Point::new(0, 0),
                Point::new(40, 20),
                Point::new(80, 0),
                Point::new(120, 20),
            ],
        );
    });
    e.set_pos(ln, 170, 120);
    let p = label::create(e, screen).unwrap();
    e.set_pos(p, 10, 110);
    e.set_width(p, 150);
    e.with_widget_mut(p, |w: &mut Label, cx| w.set_text(cx, PARAGRAPH));
    let sg = spangroup::create(e, screen).unwrap();
    e.set_pos(sg, 170, 150);
    e.set_width(sg, 140);
    e.with_widget_mut(sg, |w: &mut SpanGroup, cx| {
        for t in ["Spans ", "of several ", "styles wrap ", "across lines."] {
            let id = w.add_span(cx);
            w.set_span_text(cx, id, t);
        }
    });
    h.run_until_idle();
    (h, s, a)
}

fn widgets_controls_redraw(c: &mut Criterion) {
    let (mut h, _, _) = controls_scene();
    full_redraw(c, "widgets_controls_label_spans_redraw", &mut h);
}

/// Drags the pointer between two points one read period apart per iteration.
fn drag_frames(c: &mut Criterion, name: &str, h: &mut EngineHarness, a: Point, b: Point) {
    let period = h.engine().config().read_period;
    h.press(a);
    for p in [b, a, b] {
        h.clock().advance(period);
        h.move_to(p);
    }
    let mut at_a = false;
    c.bench_function(name, |bch| {
        bch.iter(|| {
            h.clock().advance(period);
            h.move_to(if at_a { a } else { b });
            at_a = !at_a;
        });
    });
    h.release();
}

fn widgets_slider_drag(c: &mut Criterion) {
    let (mut h, s, _) = controls_scene();
    let r = h.engine().coords(s);
    let y = (r.y0 + r.y1) / 2;
    drag_frames(
        c,
        "widgets_slider_drag_frame",
        &mut h,
        Point::new(r.x0 + 40, y),
        Point::new(r.x0 + 80, y),
    );
}

fn widgets_arc_drag(c: &mut Criterion) {
    let (mut h, _, a) = controls_scene();
    let r = h.engine().coords(a);
    let cx = (r.x0 + r.x1) / 2;
    // Points on the arc's ring (right side and bottom-right).
    drag_frames(
        c,
        "widgets_arc_drag_frame",
        &mut h,
        Point::new(r.x1 - 6, (r.y0 + r.y1) / 2),
        Point::new(cx + 30, r.y1 - 12),
    );
}

/// A roller, an open dropdown and a list of ten buttons.
fn selectors_scene() -> EngineHarness {
    let mut h = harness();
    let screen = h.screen();
    let e = h.engine_mut();
    let r = roller::create(e, screen).unwrap();
    e.set_pos(r, 10, 10);
    e.with_widget_mut(r, |w: &mut twine_widgets_ext::roller::Roller, cx| {
        w.set_options_static(
            cx,
            "January\nFebruary\nMarch\nApril\nMay\nJune\nJuly\nAugust\nSeptember\nOctober\nNovember\nDecember",
            twine_widgets_ext::roller::RollerMode::Normal,
        );
        w.set_visible_row_count(cx, 5);
        w.set_selected(cx, 5, false);
    });
    let l = list::create(e, screen).unwrap();
    e.set_pos(l, 210, 10);
    e.set_size(l, 100, 220);
    for i in 0..10 {
        list::add_button(e, l, None, ["Alpha", "Beta", "Gamma", "Delta", "Epsilon"][i % 5]).unwrap();
    }
    let d = dropdown::create(e, screen).unwrap();
    e.set_pos(d, 110, 10);
    e.set_width(d, 90);
    e.with_widget_mut(d, |w: &mut twine_widgets_ext::dropdown::Dropdown, cx| {
        w.set_options_static(cx, "One\nTwo\nThree\nFour\nFive\nSix");
        w.open(cx);
    });
    // Advance a fixed time past the open animation (this scene does not reach idle within
    // `run_until_idle`'s limit).
    h.advance(Duration::ms(1000));
    h
}

fn widgets_selectors_redraw(c: &mut Criterion) {
    let mut h = selectors_scene();
    full_redraw(c, "widgets_roller_dropdown_list_redraw", &mut h);
}

criterion_group!(
    benches,
    widgets_keyboard_redraw,
    widgets_textarea_type,
    widgets_controls_redraw,
    widgets_slider_drag,
    widgets_arc_drag,
    widgets_selectors_redraw
);
criterion_main!(benches);
