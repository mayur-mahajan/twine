//! [`Painter`]: clipping, masks, acceleration and the span pipeline every primitive uses.

use alloc::vec::Vec;
use core::mem::take;

use twine_core::{Color, ColorFormat, Opa, Rect};

use crate::blend::{BlendMode, Source, blend_row};
use crate::gradient::GradSampler;
use crate::mask::{Mask, MaskId, MaskResult, MaskStack};
use crate::{AccelResult, DrawAccel, DrawBuf, RenderCaches};

/// Draws into a [`DrawBuf`] with a clip rectangle, a mask stack and an optional accelerator.
///
/// All coordinates are absolute screen coordinates. Nothing is drawn outside the clip (which is
/// always inside the buffer's area). Drawing never allocates: scratch memory comes from the
/// [`RenderCaches`].
///
/// ```
/// use twine_core::{Color, ColorFormat, Opa, Rect};
/// use twine_render::{DrawBuf, Painter, RenderCaches};
///
/// let mut caches = RenderCaches::default();
/// let mut data = vec![0u8; 8 * 8];
/// let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 8, 8)).unwrap();
/// let mut p = Painter::new(buf, &mut caches);
/// p.with_clip(Rect::from_xywh(0, 0, 4, 8), |p| p.fill(Rect::from_xywh(0, 0, 8, 1), Color::WHITE, Opa::COVER));
/// assert_eq!(&data[..8], &[255, 255, 255, 255, 0, 0, 0, 0]);
/// ```
pub struct Painter<'a> {
    buf: DrawBuf<'a>,
    clip: Rect,
    accel: Option<&'a mut dyn DrawAccel>,
    caches: &'a mut RenderCaches,
    masks: MaskStack<'a>,
    accel_pending: bool,
    touched: Option<Rect>,
}

impl core::fmt::Debug for Painter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Painter")
            .field("buf", &self.buf)
            .field("clip", &self.clip)
            .field("accel", &self.accel.is_some())
            .field("masks", &self.masks.len())
            .finish_non_exhaustive()
    }
}

/// Scratch buffers taken out of the caches while a primitive runs.
pub(crate) struct Scratch {
    pub cov: Vec<u8>,
    pub span: Vec<u8>,
}

/// Fills a chunk's coverage: `f(chunk_x0, coverage)`.
pub(crate) type CovFn<'f> = &'f mut dyn FnMut(i32, &mut [u8]);

/// What a span is painted with (resolved per chunk into a [`Source`]).
#[derive(Clone, Copy)]
pub(crate) enum Paint<'p> {
    /// One color.
    Solid(Color),
    /// Pixels whose first pixel is at screen x `x0`.
    Src { src: Source<'p>, x0: i32 },
    /// A gradient sampled per pixel.
    Grad {
        s: &'p GradSampler,
        map: &'p [u32],
        dither: bool,
    },
}

impl<'a> Painter<'a> {
    /// A painter over `buf` whose clip is the buffer's area.
    pub fn new(buf: DrawBuf<'a>, caches: &'a mut RenderCaches) -> Self {
        let clip = buf.area();
        Self {
            buf,
            clip,
            accel: None,
            caches,
            masks: MaskStack::default(),
            accel_pending: false,
            touched: None,
        }
    }

    /// Uses `accel` for the operations it supports.
    #[must_use]
    pub fn with_accel(mut self, accel: &'a mut dyn DrawAccel) -> Self {
        self.accel = Some(accel);
        self
    }

    /// The current clip rectangle (may be empty).
    #[must_use]
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Runs `f` with the clip intersected with `clip`, then restores the previous clip.
    pub fn with_clip<R>(&mut self, clip: Rect, f: impl FnOnce(&mut Self) -> R) -> R {
        let old = self.clip;
        self.clip = old
            .intersection(&clip)
            .unwrap_or(Rect::new(old.x0, old.y0, old.x0, old.y0));
        let r = f(self);
        self.clip = old;
        r
    }

