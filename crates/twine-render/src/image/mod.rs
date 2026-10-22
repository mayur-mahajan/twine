//! Image drawing: [`Painter::image`] with [`ImageDsc`] (opacity, recolor, chroma key, tiling,
//! clip radius, bitmap mask, rotation / scale around a pivot) and [`transformed_area`].
//!
//! The untransformed path picks, per image, the cheapest way to produce a span:
//!
//! 1. **direct** — `Rgb565`, `Rgb565Swapped`, `Rgb888`, `Xrgb8888`, `L8` and straight
//!    `Argb8888` rows are handed to the blender as they are (same format, opaque, no mask →
//!    `copy_from_slice`; `Argb8888` → per-pixel alpha);
//! 2. **alpha as coverage** — `A1`…`A8` (a solid color through the alpha) and `Rgb565A8`
//!    (the RGB565 plane through its alpha plane);
//! 3. **generic** — everything else (indexed, premultiplied, recolored, chroma-keyed) is read
//!    into the `Argb8888` scratch span first.
//!
//! The transformed path (angle ≠ 0 or scale ≠ 256) runs the inverse-mapping sampler of
//! [`Painter::blit_transformed`]; tiling is ignored there (LVGL behaviour).

pub(crate) mod read;

use core::num::NonZeroU32;
use core::sync::atomic::AtomicBool;

use twine_core::color::{expand_alpha, unpack_bits};
use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Point, Rect, Scale, Transform};

use crate::blend::{BlendMode, Source};
use crate::dispatch::warn_once;
use crate::mask::{Mask, MaskResult};
use crate::num::{advance, mul8, span};
use crate::painter::Scratch;
use crate::transform_blit::is_direct;
use crate::{BlitDsc, ImagePixels, Painter};
use read::{TexelOps, pack, with_texel};

/// How [`Painter::image`] draws an image (LVGL `lv_draw_image_dsc_t`).
///
/// ```
/// use twine_core::{Angle, Opa};
/// use twine_render::ImageDsc;
/// let dsc = ImageDsc { opa: Opa::pct(50), angle: Angle::deg(30), ..ImageDsc::default() };
/// assert!(dsc.is_transformed());
/// assert!(!ImageDsc::default().is_transformed());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDsc<'a> {
    /// Overall opacity.
    pub opa: Opa,
    /// Recolor target. Alpha-only images (`A1`…`A8`) are drawn in this color when
    /// `recolor_opa` is not transparent, else in black.
    pub recolor: Color,
    /// How strongly colors are mixed towards `recolor` (transparent = no recolor).
    pub recolor_opa: Opa,
    /// Pixels of exactly this color (compared as 24-bit RGB after reading the pixel) are
    /// transparent.
    pub chroma_key: Option<Color>,
    /// Repeat the image to fill the whole area (untransformed images only).
    pub tile: bool,
    /// Corner radius of a rounded clip on the draw area (0 = none).
    pub clip_radius: i32,
    /// Bilinear filtering when transformed (else nearest neighbour).
    pub antialias: bool,
    /// Clockwise rotation around `pivot`.
    pub angle: Angle,
    /// Horizontal scale (256 = 1×).
    pub scale_x: Scale,
    /// Vertical scale (256 = 1×).
    pub scale_y: Scale,
    /// Rotation / scale center, relative to the draw area's top-left corner.
    pub pivot: Point,
    /// Blend mode.
    pub blend_mode: BlendMode,
    /// Alpha mask (`A1`…`A8` or `L8`) placed at the draw area's top-left corner; pixels
    /// outside it are transparent.
    pub bitmap_mask: Option<ImagePixels<'a>>,
}

impl Default for ImageDsc<'_> {
    fn default() -> Self {
        Self {
            opa: Opa::COVER,
            recolor: Color::BLACK,
            recolor_opa: Opa::TRANSP,
            chroma_key: None,
            tile: false,
            clip_radius: 0,
            antialias: true,
            angle: Angle::deci_deg(0),
            scale_x: Scale::ONE,
            scale_y: Scale::ONE,
            pivot: Point::new(0, 0),
            blend_mode: BlendMode::Normal,
            bitmap_mask: None,
        }
    }
}

