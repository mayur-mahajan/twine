//! Paints: [`Paint`] (solid, linear/radial gradient, image pattern), [`Stops`] and the
//! per-pixel evaluation behind [`SpanSource::Pixels`](twine_render::SpanSource::Pixels).
//!
//! Gradient and image geometry is given in *paint space*; the paint's own `transform` maps it
//! into path space and the [`VectorDsc`](crate::VectorDsc) transform maps that to the screen.
//! Each pixel center is mapped back with the inverse transform (stepped per pixel, no matrix
//! product per pixel). Gradient colors come from a 256-entry color map shared with
//! `twine-render` (its gradient cache); the parameter `t` is a fixed-point dot product
//! (linear) or the root of the two-point conical equation with an integer square root
//! (radial, with focal point).

use alloc::vec::Vec;

use twine_core::math::isqrt64;
use twine_core::{Color, Fx, Transform};
use twine_render::{
    GradExtend, GradKind, GradStop, Gradient, ImagePixels, MAX_STOPS, Painter, build_color_map,
};

use crate::geom::{FxPoint, hypot, muldiv};

/// Color stops of a gradient paint: a static list (the usual case) or an owned list (e.g.
/// parsed from SVG). Stops are sorted by `frac`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Stops {
    /// Stops in static memory.
    Static(&'static [GradStop]),
    /// Stops owned by the paint.
    Owned(Vec<GradStop>),
}

impl Stops {
    /// The stops.
    #[must_use]
    pub fn as_slice(&self) -> &[GradStop] {
        match self {
            Stops::Static(s) => s,
            Stops::Owned(v) => v,
        }
    }
}

impl From<&'static [GradStop]> for Stops {
    fn from(s: &'static [GradStop]) -> Self {
        Stops::Static(s)
    }
}

/// How a path is filled or stroked.
///
/// ```
/// use twine_core::{Color, Fx, Transform};
/// use twine_render::{GradExtend, GradStop};
/// use twine_vector::{FxPoint, Paint, Stops};
///
/// static STOPS: [GradStop; 2] = [GradStop::new(Color::RED, 0), GradStop::new(Color::BLUE, 255)];
/// let p = Paint::Linear {
///     start: FxPoint::from_int(0, 0),
///     end: FxPoint::from_int(100, 0),
///     stops: Stops::Static(&STOPS),
///     extend: GradExtend::Pad,
///     transform: Transform::IDENTITY,
/// };
/// assert!(!p.is_solid());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Paint {
    /// One color.
    Solid(Color),
    /// Linear gradient: `t = 0` at `start`, `t = 1` at `end`, constant along perpendiculars.
    Linear {
        /// Where `t = 0` (paint space).
        start: FxPoint,
        /// Where `t = 1` (paint space).
        end: FxPoint,
        /// Color stops.
        stops: Stops,
        /// Behaviour outside `[0, 1]`.
        extend: GradExtend,
        /// Paint space → path space (e.g. SVG `gradientTransform`).
        transform: Transform,
    },
    /// Radial gradient: `t = 0` at the focal point (default: the center), `t = 1` on the
    /// circle `(center, radius)`. A focal point outside the circle is moved just inside.
    Radial {
        /// Center of the end circle (paint space).
        center: FxPoint,
        /// Radius of the end circle.
        radius: Fx,
        /// Focal point (`None` = `center`).
        focal: Option<FxPoint>,
        /// Color stops.
        stops: Stops,
        /// Behaviour outside `[0, 1]`.
        extend: GradExtend,
        /// Paint space → path space.
        transform: Transform,
    },
    /// Image pattern: image pixel `(x, y)` covers `[x, x+1) × [y, y+1)` of paint space.
    Image {
        /// The pixels.
        image: ImagePixels<'static>,
        /// Paint space (image pixels) → path space.
        transform: Transform,
        /// Behaviour outside the image: `Pad` repeats the edge pixels, `Repeat` tiles,
        /// `Reflect` tiles mirrored.
        extend: GradExtend,
        /// Bilinear filtering (else nearest neighbour).
        antialias: bool,
    },
}

impl Paint {
    /// Whether the paint is a single color.
    #[must_use]
    pub fn is_solid(&self) -> bool {
        matches!(self, Paint::Solid(_))
    }
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c)
    }
}

/// Maps `t` (16.16) to a color map index after the extend mode (same rounding as
/// `twine-render`'s gradients).
#[inline]
fn lut_index(t: i64, extend: GradExtend) -> usize {
    let t = match extend {
        GradExtend::Pad => t.clamp(0, 65_536),
        GradExtend::Repeat => t.rem_euclid(65_536),
        GradExtend::Reflect => {
            let r = t.rem_euclid(131_072);
            if r > 65_536 { 131_072 - r } else { r }
        }
    };
    ((t * 255 + 32_768) >> 16) as usize
}