    /// The draw buffer. Call [`sync_accel`](Self::sync_accel) before reading pixels if an
    /// accelerator may still be working.
    #[must_use]
    pub fn buf(&self) -> &DrawBuf<'a> {
        &self.buf
    }

    /// The draw buffer, mutably (waits for queued accelerator work first).
    pub fn buf_mut(&mut self) -> &mut DrawBuf<'a> {
        self.sync_accel();
        &mut self.buf
    }

    /// The caches.
    pub fn caches(&mut self) -> &mut RenderCaches {
        self.caches
    }

    /// Waits for queued accelerator operations (called before every software write).
    pub fn sync_accel(&mut self) {
        if self.accel_pending {
            if let Some(a) = self.accel.as_deref_mut() {
                a.wait();
            }
            self.accel_pending = false;
        }
    }

    /// Union of all areas drawn since the painter was created (`None` if nothing was drawn).
    #[must_use]
    pub fn touched_area(&self) -> Option<Rect> {
        self.touched
    }

    /// Pushes a mask applied to everything drawn until it is popped. When the stack is full
    /// ([`MAX_MASKS`](crate::MAX_MASKS)) a warning is logged and [`MaskId::INVALID`] returned.
    pub fn push_mask(&mut self, m: Mask<'a>) -> MaskId {
        if let Some(id) = self.masks.push(m) {
            id
        } else {
            twine_core::warn!(target: "twine::render", "mask stack full; mask ignored");
            MaskId::INVALID
        }
    }

    /// Removes a mask pushed with [`push_mask`](Self::push_mask) (unknown ids are ignored).
    pub fn pop_mask(&mut self, id: MaskId) {
        self.masks.pop(id);
    }

    /// Whether masks are active.
    #[must_use]
    pub fn has_masks(&self) -> bool {
        !self.masks.is_empty()
    }

    /// Whether the buffer is one of the RGB565 formats (where gradients may dither).
    pub(crate) fn is_565(&self) -> bool {
        matches!(
            self.buf.format(),
            ColorFormat::Rgb565 | ColorFormat::Rgb565Swapped
        )
    }

    pub(crate) fn take_scratch(&mut self) -> Scratch {
        Scratch {
            cov: take(&mut self.caches.cov),
            span: take(&mut self.caches.span),
        }
    }

    pub(crate) fn put_scratch(&mut self, s: Scratch) {
        self.caches.cov = s.cov;
        self.caches.span = s.span;
    }

    pub(crate) fn caches_mut(&mut self) -> &mut RenderCaches {
        self.caches
    }

    /// Applies the mask stack to `cov` (row `y`, starting at screen x `x0`).
    pub(crate) fn apply_masks(&self, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
        if self.masks.is_empty() {
            MaskResult::FullCover
        } else {
            self.masks.apply(y, x0, cov)
        }
    }

    /// Adds `r` to the touched area (for primitives that write pixels directly).
    pub(crate) fn mark_touched(&mut self, r: Rect) {
        self.touched = Some(self.touched.map_or(r, |t| t.union(&r)));
    }

    /// Blends an already clipped span straight into the buffer.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn raw_blend(
        &mut self,
        y: i32,
        x0: i32,
        x1: i32,
        src: Source<'_>,
        cov: Option<&[u8]>,
        opa: Opa,
        mode: BlendMode,
    ) {
        self.sync_accel();
        let r = Rect::new(x0, y, x1, y + 1);
        self.touched = Some(self.touched.map_or(r, |t| t.union(&r)));
        blend_row(&mut self.buf, y, x0, x1, src, cov, opa, mode);
    }

    /// The single entry point of every primitive: blends `[x0, x1)` of row `y` with the
    /// primitive's own coverage `cov` (indexed from `x0`, or `None` for full coverage) after
    /// clipping and applying the mask stack.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blend_masked_span(
        &mut self,
        y: i32,
        x0: i32,
        x1: i32,
        src: Source<'_>,
        cov: Option<&mut [u8]>,
        opa: Opa,
        mode: BlendMode,
    ) {
        if opa.is_transparent() || y < self.clip.y0 || y >= self.clip.y1 {
            return;
        }
        let cx0 = x0.max(self.clip.x0);
        let cx1 = x1.min(self.clip.x1);
        if cx0 >= cx1 {
            return;
        }
        let off = (cx0 - x0) as usize;
        let src = src.offset(off);
        match cov {
            Some(c) => {
                let end = (off + (cx1 - cx0) as usize).min(c.len());
                if end <= off {
                    return;
                }
                let c = &mut c[off..end];
                let cx1 = cx0 + c.len() as i32;
                if !self.masks.is_empty() && self.masks.apply(y, cx0, c) == MaskResult::Transparent {
                    return;
                }
                self.raw_blend(y, cx0, cx1, src, Some(c), opa, mode);
            }
            None if self.masks.is_empty() => self.raw_blend(y, cx0, cx1, src, None, opa, mode),
            None => {
                let mut m = take(&mut self.caches.mask);
                let chunk = m.len().max(1) as i32;
                let mut x = cx0;
                while x < cx1 {
                    let e = (x + chunk).min(cx1);
                    let buf = &mut m[..(e - x) as usize];
                    buf.fill(255);
                    let s = src.offset((x - cx0) as usize);
                    match self.masks.apply(y, x, buf) {
                        MaskResult::Transparent => {}
                        MaskResult::FullCover => self.raw_blend(y, x, e, s, None, opa, mode),
                        MaskResult::Changed => self.raw_blend(y, x, e, s, Some(buf), opa, mode),
                    }
                    x = e;
                }
                self.caches.mask = m;
            }
        }
    }

    /// Paints `[x0, x1)` of row `y` with `paint`, in chunks of at most `max_span` pixels. When
    /// `cov` is given it fills each chunk's coverage (`cov(chunk_x0, chunk)`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn span(
        &mut self,
        sc: &mut Scratch,
        y: i32,
        x0: i32,
        x1: i32,
        paint: &Paint<'_>,
        opa: Opa,
        mode: BlendMode,
        mut cov: Option<CovFn<'_>>,
    ) {
        if y < self.clip.y0 || y >= self.clip.y1 || opa.is_transparent() {
            return;
        }
        let a0 = x0.max(self.clip.x0);
        let a1 = x1.min(self.clip.x1);
        if a0 >= a1 {
            return;
        }
        // A vertical gradient is one color per row.
        if let Paint::Grad {
            s,
            map,
            dither: false,
        } = paint
        {
            if s.is_row_constant() {
                let v = s.sample(map, a0, y);
                let c = Color::hex(v);
                let o = Opa((v >> 24) as u8).mul(opa);
                return self.span(sc, y, a0, a1, &Paint::Solid(c), o, mode, cov);
            }
        }
        let chunk = sc.cov.len().max(1) as i32;
        let mut a = a0;
        while a < a1 {
            let b = (a + chunk).min(a1);
            let k = (b - a) as usize;
            let src = match *paint {
                Paint::Solid(c) => Source::Solid(c),
                Paint::Src { src, x0 } => {
                    if a < x0 {
                        a = b;
                        continue;
                    }
                    src.offset((a - x0) as usize)
                }
                Paint::Grad { s, map, dither } => {
                    s.fill_row(map, y, a, k, dither, &mut sc.span);
                    Source::Argb(&sc.span[..k * 4])
                }
            };
            let c = match cov.as_deref_mut() {
                Some(f) => {
                    let buf = &mut sc.cov[..k];
                    f(a, buf);
                    Some(buf)
                }
                None => None,
            };
            self.blend_masked_span(y, a, b, src, c, opa, mode);
            a = b;
        }
    }

    /// Fills `area` with `color` at `opa` (clipped, masked). Large unmasked fills use the
    /// accelerator when one is set and supports it.
    pub fn fill(&mut self, area: Rect, color: Color, opa: Opa) {
        self.fill_mode(area, color, opa, BlendMode::Normal);
    }

    /// [`fill`](Self::fill) with a blend mode.
    pub fn fill_mode(&mut self, area: Rect, color: Color, opa: Opa, mode: BlendMode) {
        if opa.is_transparent() {
            return;
        }
        let Some(a) = area.intersection(&self.clip) else {
            return;
        };
        twine_core::trace!(target: "twine::render", "fill {} {} opa {}", a, color, opa.0);
        if mode == BlendMode::Normal
            && self.masks.is_empty()
            && a.area() >= ACCEL_MIN_PX
            && self
                .accel_op(a, |acc, buf| acc.fill(buf, a, color, opa))
                .is_some_and(|r| r != AccelResult::Unsupported)
        {
            return;
        }
        for y in a.y0..a.y1 {
            self.blend_masked_span(y, a.x0, a.x1, Source::Solid(color), None, opa, mode);
        }
    }

    /// Runs an accelerator operation on `area` (`None` without an accelerator). `Done` and
    /// `Queued` mark the area as touched; `Queued` makes the next software write wait.
    pub(crate) fn accel_op(
        &mut self,
        area: Rect,
        f: impl FnOnce(&mut dyn DrawAccel, &mut DrawBuf<'_>) -> AccelResult,
    ) -> Option<AccelResult> {
        let acc = self.accel.as_deref_mut()?;
        let r = f(acc, &mut self.buf);
        match r {
            AccelResult::Done => {}
            AccelResult::Queued => self.accel_pending = true,
            AccelResult::Unsupported => return Some(r),
        }
        self.touched = Some(self.touched.map_or(area, |t| t.union(&area)));
        Some(r)
    }
}

