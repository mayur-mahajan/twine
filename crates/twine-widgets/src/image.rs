//! [`Image`]: an image source drawn with inner alignment, rotation, scaling and recoloring
//! (LVGL `lv_image`).

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::{Angle, Point, Rect, Scale, Size};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, MeasureCx, NodeId, OBJ_FLAGS,
    ObjFlags, Widget, WidgetClass, WidgetCx,
};
use twine_image::{ImageHeader, ImageSource, with_pixels};
use twine_render::{BlendMode, transformed_area};
use twine_style::{Part, PropId};
use twine_text::TextLayout;

use crate::log_set;

/// `AnimProp::Custom` id animating the scale ([`Image::set_scale`]). `AnimProp::Value`
/// animates the rotation (0.1°).
pub const ANIM_SCALE: u16 = 0;

/// The class of [`Image`]: `"image"`, part `Main`, the base object's flags without
/// `CLICKABLE`, plus `ADV_HITTEST` (LVGL `lv_image_constructor`).
pub static IMAGE_CLASS: WidgetClass = WidgetClass::new("image")
    .parts(&[twine_style::Part::Main])
    .default_flags(
        OBJ_FLAGS
            .difference(ObjFlags::CLICKABLE)
            .union(ObjFlags::ADV_HITTEST),
    );

/// How the image is placed inside the widget (LVGL `lv_image_align_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ImageAlign {
    /// The top-left corner (LVGL `LV_IMAGE_ALIGN_DEFAULT`).
    Default,
    /// Top left.
    TopLeft,
    /// Top middle.
    TopMid,
    /// Top right.
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom middle.
    BottomMid,
    /// Bottom right.
    BottomRight,
    /// Left middle.
    LeftMid,
    /// Right middle.
    RightMid,
    /// Centered (the widget's default, as in LVGL).
    #[default]
    Center,
    /// Marker: aligns below it keep the rotation and scale the user set.
    AutoTransform,
    /// Scaled (not keeping the aspect ratio) to fill the widget.
    Stretch,
    /// Repeated to fill the widget.
    Tile,
    /// Scaled keeping the aspect ratio to fit inside the widget.
    Contain,
    /// Scaled keeping the aspect ratio to cover the whole widget.
    Cover,
}

impl ImageAlign {
    /// Whether the widget sets the scale itself (LVGL `align > _LV_IMAGE_ALIGN_AUTO_TRANSFORM`).
    fn transforms(self) -> bool {
        matches!(
            self,
            ImageAlign::Stretch | ImageAlign::Tile | ImageAlign::Contain | ImageAlign::Cover
        )
    }
}

/// An image widget (LVGL `lv_image`): shows an [`ImageSource`] — a converted image in flash,
/// encoded bytes (QOI, PNG, …) decoded through the engine's decoders and cache, a file read
/// through [`Engine::set_file_source`], or a symbol drawn as text.
///
/// Its content size is the source's size (untransformed). Rotation and scale happen around
/// the pivot (default: the image center) with optional anti-aliasing; the widget's extra draw
/// size covers the transformed bounds. `ImageRecolor` / `ImageRecolorOpa` recolor the pixels
/// and `ImageOpa` fades them (styles of `Part::Main`). The theme gives images no styles
/// (LVGL).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{Image as Img, ImageHeader, ImageSource};
/// use twine_testing::EngineHarness;
/// use twine_widgets::image::{self, Image};
///
/// static PX: Img = Img::new_static(ImageHeader::new(ColorFormat::L8, 4, 2), &[255; 8]);
/// let mut h = EngineHarness::new(40, 20);
/// let screen = h.screen();
/// let i = image::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(i, |w: &mut Image, cx| w.set_src(cx, ImageSource::Static(&PX)));
/// h.run_until_idle();
/// assert_eq!(h.engine().coords(i).width(), 4);
/// ```
#[derive(Debug)]
pub struct Image {
    src: Option<ImageSource>,
    header: Option<ImageHeader>,
    /// Source size (the content size).
    size: Size,
    rotation: Angle,
    scale_x: Scale,
    scale_y: Scale,
    /// `None`: the image center.
    pivot: Option<Point>,
    antialias: bool,
    align: ImageAlign,
    offset: Point,
    blend: BlendMode,
}

impl Default for Image {
    fn default() -> Self {
        Self::new()
    }
}

static WARN_DRAW: AtomicBool = AtomicBool::new(false);

