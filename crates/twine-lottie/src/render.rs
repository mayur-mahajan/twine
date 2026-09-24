//! [`LottiePlayer`]: renders a frame of a [`Composition`] into a [`Painter`] through
//! `twine-vector`.
//!
//! ## Layers
//!
//! Layers are drawn bottom-to-top (Lottie lists them top-most first). Each layer's matrix is
//! its own transform preceded by its parent chain. A layer with opacity below 100 %, masks or a
//! track matte is rendered *offscreen*: into an `Argb8888` buffer in horizontal strips of at
//! most [`RenderConfig::layer_buf_bytes`](twine_render::RenderConfig) bytes, while the masks
//! (and the matte layer's alpha) are rasterized into an 8-bit coverage buffer of the same strip;
//! the strip is then blended through [`Painter::coverage_span`] with the combined coverage and
//! the layer opacity. Offscreen layers nest up to [`MAX_OFFSCREEN_DEPTH`] levels; deeper (or
//! when the budget cannot hold one row) the layer is drawn directly with its opacity multiplied
//! into its paints and its masks/matte ignored (one warning per player).
//!
//! ## Shapes
//!
//! Within a group, geometry items (rectangle, ellipse, path) produce contours; paint items
//! (fill, stroke, gradient fill/stroke) paint every contour produced by the items *before* them
//! in the same group, nested groups included; modifiers (trim paths, repeater) rewrite the
//! contours produced before them. Items are numbered depth-first so "before" is a range of
//! numbers, and contours remember the item that produced them. Paints are drawn in reverse
//! item order (the first item is on top), each as one path per run of equal repeater opacity.
//! Geometry is kept in layer space and mapped back into the painting group's space, so stroke
//! widths, dashes and gradients live in the space where they are defined.

use alloc::vec::Vec;
use core::mem::take;

use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::{DrawBuf, GradExtend, GradStop, Painter, SpanSource};
use twine_vector::{
    Dash, FxPoint, LineCap as VCap, LineJoin as VJoin, Paint, PainterVectorExt, Path, Stops,
    Stroke as VStroke, VectorDsc,
};

use crate::eval::mix;
use crate::fmath::{self, DEG, atan2, channel, clamp, cos, fx, opa, round_i32, sin, sqrt};
use crate::geom::{Geometry, Mat, TransformValues, transform_matrix};
use crate::model::{
    Composition, DashKind, FillRule, Gradient, GradientKind, Layer, LayerKind, LineCap, LineJoin, MaskMode,
    PathData, Rgba, Shape, StrokeStyle, Vec2,
};
use crate::modifier::{self, RepeatParams};

/// Deepest nesting of offscreen layers (layers with opacity, masks or mattes inside each other).
pub const MAX_OFFSCREEN_DEPTH: usize = 4;

/// Deepest nesting of precompositions (also stops reference cycles).
pub const MAX_PRECOMP_DEPTH: u8 = 8;

/// Longest parent chain followed (also stops parent cycles).
const MAX_PARENT_DEPTH: u8 = 32;

/// Most layers drawn per frame (precomps can reference the same asset many times; this bounds
/// the work of hostile files).
pub const MAX_LAYER_DRAWS: u32 = 4096;

/// Buffers of one offscreen nesting level.
#[derive(Debug, Default)]
struct Offscreen {
    /// Layer content, `Argb8888`.
    argb: Vec<u8>,
    /// Matte layer content, `Argb8888`.
    matte: Vec<u8>,
    /// Combined coverage (masks × matte).
    cov: Vec<u8>,
    /// One mask's coverage.
    tmp: Vec<u8>,
}

/// Reusable memory of the renderer: the contour arena, the vector path, evaluation outputs and
/// offscreen buffers. Everything grows to the largest frame drawn and is then reused.
#[derive(Debug, Default)]
struct RenderScratch {
    geo: Geometry,
    tmp: Geometry,
    path: Path,
    pathdata: PathData,
    grad: Vec<f32>,
    stops: Vec<GradStop>,
    offscreen: Vec<Offscreen>,
    ones: Vec<u8>,
    warned_fallback: bool,
}

impl RenderScratch {
    fn bytes_reserved(&self) -> usize {
        let v2 = core::mem::size_of::<Vec2>();
        let c = core::mem::size_of::<crate::geom::Contour>();
        let geo = |g: &Geometry| g.contours.capacity() * c + g.pts.capacity() * v2;
        geo(&self.geo)
            + geo(&self.tmp)
            + self.path.points().len() * 8
            + (self.pathdata.v.capacity() + self.pathdata.i.capacity() + self.pathdata.o.capacity()) * v2
            + self.grad.capacity() * 4
            + self.stops.capacity() * core::mem::size_of::<GradStop>()
            + self
                .offscreen
                .iter()
                .map(|o| o.argb.capacity() + o.matte.capacity() + o.cov.capacity() + o.tmp.capacity())
                .sum::<usize>()
            + self.ones.capacity()
    }
}

