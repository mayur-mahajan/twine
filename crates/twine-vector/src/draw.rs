//! [`VectorDsc`], [`VectorCaches`] and [`PainterVectorExt`]: drawing paths through a
//! [`Painter`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::{Fx, Opa, Rect, Transform};
use twine_render::{BlendMode, FillRule, Painter, SpanSource};

use crate::flatten::{DEFAULT_TOLERANCE, Line, Polylines, flatten, flatten_for_stroke};
use crate::geom::{FxRect, hypot};
use crate::paint::{Paint, Sampler, load_lut};
use crate::path::Path;
use crate::raster::{PreparedEdges, RasterScratch, fill_lines};
use crate::stroke::{Chunk, LineJoin, Stroke, build_chunks, dash_source, stroke_polylines, stroke_source};

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
        Self::DEFAULT
    }
}

impl VectorDsc {
    /// The default descriptor (draws nothing: no fill, no stroke), usable in `const`s.
    pub(crate) const DEFAULT: VectorDsc = VectorDsc {
        transform: Transform::IDENTITY,
        fill: None,
        stroke: None,
        opa: Opa::COVER,
        fill_opa: Opa::COVER,
        stroke_opa: Opa::COVER,
        blend_mode: BlendMode::Normal,
    };
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
        self.bounds_with(path, &self.transform)
    }

    /// Like [`bounds`](Self::bounds) with `t` instead of the descriptor's transform.
    #[must_use]
    pub fn bounds_with(&self, path: &Path, t: &Transform) -> Rect {
        if path.is_empty() {
            return Rect::ZERO;
        }
        if let (None, Some(p0)) = (&self.stroke, path.points().first())
            && !t.is_translation_only()
        {
            // The transformed control points: tight for rotated shapes (curves stay inside
            // their control polygon).
            let first = p0.transformed(t);
            let b = path
                .points()
                .iter()
                .fold(FxRect::from_points(first, first), |r, p| {
                    r.include(p.transformed(t))
                });
            return b.to_rect_out().expand(1);
        }
        let mut b = path.bounds();
        if let Some((_, s)) = &self.stroke {
            let hw = Fx::from_raw(s.width.raw().max(0) / 2);
            let k = if s.join == LineJoin::Miter {
                s.miter_limit.max(Fx::from_int(2))
            } else {
                Fx::from_int(2)
            };
            b = b.outset(hw * k);
        }
        b.transformed_bounds(t).to_rect_out().expand(1)
    }
}

/// Parameters applied on top of every [`VectorDsc`] of a draw (a scene placed, faded or
/// recolored as a whole, e.g. an SVG image): no descriptor is copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawParams {
    /// Applied after each item's own transform.
    pub transform: Transform,
    /// Multiplied into each item's opacity.
    pub opa: Opa,
    /// Replaces every paint (fills and strokes) with this color when set.
    pub recolor: Option<twine_core::Color>,
}

impl Default for DrawParams {
    fn default() -> Self {
        Self {
            transform: Transform::IDENTITY,
            opa: Opa::COVER,
            recolor: None,
        }
    }
}

impl DrawParams {
    /// Only a transform.
    #[must_use]
    pub fn transform(t: Transform) -> Self {
        Self {
            transform: t,
            ..Self::default()
        }
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
    /// Hit flags of a prepared stroke's chunks (one per chunk).
    chunk_hits: Vec<bool>,
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
            chunk_hits: Vec::new(),
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
        self.lines
            .capacity()
            .saturating_mul(core::mem::size_of::<Line>())
            .saturating_add(self.raster.bytes_reserved())
            .saturating_add(self.chunk_hits.capacity())
            .saturating_add(1024)
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

    /// Draws `path` with `dsc` and the scene-wide `params` (extra transform, opacity,
    /// recolor), using the painter's [`VectorCaches`].
    fn vector_params(&mut self, path: &Path, dsc: &VectorDsc, params: &DrawParams);
}

impl PainterVectorExt for Painter<'_> {
    fn vector(&mut self, path: &Path, dsc: &VectorDsc) {
        let mut c = self.caches().take_extension::<VectorCaches>().unwrap_or_default();
        self.vector_with(&mut c, path, dsc);
        self.caches().put_extension(c);
    }

