//! The layer cache's policy: an expensive node is cached once stable, a node that changes soon
//! after being cached backs off, the budget bounds it, deleted nodes leave it — and the pixels
//! always equal drawing without the cache.
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

mod common;

use common::{boxed, white_screen};
use twine_core::{Color, Opa, Rect};
use twine_engine::{DrawCost, DrawCx, EngineConfig, NodeId, Widget, WidgetClass};
use twine_testing::EngineHarness;

static COSTLY: WidgetClass = WidgetClass::new("costly");

/// An "expensive" widget: a striped fill in its color.
struct Costly(Color);

impl Widget for Costly {
    fn class(&self) -> &'static WidgetClass {
        &COSTLY
    }
    fn draw_cost(&self) -> DrawCost {
        DrawCost::Expensive
    }
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        let c = cx.coords();
        for (i, y) in (c.y0..c.y1).step_by(3).enumerate() {
            let shade = if i % 2 == 0 { self.0 } else { Color::BLACK };
            cx.painter()
                .fill(Rect::new(c.x0, y, c.x1, (y + 3).min(c.y1)), shade, Opa::COVER);
        }
    }
}

struct Scene {
    h: EngineHarness,
    costly: NodeId,
    dot: NodeId,
}

fn scene(cache: Option<usize>) -> Scene {
    let mut h = EngineHarness::new(120, 90).config(EngineConfig {
        layer_cache_bytes: cache,
        ..EngineConfig::default()
    });
    let e = h.engine_mut();
    let s = white_screen(e);
    let costly = e.create(s, Box::new(Costly(Color::BLUE))).unwrap();
    e.set_pos(costly, 10, 10);
    e.set_size(costly, 100, 70);
    let dot = boxed(e, s, Rect::from_xywh(20, 20, 8, 8), Color::RED);
    h.run_until_idle();
    Scene { h, costly, dot }
}

impl Scene {
    /// Moves the dot (in front of the costly node) and draws a frame.
    fn move_dot(&mut self, i: i32) {
        self.h
            .engine_mut()
            .set_pos(self.dot, 15 + (i * 7) % 80, 15 + (i * 5) % 55);
        self.h.run_until_idle();
    }

    /// Changes the costly node's color and draws a frame.
    fn recolor(&mut self, c: Color) {
        self.h
            .engine_mut()
            .with_widget_mut(self.costly, |w: &mut Costly, cx| {
                w.0 = c;
                cx.invalidate();
            });
        self.h.run_until_idle();
    }
}

#[test]
fn a_node_that_keeps_changing_soon_after_caching_backs_off() {
    let mut s = scene(Some(1 << 20));
    let mut plain = scene(Some(0));
    // Two frames in front of it, then a change of the node: again and again.
    for k in 0..40 {
        for i in 0..2 {
            s.move_dot(k * 2 + i);
            plain.move_dot(k * 2 + i);
        }
        let c = if k % 2 == 0 { Color::GREEN } else { Color::BLUE };
        s.recolor(c);
        plain.recolor(c);
        assert!(s.h.panel_rgb888() == plain.h.panel_rgb888(), "cycle {k}");
    }
    // Each wasted build doubles the stable frames needed (2, 4, 8…): a handful of builds, not 40.
    let st = s.h.engine().layer_cache_stats();
    assert!(st.builds <= 3, "{st:?}");
}

#[test]
fn a_budget_too_small_for_the_pixels_caches_nothing() {
    let mut s = scene(Some(1000));
    for i in 0..10 {
        s.move_dot(i);
    }
    let st = s.h.engine().layer_cache_stats();
    assert_eq!((st.cached, st.bytes, st.budget), (0, 0, 1000), "{st:?}");
}

#[test]
fn a_deleted_node_leaves_the_cache() {
    let mut s = scene(Some(1 << 20));
    for i in 0..6 {
        s.move_dot(i);
    }
    assert_eq!(s.h.engine().layer_cache_stats().cached, 1);
    s.h.engine_mut().delete(s.costly).unwrap();
    s.h.run_until_idle();
    s.move_dot(7);
    let st = s.h.engine().layer_cache_stats();
    assert_eq!((st.watched, st.bytes), (0, 0), "{st:?}");
    // The screen shows no trace of it: the same as the same steps without the cache.
    let mut plain = scene(Some(0));
    for i in 0..6 {
        plain.move_dot(i);
    }
    plain.h.engine_mut().delete(plain.costly).unwrap();
    plain.h.run_until_idle();
    plain.move_dot(7);
    assert!(s.h.panel_rgb888() == plain.h.panel_rgb888());
}

#[test]
fn cached_pixels_are_used_for_areas_in_front() {
    let mut s = scene(Some(1 << 20));
    let mut plain = scene(Some(0));
    for i in 0..12 {
        s.move_dot(i);
        plain.move_dot(i);
        assert!(s.h.panel_rgb888() == plain.h.panel_rgb888(), "frame {i}");
    }
    let st = s.h.engine().layer_cache_stats();
    assert_eq!((st.cached, st.builds), (1, 1), "{st:?}");
    assert!(st.hits >= 8, "{st:?}");
    assert_eq!(st.bytes, 100 * 70 * 2);
}
