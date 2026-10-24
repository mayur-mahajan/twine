//! Criterion benchmarks of themed widgets (R7.S01: the baseline the theming steps are measured
//! against; design `docs/plan/r7-theming-design.md` §1.2): with the LVGL default theme
//! (`Lvgl`, light) on a 320 × 240 RGB565 display —
//!
//! - `theme_button_resolve_<state>`: resolving the properties the draw path reads outside the
//!   per-node cache (shadow, outline, transform, padding, text) for a button in the default,
//!   pressed, checked and disabled states;
//! - `theme_label_inherit_card`: a label's inherited text properties, resolved through a card
//!   and the screen;
//! - `theme_slider_draw_dsc`: the rectangle descriptors of a slider's three parts;
//! - `theme_create_100_buttons`: creating 100 themed buttons with a label each (the theme's
//!   `apply` runs for every node; the deletion is not timed);
//! - `theme_button_press_release`: pressing and releasing a button (state changes, style
//!   refresh, the theme's transitions start; letting them finish is not timed);
//! - `theme_mode_switch`: switching the display between light and dark with the controls of a
//!   small screen on it.
//!
//! Each benchmark first prints the heap allocations of one operation (counted by the global
//! allocator), checks the ones that must not allocate, and the budgets of the R7 phase gate
//! are these numbers (`docs/plan/api-evolution.md` §6, R7.S01).
//!
//! Run with `cargo bench -p twine-bench --bench theme`; CI runs it once in test mode
//! (`-- --test`, `cargo xtask ci --only bench-build`).
#![allow(missing_docs)] // the harness macros generate undocumented public items

use std::hint::black_box;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use criterion::{Criterion, criterion_group, criterion_main};
use twine_core::{Duration, Opa};
use twine_engine::{NodeId, ThemeMode};
use twine_style::{Part, PropId, State};
use twine_testing::EngineHarness;
use twine_testing::alloc::{AllocStats, CountingAllocator, count_allocs};
use twine_theme::lvgl;
use twine_widgets::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// Properties the draw path resolves outside the per-node cache (`style_cache.rs`).
const UNCACHED: &[PropId] = &[
    PropId::ShadowWidth,
    PropId::ShadowOffsetY,
    PropId::ShadowOpacity,
    PropId::OutlineWidth,
    PropId::OutlineOffset,
    PropId::TransformWidth,
    PropId::TransformHeight,
    PropId::PaddingTop,
    PropId::PaddingLeft,
    PropId::TextColor,
    PropId::TextOpacity,
    PropId::Font,
];

/// Inherited text properties.
const INHERITED: &[PropId] = &[
    PropId::TextColor,
    PropId::TextOpacity,
    PropId::Font,
    PropId::LetterSpacing,
    PropId::LineSpacing,
];

fn harness() -> EngineHarness {
    EngineHarness::new(320, 240).theme(&lvgl::THEME)
}

/// Prints the allocations of one operation.
fn report(name: &str, s: &AllocStats) {
    eprintln!(
        "{name}: {} allocations, {} bytes per operation",
        s.allocs, s.bytes
    );
}

/// Resolves `props` of `part` of `id`.
fn resolve_all(h: &EngineHarness, id: NodeId, part: Part, props: &[PropId]) {
    let e = h.engine();
    for p in props {
        black_box(e.style_prop(black_box(id), part, *p));
    }
}

fn theme_button_resolve(c: &mut Criterion) {
    let states: [(&str, State); 4] = [
        ("default", State::DEFAULT),
        ("pressed", State::PRESSED),
        ("checked", State::CHECKED),
        ("disabled", State::DISABLED),
    ];
    for (name, state) in states {
        let mut h = harness();
        let screen = h.screen();
        let e = h.engine_mut();
        let b = button::create(e, screen).unwrap();
        let l = label::create(e, b).unwrap();
        e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, "Button"));
        if state != State::DEFAULT {
            e.add_state(b, state);
        }
        h.run_until_idle();
        let id = format!("theme_button_resolve_{name}");
        let ((), s) = count_allocs(|| resolve_all(&h, b, Part::Main, UNCACHED));
        report(&id, &s);
        assert_eq!(s.allocs, 0, "resolving allocates nothing");
        c.bench_function(&id, |bch| bch.iter(|| resolve_all(&h, b, Part::Main, UNCACHED)));
    }
}

fn theme_label_inherit_card(c: &mut Criterion) {
    let mut h = harness();
    let screen = h.screen();
    let e = h.engine_mut();
    let card = container::create_card(e, screen).unwrap();
    let l = label::create(e, card).unwrap();
    e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, "Inherited"));
    h.run_until_idle();
    let ((), s) = count_allocs(|| resolve_all(&h, l, Part::Main, INHERITED));
    report("theme_label_inherit_card", &s);
    assert_eq!(s.allocs, 0, "resolving allocates nothing");
    c.bench_function("theme_label_inherit_card", |b| {
        b.iter(|| resolve_all(&h, l, Part::Main, INHERITED));
    });
}

