//! Displays, layers, coordinates, extra draw size and precise invalidation.

mod common;

use common::{boxed, style, white_screen};
use twine_core::{Color, ColorFormat, Duration, Rect};
use twine_engine::{BufferMode, Engine, EngineConfig, EngineError, InvalidateReason, NodeId, Obj, ObjFlags};
use twine_hal::DisplayInfo;
use twine_style::StyleProp;
use twine_testing::{EngineHarness, MemoryDisplay, MockFramebufferDisplay, leak_buffer};

fn harness() -> EngineHarness {
    let mut h = EngineHarness::new(200, 100).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    h
}

fn log(h: &EngineHarness) -> Vec<Rect> {
    h.engine().invalidation_log().iter().map(|(r, _)| *r).collect()
}

#[test]
fn display_has_layers_and_screen() {
    let mut ids = [None; 4];
    let mut h = EngineHarness::new(100, 60).no_theme().mount_engine(|e| {
        let d = e.default_display().unwrap();
        let layers = [
            e.bottom_layer(d).unwrap(),
            e.active_screen(d).unwrap(),
            e.top_layer(d).unwrap(),
            e.sys_layer(d).unwrap(),
        ];
        let colors = [Color::RED, Color::GREEN, Color::BLUE, Color::BLACK];
        for (i, l) in layers.iter().enumerate() {
            assert_eq!(e.coords(*l), Rect::new(0, 0, 100, 60));
            assert_eq!(e.display_of(*l), Some(d));
            // Overlapping boxes: each layer's box is shifted right by 10 px.
            ids[i] = Some(boxed(e, *l, Rect::from_xywh(10 * i as i32, 0, 40, 20), colors[i]));
        }
        assert!(!e.has_flag(layers[0], ObjFlags::CLICKABLE));
        assert!(e.has_flag(layers[1], ObjFlags::CLICKABLE));
    });
    h.run_until_idle();
    // Z order bottom < screen < top < sys: at x = 35 all four overlap and sys wins.
    assert_eq!(h.pixel(5, 5), Color::RED);
    assert_eq!(h.pixel(15, 5), Color::GREEN);
    assert_eq!(h.pixel(25, 5), Color::BLUE);
    assert_eq!(h.pixel(35, 5), Color::BLACK);
    assert_eq!(h.pixel(50, 5), Color::BLACK);
    assert_eq!(h.pixel(50, 30), Color::WHITE); // nothing covers: white background
}

#[test]
fn add_display_rejects_mismatched_buffer_mode() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let info = DisplayInfo::new(10, 10, ColorFormat::Rgb565);
    assert_eq!(
        e.add_display(MemoryDisplay::new(info), BufferMode::full()),
        Err(EngineError::BufferModeMismatch)
    );
    assert_eq!(
        e.add_display(MemoryDisplay::new(info), BufferMode::direct()),
        Err(EngineError::BufferModeMismatch)
    );
    assert_eq!(
        e.add_framebuffer_display(
            MockFramebufferDisplay::new(info, true, 0),
            BufferMode::Partial {
                a: leak_buffer(200),
                b: None
            }
        ),
        Err(EngineError::BufferModeMismatch)
    );
    assert!(e.displays().next().is_none());
}

#[test]
fn too_many_displays() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let info = DisplayInfo::new(8, 8, ColorFormat::Rgb565);
    for i in 0..4 {
        let d = e
            .add_display(
                MemoryDisplay::new(info),
                BufferMode::Partial {
                    a: leak_buffer(16 * 8),
                    b: None,
                },
            )
            .unwrap();
        assert_eq!(d.index(), i);
    }
    assert_eq!(
        e.add_display(
            MemoryDisplay::new(info),
            BufferMode::Partial {
                a: leak_buffer(128),
                b: None
            }
        ),
        Err(EngineError::TooManyDisplays)
    );
    assert_eq!(e.default_display().map(twine_engine::DisplayId::index), Some(0));
}

