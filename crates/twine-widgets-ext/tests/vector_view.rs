//! `VectorView`: a changed scene invalidates only the bounds of the items that differ, and the
//! screen after any sequence of changes equals the final scene drawn from scratch.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use proptest::prelude::*;
use twine_core::{Color, Fx, Rect};
use twine_engine::NodeId;
use twine_testing::EngineHarness;
use twine_vector::{FxPoint, Paint, Path, Stroke, VectorDsc, VectorScene};
use twine_widgets_ext::vector_view::{self, VectorView};

const W: u16 = 200;
const H: u16 = 150;

/// An item: a dot at `(x, y)` of radius `r` and color `c`, filled or (`stroke`) outlined.
#[derive(Clone, Copy, Debug)]
struct Dot {
    x: i32,
    y: i32,
    r: i32,
    c: u32,
    stroke: bool,
}

fn scene(dots: &[Dot]) -> VectorScene {
    let mut s = VectorScene::new();
    for d in dots {
        let mut p = Path::new();
        p.circle(FxPoint::from_int(d.x, d.y), Fx::from_int(d.r));
        let dsc = if d.stroke {
            VectorDsc {
                stroke: Some((
                    Paint::Solid(Color::hex(d.c)),
                    Stroke {
                        width: Fx::from_int(3),
                        ..Stroke::default()
                    },
                )),
                ..VectorDsc::default()
            }
        } else {
            VectorDsc::fill(Color::hex(d.c))
        };
        s.add(p, dsc);
    }
    s
}

fn view(h: &mut EngineHarness) -> NodeId {
    let screen = h.screen();
    let e = h.engine_mut();
    let v = vector_view::create(e, screen).expect("view");
    e.set_size(v, i32::from(W), i32::from(H));
    v
}

fn show(h: &mut EngineHarness, v: NodeId, dots: &[Dot]) {
    let s = scene(dots);
    h.engine_mut()
        .with_widget_mut(v, |w: &mut VectorView, cx| w.set_scene(cx, s));
}

#[test]
fn a_moved_item_invalidates_only_its_own_bounds() {
    let dots = [
        Dot {
            x: 20,
            y: 20,
            r: 8,
            c: 0xE53935,
            stroke: false,
        },
        Dot {
            x: 170,
            y: 20,
            r: 8,
            c: 0x43A047,
            stroke: true,
        },
        Dot {
            x: 100,
            y: 120,
            r: 8,
            c: 0x1E88E5,
            stroke: false,
        },
    ];
    let mut h = EngineHarness::new(W, H);
    let v = view(&mut h);
    show(&mut h, v, &dots);
    h.run_until_idle();
    let mut moved = dots;
    moved[2].x += 6;
    show(&mut h, v, &moved);
    h.update();
    let rects: Vec<Rect> = h.invalidations().iter().map(|(r, _)| *r).collect();
    // The third dot's old and new bounds (its circle, outset by 1 px), nothing of the others.
    let bounds = |d: &Dot| {
        let mut p = Path::new();
        p.circle(FxPoint::from_int(d.x, d.y), Fx::from_int(d.r));
        VectorDsc::fill(Color::BLACK).bounds(&p)
    };
    assert_eq!(rects, [bounds(&dots[2]), bounds(&moved[2])]);
    h.run_until_idle();
    assert!(
        h.last_frame().dirty_px < 1_000,
        "{} px redrawn",
        h.last_frame().dirty_px
    );
    // An equal scene invalidates nothing.
    show(&mut h, v, &moved);
    assert!(
        h.engine().invalidation_log().is_empty(),
        "{:?}",
        h.engine().invalidation_log()
    );
}

fn dot() -> impl Strategy<Value = Dot> {
    (0i32..200, 0i32..150, 2i32..30, 0u32..0xFFFFFF, any::<bool>()).prop_map(|(x, y, r, c, stroke)| Dot {
        x,
        y,
        r,
        c,
        stroke,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn incremental_updates_match_a_fresh_render(
        steps in proptest::collection::vec(proptest::collection::vec(dot(), 0..12), 1..6),
        edits in proptest::collection::vec((0usize..12, -20i32..20, -20i32..20), 0..6),
    ) {
        let mut h = EngineHarness::new(W, H);
        let v = view(&mut h);
        let mut last = Vec::new();
        // Whole new scenes (items added, removed, reordered, recolored)…
        for dots in &steps {
            show(&mut h, v, dots);
            h.run_until_idle();
            last.clone_from(dots);
        }
        // …then single items moved.
        for (i, dx, dy) in edits {
            if let Some(d) = last.get_mut(i) {
                d.x += dx;
                d.y += dy;
                show(&mut h, v, &last);
                h.run_until_idle();
            }
        }
        let mut fresh = EngineHarness::new(W, H);
        let fv = view(&mut fresh);
        show(&mut fresh, fv, &last);
        fresh.run_until_idle();
        prop_assert!(h.panel_rgb888() == fresh.panel_rgb888());
    }
}