/// Whether two sources are the same (identity for flash data, equality for paths/symbols).
pub(crate) fn same_source(a: &ImageSource, b: &ImageSource) -> bool {
    match (a, b) {
        (ImageSource::Static(x), ImageSource::Static(y)) => core::ptr::eq(*x, *y),
        (ImageSource::Encoded(x), ImageSource::Encoded(y)) | (ImageSource::Svg(x), ImageSource::Svg(y)) => {
            core::ptr::eq(*x, *y)
        }
        (ImageSource::File(x), ImageSource::File(y)) => x == y,
        (ImageSource::Symbol(x), ImageSource::Symbol(y)) => x == y,
        _ => false,
    }
}

/// LVGL `lv_area_align` of a `size` box inside `base` for the inner alignments.
fn align_in(base: Rect, size: Size, align: ImageAlign, ofs: Point) -> Rect {
    let (bw, bh) = (base.width(), base.height());
    let (x, y) = match align {
        ImageAlign::TopMid => ((bw - size.w) / 2, 0),
        ImageAlign::TopRight => (bw - size.w, 0),
        ImageAlign::BottomLeft => (0, bh - size.h),
        ImageAlign::BottomMid => ((bw - size.w) / 2, bh - size.h),
        ImageAlign::BottomRight => (bw - size.w, bh - size.h),
        ImageAlign::LeftMid => (0, (bh - size.h) / 2),
        ImageAlign::RightMid => (bw - size.w, (bh - size.h) / 2),
        ImageAlign::Center => ((bw - size.w) / 2, (bh - size.h) / 2),
        _ => (0, 0),
    };
    Rect::from_xywh(base.x0 + x + ofs.x, base.y0 + y + ofs.y, size.w, size.h)
}

