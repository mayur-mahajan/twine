//! The performance principles as tests: P1 (idle does nothing), P2 (work ∝ change), P4 (no
//! allocation per frame), and the top-cover optimization on a large tree.

mod common;

use std::sync::OnceLock;

use common::{boxed, white_screen};
use twine_core::{Color, Duration, Instant, Opa, Rect};
use twine_engine::{Engine, EngineConfig, InvalidateReason, NodeId, Wake};
use twine_style::{GradDir, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::scenes::{engine_boxes, styled_box};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn host_timer() -> Instant {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let s = START.get_or_init(std::time::Instant::now);
    Instant::from_micros(s.elapsed().as_micros() as u64)
}

#[test]
fn p1_idle_after_render() {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        engine_boxes(e);
    });
    h.run_until_idle();
    for _ in 0..100 {
        h.clock().advance(Duration::ms(7));
        assert_eq!(h.update(), Wake::Idle);
        assert!(h.flushes().is_empty());
    }
}

#[test]
fn p2_moving_box_invalidates_old_and_new_only() {
    let mut b = None;
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        boxed(e, s, Rect::from_xywh(200, 100, 50, 50), Color::RED);
        b = Some(boxed(e, s, Rect::from_xywh(40, 40, 20, 20), Color::BLUE));
    });
    h.run_until_idle();
    h.engine_mut().place(b.unwrap(), Rect::from_xywh(44, 40, 20, 20));
    h.clock().advance(Duration::ms(16));
    h.update();
    let s = h.last_frame();
    assert_eq!(s.dirty_areas, 1);
    assert!(s.dirty_px <= 24 * 20, "{s:?}");
    assert_eq!(h.flushes().len(), 1);
    assert_eq!(h.flushes()[0].area, Rect::from_xywh(40, 40, 24, 20));
    // With a shadow the extra draw size is added on both sides, and nothing else.
    let n = b.unwrap();
    h.engine_mut()
        .set_local_prop(n, Selector::MAIN, StyleProp::ShadowWidth(8));
    h.run_until_idle();
    h.engine_mut().place(n, Rect::from_xywh(48, 40, 20, 20));
    h.clock().advance(Duration::ms(16));
    h.update();
    let s = h.last_frame();
    let ext = 4; // shadow width / 2
    assert!(s.dirty_px <= ((24 + 2 * ext) * (20 + 2 * ext)) as u32, "{s:?}");
}

#[test]
fn p2_style_change_invalidates_node_only() {
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        for i in 0..10 {
            ids.push(boxed(e, s, Rect::from_xywh(i * 30, 10, 25, 25), Color::RED));
        }
    });
    h.run_until_idle();
    h.engine_mut()
        .set_local_prop(ids[3], Selector::MAIN, StyleProp::BgColor(Color::GREEN));
    let log = h.engine().invalidation_log().to_vec();
    assert!(!log.is_empty());
    assert!(
        log.iter().all(|(r, _)| *r == Rect::from_xywh(90, 10, 25, 25)),
        "{log:?}"
    );
    h.clock().advance(Duration::ms(16));
    h.update();
    assert_eq!(h.last_frame().dirty_px, 25 * 25);
    assert_eq!(h.pixel(100, 20), Color::GREEN);
}

#[test]
fn p4_zero_alloc_steady_state() {
    let mut mover: Option<NodeId> = None;
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        for i in 0..199 {
            let (x, y) = ((i % 20) * 16, (i / 20) * 24);
            let mut props = vec![
                StyleProp::BgColor(Color::hex(0x10_20_30 + i as u32 * 0x0001_0203)),
                StyleProp::BgOpa(Opa::COVER),
                StyleProp::Radius(3),
            ];
            if i % 7 == 0 {
                props.extend([StyleProp::ShadowWidth(6), StyleProp::ShadowOpa(Opa::P40)]);
            }
            if i % 5 == 0 {
                props.extend([
                    StyleProp::BgGradColor(Color::WHITE),
                    StyleProp::BgGradDir(GradDir::Hor),
                ]);
            }
            styled_box(e, s, Rect::from_xywh(x, y, 14, 20), &props);
        }
        mover = Some(boxed(e, s, Rect::from_xywh(0, 100, 30, 30), Color::BLACK));
    });
    assert_eq!(h.engine().tree().len(), 4 + 200);
    h.run_until_idle();
    let m = mover.unwrap();
    let frame = |h: &mut EngineHarness, i: i32| {
        h.engine_mut().place(m, Rect::from_xywh(i * 5 % 280, 100, 30, 30));
        h.advance(Duration::ms(16));
    };
    for i in 1..4 {
        frame(&mut h, i);
    }
    let ((), stats) = count_allocs(|| {
        for i in 4..54 {
            frame(&mut h, i);
        }
    });
    assert_eq!(h.last_frame().frame, 54);
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}

/// Ten nested levels of 100 nodes: each level is a container covering most of its parent,
/// holding 99 small boxes and the next level.
fn large_tree(e: &mut Engine) -> NodeId {
    let mut parent = white_screen(e);
    let mut last = parent;
    for level in 0..10 {
        let r = Rect::new(level * 8, level * 6, 320 - level * 8, 240 - level * 6);
        for i in 0..99 {
            let x = r.x0 + 2 + (i % 11) * (r.width() / 11);
            let y = r.y0 + 2 + (i / 11) * (r.height() / 9);
            boxed(
                e,
                parent,
                Rect::from_xywh(x, y, 4, 4),
                Color::hex(0x44_44_44 + level as u32 * 0x11),
            );
        }
        let c = boxed(e, parent, r, Color::hex(0xF0_F0_F0 - level as u32 * 0x0A_0A_0A));
        parent = c;
        last = c;
    }
    last
}

#[test]
fn large_tree_1000_nodes_frame_time() {
    let cfg = EngineConfig {
        hires_timer: Some(host_timer),
        ..EngineConfig::default()
    };
    let mut h = EngineHarness::new(320, 240)
        .no_theme()
        .config(cfg)
        .mount_engine(|e| {
            large_tree(e);
        });
    assert!(h.engine().tree().len() >= 1000);
    h.run_until_idle();
    let full = h.last_frame();
    eprintln!("1000 nodes, full redraw: {full:?}");
    assert!(full.nodes_drawn >= 1000);
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::from_xywh(150, 110, 12, 12), InvalidateReason::Explicit);
    h.clock().advance(Duration::ms(16));
    h.update();
    let small = h.last_frame();
    eprintln!("1000 nodes, 12x12 area: {small:?}");
    assert!(small.nodes_drawn <= 20, "top cover not used: {small:?}");
}