/// Smallest area (in pixels) handed to [`DrawAccel`] operations.
pub const ACCEL_MIN_PX: u64 = 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_clip_restores_clip() {
        let mut caches = RenderCaches::default();
        let mut d = [0u8; 100];
        let buf = DrawBuf::new_packed(&mut d, ColorFormat::L8, Rect::from_xywh(0, 0, 10, 10)).unwrap();
        let mut p = Painter::new(buf, &mut caches);
        assert_eq!(p.clip(), Rect::from_xywh(0, 0, 10, 10));
        let inner = p.with_clip(Rect::from_xywh(2, 3, 20, 4), |p| {
            p.with_clip(Rect::from_xywh(0, 0, 5, 5), |p| p.clip())
        });
        assert_eq!(inner, Rect::new(2, 3, 5, 5));
        assert_eq!(p.clip(), Rect::from_xywh(0, 0, 10, 10));
    }

    #[test]
    fn with_clip_empty_intersection_is_empty() {
        let mut caches = RenderCaches::default();
        let mut d = [0u8; 100];
        let buf = DrawBuf::new_packed(&mut d, ColorFormat::L8, Rect::from_xywh(0, 0, 10, 10)).unwrap();
        let mut p = Painter::new(buf, &mut caches);
        p.with_clip(Rect::from_xywh(50, 50, 5, 5), |p| {
            assert!(p.clip().is_empty());
            p.fill(Rect::from_xywh(0, 0, 10, 10), Color::WHITE, Opa::COVER);
        });
        assert!(p.touched_area().is_none());
        let _ = p;
        assert!(d.iter().all(|&v| v == 0));
    }
}