/// Plays a [`Composition`]: renders any frame into a [`Painter`].
///
/// Rendering needs floating point (Lottie data is float-based); geometry is converted to 16.16
/// fixed point before it reaches `twine-vector`. On MCUs without an FPU (RP2040, ESP32-C3) it
/// works with soft-float, only slower.
///
/// ```
/// use twine_core::{ColorFormat, Rect};
/// use twine_render::{DrawBuf, Painter, RenderCaches};
/// use twine_lottie::{LottiePlayer, load};
///
/// let json = br#"{"fr":30,"ip":0,"op":30,"w":40,"h":40,"layers":[
///   {"ty":4,"ks":{},"shapes":[
///     {"ty":"rc","p":{"k":[20,20]},"s":{"k":[20,20]},"r":{"k":0}},
///     {"ty":"fl","c":{"k":[0,0,1,1]},"o":{"k":100}}]}]}"#;
/// let mut player = LottiePlayer::new(load(json).unwrap());
/// let mut caches = RenderCaches::default();
/// let mut px = vec![0u8; 40 * 40 * 4];
/// let area = Rect::from_xywh(0, 0, 40, 40);
/// let mut p = Painter::new(DrawBuf::new_packed(&mut px, ColorFormat::Argb8888, area).unwrap(), &mut caches);
/// player.render_frame(0.0, &mut p, area);
/// drop(p);
/// let at = |x: usize, y: usize| &px[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4];
/// assert_eq!(at(20, 20), &[255, 0, 0, 255]); // B, G, R, A: blue
/// assert_eq!(at(2, 2)[3], 0); // outside the square: untouched
/// ```
#[derive(Debug)]
pub struct LottiePlayer {
    comp: Composition,
    scratch: RenderScratch,
}

impl LottiePlayer {
    /// A player for `comp`.
    #[must_use]
    pub fn new(comp: Composition) -> Self {
        Self {
            comp,
            scratch: RenderScratch::default(),
        }
    }

    /// The composition.
    #[must_use]
    pub fn composition(&self) -> &Composition {
        &self.comp
    }

    /// Bytes of scratch memory currently reserved (contours, paths, offscreen buffers).
    #[must_use]
    pub fn bytes_reserved(&self) -> usize {
        self.scratch.bytes_reserved()
    }

    /// The composition → screen transform that fits the composition into `dst` (contain,
    /// centered), as `(scale, offset_x, offset_y)`.
    #[must_use]
    pub fn fit(&self, dst: Rect) -> (f32, f32, f32) {
        let (dw, dh) = (dst.width() as f32, dst.height() as f32);
        let s = (dw / self.comp.w).min(dh / self.comp.h);
        let s = if s.is_finite() && s > 0.0 { s } else { 0.0 };
        (
            s,
            dst.x0 as f32 + (dw - self.comp.w * s) / 2.0,
            dst.y0 as f32 + (dh - self.comp.h * s) / 2.0,
        )
    }

    /// Renders `frame` (composition frame number, fractional allowed) fitted into `dst`
    /// (contain, centered) and clipped to it. Allocates nothing once the scratch buffers have
    /// grown to the largest frame.
    pub fn render_frame(&mut self, frame: f32, painter: &mut Painter<'_>, dst: Rect) {
        if dst.is_empty() {
            return;
        }
        let (s, ox, oy) = self.fit(dst);
        if s <= 0.0 {
            return;
        }
        let base = Mat::translate(ox, oy).mul(&Mat::scale(s, s));
        twine_core::trace!(target: "twine::lottie", "lottie: frame {} into {}", frame, dst);
        let mut r = Renderer {
            comp: &self.comp,
            s: &mut self.scratch,
            depth: 0,
            work: 0,
        };
        let layers = &self.comp.layers;
        painter.with_clip(dst, |p| r.layers(p, layers, frame, &base, 1.0, 0));
    }
}

/// Where the coverage of an offscreen layer comes from.
#[derive(Clone, Copy)]
struct CoverageSrc<'a> {
    /// The layer whose masks apply, its local frame and matrix.
    masks: Option<(&'a Layer, f32, Mat)>,
    /// The matte source.
    matte: Option<Matte<'a>>,
}

#[derive(Clone, Copy)]
struct Matte<'a> {
    list: &'a [Layer],
    index: usize,
    frame: f32,
    base: Mat,
    precomp_depth: u8,
    invert: bool,
}

/// Evaluated paint of a fill or stroke item.
#[derive(Clone, Copy)]
enum PaintSpec<'a> {
    Solid(Rgba),
    Gradient(&'a Gradient),
}

/// One frame's rendering state.
struct Renderer<'a> {
    comp: &'a Composition,
    s: &'a mut RenderScratch,
    /// Current offscreen nesting level.
    depth: usize,
    /// Layers drawn so far in this frame.
    work: u32,
}

impl<'a> Renderer<'a> {
    /// Draws `list` (top-most first) bottom-to-top.
    fn layers(
        &mut self,
        p: &mut Painter<'_>,
        list: &'a [Layer],
        frame: f32,
        base: &Mat,
        opa_mul: f32,
        pdepth: u8,
    ) {
        if pdepth > MAX_PRECOMP_DEPTH {
            return;
        }
        for i in (0..list.len()).rev() {
            if list[i].td.is_some() {
                continue; // matte sources are drawn through their target
            }
            self.layer(p, list, i, frame, base, opa_mul, pdepth);
        }
    }

