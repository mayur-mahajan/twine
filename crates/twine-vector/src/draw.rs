//! [`VectorDsc`], [`VectorCaches`] and [`PainterVectorExt`]: drawing paths through a
//! [`Painter`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::{Fx, Opa, Rect, Transform};
use twine_render::{BlendMode, FillRule, Painter, SpanSource};

use crate::flatten::{DEFAULT_TOLERANCE, Line, Polylines, flatten, flatten_for_stroke};
use crate::geom::hypot;
use crate::paint::{Paint, Sampler, load_lut};
use crate::path::Path;
use crate::raster::{RasterScratch, fill_lines};
use crate::stroke::{LineJoin, Stroke, stroke_polylines};

/// How a [`Path`] is drawn (LVGL `lv_vector_dsc`): transform, optional fill and stroke with
/// their paints, opacities and the blend mode. The fill is drawn first, then the stroke.
///
/// ```
/// use twine_core::{Color, Fx};
/// use twine_render::FillRule;
/// use twine_vector::{Paint, Stroke, VectorDsc};
///
/// let dsc = VectorDsc {
///     fill: Some((Paint::Solid(Color::RED), FillRule::NonZero)),
///     stroke: Some((Paint::Solid(Color::BLACK), Stroke { width: Fx::from_int(2), ..Stroke::default() })),
///     ..VectorDsc::default()
/// };
/// assert!(dsc.is_visible());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorDsc {
    /// Path space → screen (absolute screen coordinates).
    pub transform: Transform,
    /// Fill paint and rule.
    pub fill: Option<(Paint, FillRule)>,
    /// Stroke paint and parameters.
    pub stroke: Option<(Paint, Stroke)>,
    /// Opacity of the whole drawing.
    pub opa: Opa,
    /// Extra opacity of the fill (SVG `fill-opacity`).
    pub fill_opa: Opa,
    /// Extra opacity of the stroke (SVG `stroke-opacity`).
    pub stroke_opa: Opa,
    /// Blend mode.
    pub blend_mode: BlendMode,
}

impl Default for VectorDsc {
    fn default() -> Self {
        Self {
            transform: Transform::IDENTITY,
            fill: None,
            stroke: None,
            opa: Opa::COVER,
            fill_opa: Opa::COVER,
            stroke_opa: Opa::COVER,
            blend_mode: BlendMode::Normal,
        }
    }
}

impl VectorDsc {
    /// A solid fill of `color` with the non-zero rule.
    #[must_use]
    pub fn fill(color: twine_core::Color) -> Self {
        Self {
            fill: Some((Paint::Solid(color), FillRule::NonZero)),
            ..Self::default()
        }
    }

    /// Whether anything would be drawn (a fill or stroke and a visible opacity).
    #[must_use]
    pub fn is_visible(&self) -> bool {
        !self.opa.is_transparent() && (self.fill.is_some() || self.stroke.is_some())
    }

    /// Screen bounds of `path` drawn with this descriptor (stroke width, miters and caps
    /// included; conservative).
    #[must_use]
    pub fn bounds(&self, path: &Path) -> Rect {
        if path.is_empty() {
            return Rect::ZERO;
        }
        let mut b = path.bounds();
        if let Some((_, s)) = &self.stroke {
            let hw = Fx(s.width.0.max(0) / 2);
            let k = if s.join == LineJoin::Miter {
                s.miter_limit.max(Fx::from_int(2))
            } else {
                Fx::from_int(2)
            };
            b = b.outset(hw * k);
        }
        b.transformed_bounds(&self.transform).to_rect_out().expand(1)
    }
}

/// Reusable scratch memory of vector drawing: flattened lines and polylines, the rasterizer's
/// edge list and row accumulators, and one 256-entry gradient color map.
///
/// Budget after warm-up (nothing is allocated while drawing once the buffers are large
/// enough; they grow to the largest path drawn and never shrink):
///
/// | Buffer | Size |
/// |--------|------|
/// | lines (flattened fill / stroke outline) | 16 B × segments |
/// | edges | 20 B × segments |
/// | stroke polylines (+ dash pieces) | 8 B × points, ×2 with dashes |
/// | row accumulators | (clip width + 2) × 7 B (`i32` + `u16` + coverage `u8`) |
/// | active edges, sub-scanline crossings | 12 B × edges crossing one row |
/// | color map | 1 KiB |
///
/// For example a 64-segment circle clipped to a 320 px wide buffer uses about 4 KiB.
#[derive(Debug)]
pub struct VectorCaches {
    lines: Vec<Line>,
    polys: Polylines,
    dashed: Polylines,
    raster: RasterScratch,
    lut: Box<[u32; 256]>,
}

impl Default for VectorCaches {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorCaches {
    /// Empty caches (the buffers grow on first use).
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            polys: Polylines::new(),
            dashed: Polylines::new(),
            raster: RasterScratch::default(),
            lut: Box::new([0; 256]),
        }
    }

    /// Caches pre-sized for rows up to `max_width` pixels and paths of up to `segments` line
    /// segments, so even the first draw does not allocate for such paths.
    #[must_use]
    pub fn with_capacity(max_width: usize, segments: usize) -> Self {
        let mut c = Self::new();
        c.lines.reserve(segments);
        c.raster.reserve_edges(segments);
        c.raster.reserve_width(max_width);
        c
    }

    /// Bytes currently reserved by all buffers.
    #[must_use]
    pub fn bytes_reserved(&self) -> usize {
        self.lines.capacity() * core::mem::size_of::<Line>() + self.raster.bytes_reserved() + 1024
    }
}

/// Opacity product.
fn mul_opa(a: Opa, b: Opa) -> Opa {
    a.mul(b)
}