/// Wraps an image coordinate by the extend mode (`n > 0`).
#[inline]
fn wrap(v: i64, n: i64, extend: GradExtend) -> i64 {
    match extend {
        GradExtend::Pad => v.clamp(0, n - 1),
        GradExtend::Repeat => v.rem_euclid(n),
        GradExtend::Reflect => {
            let r = v.rem_euclid(2 * n);
            if r >= n { 2 * n - 1 - r } else { r }
        }
    }
}

/// `√v` for `u128` (exact floor for values below 2^64, else within a relative 2^-32).
fn isqrt128(v: u128) -> u64 {
    if let Ok(s) = u64::try_from(v) {
        return u64::from(isqrt64(s));
    }
    let mut shift = 0;
    while (v >> (2 * shift)) > u128::from(u64::MAX) {
        shift += 1;
    }
    u64::from(isqrt64((v >> (2 * shift)) as u64)) << shift
}

/// Precomputed per-draw evaluation of a non-solid [`Paint`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum Sampler {
    /// `t = T0 + x·dT_x + y·dT_y` with 32 fractional bits (per device pixel center).
    Linear {
        t0: i64,
        dx: i64,
        dy: i64,
        extend: GradExtend,
    },
    /// Inverse mapping (16.16, pre-scaled by `2^s` so the radius is ≈ 2^14 in 24.8 units,
    /// which keeps `t` precise for tiny and huge gradients) plus the conical equation.
    Radial {
        inv: [i64; 6],
        fx: i64,
        fy: i64,
        ex: i64,
        ey: i64,
        r: i64,
        a: i64,
        extend: GradExtend,
    },
    /// Inverse mapping into image pixels.
    Image {
        inv: Transform,
        image: ImagePixels<'static>,
        extend: GradExtend,
        aa: bool,
    },
}

/// The 16.16 paint-space position of the center of device pixel `(x, y)` under `inv`.
#[inline]
fn map_center(inv: &Transform, x: i32, y: i32) -> (i64, i64) {
    let (x2, y2) = (2 * i64::from(x) + 1, 2 * i64::from(y) + 1);
    let (a, b, c, d) = (
        i64::from(inv.a.0),
        i64::from(inv.b.0),
        i64::from(inv.c.0),
        i64::from(inv.d.0),
    );
    (
        (a * x2 + c * y2) / 2 + i64::from(inv.tx.0),
        (b * x2 + d * y2) / 2 + i64::from(inv.ty.0),
    )
}

impl Sampler {
    /// Prepares `paint` drawn with the path transform `t`. Returns `None` for a solid paint or
    /// a degenerate (non-invertible) transform.
    pub(crate) fn new(paint: &Paint, t: &Transform) -> Option<Self> {
        match paint {
            Paint::Solid(_) => None,
            Paint::Linear {
                start,
                end,
                extend,
                transform,
                ..
            } => {
                let inv = transform.then(*t).invert()?;
                let (sx, sy) = start.raw();
                let (ex, ey) = end.raw();
                let (gx, gy) = (i128::from(ex - sx), i128::from(ey - sy));
                let len2 = gx * gx + gy * gy;
                if len2 == 0 {
                    // Degenerate: SVG paints the last stop color.
                    return Some(Sampler::Linear {
                        t0: 1 << 32,
                        dx: 0,
                        dy: 0,
                        extend: GradExtend::Pad,
                    });
                }
                let (ia, ib, ic, id) = (
                    i128::from(inv.a.0),
                    i128::from(inv.b.0),
                    i128::from(inv.c.0),
                    i128::from(inv.d.0),
                );
                // t(x, y) = ((u − s)·g) / |g|², u = inv(x, y); coefficients with 32 fraction bits.
                let k = |num: i128| -> i64 {
                    let v = (num << 32) / len2;
                    v.clamp(i128::from(i64::MIN / 4), i128::from(i64::MAX / 4)) as i64
                };
                // t is linear in the pixel position: t = t0 + x·dx + y·dy (the 2^32 scales of
                // the 16.16 products cancel against |g|²).
                Some(Sampler::Linear {
                    t0: k((i128::from(inv.tx.0) - i128::from(sx)) * gx
                        + (i128::from(inv.ty.0) - i128::from(sy)) * gy),
                    dx: k(ia * gx + ib * gy),
                    dy: k(ic * gx + id * gy),
                    extend: *extend,
                })
            }
            Paint::Radial {
                center,
                radius,
                focal,
                extend,
                transform,
                ..
            } => {
                let inv = transform.then(*t).invert()?;
                let r_raw = i64::from(radius.0.max(0));
                // Scale paint space by 2^sh so the radius is ≈ 2^22 in 16.16 (2^14 in 24.8).
                let bits = 64 - r_raw.leading_zeros() as i32;
                let sh = (22 - bits).clamp(-8, 24);
                let scale = |v: i64| if sh >= 0 { v << sh } else { v >> -sh };
                let r = scale(r_raw) >> 8;
                let (cx, cy) = center.raw();
                let (cx, cy) = (scale(cx), scale(cy));
                let (fx, fy) = focal.unwrap_or(*center).raw();
                let (mut fx, mut fy) = (scale(fx), scale(fy));
                // Keep the focal point strictly inside the circle (≤ 0.99 r).
                let (mut ex, mut ey) = (cx - fx, cy - fy);
                let el = hypot(ex, ey) >> 8;
                let lim = r * 253 / 256;
                if el > lim && el > 0 {
                    ex = muldiv(ex, lim, el);
                    ey = muldiv(ey, lim, el);
                    fx = cx - ex;
                    fy = cy - ey;
                }
                let (ex, ey) = (ex >> 8, ey >> 8);
                let a = ex * ex + ey * ey - r * r;
                let m = [inv.a.0, inv.b.0, inv.c.0, inv.d.0, inv.tx.0, inv.ty.0].map(|v| scale(i64::from(v)));
                Some(Sampler::Radial {
                    inv: m,
                    fx: fx >> 8,
                    fy: fy >> 8,
                    ex,
                    ey,
                    r,
                    a,
                    extend: *extend,
                })
            }
            Paint::Image {
                image,
                transform,
                extend,
                antialias,
            } => {
                if image.w == 0 || image.h == 0 {
                    return None;
                }
                let inv = transform.then(*t).invert()?;
                Some(Sampler::Image {
                    inv,
                    image: *image,
                    extend: *extend,
                    aa: *antialias,
                })
            }
        }
    }

