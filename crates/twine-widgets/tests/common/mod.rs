//! Helpers shared by the widget tests.
#![allow(dead_code)] // each test binary uses a subset

use std::rc::Rc;

use twine_engine::{NodeId, Widget, WidgetCx};
use twine_testing::EngineHarness;
use twine_theme::DefaultTheme;

/// Light or dark default theme, for snapshots of both variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Light, Mode::Dark];

    pub fn suffix(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }
}

/// A `w × h` harness with the default theme in `mode`.
pub fn harness(w: u16, h: u16, mode: Mode) -> EngineHarness {
    let t = match mode {
        Mode::Light => DefaultTheme::light(),
        Mode::Dark => DefaultTheme::dark(),
    };
    EngineHarness::new(w, h).theme(Rc::new(t))
}

/// Calls a widget setter.
pub fn with<W: Widget, R>(
    h: &mut EngineHarness,
    id: NodeId,
    f: impl FnOnce(&mut W, &mut WidgetCx<'_>) -> R,
) -> R {
    h.engine_mut()
        .with_widget_mut(id, f)
        .expect("widget of that type")
}

/// The widget of `id`.
pub fn get<W: Widget>(h: &EngineHarness, id: NodeId) -> &W {
    h.engine().widget::<W>(id).expect("widget of that type")
}

/// A `w × h` opaque ARGB8888 image of one color (leaked for the `'static` source).
pub fn solid_image(w: u16, h: u16, c: twine_core::Color) -> twine_image::ImageSource {
    use twine_core::ColorFormat;
    use twine_image::{Image, ImageHeader};
    let px = [c.b, c.g, c.r, 0xFF];
    let data: Vec<u8> = px
        .iter()
        .copied()
        .cycle()
        .take(usize::from(w) * usize::from(h) * 4)
        .collect();
    let data: &'static [u8] = Box::leak(data.into_boxed_slice());
    let img: &'static Image = Box::leak(Box::new(Image::new_static(
        ImageHeader::new(ColorFormat::Argb8888, w, h),
        data,
    )));
    twine_image::ImageSource::Static(img)
}
