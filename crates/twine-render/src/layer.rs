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
use crate::image::read::TexelOps;
use crate::num::{advance, span};
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
            rotation: Angle::deci_deg(0),
            scale_x: Scale::ONE,
            scale_y: Scale::ONE,
            skew_x: Angle::deci_deg(0),
            skew_y: Angle::deci_deg(0),
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
        let p = Point::new(
            area.x0.saturating_add(self.pivot.x),
            area.y0.saturating_add(self.pivot.y),
        );
        Transform::scale(self.scale_x.to_fx(), self.scale_y.to_fx())
            .then(Transform::skew(self.skew_x, self.skew_y))
            .then(Transform::rotate(self.rotation))
            .around(p)
    }

    /// Whether the transform changes nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.rotation.as_deci_deg().rem_euclid(3600) == 0
            && self.scale_x == Scale::ONE
            && self.scale_y == Scale::ONE
            && self.skew_x.as_deci_deg() == 0
            && self.skew_y.as_deci_deg() == 0
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
    /// A layer inside a layer (the layer buffer is in use) is drawn directly, without a
    /// layer of its own. That case is handled inline: nested layers recurse through `f`
    /// (the engine's draw traversal), so only this small check stays on the stack per
    /// nesting level, not the strip and transform set-up of the outermost layer.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_render::{DrawBuf, LayerDsc, Painter, RenderCaches};
    ///
    /// let mut caches = RenderCaches::default();
    /// let mut data = vec![0u8; 4 * 4];
    /// let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 4, 4)).unwrap();
    /// let mut p = Painter::new(buf, &mut caches);
    /// let dsc = LayerDsc { opa: Opa::from_raw(128), ..LayerDsc::default() };
    /// p.layer(Rect::from_xywh(0, 0, 4, 4), &dsc, |p| {
    ///     p.fill(Rect::from_xywh(0, 0, 4, 4), Color::WHITE, Opa::COVER);
    ///     p.fill(Rect::from_xywh(0, 0, 2, 4), Color::WHITE, Opa::COVER); // overlap: not brighter
    /// });
    /// drop(p); // the buffer is read after the painter (and any accelerator) is done
    /// assert_eq!(data[0], data[3]);
    /// ```
    #[inline]
    pub fn layer(&mut self, area: Rect, dsc: &LayerDsc, mut f: impl FnMut(&mut Painter<'_>)) {
        if dsc.opa.is_transparent() || area.is_empty() {
            return;
        }
        if self.caches_mut().layer_buf.is_empty() {
            twine_core::debug!(target: "twine::render", "nested layer {} drawn directly", area);
            f(self);
            return;
        }
        self.layer_with_buf(area, dsc, f);
    }

    /// [`layer`](Self::layer) with the layer buffer available: renders `f` strip by strip
    /// (or once, transformed) into the buffer and composites it.
    #[inline(never)]
    fn layer_with_buf(&mut self, area: Rect, dsc: &LayerDsc, mut f: impl FnMut(&mut Painter<'_>)) {
        if let Some(t) = dsc.transform.filter(|t| !t.is_identity()) {
            let need = span(area.x0, area.x1)
                .saturating_mul(span(area.y0, area.y1))
                .saturating_mul(4);
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
        let row_bytes = span(region.x0, region.x1).saturating_mul(4);
        // `region` is a non-empty intersection, so `row_bytes ≥ 4`: the fallback is never
        // taken.
        let rows = self
            .caches_mut()
            .layer_buf
            .len()
            .checked_div(row_bytes)
            .unwrap_or(0);
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
            let strip = Rect::new(region.x0, y0, region.x1, advance(y0, rows, region.y1));
            // SAFE-ARITH: at most `rows` rows of `row_bytes`, which fit the layer buffer
            // (`rows = len / row_bytes`).
            let bytes = row_bytes.wrapping_mul(span(strip.y0, strip.y1));
            // SAFE-INDEX: as above; the fallback is never taken.
            let Some(strip_buf) = lb.get_mut(..bytes) else {
                break;
            };
            strip_buf.fill(0);
            let drawn = {
                let Ok(buf) = DrawBuf::new_packed(strip_buf, ColorFormat::Argb8888, strip) else {
                    break;
                };
                let mut child = Painter::new(buf, self.caches_mut());
                #[cfg(feature = "display-integrity")]
                {
                    child.in_layer = true;
                }
                f(&mut child);
                child.touched_area()
            };
            if let Some(t) = drawn.and_then(|t| t.intersection(&strip)) {
                // SAFE-ARITH: `t` lies inside the strip, so the byte columns lie inside a row.
                let cols = span(strip.x0, t.x0).wrapping_mul(4)..span(strip.x0, t.x1).wrapping_mul(4);
                let rows = lb.chunks_exact(row_bytes.max(1)).skip(span(strip.y0, t.y0));
                for (y, row) in (t.y0..t.y1).zip(rows) {
                    // SAFE-INDEX: `cols` lies within the row; the fallback is never taken.
                    let row = row.get(cols.clone()).unwrap_or_default();
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
        let (w, h) = (span(area.x0, area.x1), span(area.y0, area.y1));
        let mut lb = take(&mut self.caches_mut().layer_buf);
        // The caller checked that `w · h · 4` bytes fit the layer buffer.
        let bytes = w.saturating_mul(h).saturating_mul(4);
        let Some(buf_bytes) = lb.get_mut(..bytes) else {
            self.caches_mut().layer_buf = lb;
            return;
        };
        buf_bytes.fill(0);
        let drawn = {
            match DrawBuf::new_packed(buf_bytes, ColorFormat::Argb8888, area) {
                Ok(buf) => {
                    let mut child = Painter::new(buf, self.caches_mut());
                    #[cfg(feature = "display-integrity")]
                    {
                        child.in_layer = true;
                    }
                    f(&mut child);
                    child.touched_area().is_some()
                }
                Err(_) => false,
            }
        };
        if drawn {
            let img = ImagePixels::new(
                ColorFormat::Argb8888,
                u16::try_from(w).unwrap_or(u16::MAX),
                u16::try_from(h).unwrap_or(u16::MAX),
                lb.get(..bytes).unwrap_or_default(),
            );
            let transform = Transform::translate(
                twine_core::Fx::from_int(area.x0),
                twine_core::Fx::from_int(area.y0),
            )
            .then(t.to_transform(area));
            // The sampler directly, not `blit_transformed`: its integer-translation fast path
            // (`blit_recolor`, with a converter per source format) is for images; a layer
            // transform is a rotation, scale or skew (an integer translation would still
            // sample exactly), and calling it here would link a second copy of that path
            // (flash).
            self.blit_transformed_with(
                &img,
                &BlitDsc {
                    transform,
                    opa: dsc.opa,
                    antialias: t.antialias,
                    blend_mode: dsc.blend_mode,
                    recolor: None,
                },
                TexelOps::default(),
                None,
            );
        }
        self.caches_mut().layer_buf = lb;
    }
}