    fn vector_with(&mut self, caches: &mut VectorCaches, path: &Path, dsc: &VectorDsc) {
        draw_vector(self, caches, path, dsc, &DrawParams::default(), None);
    }

    fn vector_params(&mut self, path: &Path, dsc: &VectorDsc, params: &DrawParams) {
        let mut c = self.caches().take_extension::<VectorCaches>().unwrap_or_default();
        draw_vector(self, &mut c, path, dsc, params, None);
        self.caches().put_extension(c);
    }
}

/// Geometry of one path prepared for drawing and kept between draws (by a
/// [`VectorScene`](crate::VectorScene) item): the fill's flattened outline for one transform,
/// and the stroke's flattened — and dashed — polylines for one flattening tolerance with their
/// chunk index. Drawing from it gives the same pixels as drawing the path, without redoing the
/// path-space work for every clip a frame is drawn in; each part is rebuilt when its key
/// changes, and [`invalidate`](Self::invalidate)d when the item changes. The storage grows
/// fallibly: when it cannot, the path is drawn without it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Prepared {
    /// The transform `fill_edges` were flattened with (`None`: not prepared).
    fill: Option<Transform>,
    fill_edges: PreparedEdges,
    /// The tolerance `stroke_polys` were flattened with (`None`: not prepared).
    stroke: Option<Fx>,
    stroke_polys: Polylines,
    chunks: Vec<Chunk>,
    /// The transform `stroke_edges` (the whole stroke outline) were built with (`None`: not
    /// prepared, or the outline is larger than [`MAX_KEPT_EDGES`]).
    outline: Option<Transform>,
    stroke_edges: PreparedEdges,
}

/// Most edges of a stroke outline kept prepared (8192 edges, 160 KiB): larger outlines are
/// stroked per clip, near the clip only, from the prepared polylines.
const MAX_KEPT_EDGES: usize = 8192;

impl Prepared {
    /// Forgets the prepared geometry (the item changed); the storage is kept for reuse.
    pub(crate) fn invalidate(&mut self) {
        self.fill = None;
        self.stroke = None;
        self.outline = None;
    }

    /// Bytes reserved by the storage.
    pub(crate) fn bytes_reserved(&self) -> usize {
        self.fill_edges
            .bytes_reserved()
            .saturating_add(self.stroke_polys.bytes_reserved())
            .saturating_add(
                self.chunks
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Chunk>()),
            )
            .saturating_add(self.stroke_edges.bytes_reserved())
    }

    /// Prepares the fill of `path` drawn with `t` (`lines`, `raster`: scratch), unless the
    /// memory cannot be reserved.
    fn prepare_fill(&mut self, path: &Path, t: Transform, lines: &mut Vec<Line>, raster: &mut RasterScratch) {
        self.fill = None;
        lines.clear();
        flatten(path, &t, DEFAULT_TOLERANCE, lines);
        if self.fill_edges.build(lines, raster) {
            self.fill = Some(t);
        }
    }

    /// Prepares the stroke `s` of `path` at tolerance `tol` (`polys`, `dashed`: scratch).
    /// Returns whether it is prepared.
    fn prepare_stroke(
        &mut self,
        path: &Path,
        s: &Stroke,
        tol: Fx,
        polys: &mut Polylines,
        dashed: &mut Polylines,
    ) -> bool {
        self.stroke = None;
        self.outline = None;
        polys.clear();
        flatten_for_stroke(path, &Transform::IDENTITY, tol, polys);
        let src = dash_source(polys, s, dashed);
        if self.stroke_polys.try_copy_from(src) && build_chunks(&self.stroke_polys, &mut self.chunks) {
            self.stroke = Some(tol);
        }
        self.stroke.is_some()
    }

    /// Prepares the whole outline of the (prepared) stroke `s` drawn with `t` at tolerance
    /// `tol` as edges (`lines`, `raster`: scratch), unless it has more than [`MAX_KEPT_EDGES`]
    /// or the memory cannot be reserved.
    fn prepare_outline(
        &mut self,
        s: &Stroke,
        tol: Fx,
        t: Transform,
        lines: &mut Vec<Line>,
        raster: &mut RasterScratch,
    ) {
        self.outline = None;
        lines.clear();
        stroke_source(&self.stroke_polys, None, s, tol, &t, None, lines);
        if lines.len() <= MAX_KEPT_EDGES && self.stroke_edges.build(lines, raster) {
            self.outline = Some(t);
        }
    }
}

