//! Criterion benchmark of one animation frame in the engine: 20 nodes animating `X` or `Opa`
//! (infinite playback), including the animation tick, the setters, the layout pass and the
//! refresh of the changed areas (RGB565, 320 × 240, blocking in-memory panel).
//!
//! Run with `cargo bench -p twine-bench --bench anim`.
#![allow(missing_docs)] // the harness macros generate undocumented public items

use criterion::{Criterion, criterion_group, criterion_main};
use twine_anim::{Anim, AnimProp, Repeat};
use twine_core::{Color, Duration, Opa, Rect};
use twine_style::StyleProp;
use twine_testing::EngineHarness;
use twine_testing::scenes::styled_box;

fn anim_frame_20_nodes(c: &mut Criterion) {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = e.active_screen(e.default_display().unwrap()).unwrap();
        for i in 0..20 {
            let r = Rect::from_xywh((i % 5) * 60 + 5, (i / 5) * 55 + 5, 30, 30);
            let b = styled_box(
                e,
                s,
                r,
                &[
                    StyleProp::BgColor(Color::hex(0x30_60_90 + i as u32 * 0x0002_0304)),
                    StyleProp::BgOpa(Opa::COVER),
                ],
            );
            let (prop, from, to) = if i % 2 == 0 {
                (AnimProp::X, r.x0, r.x0 + 25)
            } else {
                (AnimProp::Opa, 255, 40)
            };
            e.anim_start(
                b,
                prop,
                Anim::new(from, to)
                    .duration(Duration::ms(700))
                    .playback(Duration::ms(700))
                    .repeat(Repeat::Infinite),
            );
        }
    });
    h.advance(Duration::ms(160));
    c.bench_function("anim_frame_20_nodes", |b| {
        b.iter(|| h.advance(Duration::ms(16)));
    });
}

criterion_group!(benches, anim_frame_20_nodes);
criterion_main!(benches);
