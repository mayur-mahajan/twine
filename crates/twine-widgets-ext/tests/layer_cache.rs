//! The engine's layer cache with vector views (`DrawCost::Expensive`): a static background is
//! cached once it stays unchanged, and every frame of every change — in front of it, behind it,
//! of it — shows the same pixels as without the cache.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use proptest::prelude::*;
use twine_core::{Color, Fraction, Fx, Opa, Transform};
use twine_engine::{Engine, EngineConfig, NodeId, Obj, ObjFlags};
use twine_render::{FillRule, GradExtend, GradStop};
use twine_style::{Radius, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_vector::{FxPoint, Paint, Path, Stops, Stroke, VectorDsc, VectorScene};
use twine_widgets_ext::vector_view::{self, VectorView};

const W: u16 = 160;
const H: u16 = 120;

static STOPS: [GradStop; 2] = [
    GradStop {
        color: Color::hex(0x0F2A3F),
        opa: Opa::COVER,
        frac: Fraction::from_raw(0),
    },
    GradStop {
        color: Color::hex(0x07131F),
        opa: Opa::COVER,
        frac: Fraction::from_raw(255),
    },
];

/// The background: a gradient disc (center `c`, radius `r`) with a ring.
fn background(c: (i32, i32), r: i32, ring: u32) -> VectorScene {
    let mut s = VectorScene::new();
    let center = FxPoint::from_int(c.0, c.1);
    let mut disc = Path::new();
    disc.circle(center, Fx::from_int(r));
    s.add(
        disc,
        VectorDsc {
            fill: Some((
                Paint::Radial {
                    center,
                    radius: Fx::from_int(r),
                    focal: None,
                    stops: Stops::Static(&STOPS),
                    extend: GradExtend::Pad,
                    transform: Transform::IDENTITY,
                },
                FillRule::NonZero,
            )),
            ..VectorDsc::default()
        },
    );
    let mut ring_path = Path::new();
    ring_path.circle(center, Fx::from_int(r / 2));
    s.add(
        ring_path,
        VectorDsc {
            stroke: Some((
                Paint::Solid(Color::hex(ring)),
                Stroke {
                    width: Fx::from_int(2),
                    ..Stroke::default()
                },
            )),
            ..VectorDsc::default()
        },
    );
    s
}

/// The foreground: a dot at `(x, y)`.
fn foreground(x: i32, y: i32) -> VectorScene {
    let mut s = VectorScene::new();
    let mut p = Path::new();
    p.circle(FxPoint::from_int(x, y), Fx::from_int(6));
    s.add(p, VectorDsc::fill(Color::hex(0xFFB74D)));
    s
}

/// The nodes of the scene.
struct Scene {
    h: EngineHarness,
    panel: NodeId,
    bg: NodeId,
    fg: NodeId,
    chip: NodeId,
    /// On the top layer (a root drawn after the screen).
    toast: NodeId,
}

fn set_scene(h: &mut EngineHarness, v: NodeId, s: VectorScene) {
    h.engine_mut()
        .with_widget_mut(v, |w: &mut VectorView, cx| w.set_scene(cx, s));
}

fn obj(e: &mut Engine, parent: NodeId, x: i32, y: i32, w: i32, h: i32, color: u32) -> NodeId {
    let o = e.create(parent, Box::new(Obj)).unwrap();
    e.place(o, twine_core::Rect::from_xywh(x, y, w, h));
    e.set_local_prop(o, Selector::MAIN, StyleProp::BgColor(Color::hex(color).into()));
    e.set_local_prop(o, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    o
}

fn build(cache: Option<usize>) -> Scene {
    build_with(cache, false)
}

/// The scene on partial buffers (40-row strips) or, with `framebuffers`, on two full
/// framebuffers with area sync.
fn build_with(cache: Option<usize>, framebuffers: bool) -> Scene {
    let config = EngineConfig {
        layer_cache_bytes: cache,
        ..EngineConfig::default()
    };
    let mut h = EngineHarness::new(W, H).config(config);
    if framebuffers {
        h = h.framebuffer(twine_testing::FbMode::Full, 0);
    }
    let screen = h.screen();
    let e = h.engine_mut();
    // An earlier sibling, then a rounded, clipping panel holding the background and the
    // foreground, then a chip drawn on top of everything.
    let _header = obj(e, screen, 0, 0, i32::from(W), 14, 0x22385A);
    let panel = obj(e, screen, 4, 16, 150, 100, 0x0D1A2E);
    e.set_local_prop(panel, Selector::MAIN, StyleProp::Radius(Radius::Px(12).into()));
    e.set_local_prop(panel, Selector::MAIN, StyleProp::ClipCorner(true));
    let bg = vector_view::create(e, panel).unwrap();
    e.place(bg, twine_core::Rect::from_xywh(4, 16, 150, 100));
    let fg = vector_view::create(e, panel).unwrap();
    e.place(fg, twine_core::Rect::from_xywh(4, 16, 150, 100));
    let chip = obj(e, screen, 60, 60, 20, 12, 0xE53935);
    let display = h.display();
    let e = h.engine_mut();
    let top = e.top_layer(display).unwrap();
    let toast = obj(e, top, 100, 90, 30, 10, 0x5BE38C);
    set_scene(&mut h, bg, background((75, 50), 46, 0x2E6B7E));
    set_scene(&mut h, fg, foreground(30, 30));
    h.run_until_idle();
    Scene {
        h,
        panel,
        bg,
        fg,
        chip,
        toast,
    }
}

/// A change applied to both harnesses.
#[derive(Clone, Copy, Debug)]
enum Change {
    /// In front of the background: the foreground dot moves.
    Foreground(i32, i32),
    /// In front: the chip moves.
    Chip(i32, i32),
    /// The background itself changes (its ring color).
    Background(u32),
    /// The background moves.
    MoveBackground(i32, i32),
    /// Behind it: the panel's color changes.
    Panel(u32),
    /// The background is hidden or shown.
    HideBackground(bool),
    /// On the top layer: the toast moves.
    Toast(i32, i32),
}

fn apply(s: &mut Scene, c: Change) {
    let e = s.h.engine_mut();
    match c {
        Change::Foreground(x, y) => set_scene(&mut s.h, s.fg, foreground(x, y)),
        Change::Chip(x, y) => e.place(s.chip, twine_core::Rect::from_xywh(x, y, 20, 12)),
        Change::Background(ring) => set_scene(&mut s.h, s.bg, background((75, 50), 46, ring)),
        Change::MoveBackground(x, y) => e.place(s.bg, twine_core::Rect::from_xywh(x, y, 150, 100)),
        Change::Panel(c) => {
            e.set_local_prop(s.panel, Selector::MAIN, StyleProp::BgColor(Color::hex(c).into()));
        }
        Change::HideBackground(hide) => e.set_flag(s.bg, ObjFlags::HIDDEN, hide),
        Change::Toast(x, y) => e.place(s.toast, twine_core::Rect::from_xywh(x, y, 30, 10)),
    }
    s.h.run_until_idle();
}

#[test]
fn a_static_background_is_cached_and_reused() {
    let mut s = build(Some(1 << 20));
    for i in 0..8 {
        apply(&mut s, Change::Foreground(20 + i * 9, 30 + i * 5));
    }
    let st = s.h.engine().layer_cache_stats();
    assert_eq!(st.cached, 1, "{st:?}");
    assert!(st.hits >= 4 && st.builds == 1, "{st:?}");
    // A change of the background drops its pixels; they are drawn again once it settles.
    apply(&mut s, Change::Background(0x5BE38C));
    assert_eq!(s.h.engine().layer_cache_stats().cached, 0);
    for i in 0..6 {
        apply(&mut s, Change::Foreground(80 - i * 7, 70));
    }
    let st = s.h.engine().layer_cache_stats();
    assert_eq!((st.cached, st.builds), (1, 2), "{st:?}");
}

#[test]
fn cached_frames_match_for_every_kind_of_change() {
    cached_frames_match(false);
}

#[test]
fn cached_frames_match_on_framebuffers() {
    cached_frames_match(true);
}

fn cached_frames_match(framebuffers: bool) {
    let mut cached = build_with(Some(1 << 20), framebuffers);
    let mut plain = build_with(Some(0), framebuffers);
    let mut steps = Vec::new();
    // Warm up, then every kind of change, each followed by changes in front (cache hits).
    for i in 0..4 {
        steps.push(Change::Foreground(20 + i * 10, 40));
    }
    for c in [
        Change::Chip(30, 30),
        Change::Toast(10, 100),
        Change::Chip(100, 80),
        Change::Panel(0x123456),
        Change::Background(0x5BE38C),
        Change::MoveBackground(8, 20),
        Change::HideBackground(true),
        Change::HideBackground(false),
    ] {
        steps.push(c);
        for i in 0..4 {
            steps.push(Change::Foreground(130 - i * 25, 25 + i * 20));
            steps.push(Change::Toast(i * 30, 95 - i * 20));
        }
    }
    for c in steps {
        apply(&mut cached, c);
        apply(&mut plain, c);
        assert!(cached.h.panel_rgb888() == plain.h.panel_rgb888(), "after {c:?}");
    }
    let st = cached.h.engine().layer_cache_stats();
    assert!(st.hits > 20 && st.builds >= 4, "{st:?}");
}

#[test]
fn no_cache_without_a_budget() {
    let mut s = build(Some(0));
    for i in 0..8 {
        apply(&mut s, Change::Foreground(20 + i * 9, 30));
    }
    let st = s.h.engine().layer_cache_stats();
    assert_eq!((st.cached, st.bytes, st.hits), (0, 0, 0), "{st:?}");
    // Automatic without `mem_info` (the test harness reports none): off as well.
    let mut s = build(None);
    for i in 0..8 {
        apply(&mut s, Change::Foreground(20 + i * 9, 30));
    }
    assert_eq!(s.h.engine().layer_cache_stats().cached, 0);
}

fn change() -> impl Strategy<Value = Change> {
    prop_oneof![
        6 => (0i32..150, 10i32..115).prop_map(|(x, y)| Change::Foreground(x, y)),
        3 => (0i32..150, 0i32..115).prop_map(|(x, y)| Change::Chip(x, y)),
        1 => (0u32..0xFFFFFF).prop_map(Change::Background),
        1 => (-10i32..20, 10i32..30).prop_map(|(x, y)| Change::MoveBackground(x, y)),
        1 => (0u32..0xFFFFFF).prop_map(Change::Panel),
        1 => any::<bool>().prop_map(Change::HideBackground),
        2 => (0i32..150, 0i32..115).prop_map(|(x, y)| Change::Toast(x, y)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_frame_matches_drawing_without_the_cache(
        changes in proptest::collection::vec(change(), 1..40),
        framebuffers in any::<bool>(),
    ) {
        let mut cached = build_with(Some(1 << 20), framebuffers);
        let mut plain = build_with(Some(0), framebuffers);
        for c in changes {
            apply(&mut cached, c);
            apply(&mut plain, c);
            prop_assert!(cached.h.panel_rgb888() == plain.h.panel_rgb888(), "after {c:?}");
        }
    }
}