/// Draws `path` with `dsc` and `params` through the painter's [`VectorCaches`], keeping and
/// reusing its prepared geometry in `prep`.
pub(crate) fn draw_prepared(
    p: &mut Painter<'_>,
    path: &Path,
    dsc: &VectorDsc,
    params: &DrawParams,
    prep: &mut Prepared,
) {
    let mut c = p.caches().take_extension::<VectorCaches>().unwrap_or_default();
    draw_vector(p, &mut c, path, dsc, params, Some(prep));
    p.caches().put_extension(c);
}

fn draw_vector(
    p: &mut Painter<'_>,
    caches: &mut VectorCaches,
    path: &Path,
    dsc: &VectorDsc,
    params: &DrawParams,
    mut prep: Option<&mut Prepared>,
) {
    if !dsc.is_visible() || path.is_empty() || params.opa.is_transparent() {
        return;
    }
    let t = if params.transform.is_identity() {
        dsc.transform
    } else {
        dsc.transform.then(params.transform)
    };
    let clip = p.clip();
    if clip.is_empty() || !dsc.bounds_with(path, &t).intersects(&clip) {
        return;
    }
    twine_core::trace!(
        target: "twine::vector",
        "vector: {} verbs, fill {}, stroke {}",
        path.verbs().len(),
        dsc.fill.is_some(),
        dsc.stroke.is_some()
    );
    let recolored = params.recolor.map(Paint::Solid);
    let VectorCaches {
        lines,
        polys,
        dashed,
        raster,
        lut,
        chunk_hits,
    } = caches;
    if let Some((paint, rule)) = &dsc.fill {
        let src: Geometry<'_> = if let Some(pr) = prep.as_deref_mut() {
            if pr.fill != Some(t) {
                pr.prepare_fill(path, t, lines, raster);
            }
            if pr.fill == Some(t) {
                Geometry::Edges(&pr.fill_edges)
            } else {
                Geometry::Lines(lines)
            }
        } else {
            lines.clear();
            flatten(path, &t, DEFAULT_TOLERANCE, lines);
            Geometry::Lines(lines)
        };
        let opa = mul_opa(mul_opa(dsc.opa, dsc.fill_opa), params.opa);
        paint_lines(
            p,
            raster,
            lut,
            src,
            recolored.as_ref().unwrap_or(paint),
            *rule,
            opa,
            &t,
            dsc.blend_mode,
        );
    }
    if let Some((paint, stroke)) = &dsc.stroke {
        // Stroke in path space (the width is in path units), then map the outline.
        let tol = user_tolerance(&t);
        lines.clear();
        let prepared = prep.and_then(|pr| {
            let ready = pr.stroke == Some(tol) || pr.prepare_stroke(path, stroke, tol, polys, dashed);
            ready.then_some(pr)
        });
        let geometry = if let Some(pr) = prepared {
            if pr.outline != Some(t) {
                pr.prepare_outline(stroke, tol, t, lines, raster);
            }
            if pr.outline == Some(t) {
                // The whole outline, built once: only rasterized here.
                Geometry::Edges(&pr.stroke_edges)
            } else {
                // Too large to keep: stroked near the clip only.
                lines.clear();
                stroke_source(
                    &pr.stroke_polys,
                    Some((&pr.chunks, chunk_hits)),
                    stroke,
                    tol,
                    &t,
                    Some(clip),
                    lines,
                );
                Geometry::Lines(lines)
            }
        } else {
            polys.clear();
            flatten_for_stroke(path, &Transform::IDENTITY, tol, polys);
            stroke_polylines(polys, stroke, tol, &t, Some(clip), dashed, lines);
            Geometry::Lines(lines)
        };
        let opa = mul_opa(mul_opa(dsc.opa, dsc.stroke_opa), params.opa);
        paint_lines(
            p,
            raster,
            lut,
            geometry,
            recolored.as_ref().unwrap_or(paint),
            FillRule::NonZero,
            opa,
            &t,
            dsc.blend_mode,
        );
    }
}

