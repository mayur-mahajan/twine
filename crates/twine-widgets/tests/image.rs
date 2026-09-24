//! `Image`: sources (static, encoded, file, symbol), content size, transforms and their
//! extra draw size, cover checks, inner alignment and recoloring.

mod common;

#[path = "../../../examples/src/assets/logo_argb8888.rs"]
mod logo_argb8888;
#[path = "../../../examples/src/assets/logo_rgb565.rs"]
mod logo_rgb565;
#[path = "../../../examples/src/assets/logo_rgb565a8.rs"]
mod logo_rgb565a8;

use std::collections::HashMap;

use common::{Mode, get, harness, with};
use twine_core::{Angle, Color, Duration, Opa, Point, Rect, Scale};
use twine_engine::{Anim, AnimProp, EngineConfig, MeasureCx, NodeId, ObjFlags};
use twine_image::{FileSource, ImageSource};
use twine_style::{Align, Part, Selector, StyleProp};
use twine_testing::{EngineHarness, capture_logs};
use twine_text::symbols::OK as SYMBOL_OK;
use twine_widgets::image::{self, IMAGE_CLASS, Image, ImageAlign};

static LOGO_QOI: &[u8] = include_bytes!("../../../assets/images/twine_logo.qoi");

fn cfg() -> EngineConfig {
    EngineConfig {
        image_cache_bytes: 64 * 1024,
        ..EngineConfig::default()
    }
}

fn scene(mode: Mode, src: ImageSource) -> (EngineHarness, NodeId) {
    let mut h = harness(160, 120, mode).config(cfg());
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(i, Align::Center, 0, 0);
    with(&mut h, i, |w: &mut Image, cx| w.set_src(cx, src));
    (h, i)
}

fn stat(img: &'static twine_image::Image) -> ImageSource {
    ImageSource::Static(img)
}

#[test]
fn image_defaults_match_lvgl() {
    let mut h = harness(100, 80, Mode::Light);
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    let n = h.engine().tree().node(i).unwrap();
    assert_eq!(n.class().name, "image");
    assert!(!n.flags().contains(ObjFlags::CLICKABLE));
    assert!(n.flags().contains(ObjFlags::ADV_HITTEST));
    assert_eq!(IMAGE_CLASS.parts, &[Part::Main]);
    let w = get::<Image>(&h, i);
    assert!(w.src().is_none());
    assert_eq!(w.inner_align(), ImageAlign::Center);
    assert_eq!(
        (w.rotation(), w.scale_x(), w.scale_y()),
        (Angle(0), Scale::ONE, Scale::ONE)
    );
    assert!(w.antialias());
    // The default theme gives images no styles.
    assert!(n.styles().is_empty());
    h.run_until_idle();
    assert_eq!(h.engine().coords(i).width(), 0);
    h.assert_idle();
}

#[test]
fn image_content_size_from_header() {
    let (mut h, i) = scene(Mode::Light, stat(&logo_rgb565a8::LOGO_RGB565A8));
    h.run_until_idle();
    let c = h.engine().coords(i);
    assert_eq!((c.width(), c.height()), (64, 64));
    // Encoded data is probed, not decoded.
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::Encoded(LOGO_QOI));
    });
    h.run_until_idle();
    assert_eq!(get::<Image>(&h, i).src_size(), twine_core::Size::new(64, 64));
    // Symbols are measured as text.
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::Symbol(SYMBOL_OK));
    });
    h.run_until_idle();
    let s = twine_text::TextLayout::new(SYMBOL_OK, &twine_assets::fonts::MONTSERRAT_14).measure();
    let c = h.engine().coords(i);
    assert_eq!((c.width(), c.height()), (s.w, s.h));
    h.assert_idle();
}