    /// Fills `out` (Argb8888 bytes) with the paint of row `y` from screen x `x0`.
    pub(crate) fn fill(&self, lut: &[u32; 256], y: i32, x0: i32, out: &mut [u8]) {
        match *self {
            Sampler::Linear { t0, dx, dy, extend } => {
                let xc = 2 * i64::from(x0) + 1;
                let yc = 2 * i64::from(y) + 1;
                // Pixel centers: (x + ½) · dx.
                let mut t = t0
                    .saturating_add((i128::from(xc) * i128::from(dx) / 2) as i64)
                    .saturating_add((i128::from(yc) * i128::from(dy) / 2) as i64);
                for px in out.chunks_exact_mut(4) {
                    px.copy_from_slice(&lut[lut_index(t >> 16, extend)].to_le_bytes());
                    t = t.saturating_add(dx);
                }
            }
            Sampler::Radial {
                inv,
                fx,
                fy,
                ex,
                ey,
                r,
                a,
                extend,
            } => {
                let (x2, y2) = (2 * i128::from(x0) + 1, 2 * i128::from(y) + 1);
                let row = |a: i64, c: i64, t: i64| -> i64 {
                    let v = (i128::from(a) * x2 + i128::from(c) * y2) / 2 + i128::from(t);
                    v.clamp(i128::from(i64::MIN / 4), i128::from(i64::MAX / 4)) as i64
                };
                let (mut u, mut v) = (row(inv[0], inv[2], inv[4]), row(inv[1], inv[3], inv[5]));
                let (du, dv) = (inv[0], inv[1]);
                for px in out.chunks_exact_mut(4) {
                    let lim = 1i64 << 40;
                    let (qx, qy) = (((u >> 8) - fx).clamp(-lim, lim), ((v >> 8) - fy).clamp(-lim, lim));
                    let t = radial_t(qx, qy, ex, ey, r, a);
                    let c = match t {
                        Some(t) => lut[lut_index(t, extend)],
                        None => 0,
                    };
                    px.copy_from_slice(&c.to_le_bytes());
                    u = u.saturating_add(du);
                    v = v.saturating_add(dv);
                }
            }
            Sampler::Image {
                inv,
                image,
                extend,
                aa,
            } => {
                let (mut u, mut v) = map_center(&inv, x0, y);
                let (du, dv) = (i64::from(inv.a.0), i64::from(inv.b.0));
                let (w, h) = (i64::from(image.w), i64::from(image.h));
                for px in out.chunks_exact_mut(4) {
                    let c = if aa {
                        let (su, sv) = (u - 32_768, v - 32_768);
                        let (x, y) = (su >> 16, sv >> 16);
                        let (fx, fy) = (((su >> 8) & 0xFF) as u32, ((sv >> 8) & 0xFF) as u32);
                        let get = |x: i64, y: i64| {
                            image.texel(wrap(x, w, extend) as u16, wrap(y, h, extend) as u16)
                        };
                        if fx == 0 && fy == 0 {
                            get(x, y)
                        } else {
                            bilinear(
                                [get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1)],
                                fx,
                                fy,
                            )
                        }
                    } else {
                        image.texel(wrap(u >> 16, w, extend) as u16, wrap(v >> 16, h, extend) as u16)
                    };
                    px.copy_from_slice(&c.to_le_bytes());
                    u += du;
                    v += dv;
                }
            }
        }
    }
}

