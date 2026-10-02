//! SVG sources of the image widget (`ImageSource::Svg`): drawn as vectors with the image
//! transform (feature `svg`), parsed once, optionally from a cached raster; a placeholder
//! without the feature. Run with and without `--features svg` (`cargo xtask ci` does both).

mod common;

use common::{Mode, harness, with};
use twine_engine::{EngineConfig, NodeId};
use twine_image::ImageSource;
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_widgets::image::{self, Image};

static STAR: &[u8] = include_bytes!("../../../assets/images/svg/star.svg");
#[cfg(feature = "svg")]
static BADGE: &[u8] = include_bytes!("../../../assets/images/svg/badge.svg");

fn scene(
    src: &'static [u8],
    f: impl FnOnce(&mut Image, &mut twine_engine::WidgetCx<'_>),
) -> (EngineHarness, NodeId) {
    let mut h = harness(120, 120, Mode::Light).config(EngineConfig {
        image_cache_bytes: 64 * 1024,
        ..EngineConfig::default()
    });
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(i, Align::Center, 0, 0);
    with(&mut h, i, |w: &mut Image, cx| {
        w.set_src(cx, ImageSource::Svg(src));
        f(w, cx);
    });
    h.run_until_idle();
    (h, i)
}

#[cfg(feature = "svg")]
mod with_svg {
    use twine_core::{Angle, Scale};

    use super::*;

    #[test]
    fn svg_size_from_document() {
        let (h, i) = scene(STAR, |_, _| {});
        let c = h.engine().coords(i);
        assert_eq!((c.width(), c.height()), (48, 48));
    }

    #[test]
    fn svg_image_snapshot() {
        let (mut h, _) = scene(BADGE, |_, _| {});
        h.assert_snapshot("svg_image");
    }

    #[test]
    fn svg_scaled_2x_crisp_snapshot() {
        let (mut h, _) = scene(STAR, |w, cx| w.set_scale(cx, Scale::from_raw_256(512)));
        h.assert_snapshot("svg_scaled_2x");
    }

    #[test]
    fn svg_rotated_snapshot() {
        let (mut h, _) = scene(STAR, |w, cx| w.set_rotation(cx, Angle::deci_deg(300)));
        h.assert_snapshot("svg_rotated_30");
    }

    /// The raster is composited in straight alpha over a transparent buffer first, so
    /// anti-aliased edge pixels may differ from direct drawing by one step of the display
    /// format (one RGB565 level: up to 9 in red and blue, 5 in green once expanded to 8 bits);
    /// every other pixel is identical.
    #[test]
    fn svg_cached_raster_matches_direct() {
        let (h1, _) = scene(BADGE, |_, _| {});
        let (h2, i) = scene(BADGE, |w, cx| w.set_svg_cache(cx, true));
        assert!(common::get::<Image>(&h2, i).svg_cache());
        let (mut same, mut edge) = (0, 0);
        for y in 0..120 {
            for x in 0..120 {
                let (a, b) = (h1.pixel(x, y), h2.pixel(x, y));
                let d = |p: u8, q: u8| (i32::from(p) - i32::from(q)).abs();
                if a == b {
                    same += 1;
                } else {
                    assert!(
                        d(a.r, b.r) <= 9 && d(a.g, b.g) <= 5 && d(a.b, b.b) <= 9,
                        "({x},{y}) {a:?} vs {b:?}"
                    );
                    edge += 1;
                }
            }
        }
        assert!(edge * 50 < same, "{edge} edge pixels differ");
    }

    #[test]
    fn svg_parsed_once() {
        let (mut h, i) = scene(STAR, |_, _| {});
        let screen = h.screen();
        let j = image::create(h.engine_mut(), screen).unwrap();
        with(&mut h, j, |w: &mut Image, cx| {
            w.set_src(cx, ImageSource::Svg(STAR));
        });
        for id in [i, j, i] {
            h.engine_mut()
                .invalidate(id, twine_engine::InvalidateReason::WidgetSetter("test"));
            h.run_until_idle();
        }
        assert_eq!(h.engine().svg_parse_count(), 1);
    }

    #[test]
    fn svg_recolor_overrides_paints() {
        let (h, i) = scene(STAR, |_, _| {});
        let c = h.engine().coords(i);
        let (x, y) = ((c.x0 + 24) as u32, (c.y0 + 26) as u32);
        let before = h.pixel(x, y);
        let (mut h, i) = scene(STAR, |_, _| {});
        h.engine_mut().set_local_prop(
            i,
            twine_style::Selector::MAIN,
            twine_style::StyleProp::ImageRecolor(twine_core::Color::hex(0x0000_00FF).into()),
        );
        h.engine_mut().set_local_prop(
            i,
            twine_style::Selector::MAIN,
            twine_style::StyleProp::ImageRecolorOpacity(twine_core::Opa::COVER.into()),
        );
        h.run_until_idle();
        let after = h.pixel(x, y);
        assert_ne!(before, after);
        assert_eq!(after, twine_core::Color::hex(0x0000_00FF));
    }
}

#[cfg(not(feature = "svg"))]
#[test]
fn svg_without_feature_placeholder() {
    let ((h, i), logs) = twine_testing::capture_logs(|| scene(STAR, |_, _| {}));
    let c = h.engine().coords(i);
    assert_eq!(
        (c.width(), c.height()),
        (image::MISSING_PLACEHOLDER_SIZE, image::MISSING_PLACEHOLDER_SIZE)
    );
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("placeholder")),
        "{logs:?}"
    );
}
