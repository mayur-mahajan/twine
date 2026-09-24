//! Criterion benchmarks of the refresh pipeline (RGB565, 320 × 240, blocking in-memory panel,
//! two 40-row buffers): a full redraw of the `engine_boxes` scene, a 40 × 40 partial redraw of
//! it, and a small-area redraw of a 1000-node tree (top-cover search).
//!
//! Run with `cargo bench -p twine-bench --bench refresh`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use criterion::{Criterion, criterion_group, criterion_main};
use twine_core::{Color, Duration, Opa, Rect};
use twine_engine::{Engine, InvalidateReason, NodeId};
use twine_style::{Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::scenes::{engine_boxes, styled_box};

fn redraw(c: &mut Criterion, name: &str, h: &mut EngineHarness, area: Rect) {
    let d = h.display();
    c.bench_function(name, |b| {
        b.iter(|| {
            h.engine_mut()
                .invalidate_area(d, area, InvalidateReason::Explicit);
            h.clock().advance(Duration::ms(16));
            h.update()
        });
    });
}

fn boxes() -> EngineHarness {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        engine_boxes(e);
    });
    h.run_until_idle();
    h
}

fn full_redraw_engine_boxes(c: &mut Criterion) {
    let mut h = boxes();
    redraw(
        c,
        "full_redraw_engine_boxes_320x240",
        &mut h,
        Rect::new(0, 0, 320, 240),
    );
}

fn partial_40x40_engine_boxes(c: &mut Criterion) {
    let mut h = boxes();
    redraw(
        c,
        "partial_redraw_40x40_engine_boxes",
        &mut h,
        Rect::from_xywh(40, 70, 40, 40),
    );
}

/// Ten nested levels, each with 99 small boxes and the next level on top.
fn large_tree(e: &mut Engine) -> NodeId {
    let d = e.default_display().unwrap();
    let mut parent = e.active_screen(d).unwrap();
    let solid = |c: u32| [StyleProp::BgColor(Color::hex(c)), StyleProp::BgOpa(Opa::COVER)];
    e.set_local_prop(parent, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
    for level in 0..10 {
        let r = Rect::new(level * 8, level * 6, 320 - level * 8, 240 - level * 6);
        for i in 0..99 {
            let x = r.x0 + 2 + (i % 11) * (r.width() / 11);
            let y = r.y0 + 2 + (i / 11) * (r.height() / 9);
            styled_box(e, parent, Rect::from_xywh(x, y, 4, 4), &solid(0x44_44_44));
        }
        parent = styled_box(e, parent, r, &solid(0xF0_F0_F0 - level as u32 * 0x0A_0A_0A));
    }
    parent
}

fn small_area_1000_nodes(c: &mut Criterion) {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        large_tree(e);
    });
    h.run_until_idle();
    redraw(
        c,
        "small_area_redraw_1000_nodes",
        &mut h,
        Rect::from_xywh(150, 110, 12, 12),
    );
}

criterion_group!(
    benches,
    full_redraw_engine_boxes,
    partial_40x40_engine_boxes,
    small_area_1000_nodes
);
criterion_main!(benches);
