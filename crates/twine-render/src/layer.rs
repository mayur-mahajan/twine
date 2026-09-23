//! Layers: render a group into an `Argb8888` buffer, then composite it with group opacity, a
//! blend mode and optionally a transform ([`LayerDsc`], [`LayerTransform`], [`Painter::layer`]).
//!
//! Colors are straight (not premultiplied) alpha. Untransformed layers larger than the layer
//! buffer are rendered in horizontal strips (the content closure is called once per strip).
//! A transformed layer must fit the buffer in one piece; otherwise it is drawn untransformed
//! (with a one-time warning). Layers nested inside a layer draw directly into the outer layer.

use core::mem::take;
use core::sync::atomic::AtomicBool;

use twine_core::{Angle, ColorFormat, Opa, Point, Rect, Scale, Transform};

use crate::blend::{BlendMode, Source};
use crate::dispatch::warn_once;
use crate::transform_blit::BlitDsc;
use crate::{DrawBuf, ImagePixels, Painter};

/// Rotation, scale and skew of a layer around a pivot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LayerTransform {
    /// Rotation (clockwise).
    pub rotation: Angle,
    /// Horizontal scale.
    pub scale_x: Scale,
    /// Vertical scale.
    pub scale_y: Scale,
    /// Horizontal skew.
    pub skew_x: Angle,
    /// Vertical skew.
    pub skew_y: Angle,
    /// Pivot relative to the layer area's top-left corner.
    pub pivot: Point,
    /// Bilinear filtering.
    pub antialias: bool,
}

impl Default for LayerTransform {
    fn default() -> Self {
        Self {
            rotation: Angle(0),
            scale_x: Scale::ONE,
            scale_y: Scale::ONE,
            skew_x: Angle(0),
            skew_y: Angle(0),
            pivot: Point::ZERO,
            antialias: true,
        }
    }
}

impl LayerTransform {
    /// The screen-space transform for a layer covering `area`: scale, then skew, then rotate,
    /// all around the pivot.
    ///
    /// ```
    /// use twine_core::{Angle, Point, Rect};
    /// use twine_render::LayerTransform;
    /// let t = LayerTransform { rotation: Angle::deg(90), pivot: Point::new(5, 5), ..Default::default() };
    /// let m = t.to_transform(Rect::from_xywh(10, 10, 10, 10));
    /// assert_eq!(m.map_point(Point::new(20, 15)), Point::new(15, 20));
    /// ```
    #[must_use]
    pub fn to_transform(&self, area: Rect) -> Transform {
        let p = Point::new(area.x0 + self.pivot.x, area.y0 + self.pivot.y);
        Transform::scale(self.scale_x.to_fx(), self.scale_y.to_fx())
            .then(Transform::skew(self.skew_x, self.skew_y))
            .then(Transform::rotate(self.rotation))
            .around(p)
    }

    /// Whether the transform changes nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.rotation.0.rem_euclid(3600) == 0
            && self.scale_x == Scale::ONE
            && self.scale_y == Scale::ONE
            && self.skew_x.0 == 0
            && self.skew_y.0 == 0
    }
}

/// Bounding box of `area` after the transform (plus one pixel for anti-aliasing): the extra
/// area the engine must invalidate for transformed nodes.
///
/// ```
/// use twine_core::{Angle, Point, Rect};
/// use twine_render::{LayerTransform, transformed_bounds};
/// let t = LayerTransform { rotation: Angle::deg(45), pivot: Point::new(50, 50), ..Default::default() };
/// let b = transformed_bounds(Rect::from_xywh(0, 0, 100, 100), &t);
/// assert!(b.contains_rect(&Rect::from_xywh(-20, -20, 140, 140)));
/// ```
#[must_use]
pub fn transformed_bounds(area: Rect, t: &LayerTransform) -> Rect {
    t.to_transform(area).map_rect_bounds(area).expand(1)
}

/// Parameters of [`Painter::layer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerDsc {
    /// Opacity of the whole group.
    pub opa: Opa,
    /// Blend mode of the group.
    pub blend_mode: BlendMode,
    /// Optional transform.
    pub transform: Option<LayerTransform>,
}

impl Default for LayerDsc {
    fn default() -> Self {
        Self {
            opa: Opa::COVER,
            blend_mode: BlendMode::Normal,
            transform: None,
        }
    }
}

static WARN_SMALL: AtomicBool = AtomicBool::new(false);
static WARN_TRANSFORM: AtomicBool = AtomicBool::new(false);