    /// The matrix of layer `i` (parent chain included), without `base`.
    fn layer_matrix(list: &[Layer], i: usize, frame: f32, depth: u8) -> Mat {
        let l = &list[i];
        let own = transform_matrix(&l.ks, l.local_frame(frame));
        match l.parent_index {
            Some(pi) if pi < list.len() && depth < MAX_PARENT_DEPTH => {
                Self::layer_matrix(list, pi, frame, depth + 1).mul(&own)
            }
            _ => own,
        }
    }

    /// Draws layer `i` of `list` with its opacity, masks and matte.
    #[allow(clippy::too_many_arguments)]
    fn layer(
        &mut self,
        p: &mut Painter<'_>,
        list: &'a [Layer],
        i: usize,
        frame: f32,
        base: &Mat,
        opa_mul: f32,
        pdepth: u8,
    ) {
        let l = &list[i];
        if !l.is_visible_at(frame) || matches!(l.ty, LayerKind::Null | LayerKind::Unsupported(_)) {
            return;
        }
        self.work += 1;
        if self.work > MAX_LAYER_DRAWS {
            if self.work == MAX_LAYER_DRAWS + 1 {
                twine_core::warn!(target: "twine::lottie", "lottie: more than {} layer draws in one frame; rest skipped", MAX_LAYER_DRAWS);
            }
            return;
        }
        let lf = l.local_frame(frame);
        let m = base.mul(&Self::layer_matrix(list, i, frame, 0));
        let opacity = clamp(l.ks.opacity.value(lf) / 100.0, 0.0, 1.0) * opa_mul;
        if opacity <= 0.0 {
            return;
        }
        let matte = match (l.tt, l.matte_index) {
            (Some(t @ (1 | 2)), Some(mi)) if mi < list.len() => Some(Matte {
                list,
                index: mi,
                frame,
                base: *base,
                precomp_depth: pdepth,
                invert: t == 2,
            }),
            _ => None,
        };
        let has_masks = l.masks.iter().any(|mk| mk.mode != MaskMode::None);
        if opacity >= 1.0 - 1.0 / 512.0 && !has_masks && matte.is_none() {
            self.content(p, list, i, frame, &m, 1.0, pdepth);
            return;
        }
        let mut region = p.clip();
        if has_masks {
            if let Some(b) = self.mask_bounds(l, lf, &m) {
                region = region.intersection(&b).unwrap_or(Rect::ZERO);
            }
        }
        if region.is_empty() {
            return;
        }
        let cov = CoverageSrc {
            masks: has_masks.then_some((l, lf, m)),
            matte,
        };
        let drawn = self.offscreen(
            p,
            region,
            opa(opacity),
            cov,
            &mut |r: &mut Self, child: &mut Painter<'_>| {
                r.content(child, list, i, frame, &m, 1.0, pdepth);
            },
        );
        if !drawn {
            self.content(p, list, i, frame, &m, opacity, pdepth);
        }
    }

    /// Screen bounds of the masks of `l` when they only add (else `None` = unbounded).
    fn mask_bounds(&mut self, l: &Layer, lf: f32, m: &Mat) -> Option<Rect> {
        let mut b: Option<(f32, f32, f32, f32)> = None;
        for mk in &l.masks {
            match mk.mode {
                MaskMode::None => continue,
                MaskMode::Add if !mk.inv => {}
                _ => return None,
            }
            mk.pt.eval_into(lf, &mut self.s.pathdata);
            let d = &self.s.pathdata;
            let pts = d.v.iter().copied().chain(
                d.v.iter()
                    .zip(&d.i)
                    .chain(d.v.iter().zip(&d.o))
                    .map(|(v, t)| [v[0] + t[0], v[1] + t[1]]),
            );
            for q in pts {
                let q = m.apply(q);
                b = Some(match b {
                    None => (q[0], q[1], q[0], q[1]),
                    Some(b) => (b.0.min(q[0]), b.1.min(q[1]), b.2.max(q[0]), b.3.max(q[1])),
                });
            }
        }
        let b = b?;
        Some(screen_rect(b))
    }

    /// Draws the content of layer `i` (shapes, solid or precomp) with matrix `m`.
    #[allow(clippy::too_many_arguments)]
    fn content(
        &mut self,
        p: &mut Painter<'_>,
        list: &'a [Layer],
        i: usize,
        frame: f32,
        m: &Mat,
        opa_mul: f32,
        pdepth: u8,
    ) {
        let l = &list[i];
        let lf = l.local_frame(frame);
        match l.ty {
            LayerKind::Shape => self.shapes(p, &l.shapes, lf, m, opa_mul),
            LayerKind::Solid => {
                if l.sw <= 0.0 || l.sh <= 0.0 {
                    return;
                }
                let path = &mut self.s.path;
                path.clear();
                let corner = |x: f32, y: f32| FxPoint::new(fx(x), fx(y));
                path.move_to(corner(0.0, 0.0))
                    .line_to(corner(l.sw, 0.0))
                    .line_to(corner(l.sw, l.sh))
                    .line_to(corner(0.0, l.sh))
                    .close();
                let dsc = VectorDsc {
                    transform: m.to_fx(),
                    fill: Some((Paint::Solid(color(l.sc)), twine_render::FillRule::NonZero)),
                    opa: opa(opa_mul),
                    ..VectorDsc::default()
                };
                p.vector(path, &dsc);
            }
            LayerKind::Precomp => {
                let comp = self.comp;
                let Some(asset) = l.asset.and_then(|a| comp.assets.get(a)) else {
                    return;
                };
                let child_frame = match &l.tm {
                    Some(tm) => tm.value(lf) * comp.fr,
                    None => lf,
                };
                let clip = if l.w > 0.0 && l.h > 0.0 {
                    screen_rect(m.map_rect_bounds(l.w, l.h))
                } else {
                    p.clip()
                };
                let layers = &asset.layers;
                p.with_clip(clip, |p| {
                    self.layers(p, layers, child_frame, m, opa_mul, pdepth + 1);
                });
            }
            LayerKind::Null | LayerKind::Unsupported(_) => {}
        }
    }

    /// Renders `content` offscreen over `region` and blends it with the coverage of `cov` and
    /// `opa`. Returns `false` (drawing nothing) when no offscreen buffer is available.
    fn offscreen(
        &mut self,
        p: &mut Painter<'_>,
        region: Rect,
        opa: Opa,
        cov: CoverageSrc<'a>,
        content: &mut dyn FnMut(&mut Self, &mut Painter<'_>),
    ) -> bool {
        let Some(region) = region.intersection(&p.clip()) else {
            return true;
        };
        let w = region.width() as usize;
        let budget = p.caches().config().layer_buf_bytes as usize;
        let rows = (budget / (w * 4)).min(region.height() as usize);
        if rows == 0 || self.depth >= MAX_OFFSCREEN_DEPTH {
            if !self.s.warned_fallback {
                self.s.warned_fallback = true;
                twine_core::warn!(
                    target: "twine::lottie",
                    "lottie: layer {} drawn without offscreen buffer (budget {} B, depth {}); masks and mattes ignored",
                    region,
                    budget,
                    self.depth
                );
            }
            return false;
        }
        let d = self.depth;
        while self.s.offscreen.len() <= d {
            self.s.offscreen.push(Offscreen::default());
        }
        if self.s.ones.len() < w {
            self.s.ones.resize(w, 255);
        }
        let mut bufs = take(&mut self.s.offscreen[d]);
        let n = rows * w;
        grow(&mut bufs.argb, n * 4);
        let has_cov = cov.masks.is_some() || cov.matte.is_some();
        if has_cov {
            grow(&mut bufs.cov, n);
        }
        if cov.masks.is_some() {
            grow(&mut bufs.tmp, n);
        }
        if cov.matte.is_some() {
            grow(&mut bufs.matte, n * 4);
        }
        self.depth += 1;
        let mut y0 = region.y0;
        while y0 < region.y1 {
            let strip = Rect::new(region.x0, y0, region.x1, (y0 + rows as i32).min(region.y1));
            y0 = strip.y1;
            let n = w * strip.height() as usize;
            let px = &mut bufs.argb[..n * 4];
            px.fill(0);
            let touched = match DrawBuf::new_packed(px, ColorFormat::Argb8888, strip) {
                Ok(buf) => {
                    let mut child = Painter::new(buf, p.caches());
                    content(self, &mut child);
                    child.touched_area()
                }
                Err(_) => None,
            };
            let Some(t) = touched.and_then(|t| t.intersection(&strip)) else {
                continue;
            };
            if has_cov {
                self.coverage(p, &cov, strip, &mut bufs);
            }
            let (tx0, tw) = ((t.x0 - strip.x0) as usize, t.width() as usize);
            for y in t.y0..t.y1 {
                let row = (y - strip.y0) as usize * w + tx0;
                let pixels = &bufs.argb[row * 4..(row + tw) * 4];
                let cov_row = if has_cov {
                    &bufs.cov[row..row + tw]
                } else {
                    &self.s.ones[..tw]
                };
                let x0 = t.x0;
                let f = |_y: i32, x: i32, out: &mut [u8]| {
                    let o = (x - x0) as usize * 4;
                    if let Some(src) = pixels.get(o..o + out.len()) {
                        out.copy_from_slice(src);
                    }
                };
                p.coverage_span(y, t.x0, cov_row, &SpanSource::Pixels(&f, opa));
            }
        }
        self.depth -= 1;
        self.s.offscreen[d] = bufs;
        true
    }

    /// Fills `bufs.cov` (the strip's coverage) from masks and matte.
    fn coverage(&mut self, p: &mut Painter<'_>, cov: &CoverageSrc<'a>, strip: Rect, bufs: &mut Offscreen) {
        let n = strip.width() as usize * strip.height() as usize;
        let acc = &mut bufs.cov[..n];
        acc.fill(255);
        if let Some((l, lf, m)) = cov.masks {
            let mut first = true;
            for mk in l.masks.iter().filter(|mk| mk.mode != MaskMode::None) {
                if first {
                    acc.fill(if mk.mode == MaskMode::Add { 0 } else { 255 });
                    first = false;
                }
                let tmp = &mut bufs.tmp[..n];
                tmp.fill(0);
                mk.pt.eval_into(lf, &mut self.s.pathdata);
                self.s.geo.clear();
                self.s.geo.path(&self.s.pathdata, false, 0, &Mat::IDENTITY);
                build_path(
                    &mut self.s.path,
                    &self.s.geo,
                    0,
                    0..u32::MAX,
                    &Mat::IDENTITY,
                    None,
                );
                if let Ok(buf) = DrawBuf::new_packed(tmp, ColorFormat::L8, strip) {
                    let mut child = Painter::new(buf, p.caches());
                    let dsc = VectorDsc {
                        transform: m.to_fx(),
                        ..VectorDsc::fill(Color::WHITE)
                    };
                    child.vector(&self.s.path, &dsc);
                }
                let o = u32::from(opa(mk.o.value(lf) / 100.0).0);
                for (a, &t) in acc.iter_mut().zip(bufs.tmp[..n].iter()) {
                    let t = if mk.inv { 255 - t } else { t };
                    let mv = div255(u32::from(t) * o);
                    let av = u32::from(*a);
                    *a = match mk.mode {
                        MaskMode::Add => (av + mv - div255(av * mv)) as u8,
                        MaskMode::Subtract => div255(av * (255 - mv)) as u8,
                        MaskMode::Intersect => div255(av * mv) as u8,
                        MaskMode::None => *a,
                    };
                }
            }
        }
        if let Some(mt) = cov.matte {
            let px = &mut bufs.matte[..n * 4];
            px.fill(0);
            if let Ok(buf) = DrawBuf::new_packed(px, ColorFormat::Argb8888, strip) {
                let mut child = Painter::new(buf, p.caches());
                self.layer(
                    &mut child,
                    mt.list,
                    mt.index,
                    mt.frame,
                    &mt.base,
                    1.0,
                    mt.precomp_depth,
                );
            }
            for (a, px) in acc.iter_mut().zip(bufs.matte[..n * 4].chunks_exact(4)) {
                let m = if mt.invert { 255 - px[3] } else { px[3] };
                *a = div255(u32::from(*a) * u32::from(m)) as u8;
            }
        }
    }

    /// Collects and paints the shapes of a shape layer.
    fn shapes(&mut self, p: &mut Painter<'_>, items: &'a [Shape], lf: f32, layer_m: &Mat, opa_mul: f32) {
        self.s.geo.clear();
        self.collect(items, 0, &Mat::IDENTITY, lf);
        let end: u32 = items.iter().map(Shape::size).fold(0, u32::saturating_add);
        self.paint_items(p, items, 0, end, &Mat::IDENTITY, layer_m, opa_mul, lf);
    }

    /// Builds the contours of `items` (numbered from `first_id`) in layer space; `mg` maps the
    /// group's space to layer space.
    fn collect(&mut self, items: &[Shape], first_id: u32, mg: &Mat, lf: f32) {
        let gstart = self.s.geo.contours.len();
        let mut id = first_id;
        for item in items {
            match item {
                Shape::Group(g) => {
                    let gm = mg.mul(&transform_matrix(&g.transform, lf));
                    self.collect(&g.items, id + 1, &gm, lf);
                }
                Shape::Rect(r) => {
                    let (pos, size, round) = (r.p.value(lf), r.s.value(lf), r.r.value(lf));
                    self.s.geo.rect(pos, size, round, r.reversed, id, mg);
                }
                Shape::Ellipse(e) => {
                    self.s
                        .geo
                        .ellipse(e.p.value(lf), e.s.value(lf), e.reversed, id, mg);
                }
                Shape::Path(ps) => {
                    ps.ks.eval_into(lf, &mut self.s.pathdata);
                    self.s.geo.path(&self.s.pathdata, ps.reversed, id, mg);
                }
                Shape::Trim(t) => {
                    let (s, e, o) = (t.s.value(lf), t.e.value(lf), t.o.value(lf));
                    modifier::trim(&mut self.s.geo, gstart, s, e, o, t.m, &mut self.s.tmp);
                }
                Shape::Repeater(rp) => {
                    let params = RepeatParams {
                        copies: rp.c.value(lf),
                        offset: rp.o.value(lf),
                        tr: TransformValues::eval(&rp.tr, lf),
                        start_opacity: rp.tr.start_opacity.value(lf),
                        end_opacity: rp.tr.end_opacity.value(lf),
                        composite: rp.m,
                    };
                    modifier::repeat(&mut self.s.geo, gstart, &params, mg, &mut self.s.tmp);
                }
                _ => {}
            }
            id = id.saturating_add(item.size());
        }
    }

    /// Paints the paint items of `items` (ids `first_id..end_id`) in reverse order.
    #[allow(clippy::too_many_arguments)]
    fn paint_items(
        &mut self,
        p: &mut Painter<'_>,
        items: &'a [Shape],
        first_id: u32,
        end_id: u32,
        mg: &Mat,
        layer_m: &Mat,
        opa_mul: f32,
        lf: f32,
    ) {
        let mut end = end_id;
        for item in items.iter().rev() {
            let id = end.saturating_sub(item.size());
            end = id;
            let owners = first_id..id;
            match item {
                Shape::Group(g) => {
                    let tv = transform_matrix(&g.transform, lf);
                    let go = clamp(g.transform.opacity.value(lf) / 100.0, 0.0, 1.0);
                    if go > 0.0 {
                        let gm = mg.mul(&tv);
                        let gend = id.saturating_add(g.size);
                        self.paint_items(p, &g.items, id + 1, gend, &gm, layer_m, opa_mul * go, lf);
                    }
                }
                Shape::Fill(f) => {
                    let o = f.o.value(lf) / 100.0 * opa_mul;
                    self.paint(
                        p,
                        PaintSpec::Solid(f.c.value(lf)),
                        Some(f.rule),
                        None,
                        o,
                        owners,
                        mg,
                        layer_m,
                        lf,
                    );
                }
                Shape::Stroke(s) => {
                    let o = s.o.value(lf) / 100.0 * opa_mul;
                    self.paint(
                        p,
                        PaintSpec::Solid(s.c.value(lf)),
                        None,
                        Some(&s.style),
                        o,
                        owners,
                        mg,
                        layer_m,
                        lf,
                    );
                }
                Shape::GradientFill(g) => {
                    let o = g.o.value(lf) / 100.0 * opa_mul;
                    self.paint(
                        p,
                        PaintSpec::Gradient(&g.g),
                        Some(g.rule),
                        None,
                        o,
                        owners,
                        mg,
                        layer_m,
                        lf,
                    );
                }
                Shape::GradientStroke(g) => {
                    let o = g.o.value(lf) / 100.0 * opa_mul;
                    self.paint(
                        p,
                        PaintSpec::Gradient(&g.g),
                        None,
                        Some(&g.style),
                        o,
                        owners,
                        mg,
                        layer_m,
                        lf,
                    );
                }
                _ => {}
            }
        }
    }

    /// Draws the contours owned by `owners` with a fill (`rule`) or a stroke (`style`).
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        p: &mut Painter<'_>,
        spec: PaintSpec<'_>,
        rule: Option<FillRule>,
        style: Option<&StrokeStyle>,
        opacity: f32,
        owners: core::ops::Range<u32>,
        mg: &Mat,
        layer_m: &Mat,
        lf: f32,
    ) {
        if !(opacity > 0.0) || owners.is_empty() {
            return;
        }
        let Some(inv) = mg.invert() else {
            return;
        };
        let paint = match spec {
            PaintSpec::Solid(c) => Paint::Solid(color(c)),
            PaintSpec::Gradient(g) => match self.gradient_paint(g, lf) {
                Some(p) => p,
                None => return,
            },
        };
        let mut dsc = VectorDsc {
            transform: layer_m.mul(mg).to_fx(),
            ..VectorDsc::default()
        };
        let mut paint = Some(paint);
        if let Some(rule) = rule {
            let r = match rule {
                FillRule::NonZero => twine_render::FillRule::NonZero,
                FillRule::EvenOdd => twine_render::FillRule::EvenOdd,
            };
            dsc.fill = paint.take().map(|p| (p, r));
        } else if let Some(st) = style {
            let w = st.w.value(lf);
            if w > 0.0 {
                dsc.stroke = paint.take().map(|p| (p, stroke(st, w, lf)));
            }
        }
        if dsc.fill.is_some() || dsc.stroke.is_some() {
            // One path per run of contours with equal repeater opacity.
            let RenderScratch { geo, path, .. } = &mut *self.s;
            let mut from = 0;
            while let Some(alpha) = build_path(path, geo, from, owners.clone(), &inv, Some(&mut from)) {
                dsc.opa = opa(opacity * alpha);
                p.vector(path, &dsc);
            }
        }
        // Give the gradient stops back for reuse.
        let used = paint.or(dsc.fill.map(|f| f.0)).or(dsc.stroke.map(|s| s.0));
        if let Some(
            Paint::Linear {
                stops: Stops::Owned(v),
                ..
            }
            | Paint::Radial {
                stops: Stops::Owned(v),
                ..
            },
        ) = used
        {
            self.s.stops = v;
        }
    }

    /// The `twine-vector` paint of gradient `g` at `lf` (group space).
    fn gradient_paint(&mut self, g: &Gradient, lf: f32) -> Option<Paint> {
        g.stops.eval_into(lf, &mut self.s.grad);
        let mut stops = take(&mut self.s.stops);
        build_stops(&self.s.grad, g.points as usize, &mut stops);
        if stops.is_empty() {
            self.s.stops = stops;
            return None;
        }
        let (s, e) = (g.s.value(lf), g.e.value(lf));
        let pt = |v: Vec2| FxPoint::new(fx(v[0]), fx(v[1]));
        Some(match g.kind {
            GradientKind::Linear => Paint::Linear {
                start: pt(s),
                end: pt(e),
                stops: Stops::Owned(stops),
                extend: GradExtend::Pad,
                transform: twine_core::Transform::IDENTITY,
            },
            GradientKind::Radial => {
                let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
                let r = sqrt(dx * dx + dy * dy);
                let h = clamp(g.h.value(lf) / 100.0, -0.99, 0.99);
                let focal = if h == 0.0 {
                    None
                } else {
                    let ang = atan2(dy, dx) + g.a.value(lf) * DEG;
                    Some(pt([s[0] + cos(ang) * r * h, s[1] + sin(ang) * r * h]))
                };
                Paint::Radial {
                    center: pt(s),
                    radius: fx(r),
                    focal,
                    stops: Stops::Owned(stops),
                    extend: GradExtend::Pad,
                    transform: twine_core::Transform::IDENTITY,
                }
            }
        })
    }
}