/// The flattening tolerance in path units for a device tolerance of 0.25 px under `t`.
fn user_tolerance(t: &Transform) -> Fx {
    let sx = hypot(i64::from(t.a.raw()), i64::from(t.b.raw()));
    let sy = hypot(i64::from(t.c.raw()), i64::from(t.d.raw()));
    let s = sx.max(sy).max(1 << 8);
    // SAFE-ARITH: the tolerance is `2^14`, times `2^16`. `s ≥ 256` (`.max(1 << 8)` above): the
    // division's fallback is never taken.
    let tol = i64::from(DEFAULT_TOLERANCE.raw())
        .wrapping_mul(65_536)
        .checked_div(s)
        .unwrap_or(0);
    Fx::from_raw(tol.clamp(64, i64::from(i32::MAX)) as i32)
}

/// What [`paint_lines`] rasterizes: lines (collected into edges for this draw) or edges
/// prepared earlier.
#[derive(Clone, Copy)]
enum Geometry<'a> {
    Lines(&'a [Line]),
    Edges(&'a PreparedEdges),
}

impl Geometry<'_> {
    fn is_empty(&self) -> bool {
        match self {
            Geometry::Lines(l) => l.is_empty(),
            Geometry::Edges(e) => e.len() == 0,
        }
    }

    fn fill(
        &self,
        p: &mut Painter<'_>,
        raster: &mut RasterScratch,
        rule: FillRule,
        src: &SpanSource<'_>,
        mode: BlendMode,
    ) {
        match self {
            Geometry::Lines(l) => fill_lines(p, raster, l, rule, src, mode),
            Geometry::Edges(e) => e.fill(p, raster, rule, src, mode),
        }
    }
}

/// Rasterizes `geometry` with `paint` (`raster`, `lut`: the caches' scratch).
#[allow(clippy::too_many_arguments)] // the scratch is passed apart from the geometry it draws
fn paint_lines(
    p: &mut Painter<'_>,
    raster: &mut RasterScratch,
    lut: &mut [u32; 256],
    geometry: Geometry<'_>,
    paint: &Paint,
    rule: FillRule,
    opa: Opa,
    transform: &Transform,
    blend_mode: BlendMode,
) {
    if opa.is_transparent() || geometry.is_empty() {
        return;
    }
    match paint {
        Paint::Solid(c) => {
            geometry.fill(p, raster, rule, &SpanSource::Solid(*c, opa), blend_mode);
        }
        Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => {
            let Some(s) = Sampler::new(paint, transform) else {
                return;
            };
            load_lut(p, stops.as_slice(), lut);
            let lut: &[u32; 256] = lut;
            let f = |y: i32, x0: i32, out: &mut [u8]| s.fill(lut, y, x0, out);
            geometry.fill(p, raster, rule, &SpanSource::Pixels(&f, opa), blend_mode);
        }
        Paint::Image { .. } => {
            let Some(s) = Sampler::new(paint, transform) else {
                return;
            };
            let lut: &[u32; 256] = lut;
            let f = |y: i32, x0: i32, out: &mut [u8]| s.fill(lut, y, x0, out);
            geometry.fill(p, raster, rule, &SpanSource::Pixels(&f, opa), blend_mode);
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