#[test]
fn set_same_src_no_invalidate() {
    let (mut h, i) = scene(Mode::Light, stat(&logo_rgb565a8::LOGO_RGB565A8));
    h.run_until_idle();
    // Static images compare by address.
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, stat(&logo_rgb565a8::LOGO_RGB565A8));
    });
    assert!(h.engine().invalidation_log().is_empty());
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::Symbol(SYMBOL_OK));
    });
    h.run_until_idle();
    // Symbols compare by string equality (a different `&'static str` with the same text).
    let copy: &'static str = Box::leak(SYMBOL_OK.to_owned().into_boxed_str());
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::Symbol(copy));
    });
    assert!(h.engine().invalidation_log().is_empty());
    // Every transform setter is idempotent too.
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_rotation(cx, Angle(3600));
        w.set_scale(cx, Scale::ONE);
        w.set_antialias(cx, true);
        w.set_inner_align(cx, ImageAlign::Center);
        w.set_offset(cx, Point::ZERO);
        w.set_blend_mode(cx, twine_render::BlendMode::Normal);
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn rotation_extends_ext_draw() {
    let (mut h, i) = scene(Mode::Light, stat(&logo_rgb565::LOGO_RGB565));
    h.run_until_idle();
    assert_eq!(h.engine().tree().node(i).unwrap().ext_draw(), 0);
    with(&mut h, i, |w: &mut Image, cx| w.set_rotation(cx, Angle::deg(45)));
    // A 64 px square rotated by 45°: half diagonal 45.25 - 32 → 14 px (rounded out).
    let ext = h.engine().tree().node(i).unwrap().ext_draw();
    assert!((13..=15).contains(&ext), "ext_draw {ext}");
    with(&mut h, i, |w: &mut Image, cx| w.set_rotation(cx, Angle(0)));
    assert_eq!(h.engine().tree().node(i).unwrap().ext_draw(), 0);
    with(&mut h, i, |w: &mut Image, cx| w.set_scale(cx, Scale(512)));
    assert_eq!(h.engine().tree().node(i).unwrap().ext_draw(), 32);
}

#[test]
fn image_covers_only_when_opaque() {
    let (mut h, i) = scene(Mode::Light, stat(&logo_rgb565::LOGO_RGB565));
    h.run_until_idle();
    let c = h.engine().coords(i);
    let inner = Rect::new(c.x0 + 4, c.y0 + 4, c.x1 - 4, c.y1 - 4);
    let covers = |h: &EngineHarness| {
        let e = h.engine();
        e.tree()
            .node(i)
            .unwrap()
            .widget()
            .covers(&MeasureCx::new(e, i), inner)
    };
    assert!(covers(&h), "an opaque RGB565 image covers its area");
    with(&mut h, i, |w: &mut Image, cx| w.set_rotation(cx, Angle::deg(10)));
    assert!(!covers(&h), "not rotated");
    with(&mut h, i, |w: &mut Image, cx| w.set_rotation(cx, Angle(0)));
    h.engine_mut()
        .set_local_prop(i, Selector::MAIN, StyleProp::ImageOpa(Opa(200)));
    assert!(!covers(&h), "not faded");
    h.engine_mut()
        .set_local_prop(i, Selector::MAIN, StyleProp::ImageOpa(Opa::COVER));
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, stat(&logo_argb8888::LOGO_ARGB8888));
    });
    h.run_until_idle();
    assert!(!covers(&h), "images with alpha do not cover");
}

#[test]
fn missing_file_logs_and_is_empty() {
    let (mut h, i) = scene(Mode::Light, stat(&logo_rgb565::LOGO_RGB565));
    h.run_until_idle();
    let ((), logs) = capture_logs(|| {
        with(&mut h, i, |w: &mut Image, cx| {
            w.set_src(cx, ImageSource::file("S:/missing.png").unwrap());
        });
    });
    assert!(
        logs.iter().any(|l| l.level == log::Level::Warn
            && l.target == "twine::image"
            && l.message.contains("missing.png")),
        "{logs:?}"
    );
    assert!(get::<Image>(&h, i).src().is_none());
    h.run_until_idle();
    assert_eq!(h.engine().coords(i).width(), 0);
    h.assert_idle();
}

/// Files from memory.
struct MemFs(HashMap<&'static str, &'static [u8]>);
impl FileSource for MemFs {
    fn read_all(&mut self, path: &str, out: &mut Vec<u8>) -> Result<(), twine_image::Error> {
        let d = self.0.get(path).ok_or(twine_image::Error::NotFound)?;
        out.clear();
        out.extend_from_slice(d);
        Ok(())
    }
}

#[test]
fn file_source_through_the_engine() {
    let mut h = harness(160, 120, Mode::Light).config(cfg());
    h.engine_mut()
        .set_file_source(Some(Box::new(MemFs(HashMap::from([("S:/logo.qoi", LOGO_QOI)])))));
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(i, Align::Center, 0, 0);
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::file("S:/logo.qoi").unwrap());
    });
    h.run_until_idle();
    assert_eq!(h.engine().coords(i).width(), 64);
    // Drawn: the logo's center is not the screen background.
    let c = h.engine().coords(i);
    let px = h.pixel(((c.x0 + c.x1) / 2) as u32, ((c.y0 + c.y1) / 2) as u32);
    assert_ne!(px, Color::hex(0x00F5_F5F5));
    h.assert_idle();
}