/// Grows `v` to at least `n` bytes (never shrinks).
fn grow(v: &mut Vec<u8>, n: usize) {
    if v.len() < n {
        v.resize(n, 0);
    }
}

#[inline]
fn div255(v: u32) -> u32 {
    (v + 128 + ((v + 128) >> 8)) >> 8
}

/// A float bounding box `(x0, y0, x1, y1)` as the covering integer rectangle (+1 px for
/// anti-aliasing), saturated to a sane range.
fn screen_rect(b: (f32, f32, f32, f32)) -> Rect {
    let lim = 1_000_000.0;
    let c = |v: f32| clamp(v, -lim, lim);
    Rect::new(
        round_i32(fmath::floor(c(b.0))) - 1,
        round_i32(fmath::floor(c(b.1))) - 1,
        round_i32(-fmath::floor(-c(b.2))) + 1,
        round_i32(-fmath::floor(-c(b.3))) + 1,
    )
}

fn color(c: Rgba) -> Color {
    Color::new(channel(c[0]), channel(c[1]), channel(c[2]))
}

/// The `twine-vector` stroke of `st` with width `w`.
fn stroke(st: &StrokeStyle, w: f32, lf: f32) -> VStroke {
    let mut pattern = [twine_core::Fx::ZERO; twine_vector::MAX_DASHES];
    let mut n = 0;
    let mut offset = 0.0;
    for d in &st.dash {
        let v = d.v.value(lf);
        match d.kind {
            DashKind::Offset => offset = v,
            DashKind::Dash | DashKind::Gap if n < pattern.len() => {
                pattern[n] = fx(v.max(0.0));
                n += 1;
            }
            _ => {}
        }
    }
    let dash = Some(Dash::new(&pattern[..n], fx(offset))).filter(Dash::is_valid);
    VStroke {
        width: fx(w),
        join: match st.lj {
            LineJoin::Miter => VJoin::Miter,
            LineJoin::Round => VJoin::Round,
            LineJoin::Bevel => VJoin::Bevel,
        },
        cap: match st.lc {
            LineCap::Butt => VCap::Butt,
            LineCap::Round => VCap::Round,
            LineCap::Square => VCap::Square,
        },
        miter_limit: fx(st.ml),
        dash,
    }
}