#[test]
fn place_moves_subtree_by_delta() {
    let mut h = harness();
    let s = h.screen();
    let e = h.engine_mut();
    let parent = boxed(e, s, Rect::from_xywh(10, 10, 50, 50), Color::RED);
    let child = boxed(e, parent, Rect::from_xywh(20, 20, 10, 10), Color::BLUE);
    let grandchild = boxed(e, child, Rect::from_xywh(22, 22, 4, 4), Color::GREEN);
    e.place(parent, Rect::from_xywh(40, 5, 60, 50));
    assert_eq!(e.coords(parent), Rect::from_xywh(40, 5, 60, 50));
    assert_eq!(e.coords(child), Rect::from_xywh(50, 15, 10, 10));
    assert_eq!(e.coords(grandchild), Rect::from_xywh(52, 17, 4, 4));
}

#[test]
fn place_same_rect_is_noop() {
    let mut h = harness();
    let s = h.screen();
    let b = boxed(h.engine_mut(), s, Rect::from_xywh(10, 10, 20, 20), Color::RED);
    h.run_until_idle();
    h.engine_mut().place(b, Rect::from_xywh(10, 10, 20, 20));
    assert!(log(&h).is_empty());
    h.assert_idle();
    // A move invalidates exactly the old and the new area.
    h.engine_mut().place(b, Rect::from_xywh(14, 10, 20, 20));
    assert_eq!(
        log(&h),
        [Rect::from_xywh(10, 10, 20, 20), Rect::from_xywh(14, 10, 20, 20)]
    );
}

#[test]
fn invalidate_clipped_by_parent() {
    let mut h = harness();
    let s = h.screen();
    let e = h.engine_mut();
    let parent = boxed(e, s, Rect::from_xywh(10, 10, 30, 30), Color::RED);
    let child = boxed(e, parent, Rect::from_xywh(30, 30, 30, 30), Color::BLUE);
    h.run_until_idle();
    h.engine_mut().invalidate(child, InvalidateReason::Explicit);
    assert_eq!(log(&h), [Rect::new(30, 30, 40, 40)]);
}

#[test]
fn invalidate_not_clipped_with_overflow_visible() {
    let mut h = harness();
    let s = h.screen();
    let e = h.engine_mut();
    let parent = boxed(e, s, Rect::from_xywh(10, 10, 30, 30), Color::RED);
    e.set_flag(parent, ObjFlags::OVERFLOW_VISIBLE, true);
    let child = boxed(e, parent, Rect::from_xywh(30, 30, 30, 30), Color::BLUE);
    h.run_until_idle();
    h.engine_mut().invalidate(child, InvalidateReason::Explicit);
    assert_eq!(log(&h), [Rect::from_xywh(30, 30, 30, 30)]);
    // And it is drawn outside the parent.
    h.run_until_idle();
    assert_eq!(h.pixel(55, 55), Color::BLUE);
}

#[test]
fn hidden_ancestor_skips_invalidation() {
    let mut h = harness();
    let s = h.screen();
    let e = h.engine_mut();
    let parent = boxed(e, s, Rect::from_xywh(10, 10, 30, 30), Color::RED);
    let child = boxed(e, parent, Rect::from_xywh(15, 15, 5, 5), Color::BLUE);
    e.set_flag(parent, ObjFlags::HIDDEN, true);
    h.run_until_idle();
    assert_eq!(h.pixel(20, 20), Color::WHITE);
    h.engine_mut().invalidate(child, InvalidateReason::Explicit);
    h.engine_mut().set_local_prop(
        child,
        twine_style::Selector::MAIN,
        StyleProp::BgColor(Color::GREEN),
    );
    assert!(log(&h).is_empty());
    h.assert_idle();
    h.engine_mut().set_flag(parent, ObjFlags::HIDDEN, false);
    h.run_until_idle();
    assert_eq!(h.pixel(16, 16), Color::GREEN);
}

