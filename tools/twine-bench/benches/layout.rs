//! Criterion benchmarks of the layout pass (`Engine::update_layout`, invalidation included,
//! no rendering) on a 320 × 240 display: a 200-child wrapping flex container whose first
//! child changes width (every child moves), a 10 × 10 grid whose column gap changes (every
//! child moves), and a relayout of the flex container that changes nothing.
//!
//! Run with `cargo bench -p twine-bench --bench layout`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use criterion::{Criterion, criterion_group, criterion_main};
use twine_core::{Color, Opa};
use twine_engine::{Engine, NodeId, Obj};
use twine_style::{FlexFlow, GridTrack, LayoutKind, Length, Selector, StyleProp};
use twine_testing::EngineHarness;

fn solid(e: &mut Engine, n: NodeId, c: u32) {
    e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(Color::hex(c)));
    e.set_local_prop(n, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
}

/// A full-screen container with `layout`, returned with its children.
fn container(e: &mut Engine, layout: LayoutKind) -> NodeId {
    let s = e.active_screen(e.default_display().unwrap()).unwrap();
    let c = e.create(s, Box::new(Obj)).unwrap();
    e.set_size(c, Length::pct(100), Length::pct(100));
    e.set_layout(c, layout);
    e.set_local_prop(c, Selector::MAIN, StyleProp::PadRow(2));
    e.set_local_prop(c, Selector::MAIN, StyleProp::PadColumn(2));
    c
}

fn flex_200() -> (EngineHarness, NodeId, NodeId) {
    let mut ids = None;
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let c = container(e, LayoutKind::Flex);
        e.set_flex_flow(c, FlexFlow::RowWrap);
        let mut first = None;
        for i in 0..200 {
            let n = e.create(c, Box::new(Obj)).unwrap();
            e.set_size(n, 8 + i % 7, 8 + i % 5);
            solid(e, n, 0x20_40_80 + i as u32 * 0x0103);
            first.get_or_insert(n);
        }
        ids = Some((c, first.unwrap()));
    });
    h.run_until_idle();
    let (c, first) = ids.unwrap();
    (h, c, first)
}

fn flex_200_wrap_relayout(c: &mut Criterion) {
    let (mut h, _, first) = flex_200();
    let mut w = 8;
    c.bench_function("layout_flex_200_wrap_relayout", |b| {
        b.iter(|| {
            w = if w == 8 { 12 } else { 8 };
            let e = h.engine_mut();
            e.set_width(first, w);
            e.update_layout();
            e.layout_stats().moved
        });
    });
}

fn flex_200_unchanged_relayout(c: &mut Criterion) {
    let (mut h, cont, _) = flex_200();
    c.bench_function("layout_flex_200_unchanged_relayout", |b| {
        b.iter(|| {
            let e = h.engine_mut();
            e.mark_layout_dirty(cont);
            e.update_layout();
            e.layout_stats().moved
        });
    });
}

static TRACKS: [GridTrack; 10] = [GridTrack::Fr(1); 10];

fn grid_10x10_relayout(c: &mut Criterion) {
    let mut cont = None;
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let g = container(e, LayoutKind::Grid);
        e.set_grid_dsc_array(g, &TRACKS, &TRACKS);
        for i in 0..100 {
            let n = e.create(g, Box::new(Obj)).unwrap();
            e.set_grid_cell(
                n,
                twine_style::GridAlign::Stretch,
                i % 10,
                1,
                twine_style::GridAlign::Stretch,
                i / 10,
                1,
            );
            solid(e, n, 0x30_60_90 + i as u32 * 0x0201);
        }
        cont = Some(g);
    });
    h.run_until_idle();
    let g = cont.unwrap();
    let mut gap = 2;
    c.bench_function("layout_grid_10x10_relayout", |b| {
        b.iter(|| {
            gap = if gap == 2 { 3 } else { 2 };
            let e = h.engine_mut();
            e.set_local_prop(g, Selector::MAIN, StyleProp::PadColumn(gap));
            e.update_layout();
            e.layout_stats().moved
        });
    });
}

criterion_group!(
    benches,
    flex_200_wrap_relayout,
    flex_200_unchanged_relayout,
    grid_10x10_relayout
);
criterion_main!(benches);