/// Builds into `path` the next run of contours of `geo` (from contour index `*next` or `from`)
/// whose owner is in `owners` and whose alpha equals the run's first contour, mapped by `m`.
/// Returns the run's alpha, or `None` when no contour is left. With `next = None`, all matching
/// contours are added regardless of alpha.
fn build_path(
    path: &mut Path,
    geo: &Geometry,
    from: usize,
    owners: core::ops::Range<u32>,
    m: &Mat,
    next: Option<&mut usize>,
) -> Option<f32> {
    path.clear();
    let pt = |v: Vec2| {
        let q = m.apply(v);
        FxPoint::new(fx(q[0]), fx(q[1]))
    };
    let mut alpha: Option<f32> = None;
    let mut i = from;
    let by_alpha = next.is_some();
    while let Some(c) = geo.contours.get(i) {
        if owners.contains(&c.owner) {
            match alpha {
                Some(a) if by_alpha && a != c.alpha => break,
                None => alpha = Some(c.alpha),
                _ => {}
            }
            let pts = c.pts(&geo.pts);
            path.move_to(pt(pts[0]));
            for s in pts[1..].chunks_exact(3) {
                let (p0, c1, c2, p1) = (path.current_point(), pt(s[0]), pt(s[1]), pt(s[2]));
                if c1 == p0 && c2 == p1 {
                    path.line_to(p1);
                } else {
                    path.cubic_to(c1, c2, p1);
                }
            }
            if c.closed {
                path.close();
            }
        }
        i += 1;
    }
    if let Some(n) = next {
        *n = i;
    }
    alpha
}