#[test]
fn inactive_screen_skips_invalidation() {
    let mut h = harness();
    let d = h.display();
    let other = h.engine_mut().create_screen(d).unwrap();
    let b = boxed(h.engine_mut(), other, Rect::from_xywh(0, 0, 10, 10), Color::RED);
    h.run_until_idle();
    h.engine_mut().invalidate(b, InvalidateReason::Explicit);
    assert!(log(&h).is_empty());
    h.assert_idle();
    h.engine_mut().load_screen(other);
    assert_eq!(log(&h), [Rect::new(0, 0, 200, 100)]);
    h.run_until_idle();
    assert_eq!(h.pixel(5, 5), Color::RED);
}

#[test]
fn ext_draw_includes_shadow_and_outline() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let n = e.create_root(Box::new(Obj)).unwrap();
    e.place(n, Rect::from_xywh(0, 0, 40, 40));
    style(
        &mut e,
        n,
        &[
            StyleProp::ShadowWidth(20),
            StyleProp::ShadowOffsetX(3),
            StyleProp::ShadowOffsetY(-5),
            StyleProp::ShadowSpread(2),
        ],
    );
    // width / 2 + max(|ofs|) + spread
    assert_eq!(e.tree().node(n).unwrap().ext_draw(), 10 + 5 + 2);
    style(
        &mut e,
        n,
        &[StyleProp::OutlineWidth(15), StyleProp::OutlinePad(10)],
    );
    assert_eq!(e.tree().node(n).unwrap().ext_draw(), 25);
    style(
        &mut e,
        n,
        &[
            StyleProp::ShadowOpa(twine_core::Opa::TRANSP),
            StyleProp::OutlineWidth(1),
        ],
    );
    assert_eq!(e.tree().node(n).unwrap().ext_draw(), 11);
    // A 90° rotation of a non-square node grows the area by the length difference / 2.
    let m = e.create_root(Box::new(Obj)).unwrap();
    e.place(m, Rect::from_xywh(0, 0, 40, 20));
    style(
        &mut e,
        m,
        &[
            StyleProp::TransformRotation(twine_core::Angle::deg(90)),
            StyleProp::TransformPivotX(twine_style::Length::Pct(50)),
            StyleProp::TransformPivotY(twine_style::Length::Pct(50)),
        ],
    );
    let ext = e.tree().node(m).unwrap().ext_draw();
    assert!((10..=12).contains(&ext), "ext {ext}");
}

#[test]
fn rectset_overflow_falls_back_to_bounding_box() {
    let mut h = harness();
    let d = h.display();
    for i in 0..33 {
        let x = (i % 11) * 18;
        let y = (i / 11) * 30;
        h.engine_mut()
            .invalidate_area(d, Rect::from_xywh(x, y, 5, 5), InvalidateReason::Explicit);
    }
    h.advance(Duration::ms(16));
    assert_eq!(h.last_frame().dirty_areas, 1);
    assert_eq!(h.flushes()[0].area.x0, 0);
    assert_eq!(h.last_frame().dirty_px, 185 * 65);
}

#[test]
fn delete_invalidates_area() {
    let mut h = harness();
    let s = h.screen();
    let b = boxed(h.engine_mut(), s, Rect::from_xywh(50, 50, 20, 10), Color::RED);
    let c: NodeId = boxed(h.engine_mut(), b, Rect::from_xywh(52, 52, 4, 4), Color::BLUE);
    h.run_until_idle();
    assert_eq!(h.pixel(60, 55), Color::RED);
    h.engine_mut().delete(b).unwrap();
    assert!(!h.engine().tree().contains(c));
    assert!(log(&h).contains(&Rect::from_xywh(50, 50, 20, 10)));
    h.run_until_idle();
    assert_eq!(h.pixel(60, 55), Color::WHITE);
    let s = h.screen();
    assert!(h.engine_mut().delete(s).is_err());
}