/// The rectangle descriptors of a slider's parts.
fn slider_dscs(h: &EngineHarness, s: NodeId) {
    let e = h.engine();
    for part in [Part::Main, Part::Indicator, Part::Knob] {
        black_box(e.rect_dsc(black_box(s), part, Opa::COVER));
    }
}

fn theme_slider_draw_dsc(c: &mut Criterion) {
    let mut h = harness();
    let screen = h.screen();
    let e = h.engine_mut();
    let s = slider::create(e, screen).unwrap();
    e.set_size(s, 200, 10);
    e.with_widget_mut(s, |w: &mut Slider, cx| w.set_value(cx, 40, false));
    h.run_until_idle();
    let ((), st) = count_allocs(|| slider_dscs(&h, s));
    report("theme_slider_draw_dsc", &st);
    assert_eq!(st.allocs, 0, "draw descriptors allocate nothing");
    c.bench_function("theme_slider_draw_dsc", |b| b.iter(|| slider_dscs(&h, s)));
}

/// Creates a container with 100 buttons of one label each under `parent`.
fn create_100(h: &mut EngineHarness, parent: NodeId) -> NodeId {
    let e = h.engine_mut();
    let cont = container::create(e, parent).unwrap();
    for _ in 0..100 {
        let b = button::create(e, cont).unwrap();
        let l = label::create(e, b).unwrap();
        e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, "OK"));
    }
    cont
}

fn theme_create_100_buttons(c: &mut Criterion) {
    let mut h = harness();
    let screen = h.screen();
    h.run_until_idle();
    // Warm up (theme style set, arenas).
    let warm = create_100(&mut h, screen);
    let _ = h.engine_mut().delete(warm);
    let (cont, s) = count_allocs(|| create_100(&mut h, screen));
    report("theme_create_100_buttons", &s);
    let _ = h.engine_mut().delete(cont);
    c.bench_function("theme_create_100_buttons", |b| {
        b.iter_custom(|iters| {
            let mut total = StdDuration::ZERO;
            for _ in 0..iters {
                let t = StdInstant::now();
                let cont = create_100(&mut h, screen);
                total += t.elapsed();
                let _ = h.engine_mut().delete(cont);
            }
            total
        });
    });
}

fn theme_button_press_release(c: &mut Criterion) {
    let mut h = harness();
    let screen = h.screen();
    let e = h.engine_mut();
    let b = button::create(e, screen).unwrap();
    let l = label::create(e, b).unwrap();
    e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, "Press"));
    h.run_until_idle();
    let toggle = |h: &mut EngineHarness| {
        let e = h.engine_mut();
        e.add_state(b, State::PRESSED);
        e.clear_state(b, State::PRESSED);
    };
    // Lets the started transitions finish (not timed: drawing is not what is measured).
    let settle = |h: &mut EngineHarness| {
        h.clock().advance(Duration::ms(1000));
        h.update();
    };
    // Warm up (transition slots, timeline).
    for _ in 0..3 {
        toggle(&mut h);
        settle(&mut h);
    }
    let ((), s) = count_allocs(|| toggle(&mut h));
    settle(&mut h);
    report("theme_button_press_release", &s);
    assert_eq!(
        s.allocs, 0,
        "a state change with transitions allocates nothing after warm-up"
    );
    c.bench_function("theme_button_press_release", |bch| {
        bch.iter_custom(|iters| {
            let mut total = StdDuration::ZERO;
            for _ in 0..iters {
                let t = StdInstant::now();
                toggle(&mut h);
                total += t.elapsed();
                settle(&mut h);
            }
            total
        });
    });
}

fn theme_mode_switch(c: &mut Criterion) {
    let mut h = harness();
    let d = h.display();
    let screen = h.screen();
    let e = h.engine_mut();
    let card = container::create_card(e, screen).unwrap();
    e.set_size(card, 300, 220);
    for _ in 0..4 {
        let b = button::create(e, card).unwrap();
        let l = label::create(e, b).unwrap();
        e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, "Button"));
    }
    let _ = slider::create(e, card).unwrap();
    let _ = switch::create(e, card).unwrap();
    let _ = checkbox::create(e, card).unwrap();
    h.run_until_idle();
    let mut dark = false;
    let mut switch_mode = |h: &mut EngineHarness| {
        dark = !dark;
        let mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
        h.engine_mut().set_theme_mode(d, mode);
    };
    // Warm up: both modes' tables built once.
    for _ in 0..4 {
        switch_mode(&mut h);
    }
    let ((), s) = count_allocs(|| switch_mode(&mut h));
    report("theme_mode_switch", &s);
    assert_eq!(
        s.allocs, 0,
        "a mode switch allocates nothing after warm-up (R2.S02)"
    );
    c.bench_function("theme_mode_switch", |b| b.iter(|| switch_mode(&mut h)));
}

criterion_group!(
    benches,
    theme_button_resolve,
    theme_label_inherit_card,
    theme_slider_draw_dsc,
    theme_create_100_buttons,
    theme_button_press_release,
    theme_mode_switch
);
criterion_main!(benches);