impl ImageDsc<'_> {
    /// Whether the image is rotated or scaled (`angle` is taken modulo 360°).
    #[must_use]
    pub fn is_transformed(&self) -> bool {
        self.angle.normalized().as_deci_deg() != 0 || self.scale_x != Scale::ONE || self.scale_y != Scale::ONE
    }

    /// The texel operations for an image of `format`.
    fn texel_ops(&self, format: ColorFormat) -> TexelOps {
        let recolor = if self.recolor_opa.is_transparent() {
            None
        } else if format.is_alpha_only() {
            Some((self.recolor, Opa::COVER))
        } else {
            Some((self.recolor, self.recolor_opa))
        };
        TexelOps {
            recolor,
            chroma: self.chroma_key.map(|c| pack(c, 0)),
        }
    }
}

/// Bounding box of `area` rotated by `angle` and scaled by `scale_x`/`scale_y` around `pivot`
/// (relative to `area`'s top-left corner), rounded outwards. Widgets use it to enlarge their
/// drawing area for transformed images.
///
/// ```
/// use twine_core::{Angle, Point, Rect, Scale};
/// use twine_render::transformed_area;
/// let a = Rect::from_xywh(0, 0, 100, 50);
/// let r = transformed_area(a, Angle::deg(90), Scale::ONE, Scale::ONE, Point::new(50, 25));
/// assert_eq!((r.width(), r.height()), (50, 100));
/// ```
#[must_use]
pub fn transformed_area(area: Rect, angle: Angle, scale_x: Scale, scale_y: Scale, pivot: Point) -> Rect {
    if angle.normalized().as_deci_deg() == 0 && scale_x == Scale::ONE && scale_y == Scale::ONE {
        return area;
    }
    let p = Point::new(area.x0.saturating_add(pivot.x), area.y0.saturating_add(pivot.y));
    Transform::from_rotate_scale(angle, scale_x, scale_y, p).map_rect_bounds(area)
}

static WARN_MASK: AtomicBool = AtomicBool::new(false);

/// Destination-space coverage of an image: clip radius and bitmap mask.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageMasks<'m> {
    radius: Option<Mask<'static>>,
    bitmap: Option<(ImagePixels<'m>, Rect)>,
    area: Rect,
}

impl<'m> ImageMasks<'m> {
    /// The masks of `dsc` for an image drawn in `area` (`None` when there are none).
    fn new(area: Rect, dsc: &ImageDsc<'m>) -> Option<Self> {
        let radius = (dsc.clip_radius > 0).then_some(Mask::Radius {
            area,
            radius: dsc.clip_radius,
            outer: false,
        });
        let bitmap = dsc.bitmap_mask.and_then(|m| {
            if m.format.is_alpha_only() || m.format == ColorFormat::L8 {
                Some((m, m.rect_at(area.x0, area.y0)))
            } else {
                if warn_once(&WARN_MASK) {
                    twine_core::warn!(target: "twine::render", "image bitmap mask must be A1..A8 or L8, not {}; ignored", m.format);
                }
                None
            }
        });
        (radius.is_some() || bitmap.is_some()).then_some(Self { radius, bitmap, area })
    }