impl Image {
    /// An image without a source.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            src: None,
            header: None,
            size: Size::ZERO,
            rotation: Angle(0),
            scale_x: Scale::ONE,
            scale_y: Scale::ONE,
            pivot: None,
            antialias: true,
            align: ImageAlign::Center,
            offset: Point::ZERO,
            blend: BlendMode::Normal,
        }
    }

    /// The source.
    #[must_use]
    pub fn src(&self) -> Option<&ImageSource> {
        self.src.as_ref()
    }

    /// The source's header (`None` without a source, for symbols and unreadable sources).
    #[must_use]
    pub fn header(&self) -> Option<ImageHeader> {
        self.header
    }

    /// The source's size (0×0 without a readable source).
    #[must_use]
    pub fn src_size(&self) -> Size {
        self.size
    }

    /// The rotation.
    #[must_use]
    pub fn rotation(&self) -> Angle {
        self.rotation
    }

    /// The horizontal scale.
    #[must_use]
    pub fn scale_x(&self) -> Scale {
        self.scale_x
    }

    /// The vertical scale.
    #[must_use]
    pub fn scale_y(&self) -> Scale {
        self.scale_y
    }

    /// The pivot relative to the image's top-left corner.
    #[must_use]
    pub fn pivot(&self) -> Point {
        self.pivot.unwrap_or(Point::new(self.size.w / 2, self.size.h / 2))
    }

    /// Whether transformed drawing is anti-aliased.
    #[must_use]
    pub fn antialias(&self) -> bool {
        self.antialias
    }

    /// The inner alignment.
    #[must_use]
    pub fn inner_align(&self) -> ImageAlign {
        self.align
    }

    /// The offset of the image inside the widget.
    #[must_use]
    pub fn offset(&self) -> Point {
        self.offset
    }

    /// The blend mode.
    #[must_use]
    pub fn blend_mode(&self) -> BlendMode {
        self.blend
    }

    /// Whether a rotation or scale is in effect.
    fn transformed(&self) -> bool {
        self.rotation.normalized().0 != 0 || self.scale_x != Scale::ONE || self.scale_y != Scale::ONE
    }

    /// The transformed bounds of the widget's area (LVGL `lv_image_buf_get_transformed_area`
    /// of the object size), relative to the widget's top-left corner.
    fn transformed_bounds(&self, w: i32, h: i32) -> Rect {
        transformed_area(
            Rect::from_xywh(0, 0, w, h),
            self.rotation,
            self.scale_x,
            self.scale_y,
            self.pivot(),
        )
    }

    /// The extra draw size for the transform (LVGL `LV_EVENT_REFR_EXT_DRAW_SIZE`).
    fn ext_for(&self, coords: Rect) -> u16 {
        // The self-scaling alignments draw inside the widget only.
        if !self.transformed() || self.align.transforms() {
            return 0;
        }
        let (w, h) = (coords.width(), coords.height());
        let a = self.transformed_bounds(w, h);
        let s = (-a.x0).max(-a.y0).max(a.x1 - w).max(a.y1 - h).max(0);
        u16::try_from(s).unwrap_or(u16::MAX)
    }

    /// Invalidates the transformed area (plus one pixel, LVGL `scale_update`).
    fn invalidate_transformed(&self, cx: &mut WidgetCx<'_>) {
        let c = cx.coords();
        let a = self.transformed_bounds(c.width(), c.height());
        let r = Rect::new(c.x0 + a.x0 - 1, c.y0 + a.y0 - 1, c.x0 + a.x1 + 1, c.y0 + a.y1 + 1);
        cx.invalidate_area(r);
    }

    /// Changes a transform parameter with `f`: invalidates the old transformed area,
    /// updates the extra draw size, invalidates the new area (LVGL `lv_image_set_rotation`).
    fn update_transform(&mut self, cx: &mut WidgetCx<'_>, f: impl FnOnce(&mut Self)) {
        self.invalidate_transformed(cx);
        f(self);
        let ext = self.ext_for(cx.coords());
        cx.refresh_ext_draw_with(ext);
        self.invalidate_transformed(cx);
    }

    /// Sets the source. Idempotent (compared by identity: the same static image or byte
    /// slice, an equal path or symbol). The header is read right away (probing encoded data
    /// and files through the header cache) to size the widget; an unreadable source logs
    /// `warn!` and leaves an empty 0×0 image.
    pub fn set_src(&mut self, cx: &mut WidgetCx<'_>, src: ImageSource) {
        if self.src.as_ref().is_some_and(|s| same_source(s, &src)) {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "src");
        cx.invalidate_for("image.set_src");
        self.load(cx, src);
    }

    /// Reads the header of `src` and stores it (also used to re-measure symbols).
    fn load(&mut self, cx: &mut WidgetCx<'_>, src: ImageSource) {
        let (header, size) = match &src {
            ImageSource::Symbol(s) => {
                let d = cx.text_dsc(Part::Main);
                let mut l = TextLayout::new(s, d.font);
                l.letter_space = d.letter_space;
                l.line_space = d.line_space;
                (None, l.measure())
            }
            other => match cx.engine_mut().image_header(other) {
                Ok(h) => (Some(h), Size::new(i32::from(h.w), i32::from(h.h))),
                Err(e) => {
                    twine_core::warn!(target: "twine::image", "image: cannot read {}: {:?}", other, e);
                    self.src = None;
                    self.header = None;
                    self.size_changed(cx, Size::ZERO);
                    return;
                }
            },
        };
        self.src = Some(src);
        self.header = header;
        self.size_changed(cx, size);
        if self.align.transforms() {
            self.update_align(cx);
        }
    }

    fn size_changed(&mut self, cx: &mut WidgetCx<'_>, size: Size) {
        if self.size != size {
            self.size = size;
            cx.mark_layout();
        }
        let ext = self.ext_for(cx.coords());
        cx.refresh_ext_draw_with(ext);
        cx.invalidate_for("image");
    }

    /// Sets the rotation around the pivot (normalized to 0…360°). Ignored (0) while the inner
    /// alignment scales the image itself. Idempotent.
    pub fn set_rotation(&mut self, cx: &mut WidgetCx<'_>, angle: Angle) {
        let angle = if self.align.transforms() {
            Angle(0)
        } else {
            angle.normalized()
        };
        if angle == self.rotation {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "rotation");
        self.update_transform(cx, |s| s.rotation = angle);
    }

    /// Sets both scales (256 = 1×; 0 counts as 1/256). Ignored while the inner alignment
    /// scales the image itself. Idempotent.
    pub fn set_scale(&mut self, cx: &mut WidgetCx<'_>, s: Scale) {
        if self.align.transforms() {
            return;
        }
        self.set_scale_xy(cx, s, s);
    }

    /// Sets the horizontal scale. Idempotent.
    pub fn set_scale_x(&mut self, cx: &mut WidgetCx<'_>, s: Scale) {
        if self.align.transforms() {
            return;
        }
        self.set_scale_xy(cx, s, self.scale_y);
    }

    /// Sets the vertical scale. Idempotent.
    pub fn set_scale_y(&mut self, cx: &mut WidgetCx<'_>, s: Scale) {
        if self.align.transforms() {
            return;
        }
        self.set_scale_xy(cx, self.scale_x, s);
    }

    fn set_scale_xy(&mut self, cx: &mut WidgetCx<'_>, x: Scale, y: Scale) {
        let (x, y) = (Scale(x.0.max(1)), Scale(y.0.max(1)));
        if (x, y) == (self.scale_x, self.scale_y) {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "scale");
        self.update_transform(cx, |s| {
            s.scale_x = x;
            s.scale_y = y;
        });
    }

    /// Sets the rotation / scale center relative to the image's top-left corner. Idempotent.
    pub fn set_pivot(&mut self, cx: &mut WidgetCx<'_>, p: Point) {
        if self.pivot == Some(p) {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "pivot");
        self.update_transform(cx, |s| s.pivot = Some(p));
    }

    /// Bilinear filtering of transformed images. Idempotent.
    pub fn set_antialias(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.antialias == on {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "antialias");
        self.antialias = on;
        cx.invalidate_for("image.set_antialias");
    }

    /// Sets the inner alignment. `Stretch`, `Contain`, `Cover` and `Tile` set the scale
    /// (and reset rotation and pivot) themselves; leaving them resets the scale. Idempotent.
    pub fn set_inner_align(&mut self, cx: &mut WidgetCx<'_>, align: ImageAlign) {
        if self.align == align {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "inner_align");
        if self.align.transforms() {
            self.align = align;
            self.set_scale_xy(cx, Scale::ONE, Scale::ONE);
        }
        self.align = align;
        self.update_align(cx);
        cx.invalidate_for("image.set_inner_align");
    }

    /// Sets the offset of the image inside the widget. Idempotent.
    pub fn set_offset(&mut self, cx: &mut WidgetCx<'_>, p: Point) {
        if self.offset == p {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "offset");
        self.offset = p;
        cx.invalidate_for("image.set_offset");
    }

    /// Sets the blend mode. Idempotent.
    pub fn set_blend_mode(&mut self, cx: &mut WidgetCx<'_>, m: BlendMode) {
        if self.blend == m {
            return;
        }
        log_set(IMAGE_CLASS.name, cx.node(), "blend_mode");
        self.blend = m;
        cx.invalidate_for("image.set_blend_mode");
    }

    /// LVGL `update_align`: the scale of the self-scaling alignments.
    fn update_align(&mut self, cx: &mut WidgetCx<'_>) {
        if !self.align.transforms() {
            return;
        }
        let c = cx.coords();
        let (iw, ih) = (self.size.w, self.size.h);
        let (sx, sy) = if iw > 0 && ih > 0 {
            let sx = c.width() * 256 / iw;
            let sy = c.height() * 256 / ih;
            match self.align {
                ImageAlign::Stretch => (sx, sy),
                ImageAlign::Contain => (sx.min(sy), sx.min(sy)),
                ImageAlign::Cover => (sx.max(sy), sx.max(sy)),
                _ => (256, 256),
            }
        } else {
            (256, 256)
        };
        let clamp = |v: i32| Scale(u16::try_from(v.clamp(1, i32::from(u16::MAX))).unwrap_or(u16::MAX));
        let (sx, sy) = (clamp(sx), clamp(sy));
        if (sx, sy, self.rotation, self.pivot) != (self.scale_x, self.scale_y, Angle(0), Some(Point::ZERO)) {
            self.update_transform(cx, |s| {
                s.rotation = Angle(0);
                s.pivot = Some(Point::ZERO);
                s.scale_x = sx;
                s.scale_y = sy;
            });
        }
    }

    /// Whether the drawn image fully covers `area` with opaque pixels.
    fn image_covers(&self, cx: &MeasureCx<'_>, area: Rect) -> bool {
        let Some(h) = self.header else {
            return false;
        };
        if h.format.has_alpha()
            || self.rotation.normalized().0 != 0
            || self.blend != BlendMode::Normal
            || !cx
                .style(Part::Main, PropId::ImageOpa)
                .as_opa()
                .is_some_and(twine_core::Opa::is_cover)
            || !cx.engine().opa_recursive(cx.node()).is_cover()
        {
            return false;
        }
        let c = cx.coords();
        let img = if self.scale_x == Scale::ONE && self.scale_y == Scale::ONE {
            match self.align {
                ImageAlign::Tile => c,
                ImageAlign::Stretch | ImageAlign::AutoTransform => {
                    Rect::from_xywh(c.x0, c.y0, self.size.w, self.size.h)
                }
                a => align_in(c, self.size, a, self.offset),
            }
        } else {
            let t = self.transformed_bounds(c.width(), c.height());
            Rect::new(c.x0 + t.x0, c.y0 + t.y0, c.x0 + t.x1, c.y0 + t.y1)
        };
        match img.intersection(&c) {
            Some(i) => i.contains_rect(&area),
            None => false,
        }
    }
}

