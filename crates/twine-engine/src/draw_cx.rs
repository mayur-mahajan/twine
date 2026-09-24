//! [`DrawCx`]: what a widget sees while drawing — the painter, its node's resolved styles as
//! draw descriptors, and [`DrawCx::draw_base`].

use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::{Opa, Rect};
use twine_image::{
    DecoderRegistry, FileSource, ImageCache, ImageContext, ImageHeaderCache, ImageSource, with_pixels,
};
use twine_render::{ArcDsc, ImageDsc, LineDsc, Painter, RectDsc, ShadowDsc};
use twine_style::{Part, PropId, State, StyleValue, TextAlign};
use twine_text::{GlyphCache, TextDsc};

use crate::{Engine, NodeId, RectStyle};

/// Caches besides the renderer's own that drawing needs (glyphs, images). Owned by the engine
/// and lent to every [`DrawCx`].
pub(crate) struct AuxRes {
    pub glyphs: GlyphCache,
    pub images: ImageCache,
    pub headers: ImageHeaderCache,
    pub registry: DecoderRegistry,
    /// Where `ImageSource::File` images are read from (`Engine::set_file_source`).
    pub fs: Option<alloc::boxed::Box<dyn FileSource>>,
}

impl core::fmt::Debug for AuxRes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuxRes")
            .field("images", &self.images.stats())
            .field("fs", &self.fs.is_some())
            .finish_non_exhaustive()
    }
}

/// Drawing context of one node: the [`Painter`] (clipped to what the node may draw), the
/// engine for style lookups, and helpers building draw descriptors from the node's styles.
///
/// All opacities in the descriptors are multiplied by the node's effective opacity (its `Opa`
/// style times its ancestors', LVGL `lv_obj_get_style_opa_recursive`).
pub struct DrawCx<'a, 'p> {
    painter: &'a mut Painter<'p>,
    engine: &'a Engine,
    aux: &'a mut AuxRes,
    node: NodeId,
    clip: Rect,
    opa: Opa,
}

impl core::fmt::Debug for DrawCx<'_, '_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrawCx")
            .field("node", &self.node)
            .field("clip", &self.clip)
            .field("opa", &self.opa)
            .finish_non_exhaustive()
    }
}

static WARN_IMAGE: AtomicBool = AtomicBool::new(false);

impl<'a, 'p> DrawCx<'a, 'p> {
    pub(crate) fn new(
        painter: &'a mut Painter<'p>,
        engine: &'a Engine,
        aux: &'a mut AuxRes,
        node: NodeId,
        opa: Opa,
    ) -> Self {
        let clip = painter.clip();
        Self {
            painter,
            engine,
            aux,
            node,
            clip,
            opa,
        }
    }

