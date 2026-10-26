//! The image widget on a scaled display (R7.S04e): a plain image keeps its logical size (drawn
//! zoomed), a `ScaledImage` draws the rendering made for the display at its own pixels.
//! Runs with and without the engine's `display-scale` feature.

use twine_core::{ColorFormat, DisplayScale};
use twine_image::{Image as Img, ImageHeader, ImageSource, ScaledImage};
use twine_testing::EngineHarness;
use twine_widgets::image::{self, Image};

static DOT: Img = Img::new_static(ImageHeader::new(ColorFormat::L8, 4, 2), &[255; 8]);
static DOT_2X: Img = Img::new_static(ImageHeader::new(ColorFormat::L8, 8, 4), &[128; 32]);
static DOTS: ScaledImage = ScaledImage::new(&DOT, &[(DisplayScale::X2, &DOT_2X)]);

/// The device size of an image of `src` on a 2× panel, and the scale the engine uses.
fn at_2x(src: ImageSource) -> ((i32, i32), Option<ImageHeader>, i32) {
    let mut h = EngineHarness::new(80, 40).display_scale(DisplayScale::X2);
    let screen = h.screen();
    let i = image::create(h.engine_mut(), screen).unwrap();
    h.engine_mut()
        .with_widget_mut(i, |w: &mut Image, cx| w.set_src(cx, src));
    h.run_until_idle();
    let c = h.engine().coords(i);
    let k = i32::from(h.engine().display_scale(h.display()).raw_256() / 256);
    let header = h.engine().widget::<Image>(i).unwrap().header();
    ((c.width(), c.height()), header, k)
}

#[test]
fn plain_images_keep_their_logical_size() {
    let (size, header, k) = at_2x(ImageSource::Static(&DOT));
    assert_eq!(size, (4 * k, 2 * k));
    assert_eq!(header.map(|h| h.w), Some(4)); // drawn zoomed from its own pixels
}

#[test]
fn scaled_images_draw_the_rendering_for_the_display() {
    let (size, header, k) = at_2x(ImageSource::Scaled(&DOTS));
    assert_eq!(size, (4 * k, 2 * k));
    assert_eq!(header.map(|h| h.w), Some(4 * u16::try_from(k).unwrap()));
}

/// Whether the device pixel `(x, y)` is lit (the image is white on a black screen).
fn lit(h: &EngineHarness, x: u32, y: u32) -> bool {
    h.pixel(x, y).r > 128
}

#[test]
fn a_zoomed_image_covers_exactly_its_device_rectangle() {
    use twine_style::{Selector, StyleProp};
    let mut h = EngineHarness::new(40, 20)
        .no_theme()
        .display_scale(DisplayScale::X2);
    let screen = h.screen();
    let e = h.engine_mut();
    e.set_local_prop(
        screen,
        Selector::MAIN,
        StyleProp::BgColor(twine_core::Color::BLACK.into()),
    );
    e.set_local_prop(
        screen,
        Selector::MAIN,
        StyleProp::BgOpacity(twine_core::Opa::COVER.into()),
    );
    let i = image::create(e, screen).unwrap();
    e.set_pos(i, 3, 2);
    e.with_widget_mut(i, |w: &mut Image, cx| w.set_src(cx, ImageSource::Static(&DOT)));
    h.run_until_idle();
    let k = u32::from(h.engine().display_scale(h.display()).raw_256() / 256);
    let (x0, y0, w, hh) = (3 * k, 2 * k, 4 * k, 2 * k);
    for y in 0..20 {
        for x in 0..40 {
            let inside = (x0..x0 + w).contains(&x) && (y0..y0 + hh).contains(&y);
            assert_eq!(lit(&h, x, y), inside, "pixel ({x}, {y}) at {k}×");
        }
    }
}
