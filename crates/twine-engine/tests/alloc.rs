//! Allocation audits with a counting global allocator: tree iterators and the steady-state
//! frame allocate nothing (P4).

mod common;

use twine_core::{Color, Duration, Opa, Rect};
use twine_engine::{Engine, EngineConfig, InvalidateReason, Obj};
use twine_style::{GradDir, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::scenes::styled_box;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

#[test]
fn iterators_do_not_allocate() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let mut parent = root;
    for i in 0..1000 {
        let n = e.create(parent, Box::new(Obj)).unwrap();
        if i % 10 == 9 {
            parent = n;
        }
    }
    let t = e.tree();
    let (sum, stats) = count_allocs(|| {
        let mut n = 0usize;
        for id in t.descendants(root) {
            n += t.children(id).count() + t.children_rev(id).count() + t.ancestors(id).count();
        }
        n
    });
    assert!(sum > 1000);
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn steady_state_frame_allocates_nothing() {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        for i in 0..50 {
            let (x, y) = ((i % 10) * 30 + 5, (i / 10) * 45 + 5);
            let mut props = vec![
                StyleProp::BgColor(Color::hex(0x20_40_80 + i as u32 * 0x0003_0107)),
                StyleProp::BgOpa(Opa::COVER),
                StyleProp::Radius(6),
            ];
            if i % 3 == 0 {
                props.extend([StyleProp::ShadowWidth(10), StyleProp::ShadowOpa(Opa::P50)]);
            }
            if i % 4 == 1 {
                props.extend([
                    StyleProp::BgGradColor(Color::WHITE),
                    StyleProp::BgGradDir(GradDir::Ver),
                ]);
            }
            if i % 5 == 2 {
                props.extend([StyleProp::BorderWidth(2), StyleProp::OutlineWidth(1)]);
            }
            styled_box(e, s, Rect::from_xywh(x, y, 24, 36), &props);
        }
    });
    h.run_until_idle();
    let d = h.display();
    let area = Rect::from_xywh(40, 40, 100, 100);
    // Warm-up frames grow every buffer to its steady size.
    for _ in 0..2 {
        h.engine_mut()
            .invalidate_area(d, area, InvalidateReason::Explicit);
        h.advance(Duration::ms(16));
    }
    let ((), stats) = count_allocs(|| {
        h.engine_mut()
            .invalidate_area(d, area, InvalidateReason::Explicit);
        h.advance(Duration::ms(16));
    });
    assert!(h.last_frame().dirty_px >= 100 * 100);
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}

#[test]
fn anim_frame_allocates_nothing() {
    use twine_anim::{Anim, AnimProp, Repeat};
    let mut boxes = Vec::new();
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        for i in 0..20 {
            let r = Rect::from_xywh((i % 5) * 60 + 5, (i / 5) * 55 + 5, 30, 30);
            boxes.push(common::boxed(
                e,
                s,
                r,
                Color::hex(0x30_60_90 + i as u32 * 0x0002_0304),
            ));
        }
    });
    h.run_until_idle();
    for (i, &b) in boxes.iter().enumerate() {
        let (prop, from, to) = if i % 2 == 0 {
            (AnimProp::X, 0, 25)
        } else {
            (AnimProp::Opa, 255, 40)
        };
        h.engine_mut().anim_start(
            b,
            prop,
            Anim::new(from, to)
                .duration(Duration::ms(700))
                .playback(Duration::ms(700))
                .repeat(Repeat::Infinite),
        );
    }
    // Warm-up: every queue, dirty set and layout buffer reaches its steady size.
    for _ in 0..10 {
        h.advance(Duration::ms(16));
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..30 {
            h.advance(Duration::ms(16));
        }
    });
    assert!(h.last_frame().dirty_px > 0);
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}