    /// The painter (its clip is [`clip`](Self::clip)).
    pub fn painter(&mut self) -> &mut Painter<'p> {
        self.painter
    }

    /// The node being drawn.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The engine (styles, tree).
    #[must_use]
    pub fn engine(&self) -> &'a Engine {
        self.engine
    }

    /// Absolute coordinates of the node.
    #[must_use]
    pub fn coords(&self) -> Rect {
        self.engine.coords(self.node)
    }

    /// The coordinates minus padding and border width of `Part::Main`.
    #[must_use]
    pub fn content_area(&self) -> Rect {
        self.engine.content_area(self.node)
    }

    /// What the node may draw into (the chunk, clipped by its ancestors).
    #[must_use]
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// The node's state.
    #[must_use]
    pub fn state(&self) -> State {
        self.engine
            .tree()
            .node(self.node)
            .map_or(State::DEFAULT, crate::Node::state)
    }

    /// The node's effective opacity (own `Opa` × ancestors').
    #[must_use]
    pub fn opa(&self) -> Opa {
        self.opa
    }

    /// A resolved style property of the node.
    #[must_use]
    pub fn style(&self, part: Part, prop: PropId) -> StyleValue {
        self.engine.style_prop(self.node, part, prop)
    }

    /// A resolved integer property.
    #[must_use]
    pub fn style_i32(&self, part: Part, prop: PropId) -> i32 {
        self.engine.style_i32(self.node, part, prop)
    }

    fn opa_of(&self, part: Part, prop: PropId) -> Opa {
        self.engine.style_opa(self.node, part, prop).mul(self.opa)
    }

    /// The rectangle style of `part` (LVGL `lv_obj_init_draw_rect_dsc`): background (color,
    /// gradient), border, outline, shadow and radius, from the resolved styles (`Main` reads
    /// the node's style cache), with the node's opacity and recolor applied.
    #[must_use]
    pub fn rect_dsc(&self, part: Part) -> RectStyle {
        self.engine.rect_dsc(self.node, part, self.opa)
    }

    /// The line style of `part` (`Line*` properties).
    #[must_use]
    pub fn line_dsc(&self, part: Part) -> LineDsc {
        self.engine.line_dsc(self.node, part, self.opa)
    }

    /// The arc style of `part` (`Arc*` properties; an arc image source is not resolved here).
    #[must_use]
    pub fn arc_dsc(&self, part: Part) -> ArcDsc<'static> {
        self.engine.arc_dsc(self.node, part, self.opa)
    }

    /// The text style of `part` (`Text*` properties, inherited ones included).
    #[must_use]
    pub fn text_dsc(&self, part: Part) -> TextDsc {
        self.engine.text_dsc(self.node, part, self.opa)
    }

    /// The rectangle style of `part` as if the node were in `state` (transitions ignored; see
    /// [`Engine::rect_dsc_for_state`]), with the node's opacity and recolor applied.
    #[must_use]
    pub fn rect_dsc_for_state(&self, part: Part, state: twine_style::State) -> RectStyle {
        self.engine.rect_dsc_for_state(self.node, part, state, self.opa)
    }

    /// The text style of `part` as if the node were in `state` (see
    /// [`Engine::text_dsc_for_state`]).
    #[must_use]
    pub fn text_dsc_for_state(&self, part: Part, state: twine_style::State) -> TextDsc {
        self.engine.text_dsc_for_state(self.node, part, state, self.opa)
    }

    /// The image style of `part` (`Image*` properties).
    #[must_use]
    pub fn image_dsc(&self, part: Part) -> ImageDsc<'static> {
        self.engine.image_dsc(self.node, part, self.opa)
    }

    /// The image context (caches, decoders, file source) for drawing image sources, with the
    /// painter: `f(painter, image_context)`.
    pub fn with_images<R>(&mut self, f: impl FnOnce(&mut Painter<'p>, &mut ImageContext<'_>) -> R) -> R {
        let AuxRes {
            images,
            headers,
            registry,
            fs,
            ..
        } = &mut *self.aux;
        let mut icx = ImageContext {
            cache: images,
            header_cache: headers,
            registry,
            fs: fs.as_deref_mut().map(|f| f as &mut dyn FileSource),
        };
        f(self.painter, &mut icx)
    }

    /// Runs `f` with the clip narrowed to `area` (intersected with the current clip).
    pub fn with_clip<R>(&mut self, area: Rect, f: impl FnOnce(&mut DrawCx<'_, 'p>) -> R) -> Option<R> {
        let clip = self.clip.intersection(&area)?;
        let (engine, node, opa) = (self.engine, self.node, self.opa);
        let aux = &mut *self.aux;
        Some(self.painter.with_clip(clip, |p| {
            let mut cx = DrawCx::new(p, engine, aux, node, opa);
            f(&mut cx)
        }))
    }

    /// Draws `text` in `area` with `dsc` (uses the engine's glyph cache).
    pub fn draw_text(&mut self, area: Rect, text: &str, dsc: &TextDsc) {
        twine_text::draw_text(self.painter, area, text, dsc, &mut self.aux.glyphs);
    }

    /// Draws the styled rectangle of `part` over the node's coordinates, like LVGL's
    /// `lv_obj` draw: shadow → background (color or gradient) → background image → border
    /// (unless `BorderPost`) → outline. Invisible elements are skipped.
    pub fn draw_base(&mut self, part: Part) {
        let area = if part == Part::Main {
            self.engine.draw_area(self.node)
        } else {
            self.coords()
        };
        self.draw_rect_style(area, part);
    }

    /// Like [`draw_base`](Self::draw_base) over `area` instead of the node's coordinates.
    pub fn draw_rect_style(&mut self, area: Rect, part: Part) {
        if area.is_empty() {
            return;
        }
        let rs = self.rect_dsc(part);
        let img = self.style(part, PropId::BgImageSrc).get::<&'static ImageSource>();
        let img_opa = self.opa_of(part, PropId::BgImageOpa);
        match img {
            Some(src) if !img_opa.is_transparent() => {
                // Background image between background and border: two passes.
                let d = rs.dsc();
                self.painter.rect(
                    area,
                    &RectDsc {
                        border_width: 0,
                        outline_width: 0,
                        ..d
                    },
                );
                self.draw_bg_image(area, part, src, img_opa);
                self.painter.rect(
                    area,
                    &RectDsc {
                        bg_opa: Opa::TRANSP,
                        shadow: ShadowDsc::default(),
                        ..d
                    },
                );
            }
            _ => self.painter.rect(area, &rs.dsc()),
        }
    }

    fn draw_bg_image(&mut self, area: Rect, part: Part, src: &'static ImageSource, opa: Opa) {
        let tiled = self.style(part, PropId::BgImageTiled).as_bool().unwrap_or(false);
        let (recolor, recolor_opa) = self.engine.bg_image_recolor(self.node, part);
        let dsc = ImageDsc {
            opa,
            recolor,
            recolor_opa,
            tile: tiled,
            ..ImageDsc::default()
        };
        if let ImageSource::Symbol(s) = src {
            let mut t = self.text_dsc(part);
            t.opa = t.opa.mul(opa);
            t.align = TextAlign::Center;
            let h = i32::from(t.font.line_height);
            let y = area.y0 + (area.height() - h) / 2;
            let a = Rect::new(area.x0, y, area.x1, y + h);
            self.draw_text(a, s, &t);
            return;
        }
        let AuxRes {
            images,
            headers,
            registry,
            fs,
            ..
        } = &mut *self.aux;
        let mut icx = ImageContext {
            cache: images,
            header_cache: headers,
            registry,
            fs: fs.as_deref_mut().map(|f| f as &mut dyn FileSource),
        };
        let painter = &mut *self.painter;
        let r = with_pixels(src, &mut icx, |px| {
            let img_area = if tiled {
                area
            } else {
                let (w, h) = (i32::from(px.w), i32::from(px.h));
                Rect::from_xywh(
                    area.x0 + (area.width() - w) / 2,
                    area.y0 + (area.height() - h) / 2,
                    w,
                    h,
                )
            };
            painter.with_clip(area, |p| p.image(img_area, px, &dsc));
        });
        if let Err(e) = r {
            if !WARN_IMAGE.load(Ordering::Relaxed) {
                WARN_IMAGE.store(true, Ordering::Relaxed);
                twine_core::warn!(target: "twine::engine", "background image not drawn: {:?}", e);
            }
        }
    }

    /// Draws the border of `part` when it has `BorderPost` (called by the engine after the
    /// children).
    pub fn draw_border_post(&mut self, part: Part) {
        if !self.style(part, PropId::BorderPost).as_bool().unwrap_or(false) {
            return;
        }
        let area = if part == Part::Main {
            self.engine.draw_area(self.node)
        } else {
            self.coords()
        };
        let rs = self.rect_dsc(part);
        self.painter.rect_border_post(area, &rs.dsc());
    }
}