#[test]
fn rotating_never_invalidates_outside_ext_draw() {
    let (mut h, i) = scene(Mode::Light, stat(&logo_rgb565a8::LOGO_RGB565A8));
    h.run_until_idle();
    // The largest extra draw size of the rotation (at 45°).
    let c = h.engine().coords(i);
    let bound = c.expand(15);
    h.engine_mut()
        .anim_start(i, AnimProp::Value, Anim::new(0, 3600).duration(Duration::secs(1)));
    for _ in 0..70 {
        h.advance(Duration::ms(16));
        let ext = i32::from(h.engine().tree().node(i).unwrap().ext_draw());
        assert!(ext <= 15);
        for f in h.flushes() {
            assert!(bound.contains_rect(&f.area), "{} outside {bound}", f.area);
        }
    }
    h.run_until_idle();
    h.assert_idle();
}

#[allow(clippy::needless_pass_by_value)] // call sites build the source inline
fn snap(name: &str, src: ImageSource, f: impl Fn(&mut EngineHarness, NodeId)) {
    for m in Mode::ALL {
        let (mut h, i) = scene(m, src.clone());
        f(&mut h, i);
        h.run_until_idle();
        h.assert_snapshot(&format!("{name}_{}", m.suffix()));
    }
}

#[test]
fn snapshot_image_rgb565() {
    snap("image_rgb565", stat(&logo_rgb565::LOGO_RGB565), |_, _| {});
}

#[test]
fn snapshot_image_argb_on_bg() {
    snap("image_argb_on_bg", stat(&logo_argb8888::LOGO_ARGB8888), |h, i| {
        let e = h.engine_mut();
        e.set_local_prop(
            i,
            Selector::MAIN,
            StyleProp::BgColor(twine_theme::Palette::Amber.main()),
        );
        e.set_local_prop(i, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
        e.set_size(i, 80, 80);
    });
}

#[test]
fn snapshot_image_rot45_aa() {
    snap("image_rot45_aa", stat(&logo_rgb565a8::LOGO_RGB565A8), |h, i| {
        with(h, i, |w: &mut Image, cx| w.set_rotation(cx, Angle::deg(45)));
    });
}

#[test]
fn snapshot_image_scale_150() {
    snap("image_scale_150", ImageSource::Encoded(LOGO_QOI), |h, i| {
        with(h, i, |w: &mut Image, cx| w.set_scale(cx, Scale(384)));
    });
}

#[test]
fn snapshot_image_recolor() {
    snap("image_recolor", stat(&logo_rgb565a8::LOGO_RGB565A8), |h, i| {
        let e = h.engine_mut();
        e.set_local_prop(
            i,
            Selector::MAIN,
            StyleProp::ImageRecolor(twine_theme::Palette::Green.main()),
        );
        e.set_local_prop(i, Selector::MAIN, StyleProp::ImageRecolorOpa(Opa::P70));
    });
}

#[test]
fn snapshot_image_symbol() {
    snap("image_symbol", ImageSource::Symbol(SYMBOL_OK), |h, i| {
        let e = h.engine_mut();
        e.set_local_prop(
            i,
            Selector::MAIN,
            StyleProp::TextFont(&twine_assets::fonts::MONTSERRAT_20),
        );
        e.set_local_prop(
            i,
            Selector::MAIN,
            StyleProp::TextColor(twine_theme::Palette::Blue.main()),
        );
    });
}

#[test]
fn snapshot_image_tile() {
    snap("image_tile", stat(&logo_rgb565a8::LOGO_RGB565A8), |h, i| {
        h.engine_mut().set_size(i, 150, 100);
        with(h, i, |w: &mut Image, cx| {
            w.set_inner_align(cx, ImageAlign::Tile);
            w.set_offset(cx, Point::new(10, 5));
        });
    });
}

#[test]
fn snapshot_image_cover() {
    snap("image_cover", stat(&logo_rgb565a8::LOGO_RGB565A8), |h, i| {
        h.engine_mut().set_size(i, 140, 60);
        with(h, i, |w: &mut Image, cx| w.set_inner_align(cx, ImageAlign::Cover));
    });
}