    /// Where anything can be visible.
    pub(crate) fn bounds(&self) -> Option<Rect> {
        let r = if self.radius.is_some() {
            self.area
        } else {
            Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2)
        };
        match self.bitmap {
            Some((_, b)) => r.intersection(&b),
            None => Some(r),
        }
    }

    /// Writes the coverage of `[x0, x0 + cov.len())` on row `y`; `false` when all transparent.
    pub(crate) fn fill(&self, y: i32, x0: i32, cov: &mut [u8]) -> bool {
        cov.fill(255);
        if let Some(m) = &self.radius {
            if m.apply(y, x0, cov) == MaskResult::Transparent {
                return false;
            }
        }
        if let Some((m, r)) = &self.bitmap {
            if y < r.y0 || y >= r.y1 {
                return false;
            }
            // `r` is the mask's rectangle (at most `u16::MAX` rows).
            let row = m.row(u16::try_from(span(r.y0, y)).unwrap_or(u16::MAX));
            let bpp = m.format.bpp();
            // The coverage ends before `i32::MAX`: the bounded range covers it.
            for (c, x) in cov.iter_mut().zip(x0..i32::MAX) {
                let a = if x < r.x0 || x >= r.x1 {
                    0
                } else {
                    let v = unpack_bits(row, bpp, span(r.x0, x));
                    if m.format == ColorFormat::L8 {
                        v
                    } else {
                        expand_alpha(v, bpp)
                    }
                };
                *c = mul8(*c, a);
            }
        }
        true
    }
}

/// How the untransformed path reads an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImagePath {
    /// Rows are a [`Source::Pixels`].
    Direct,
    /// `A1`…`A8`: a solid color through the alpha values.
    AlphaSolid(Color),
    /// `Rgb565A8`: the RGB565 plane through the alpha plane.
    Rgb565A8,
    /// Read into the `Argb8888` scratch span.
    Generic,
}

impl ImagePath {
    fn choose(px: &ImagePixels<'_>, ops: TexelOps) -> Self {
        let premul = px.is_premultiplied();
        if ops.chroma.is_none() {
            if px.format.is_alpha_only() {
                return ImagePath::AlphaSolid(ops.recolor.map_or(Color::BLACK, |(c, _)| c));
            }
            if ops.recolor.is_none() && !premul {
                if is_direct(px.format) {
                    return ImagePath::Direct;
                }
                if px.format == ColorFormat::Rgb565A8 {
                    return ImagePath::Rgb565A8;
                }
            }
        }
        ImagePath::Generic
    }
}

/// One run of an untransformed image: screen `[x, e)` of row `y`, source column `ix`, row `iy`.
#[derive(Clone, Copy)]
struct Run {
    y: i32,
    x: i32,
    e: i32,
    ix: usize,
    iy: u16,
}