/// Radial `t` (16.16) of the point `q` (relative to the focal point, 24.8) for the end circle
/// offset `e` and radius `r` (24.8), `a = |e|² − r²`.
#[inline]
fn radial_t(qx: i64, qy: i64, ex: i64, ey: i64, r: i64, a: i64) -> Option<i64> {
    let b = i128::from(qx) * i128::from(ex) + i128::from(qy) * i128::from(ey);
    let c = i128::from(qx) * i128::from(qx) + i128::from(qy) * i128::from(qy);
    let a = i128::from(a);
    if a > 0 {
        return None;
    }
    if a == 0 {
        // Zero radius and focal = center: everything is past the end.
        return (r == 0).then_some(1 << 17);
    }
    let disc = b * b - a * c;
    if disc < 0 {
        return None;
    }
    let s = i128::from(isqrt128(disc as u128));
    // a < 0 always (focal inside the circle): the larger root is (s − b) / (−a).
    let t = ((s - b) << 16) / (-a);
    Some(t.clamp(i128::from(i64::MIN / 2), i128::from(i64::MAX / 2)) as i64)
}

/// Bilinear blend of four packed straight-alpha texels, weights `fx`, `fy` in `0..256`.
fn bilinear(t: [u32; 4], fx: u32, fy: u32) -> u32 {
    let w = [(256 - fx) * (256 - fy), fx * (256 - fy), (256 - fx) * fy, fx * fy];
    let a: u32 = (0..4).map(|i| (t[i] >> 24) * w[i]).sum();
    if a == 0 {
        return 0;
    }
    let out_a = ((a + 32_768) >> 16).min(255);
    let ch = |s: u32| -> u32 {
        let v: u64 = (0..4)
            .map(|i| u64::from((t[i] >> s) & 0xFF) * u64::from((t[i] >> 24) * w[i]))
            .sum();
        ((v + u64::from(a) / 2) / u64::from(a)).min(255) as u32
    };
    (out_a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Fills `lut` with the color map of `stops`, through the render caches' gradient cache when
/// the stops fit a render [`Gradient`] (≤ [`MAX_STOPS`]).
pub(crate) fn load_lut(p: &mut Painter<'_>, stops: &[GradStop], lut: &mut [u32; 256]) {
    if stops.len() <= MAX_STOPS {
        let g = Gradient::new(GradKind::Hor, stops);
        lut.copy_from_slice(p.caches().gradient_color_map(&g));
    } else {
        build_color_map(stops, lut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_indices() {
        assert_eq!(lut_index(-5, GradExtend::Pad), 0);
        assert_eq!(lut_index(70_000, GradExtend::Pad), 255);
        assert_eq!(lut_index(65_536 + 32_768, GradExtend::Repeat), 128);
        assert_eq!(
            lut_index(65_536 + 16_384, GradExtend::Reflect),
            lut_index(65_536 - 16_384, GradExtend::Reflect)
        );
        assert_eq!(wrap(-1, 4, GradExtend::Repeat), 3);
        assert_eq!(wrap(-1, 4, GradExtend::Reflect), 0);
        assert_eq!(wrap(5, 4, GradExtend::Reflect), 2);
        assert_eq!(wrap(9, 4, GradExtend::Pad), 3);
    }

    #[test]
    fn radial_centered_is_distance_over_radius() {
        // r = 40 px (24.8), focal = center.
        let r = 40 * 256;
        let a = -r * r;
        assert_eq!(radial_t(0, 0, 0, 0, r, a), Some(0));
        assert_eq!(radial_t(20 * 256, 0, 0, 0, r, a), Some(32_768));
        assert_eq!(radial_t(0, 40 * 256, 0, 0, r, a), Some(65_536));
    }

    #[test]
    fn linear_steps_match_direct() {
        let p = Paint::Linear {
            start: FxPoint::from_int(10, 0),
            end: FxPoint::from_int(110, 0),
            stops: Stops::Owned(Vec::new()),
            extend: GradExtend::Pad,
            transform: Transform::IDENTITY,
        };
        let Some(Sampler::Linear { t0, dx, dy, .. }) = Sampler::new(&p, &Transform::IDENTITY) else {
            panic!("linear sampler");
        };
        // t at pixel center x = 59.5 is 0.495.
        let t = t0 + (119 * i128::from(dx) / 2) as i64 + (i128::from(dy) / 2) as i64;
        assert!(((t >> 16) - 32_440).abs() <= 2, "{}", t >> 16);
    }
}