/// Lottie's flat gradient data → sorted stops: `points` × `[offset, r, g, b]`, then optional
/// `[offset, alpha]` pairs merged in (color and alpha interpolated at every offset).
fn build_stops(data: &[f32], points: usize, out: &mut Vec<GradStop>) {
    out.clear();
    let points = points.min(data.len() / 4);
    if points == 0 {
        return;
    }
    let colors = &data[..points * 4];
    let alphas = &data[points * 4..];
    let alphas = &alphas[..alphas.len() / 2 * 2];
    let frac = |o: f32| round_i32(clamp(o, 0.0, 1.0) * 255.0) as u8;
    let color_at = |o: f32| -> Rgba {
        let n = colors.len() / 4;
        let c = |k: usize| [colors[4 * k + 1], colors[4 * k + 2], colors[4 * k + 3], 1.0];
        if o <= colors[0] {
            return c(0);
        }
        for k in 1..n {
            let (o0, o1) = (colors[4 * (k - 1)], colors[4 * k]);
            if o <= o1 {
                let t = if o1 > o0 { (o - o0) / (o1 - o0) } else { 1.0 };
                let (a, b) = (c(k - 1), c(k));
                return core::array::from_fn(|i| mix(a[i], b[i], t));
            }
        }
        c(n - 1)
    };
    let alpha_at = |o: f32| -> f32 {
        let n = alphas.len() / 2;
        if n == 0 {
            return 1.0;
        }
        if o <= alphas[0] {
            return alphas[1];
        }
        for k in 1..n {
            let (o0, o1) = (alphas[2 * (k - 1)], alphas[2 * k]);
            if o <= o1 {
                let t = if o1 > o0 { (o - o0) / (o1 - o0) } else { 1.0 };
                return mix(alphas[2 * k - 1], alphas[2 * k + 1], t);
            }
        }
        alphas[2 * n - 1]
    };
    // Merge the (sorted) color and alpha offsets.
    let (mut i, mut j) = (0, 0);
    let (nc, na) = (colors.len() / 4, alphas.len() / 2);
    while i < nc || j < na {
        let oc = if i < nc { colors[4 * i] } else { f32::INFINITY };
        let oa = if j < na { alphas[2 * j] } else { f32::INFINITY };
        let o = oc.min(oa);
        if oc <= oa {
            i += 1;
        }
        if oa <= oc {
            j += 1;
        }
        let c = color_at(o);
        let stop = GradStop::with_opa(color(c), opa(alpha_at(o)), frac(o));
        if out.last().is_some_and(|l: &GradStop| l.frac > stop.frac) {
            continue; // unsorted input: skip stops that go backwards
        }
        out.push(stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stops(data: &[f32], points: usize) -> Vec<GradStop> {
        let mut v = Vec::new();
        build_stops(data, points, &mut v);
        v
    }

    #[test]
    fn linear_gradient_stops_parsed() {
        // Two color stops (red at 0, blue at 1), no alpha.
        let s = stops(&[0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0], 2);
        assert_eq!(
            s,
            [
                GradStop::new(Color::new(255, 0, 0), 0),
                GradStop::new(Color::new(0, 0, 255), 255)
            ]
        );
        // Three color stops plus alpha stops at 0 (opaque) and 1 (transparent): the alpha
        // offsets coincide with color offsets, the middle color gets interpolated alpha.
        let s = stops(
            &[
                0.0, 1.0, 0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0,
            ],
            3,
        );
        assert_eq!(s.len(), 3);
        assert_eq!(s[1].color, Color::new(0, 255, 0));
        assert_eq!(s[1].frac, 128);
        assert_eq!(s[1].opa, Opa(128));
        assert_eq!((s[0].opa, s[2].opa), (Opa::COVER, Opa(0)));
        // Alpha stop between color stops: merged in with the interpolated color.
        let s = stops(
            &[
                0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.5, 1.0, 1.0,
            ],
            2,
        );
        assert_eq!(s.iter().map(|g| g.frac).collect::<Vec<_>>(), [0, 128, 255]);
        assert_eq!(s[1].color, Color::new(128, 128, 128));
        assert_eq!(s[1].opa, Opa(128));
        // Garbage: too few values → no stops.
        assert!(stops(&[0.0, 1.0], 2).is_empty());
        assert!(stops(&[], 0).is_empty());
    }

    #[test]
    fn radial_highlight_focal() {
        let json = br#"{"fr":30,"ip":0,"op":30,"w":100,"h":100,"layers":[{"ty":4,"ks":{},"shapes":[
            {"ty":"el","p":{"k":[50,50]},"s":{"k":[80,80]}},
            {"ty":"gf","t":2,"s":{"k":[50,50]},"e":{"k":[90,50]},"h":{"k":50},"a":{"k":90},
             "g":{"p":2,"k":{"k":[0,1,1,1,1,0,0,0]}},"o":{"k":100}}]}]}"#;
        let comp = crate::load(json).unwrap();
        let crate::model::Shape::GradientFill(gf) = &comp.layers[0].shapes[1] else {
            panic!("not a gradient fill");
        };
        let mut scratch = RenderScratch::default();
        let mut r = Renderer {
            comp: &comp,
            s: &mut scratch,
            depth: 0,
            work: 0,
        };
        let Some(Paint::Radial {
            center,
            radius,
            focal,
            stops,
            ..
        }) = r.gradient_paint(&gf.g, 0.0)
        else {
            panic!("not radial");
        };
        assert_eq!(center, FxPoint::from_int(50, 50));
        assert_eq!(radius, twine_core::Fx::from_int(40));
        // Highlight 50 % of the radius at 90° from the start→end direction (+x): straight down.
        let f = focal.unwrap();
        assert!((f.x.0 - (50 << 16)).abs() < 64, "{f:?}");
        assert!((f.y.0 - (70 << 16)).abs() < 64, "{f:?}");
        assert_eq!(stops.as_slice().len(), 2);
    }
}