impl Painter<'_> {
    /// Draws the image `px` into `area` with `dsc`.
    ///
    /// Untransformed, the image's top-left corner is at `area`'s; pixels of `area` beyond the
    /// image are left untouched unless `dsc.tile` repeats the image. Transformed (angle ≠ 0 or
    /// scale ≠ 256), the image is rotated/scaled around `area.origin + dsc.pivot` and may draw
    /// outside `area` (see [`transformed_area`]); the clip radius and bitmap mask stay on
    /// `area`. Every [`ColorFormat`] can be drawn into every enabled buffer format. Never
    /// allocates.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_render::{DrawBuf, ImageDsc, ImagePixels, Painter, RenderCaches};
    ///
    /// let mut caches = RenderCaches::default();
    /// let mut data = vec![0u8; 4 * 4];
    /// let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 4, 4)).unwrap();
    /// let mut p = Painter::new(buf, &mut caches);
    /// // A 2×1 image tiled over the top row.
    /// let img = ImagePixels::new(ColorFormat::L8, 2, 1, &[10, 20]);
    /// p.image(Rect::from_xywh(0, 0, 4, 1), &img, &ImageDsc { tile: true, ..ImageDsc::default() });
    /// drop(p);
    /// assert_eq!(&data[..4], &[10, 20, 10, 20]);
    /// ```
    pub fn image(&mut self, area: Rect, px: &ImagePixels<'_>, dsc: &ImageDsc<'_>) {
        if dsc.opa.is_transparent() || px.w == 0 || px.h == 0 || area.is_empty() {
            return;
        }
        let masks = ImageMasks::new(area, dsc);
        let ops = dsc.texel_ops(px.format);
        if dsc.is_transformed() {
            let pivot = Point::new(
                area.x0.saturating_add(dsc.pivot.x),
                area.y0.saturating_add(dsc.pivot.y),
            );
            let t = Transform::translate(Fx::from_int(area.x0), Fx::from_int(area.y0)).then(
                Transform::from_rotate_scale(dsc.angle, dsc.scale_x, dsc.scale_y, pivot),
            );
            let b = BlitDsc {
                transform: t,
                opa: dsc.opa,
                antialias: dsc.antialias,
                blend_mode: dsc.blend_mode,
                recolor: ops.recolor,
            };
            self.blit_transformed_with(px, &b, ops, masks.as_ref());
            return;
        }
        let img = px.rect_at(area.x0, area.y0);
        let drawn = if dsc.tile {
            Some(area)
        } else {
            area.intersection(&img)
        };
        let Some(mut draw) = drawn.and_then(|d| d.intersection(&self.clip())) else {
            return;
        };
        if let Some(m) = &masks {
            match m.bounds().and_then(|b| b.intersection(&draw)) {
                Some(d) => draw = d,
                None => return,
            }
        }
        let plain = masks.is_none() && ops == TexelOps::default() && dsc.blend_mode == BlendMode::Normal;
        if plain
            && !dsc.tile
            && draw == img
            && !self.has_masks()
            && self.accel_blit(img, px, dsc.opa) == Some(true)
        {
            return;
        }
        let path = ImagePath::choose(px, ops);
        let copy = path == ImagePath::Direct
            && plain
            && !self.has_masks()
            && dsc.opa.is_cover()
            && px.format == self.buf().format()
            && !px.format.has_alpha();
        // `px.w`, `px.h` are not 0 (returned at the top of `image`): the fallbacks are never
        // taken. A remainder by a `NonZeroU32` needs no check at all.
        let (Some(w), Some(h)) = (NonZeroU32::new(u32::from(px.w)), NonZeroU32::new(u32::from(px.h))) else {
            return;
        };
        let mut sc = self.take_scratch();
        let chunk = sc.cov.len().max(1);
        // `draw` lies inside `area` (`area` itself or the part covered by the image), so the
        // offsets from `area`'s origin are non-negative (`abs_diff` is the exact offset).
        for y in draw.y0..draw.y1 {
            // The remainder is below `h ≤ u16::MAX`.
            // DIV: `h` is a `NonZeroU32` (the `let` above).
            let iy = (y.abs_diff(area.y0) % h) as u16;
            if copy {
                let c = &mut self.caches_mut().image_copy_rows;
                *c = c.wrapping_add(1);
            }
            let mut x = draw.x0;
            while x < draw.x1 {
                // DIV: `w` is a `NonZeroU32` (the `let` above).
                let ix = x.abs_diff(area.x0) % w;
                // SAFE-ARITH: `ix < w`.
                let e = advance(x, (w.get().wrapping_sub(ix) as usize).min(chunk), draw.x1);
                let ix = ix as usize;
                let run = Run { y, x, e, ix, iy };
                self.image_run(&mut sc, px, path, run, ops, masks.as_ref(), dsc);
                x = e;
            }
        }
        self.put_scratch(sc);
    }

    /// Draws one run of an untransformed image.
    #[allow(clippy::too_many_arguments)]
    fn image_run(
        &mut self,
        sc: &mut Scratch,
        px: &ImagePixels<'_>,
        path: ImagePath,
        r: Run,
        ops: TexelOps,
        masks: Option<&ImageMasks<'_>>,
        dsc: &ImageDsc<'_>,
    ) {
        let n = span(r.x, r.e);
        let (opa, mode) = (dsc.opa, dsc.blend_mode);
        // SAFE-INDEX: `n ≤ chunk = cov.len()` (the run length, `image`); the fallback is never
        // taken.
        let Some(cov) = sc.cov.get_mut(..n) else {
            return;
        };
        match path {
            ImagePath::Direct | ImagePath::Generic => {
                let has_cov = match masks {
                    Some(m) => {
                        if !m.fill(r.y, r.x, cov) {
                            return;
                        }
                        true
                    }
                    None => false,
                };
                let cov = if has_cov { Some(cov) } else { None };
                if path == ImagePath::Direct {
                    let row = px.row(r.iy);
                    let skip = r.ix.saturating_mul(usize::from(px.format.bpp())) / 8;
                    let src = Source::Pixels {
                        data: row.get(skip..).unwrap_or_default(),
                        format: px.format,
                    };
                    self.blend_masked_span(r.y, r.x, r.e, src, cov, opa, mode);
                } else {
                    with_texel!(px, T => convert_row::<T>(px, r.ix, usize::from(r.iy), n, ops, &mut sc.span));
                    // SAFE-ARITH: `n ≤ max_span ≤ 65_535`. SAFE-INDEX: the span scratch holds
                    // `4 · max_span` bytes; the fallback is never taken.
                    let argb = sc.span.get(..n.wrapping_mul(4)).unwrap_or_default();
                    self.blend_masked_span(r.y, r.x, r.e, Source::Argb(argb), cov, opa, mode);
                }
            }
            ImagePath::AlphaSolid(_) | ImagePath::Rgb565A8 => {
                if path == ImagePath::Rgb565A8 {
                    let a = px.alpha_row(r.iy);
                    match a.get(r.ix..r.ix.saturating_add(n)) {
                        Some(a) => cov.copy_from_slice(a),
                        None => return,
                    }
                } else {
                    let (row, bpp) = (px.row(r.iy), px.format.bpp());
                    // `ix + n ≤ w < 2^16`: the bounded range covers the run.
                    for (c, x) in cov.iter_mut().zip(r.ix..usize::MAX) {
                        *c = expand_alpha(unpack_bits(row, bpp, x), bpp);
                    }
                }
                if let Some(m) = masks {
                    // SAFE-INDEX: as above (the span scratch is larger); never falls back.
                    let Some(tmp) = sc.span.get_mut(..n) else {
                        return;
                    };
                    if !m.fill(r.y, r.x, tmp) {
                        return;
                    }
                    for (c, &t) in cov.iter_mut().zip(tmp.iter()) {
                        *c = mul8(*c, t);
                    }
                }
                let src = if let ImagePath::AlphaSolid(c) = path {
                    Source::Solid(c)
                } else {
                    Source::Pixels {
                        data: px.row(r.iy).get(r.ix.saturating_mul(2)..).unwrap_or(&[]),
                        format: ColorFormat::Rgb565,
                    }
                };
                self.blend_masked_span(r.y, r.x, r.e, src, Some(cov), opa, mode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_selection() {
        let d = [0u8; 64];
        let none = TexelOps::default();
        let rc = TexelOps {
            recolor: Some((Color::RED, Opa::COVER)),
            chroma: None,
        };
        let key = TexelOps {
            recolor: None,
            chroma: Some(0),
        };
        let p = |f| ImagePixels::new(f, 2, 2, &d);
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::Rgb565), none),
            ImagePath::Direct
        );
        assert_eq!(ImagePath::choose(&p(ColorFormat::Rgb565), rc), ImagePath::Generic);
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::Argb8888), key),
            ImagePath::Generic
        );
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::A4), rc),
            ImagePath::AlphaSolid(Color::RED)
        );
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::Rgb565A8), none),
            ImagePath::Rgb565A8
        );
        assert_eq!(ImagePath::choose(&p(ColorFormat::I4), none), ImagePath::Generic);
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::Argb8888Premultiplied), none),
            ImagePath::Generic
        );
    }

    #[test]
    fn transformed_area_identity_and_scale() {
        let a = Rect::from_xywh(10, 10, 20, 10);
        assert_eq!(
            transformed_area(a, Angle::deg(360), Scale::ONE, Scale::ONE, Point::new(3, 3)),
            a
        );
        let r = transformed_area(
            a,
            Angle::deci_deg(0),
            Scale::from_raw_256(512),
            Scale::from_raw_256(512),
            Point::new(0, 0),
        );
        assert_eq!(r, Rect::from_xywh(10, 10, 40, 20));
    }
}
