//! [`Tile`]: a labelled box for the layout examples. Its content size is the size of its label
//! (so `Length::Content` sizes and grid `Content` tracks follow the text).

use twine_core::{Color, Opa, Rect, Size};
use twine_engine::{DrawCx, Engine, MeasureCx, NodeId, Widget, WidgetClass};
use twine_style::{Part, Selector, StyleProp};
use twine_text::TextDsc;

/// Class of [`Tile`].
pub static TILE_CLASS: WidgetClass = WidgetClass::new("tile");

/// A box drawing its label centered with the engine's default font.
#[derive(Debug, Clone)]
pub struct Tile {
    /// The label.
    pub label: String,
}

impl Tile {
    fn text_size(&self, engine: &Engine) -> Size {
        engine.config().default_font.map_or(Size::ZERO, |f| {
            TextDsc::new(f).layout(&self.label, i32::MAX).measure()
        })
    }
}

impl Widget for Tile {
    fn class(&self) -> &'static WidgetClass {
        &TILE_CLASS
    }

    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        self.text_size(cx.engine())
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let Some(font) = cx.engine().config().default_font else {
            return;
        };
        let s = self.text_size(cx.engine());
        let c = cx.coords();
        let (x, y) = (c.x0 + (c.width() - s.w) / 2, c.y0 + (c.height() - s.h) / 2);
        let mut dsc = TextDsc::new(font);
        dsc.color = Color::WHITE;
        cx.draw_text(Rect::from_xywh(x, y, s.w, s.h), &self.label, &dsc);
    }

    fn text(&self) -> Option<&str> {
        Some(&self.label)
    }
}

/// Creates a [`Tile`] under `parent` with a colored, rounded background and 4 px padding.
pub fn tile(e: &mut Engine, parent: NodeId, label: impl Into<String>, color: Color) -> NodeId {
    let t = e
        .create(parent, Box::new(Tile { label: label.into() }))
        .expect("parent exists");
    for p in [
        StyleProp::BgColor(color),
        StyleProp::BgOpa(Opa::COVER),
        StyleProp::Radius(4),
        StyleProp::PadLeft(4),
        StyleProp::PadRight(4),
        StyleProp::PadTop(4),
        StyleProp::PadBottom(4),
    ] {
        e.set_local_prop(t, Selector::MAIN, p);
    }
    t
}

/// A palette of distinct tile colors.
pub const PALETTE: [u32; 8] = [
    0xE5_39_35, 0xFB_8C_00, 0x43_A0_47, 0x1E_88_E5, 0x8E_24_AA, 0x00_89_7B, 0x6D_4C_41, 0x3F_51_B5,
];
