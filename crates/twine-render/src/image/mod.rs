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

use core::sync::atomic::AtomicBool;

use twine_core::color::{expand_alpha, unpack_bits};
use twine_core::math::udiv255;
use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Point, Rect, Scale, Transform};

use crate::blend::{BlendMode, Source};
use crate::dispatch::warn_once;
use crate::mask::{Mask, MaskResult};
use crate::painter::Scratch;
use crate::transform_blit::is_direct;
use crate::{BlitDsc, ImagePixels, Painter};
use read::{TexelOps, pack, with_texel};

/// How [`Painter::image`] draws an image (LVGL `lv_draw_image_dsc_t`).
///
/// ```
/// use twine_core::{Angle, Opa};
/// use twine_render::ImageDsc;
/// let dsc = ImageDsc { opa: Opa::from_percent(50), angle: Angle::deg(30), ..ImageDsc::default() };
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
            angle: Angle(0),
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
        self.angle.normalized().0 != 0 || self.scale_x != Scale::ONE || self.scale_y != Scale::ONE
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
    if angle.normalized().0 == 0 && scale_x == Scale::ONE && scale_y == Scale::ONE {
        return area;
    }
    let p = Point::new(area.x0 + pivot.x, area.y0 + pivot.y);
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
        (radius.is_some() || bitmap.is_some()).then_some(Self {
            radius,
            bitmap,
            area,
        })
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
            let row = m.row((y - r.y0) as u16);
            let bpp = m.format.bpp();
            for (i, c) in cov.iter_mut().enumerate() {
                let x = x0 + i as i32;
                let a = if x < r.x0 || x >= r.x1 {
                    0
                } else {
                    let v = unpack_bits(row, bpp, (x - r.x0) as usize);
                    if m.format == ColorFormat::L8 { v } else { expand_alpha(v, bpp) }
                };
                *c = udiv255(u32::from(*c) * u32::from(a)) as u8;
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
            let pivot = Point::new(area.x0 + dsc.pivot.x, area.y0 + dsc.pivot.y);
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
        if plain && !dsc.tile && draw == img && !self.has_masks() && self.accel_blit(img, px, dsc.opa) == Some(true)
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
        let (w, h) = (i32::from(px.w), i32::from(px.h));
        let mut sc = self.take_scratch();
        let chunk = sc.cov.len().max(1) as i32;
        for y in draw.y0..draw.y1 {
            let iy = (y - area.y0).rem_euclid(h) as u16;
            if copy {
                self.caches_mut().image_copy_rows += 1;
            }
            let mut x = draw.x0;
            while x < draw.x1 {
                let ix = (x - area.x0).rem_euclid(w);
                let e = draw.x1.min(x + (w - ix)).min(x + chunk);
                let run = Run {
                    y,
                    x,
                    e,
                    ix: ix as usize,
                    iy,
                };
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
        let n = (r.e - r.x) as usize;
        let (opa, mode) = (dsc.opa, dsc.blend_mode);
        match path {
            ImagePath::Direct | ImagePath::Generic => {
                let has_cov = match masks {
                    Some(m) => {
                        if !m.fill(r.y, r.x, &mut sc.cov[..n]) {
                            return;
                        }
                        true
                    }
                    None => false,
                };
                let cov = if has_cov { Some(&mut sc.cov[..n]) } else { None };
                if path == ImagePath::Direct {
                    let row = px.row(r.iy);
                    let skip = (r.ix * usize::from(px.format.bpp()) / 8).min(row.len());
                    let src = Source::Pixels {
                        data: &row[skip..],
                        format: px.format,
                    };
                    self.blend_masked_span(r.y, r.x, r.e, src, cov, opa, mode);
                } else {
                    with_texel!(px, T => convert_row::<T>(px, r.ix, usize::from(r.iy), n, ops, &mut sc.span));
                    self.blend_masked_span(r.y, r.x, r.e, Source::Argb(&sc.span[..n * 4]), cov, opa, mode);
                }
            }
            ImagePath::AlphaSolid(_) | ImagePath::Rgb565A8 => {
                let cov = &mut sc.cov[..n];
                if path == ImagePath::Rgb565A8 {
                    let a = px.alpha_row(r.iy);
                    match a.get(r.ix..r.ix + n) {
                        Some(a) => cov.copy_from_slice(a),
                        None => return,
                    }
                } else {
                    let (row, bpp) = (px.row(r.iy), px.format.bpp());
                    for (i, c) in cov.iter_mut().enumerate() {
                        *c = expand_alpha(unpack_bits(row, bpp, r.ix + i), bpp);
                    }
                }
                if let Some(m) = masks {
                    let tmp = &mut sc.span[..n];
                    if !m.fill(r.y, r.x, tmp) {
                        return;
                    }
                    for (c, &t) in cov.iter_mut().zip(tmp.iter()) {
                        *c = udiv255(u32::from(*c) * u32::from(t)) as u8;
                    }
                }
                let src = if let ImagePath::AlphaSolid(c) = path {
                    Source::Solid(c)
                } else {
                    Source::Pixels {
                        data: px.row(r.iy).get(r.ix * 2..).unwrap_or(&[]),
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
        assert_eq!(ImagePath::choose(&p(ColorFormat::Rgb565), none), ImagePath::Direct);
        assert_eq!(ImagePath::choose(&p(ColorFormat::Rgb565), rc), ImagePath::Generic);
        assert_eq!(ImagePath::choose(&p(ColorFormat::Argb8888), key), ImagePath::Generic);
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::A4), rc),
            ImagePath::AlphaSolid(Color::RED)
        );
        assert_eq!(ImagePath::choose(&p(ColorFormat::Rgb565A8), none), ImagePath::Rgb565A8);
        assert_eq!(ImagePath::choose(&p(ColorFormat::I4), none), ImagePath::Generic);
        assert_eq!(
            ImagePath::choose(&p(ColorFormat::Argb8888Premultiplied), none),
            ImagePath::Generic
        );
    }

    #[test]
    fn transformed_area_identity_and_scale() {
        let a = Rect::from_xywh(10, 10, 20, 10);
        assert_eq!(transformed_area(a, Angle::deg(360), Scale::ONE, Scale::ONE, Point::new(3, 3)), a);
        let r = transformed_area(a, Angle(0), Scale(512), Scale(512), Point::new(0, 0));
        assert_eq!(r, Rect::from_xywh(10, 10, 40, 20));
    }
}