/// Creates an image without a source as the last child of `parent` (LVGL `lv_image_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Image::new()))
}

impl Widget for Image {
    fn class(&self) -> &'static WidgetClass {
        &IMAGE_CLASS
    }

    fn content_size(&self, _cx: &MeasureCx<'_>) -> Size {
        self.size
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        self.ext_for(cx.coords())
    }

    fn covers(&self, cx: &MeasureCx<'_>, area: Rect) -> bool {
        twine_engine::default_covers(cx, area) || self.image_covers(cx, area)
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        match ev.code {
            EventCode::StyleChanged => {
                let mut wcx = cx.widget_cx();
                if let Some(ImageSource::Symbol(s)) = self.src {
                    // The font may have changed (LVGL sets the symbol source again).
                    self.load(&mut wcx, ImageSource::Symbol(s));
                } else {
                    let ext = self.ext_for(wcx.coords());
                    wcx.refresh_ext_draw_with(ext);
                }
            }
            EventCode::SizeChanged => {
                let mut wcx = cx.widget_cx();
                if self.align.transforms() {
                    self.update_align(&mut wcx);
                }
                let ext = self.ext_for(wcx.coords());
                if wcx.refresh_ext_draw_with(ext) {
                    wcx.invalidate();
                }
            }
            _ => {}
        }
        EventResult::Continue
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let Some(src) = &self.src else {
            return;
        };
        if self.size.w == 0 || self.size.h == 0 {
            return;
        }
        let c = cx.coords();
        if let ImageSource::Symbol(s) = src {
            let dsc = cx.text_dsc(Part::Main);
            let area = if self.align.transforms()
                || (self.size.w == c.width() && self.size.h == c.height() && self.offset == Point::ZERO)
            {
                c
            } else {
                align_in(c, Size::new(self.size.w, self.size.h), self.align, self.offset)
            };
            cx.draw_text(area, s, &dsc);
            return;
        }
        let mut dsc = cx.image_dsc(Part::Main);
        dsc.angle = self.rotation;
        dsc.scale_x = self.scale_x;
        dsc.scale_y = self.scale_y;
        dsc.pivot = self.pivot();
        dsc.antialias = self.antialias;
        dsc.blend_mode = self.blend;
        dsc.clip_radius = cx.style_i32(Part::Main, PropId::Radius);
        let size = self.size;
        let (area, clip) = match self.align {
            ImageAlign::Contain | ImageAlign::Cover => {
                let s = i32::from(self.scale_x.0);
                let ox = (c.width() - size.w * s / 256) / 2 + self.offset.x;
                let oy = (c.height() - size.h * s / 256) / 2 + self.offset.y;
                // Clipped to the widget (the scaled image of `Cover` is larger).
                let Some(clip) = cx.clip().intersection(&c) else {
                    return;
                };
                (Rect::from_xywh(c.x0 + ox, c.y0 + oy, size.w, size.h), clip)
            }
            ImageAlign::Tile => {
                // Start one image before the clip so the tiles line up with the offset.
                let Some(clip) = cx.clip().intersection(&c) else {
                    return;
                };
                let (x0, y0) = (c.x0 + self.offset.x, c.y0 + self.offset.y);
                let sx = x0 + (clip.x0 - x0).div_euclid(size.w) * size.w;
                let sy = y0 + (clip.y0 - y0).div_euclid(size.h) * size.h;
                dsc.tile = true;
                (Rect::new(sx, sy, clip.x1, clip.y1), clip)
            }
            ImageAlign::Stretch | ImageAlign::AutoTransform => (
                Rect::from_xywh(c.x0 + self.offset.x, c.y0 + self.offset.y, size.w, size.h),
                cx.clip(),
            ),
            a => (align_in(c, size, a, self.offset), cx.clip()),
        };
        let r = cx.with_clip(clip, |cx| {
            cx.with_images(|painter, icx| with_pixels(src, icx, |px| painter.image(area, px, &dsc)))
        });
        if let Some(Err(e)) = r {
            if !WARN_DRAW.load(Ordering::Relaxed) {
                WARN_DRAW.store(true, Ordering::Relaxed);
                twine_core::warn!(target: "twine::image", "image not drawn: {:?}", e);
            }
        }
    }

    /// `AnimProp::Value` animates the rotation (0.1°).
    fn anim_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        self.set_rotation(cx, Angle(v));
    }

    /// `AnimProp::Custom(ANIM_SCALE)` animates the scale (256 = 1×).
    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id == ANIM_SCALE {
            let s = Scale(u16::try_from(v.clamp(1, i32::from(u16::MAX))).unwrap_or(u16::MAX));
            self.set_scale(cx, s);
        }
    }
}