/// Draws [`Path`]s on a [`Painter`] (an extension trait because `twine-render` does not know
/// about vector graphics).
///
/// ```
/// use twine_core::{Color, ColorFormat, Fx, Rect};
/// use twine_render::{DrawBuf, Painter, RenderCaches};
/// use twine_vector::{FxPoint, Path, PainterVectorExt, VectorDsc};
///
/// let mut caches = RenderCaches::default();
/// let mut px = vec![0u8; 20 * 20];
/// let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, 20, 20)).unwrap();
/// let mut p = Painter::new(buf, &mut caches);
/// let mut path = Path::new();
/// path.circle(FxPoint::from_int(10, 10), Fx::from_int(8));
/// p.vector(&path, &VectorDsc::fill(Color::WHITE));
/// drop(p);
/// assert_eq!(px[10 * 20 + 10], 255); // center filled
/// assert_eq!(px[0], 0); // corner outside the circle
/// ```
pub trait PainterVectorExt {
    /// Draws `path` with `dsc`, using [`VectorCaches`] kept in the painter's
    /// [`RenderCaches`](twine_render::RenderCaches) extension slot (created on first use).
    fn vector(&mut self, path: &Path, dsc: &VectorDsc);

    /// Draws `path` with `dsc` using explicit `caches`.
    fn vector_with(&mut self, caches: &mut VectorCaches, path: &Path, dsc: &VectorDsc);
}

impl PainterVectorExt for Painter<'_> {
    fn vector(&mut self, path: &Path, dsc: &VectorDsc) {
        let mut c = self.caches().take_extension::<VectorCaches>().unwrap_or_default();
        self.vector_with(&mut c, path, dsc);
        self.caches().put_extension(c);
    }

    fn vector_with(&mut self, caches: &mut VectorCaches, path: &Path, dsc: &VectorDsc) {
        if !dsc.is_visible() || path.is_empty() {
            return;
        }
        let clip = self.clip();
        if clip.is_empty() || !dsc.bounds(path).intersects(&clip) {
            return;
        }
        twine_core::trace!(
            target: "twine::vector",
            "vector: {} verbs, fill {}, stroke {}",
            path.verbs().len(),
            dsc.fill.is_some(),
            dsc.stroke.is_some()
        );
        if let Some((paint, rule)) = &dsc.fill {
            caches.lines.clear();
            flatten(path, &dsc.transform, DEFAULT_TOLERANCE, &mut caches.lines);
            let opa = mul_opa(dsc.opa, dsc.fill_opa);
            paint_lines(self, caches, paint, *rule, opa, dsc);
        }
        if let Some((paint, stroke)) = &dsc.stroke {
            // Stroke in path space (the width is in path units), then map the outline.
            let tol = user_tolerance(&dsc.transform);
            caches.polys.clear();
            flatten_for_stroke(path, &Transform::IDENTITY, tol, &mut caches.polys);
            caches.lines.clear();
            stroke_polylines(
                &caches.polys,
                stroke,
                tol,
                &dsc.transform,
                &mut caches.dashed,
                &mut caches.lines,
            );
            let opa = mul_opa(dsc.opa, dsc.stroke_opa);
            paint_lines(self, caches, paint, FillRule::NonZero, opa, dsc);
        }
    }
}

/// The flattening tolerance in path units for a device tolerance of 0.25 px under `t`.
fn user_tolerance(t: &Transform) -> Fx {
    let sx = hypot(i64::from(t.a.0), i64::from(t.b.0));
    let sy = hypot(i64::from(t.c.0), i64::from(t.d.0));
    let s = sx.max(sy).max(1 << 8);
    let tol = i64::from(DEFAULT_TOLERANCE.0) * 65_536 / s;
    Fx(tol.clamp(64, i64::from(i32::MAX)) as i32)
}

/// Rasterizes `caches.lines` with `paint`.
fn paint_lines(
    p: &mut Painter<'_>,
    caches: &mut VectorCaches,
    paint: &Paint,
    rule: FillRule,
    opa: Opa,
    dsc: &VectorDsc,
) {
    if opa.is_transparent() || caches.lines.is_empty() {
        return;
    }
    let VectorCaches {
        lines, raster, lut, ..
    } = caches;
    match paint {
        Paint::Solid(c) => {
            fill_lines(
                p,
                raster,
                lines,
                rule,
                &SpanSource::Solid(*c, opa),
                dsc.blend_mode,
            );
        }
        Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => {
            let Some(s) = Sampler::new(paint, &dsc.transform) else {
                return;
            };
            load_lut(p, stops.as_slice(), lut);
            let lut: &[u32; 256] = lut;
            let f = |y: i32, x0: i32, out: &mut [u8]| s.fill(lut, y, x0, out);
            fill_lines(
                p,
                raster,
                lines,
                rule,
                &SpanSource::Pixels(&f, opa),
                dsc.blend_mode,
            );
        }
        Paint::Image { .. } => {
            let Some(s) = Sampler::new(paint, &dsc.transform) else {
                return;
            };
            let lut: &[u32; 256] = lut;
            let f = |y: i32, x0: i32, out: &mut [u8]| s.fill(lut, y, x0, out);
            fill_lines(
                p,
                raster,
                lines,
                rule,
                &SpanSource::Pixels(&f, opa),
                dsc.blend_mode,
            );
        }
    }
}

/// Screen bounds helper used by scenes: the union of `dsc.bounds(path)` over items.
pub(crate) fn union_rect(a: Option<Rect>, b: Rect) -> Option<Rect> {
    if b.is_empty() {
        return a;
    }
    Some(a.map_or(b, |a| a.union(&b)))
}