impl Painter<'_> {
    /// Renders `f` into an `Argb8888` layer and composites it with `dsc`. `f` draws in screen
    /// coordinates and may be called several times (once per strip) for large layers.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_render::{DrawBuf, LayerDsc, Painter, RenderCaches};
    ///
    /// let mut caches = RenderCaches::default();
    /// let mut data = vec![0u8; 4 * 4];
    /// let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 4, 4)).unwrap();
    /// let mut p = Painter::new(buf, &mut caches);
    /// let dsc = LayerDsc { opa: Opa(128), ..LayerDsc::default() };
    /// p.layer(Rect::from_xywh(0, 0, 4, 4), &dsc, |p| {
    ///     p.fill(Rect::from_xywh(0, 0, 4, 4), Color::WHITE, Opa::COVER);
    ///     p.fill(Rect::from_xywh(0, 0, 2, 4), Color::WHITE, Opa::COVER); // overlap: not brighter
    /// });
    /// assert_eq!(data[0], data[3]);
    /// ```
    pub fn layer(&mut self, area: Rect, dsc: &LayerDsc, mut f: impl FnMut(&mut Painter<'_>)) {
        if dsc.opa.is_transparent() || area.is_empty() {
            return;
        }
        if self.caches_mut().layer_buf.is_empty() {
            twine_core::debug!(target: "twine::render", "nested layer {} drawn directly", area);
            f(self);
            return;
        }
        if let Some(t) = dsc.transform.filter(|t| !t.is_identity()) {
            let need = area.width() as usize * area.height() as usize * 4;
            if need <= self.caches_mut().layer_buf.len() {
                self.layer_transformed(area, dsc, &t, f);
                return;
            }
            if warn_once(&WARN_TRANSFORM) {
                twine_core::warn!(
                    target: "twine::render",
                    "transformed layer {} needs {} bytes, layer buffer has {}; drawn untransformed",
                    area,
                    need,
                    self.caches_mut().layer_buf.len()
                );
            }
        }
        let Some(region) = area.intersection(&self.clip()) else {
            return;
        };
        let row_bytes = region.width() as usize * 4;
        let rows = (self.caches_mut().layer_buf.len() / row_bytes) as i32;
        if rows == 0 {
            if warn_once(&WARN_SMALL) {
                twine_core::warn!(
                    target: "twine::render",
                    "layer buffer too small for one row of {}; drawn without layer",
                    region
                );
            }
            f(self);
            return;
        }
        let mut lb = take(&mut self.caches_mut().layer_buf);
        let mut y0 = region.y0;
        while y0 < region.y1 {
            let strip = Rect::new(region.x0, y0, region.x1, (y0 + rows).min(region.y1));
            let bytes = row_bytes * strip.height() as usize;
            lb[..bytes].fill(0);
            let drawn = {
                let Ok(buf) = DrawBuf::new_packed(&mut lb[..bytes], ColorFormat::Argb8888, strip) else {
                    break;
                };
                let mut child = Painter::new(buf, self.caches_mut());
                f(&mut child);
                child.touched_area()
            };
            if let Some(t) = drawn.and_then(|t| t.intersection(&strip)) {
                for y in t.y0..t.y1 {
                    let o = (y - strip.y0) as usize * row_bytes + (t.x0 - strip.x0) as usize * 4;
                    let row = &lb[o..o + t.width() as usize * 4];
                    self.blend_masked_span(y, t.x0, t.x1, Source::Argb(row), None, dsc.opa, dsc.blend_mode);
                }
            }
            y0 = strip.y1;
        }
        self.caches_mut().layer_buf = lb;
    }

    fn layer_transformed(
        &mut self,
        area: Rect,
        dsc: &LayerDsc,
        t: &LayerTransform,
        mut f: impl FnMut(&mut Painter<'_>),
    ) {
        let (w, h) = (area.width() as usize, area.height() as usize);
        let mut lb = take(&mut self.caches_mut().layer_buf);
        lb[..w * h * 4].fill(0);
        let drawn = {
            match DrawBuf::new_packed(&mut lb[..w * h * 4], ColorFormat::Argb8888, area) {
                Ok(buf) => {
                    let mut child = Painter::new(buf, self.caches_mut());
                    f(&mut child);
                    child.touched_area().is_some()
                }
                Err(_) => false,
            }
        };
        if drawn {
            let img = ImagePixels::new(ColorFormat::Argb8888, w as u16, h as u16, &lb[..w * h * 4]);
            let transform = Transform::translate(
                twine_core::Fx::from_int(area.x0),
                twine_core::Fx::from_int(area.y0),
            )
            .then(t.to_transform(area));
            self.blit_transformed(
                &img,
                &BlitDsc {
                    transform,
                    opa: dsc.opa,
                    antialias: t.antialias,
                    blend_mode: dsc.blend_mode,
                    recolor: None,
                },
            );
        }
        self.caches_mut().layer_buf = lb;
    }
}
