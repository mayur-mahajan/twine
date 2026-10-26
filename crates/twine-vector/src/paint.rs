//! Paints: [`Paint`] (solid, linear/radial gradient, image pattern), [`Stops`] and the
//! per-pixel evaluation behind [`SpanSource::Pixels`](twine_render::SpanSource::Pixels).
//!
//! Gradient and image geometry is given in *paint space*; the paint's own `transform` maps it
//! into path space and the [`VectorDsc`](crate::VectorDsc) transform maps that to the screen.
//! Each pixel center is mapped back with the inverse transform (stepped per pixel, no matrix
//! product per pixel). Gradient colors come from a 256-entry color map shared with
//! `twine-render` (its gradient cache); the parameter `t` is a fixed-point dot product
//! (linear) or the root of the two-point conical equation (radial, with focal point): exact
//! 64-bit tests tracked along the row, with an integer square root where the tracking cannot
//! follow (`RadialTrack`).

use alloc::vec::Vec;
use core::num::{NonZeroU32, NonZeroU64};

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
/// use twine_core::{Color, Fraction, Fx, Transform};
/// use twine_render::{GradExtend, GradStop};
/// use twine_vector::{FxPoint, Paint, Stops};
///
/// static STOPS: [GradStop; 2] =
///     [GradStop::new(Color::RED, Fraction::ZERO), GradStop::new(Color::BLUE, Fraction::ONE)];
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
fn lut_index(t: i64, extend: GradExtend) -> u8 {
    let t = match extend {
        GradExtend::Pad => t.clamp(0, 65_536),
        GradExtend::Repeat => t.rem_euclid(65_536),
        GradExtend::Reflect => {
            let r = t.rem_euclid(131_072);
            // SAFE-ARITH: `65_536 < r < 131_072` here.
            if r > 65_536 { 131_072i64.wrapping_sub(r) } else { r }
        }
    };
    // SAFE-ARITH: `0 ≤ t ≤ 65_536`, so the index is `0..=255`.
    (t.wrapping_mul(255).wrapping_add(32_768) >> 16) as u8
}

/// The color of `lut` at `t` (a `u8` index into 256 entries: no check remains).
#[inline(always)]
fn lut_at(lut: &[u32; 256], t: i64, extend: GradExtend) -> u32 {
    lut.get(usize::from(lut_index(t, extend))).copied().unwrap_or(0)
}

/// Wraps an image coordinate by the extend mode (`1 ≤ n ≤ u16::MAX`).
#[inline]
fn wrap(v: i64, n: i64, extend: GradExtend) -> i64 {
    // `n` is the image width or height, ≥ 1: `Sampler::Image` is built only for a non-empty
    // image (`Paint::Image` returns `None` otherwise), so the division fallbacks are never
    // taken. SAFE-ARITH: `1 ≤ n < 2^16` and `0 ≤ r < 2n`.
    match extend {
        GradExtend::Pad => v.clamp(0, n.wrapping_sub(1)),
        GradExtend::Repeat => v.checked_rem_euclid(n).unwrap_or(0),
        GradExtend::Reflect => {
            let n2 = n.wrapping_mul(2);
            let r = v.checked_rem_euclid(n2).unwrap_or(0);
            if r >= n {
                n2.wrapping_sub(1).wrapping_sub(r)
            } else {
                r
            }
        }
    }
}

/// `√v` for `u128` (exact floor for values below 2^64, else within a relative 2^-32).
fn isqrt128(v: u128) -> u64 {
    if let Ok(s) = u64::try_from(v) {
        return u64::from(isqrt64(s));
    }
    let mut shift = 0u32;
    // SHIFT: the loop stops by `shift = 32` (`u128::MAX >> 64 = u64::MAX`), so `2·shift ≤ 64` on
    // the `u128` and `shift ≤ 32` on the `u64` below. SAFE-ARITH: the same bound.
    while (v >> shift.wrapping_mul(2)) > u128::from(u64::MAX) {
        shift = shift.wrapping_add(1);
    }
    // SHIFT: `2·shift ≤ 64` (`u128`) and `shift ≤ 32` (`u64`), as above.
    u64::from(isqrt64((v >> shift.wrapping_mul(2)) as u64)) << shift
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
        /// The constants of the tracked evaluation (`None` when `a ≥ 0`).
        track: Option<RadialTrack>,
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

/// Largest focal offset (24.8) a radial gradient is evaluated at (`2^40`, i.e. 2^32 px: far
/// outside every gradient).
const RADIAL_LIM: i64 = 1 << 40;

/// Largest paint-space position (16.16) a row starts at: `±2^62`, so that the per-pixel steps
/// (`< 2^31` each, at most `2^16` of them) cannot overflow. Farther positions are far outside
/// every image (only a repeated pattern's phase could differ, at 2^46 px).
const POS_MAX: i128 = 1 << 62;

/// The 16.16 paint-space position of the center of device pixel `(x, y)` under `inv`.
#[inline]
fn map_center(inv: &Transform, x: i32, y: i32) -> (i64, i64) {
    // SAFE-ARITH: doubled `i32`s fit in `i64`; the products of `i32` coefficients with them
    // (below 2^65) are formed in `i128`.
    let (x2, y2) = (
        i128::from(x).wrapping_mul(2).wrapping_add(1),
        i128::from(y).wrapping_mul(2).wrapping_add(1),
    );
    let m = |k1: Fx, k2: Fx, t: Fx| -> i64 {
        let v = i128::from(k1.raw())
            .wrapping_mul(x2)
            .wrapping_add(i128::from(k2.raw()).wrapping_mul(y2))
            / 2;
        v.wrapping_add(i128::from(t.raw()))
            .clamp(POS_MAX.wrapping_neg(), POS_MAX) as i64
    };
    (m(inv.a, inv.c, inv.tx), m(inv.b, inv.d, inv.ty))
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
                // SAFE-ARITH (the linear set-up): coordinates and coefficients are `i32`s, so
                // the differences are below 2^32, `len2 < 2^65` and the numerators below 2^66,
                // shifted below 2^98 in `i128`.
                let (gx, gy) = (i128::from(ex.wrapping_sub(sx)), i128::from(ey.wrapping_sub(sy)));
                let len2 = gx.wrapping_mul(gx).wrapping_add(gy.wrapping_mul(gy));
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
                    i128::from(inv.a.raw()),
                    i128::from(inv.b.raw()),
                    i128::from(inv.c.raw()),
                    i128::from(inv.d.raw()),
                );
                // t(x, y) = ((u − s)·g) / |g|², u = inv(x, y); coefficients with 32 fraction bits.
                let k = |num: i128| -> i64 {
                    // `len2 != 0` (returned above): the fallback is never taken.
                    let v = (num << 32).checked_div(len2).unwrap_or(0);
                    v.clamp(i128::from(i64::MIN / 4), i128::from(i64::MAX / 4)) as i64
                };
                let dot = |a: i128, b: i128| a.wrapping_mul(gx).wrapping_add(b.wrapping_mul(gy));
                // t is linear in the pixel position: t = t0 + x·dx + y·dy (the 2^32 scales of
                // the 16.16 products cancel against |g|²).
                Some(Sampler::Linear {
                    t0: k(dot(
                        i128::from(inv.tx.raw()).wrapping_sub(i128::from(sx)),
                        i128::from(inv.ty.raw()).wrapping_sub(i128::from(sy)),
                    )),
                    dx: k(dot(ia, ib)),
                    dy: k(dot(ic, id)),
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
                let r_raw = i64::from(radius.raw().max(0));
                // Scale paint space by 2^sh so the radius is ≈ 2^22 in 16.16 (2^14 in 24.8).
                // SAFE-ARITH (the radial set-up): `bits ≤ 32` (an `i32` radius), the scaled
                // values are `i32`s shifted by at most 24 (below 2^55, differences below 2^56);
                // after scaling `r < 2^16` (24.8) and the focal offset `|e| ≤ r` (moved inside
                // below), so `a = |e|² − r²` is within `±2^33`.
                let bits = 64i32.wrapping_sub(r_raw.leading_zeros() as i32);
                let sh = 22i32.wrapping_sub(bits).clamp(-8, 24);
                // SHIFT: `sh` is clamped to `-8..=24`: amounts `0..=24` and `1..=8`, on an `i64`.
                let scale = |v: i64| if sh >= 0 { v << sh } else { v >> sh.wrapping_neg() };
                let r = scale(r_raw) >> 8;
                let (cx, cy) = center.raw();
                let (cx, cy) = (scale(cx), scale(cy));
                let (fx, fy) = focal.unwrap_or(*center).raw();
                let (mut fx, mut fy) = (scale(fx), scale(fy));
                // Keep the focal point strictly inside the circle (≤ 0.99 r).
                let (mut ex, mut ey) = (cx.wrapping_sub(fx), cy.wrapping_sub(fy));
                let el = hypot(ex, ey) >> 8;
                let lim = r.wrapping_mul(253) / 256;
                if el > lim && el > 0 {
                    ex = muldiv(ex, lim, el);
                    ey = muldiv(ey, lim, el);
                    fx = cx.wrapping_sub(ex);
                    fy = cy.wrapping_sub(ey);
                }
                let (ex, ey) = (ex >> 8, ey >> 8);
                let a = ex
                    .wrapping_mul(ex)
                    .wrapping_add(ey.wrapping_mul(ey))
                    .wrapping_sub(r.wrapping_mul(r));
                let m = [
                    inv.a.raw(),
                    inv.b.raw(),
                    inv.c.raw(),
                    inv.d.raw(),
                    inv.tx.raw(),
                    inv.ty.raw(),
                ]
                .map(|v| scale(i64::from(v)));
                Some(Sampler::Radial {
                    inv: m,
                    fx: fx >> 8,
                    fy: fy >> 8,
                    ex,
                    ey,
                    r,
                    a,
                    track: RadialTrack::new(ex, ey, a),
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
                // SAFE-ARITH: doubled `i32`s fit in `i64`; their products with `|dx|, |dy| ≤
                // 2^61` are formed in `i128`.
                let xc = i128::from(x0).wrapping_mul(2).wrapping_add(1);
                let yc = i128::from(y).wrapping_mul(2).wrapping_add(1);
                // Pixel centers: (x + ½) · dx.
                let mut t = t0
                    .saturating_add((xc.wrapping_mul(i128::from(dx)) / 2) as i64)
                    .saturating_add((yc.wrapping_mul(i128::from(dy)) / 2) as i64);
                for px in out.chunks_exact_mut(4) {
                    px.copy_from_slice(&lut_at(lut, t >> 16, extend).to_le_bytes());
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
                track,
                extend,
            } => {
                // SAFE-ARITH: doubled `i32`s; the scaled coefficients are below 2^55, so the
                // products are below 2^89 in `i128`; per pixel `|u >> 8|, |fx| < 2^56`.
                let (x2, y2) = (
                    i128::from(x0).wrapping_mul(2).wrapping_add(1),
                    i128::from(y).wrapping_mul(2).wrapping_add(1),
                );
                let row = |a: i64, c: i64, t: i64| -> i64 {
                    let v = i128::from(a)
                        .wrapping_mul(x2)
                        .wrapping_add(i128::from(c).wrapping_mul(y2))
                        / 2;
                    let v = v.wrapping_add(i128::from(t));
                    v.clamp(i128::from(i64::MIN / 4), i128::from(i64::MAX / 4)) as i64
                };
                let [i0, i1, i2, i3, i4, i5] = inv;
                let (mut u, mut v) = (row(i0, i2, i4), row(i1, i3, i5));
                let (du, dv) = (i0, i1);
                // The previous pixel's `t` and its change from the one before (the prediction
                // of the tracked evaluation); `None` at the row start and after a pixel
                // outside the tracked range.
                let mut prev: Option<(u64, i64)> = None;
                for px in out.chunks_exact_mut(4) {
                    let (qx, qy) = ((u >> 8).wrapping_sub(fx), (v >> 8).wrapping_sub(fy));
                    let t = match track {
                        Some(k) if k.covers(qx, qy) => {
                            let tk = k.t(qx, qy, prev);
                            // SAFE-ARITH: both are below 2^49 (`RadialTrack::t`).
                            let dt = prev.map_or(0, |(p, _)| (tk as i64).wrapping_sub(p as i64));
                            prev = Some((tk, dt));
                            // `tk < 2^49`.
                            Some(tk as i64)
                        }
                        _ => {
                            prev = None;
                            let (qx, qy) = (
                                qx.clamp(-RADIAL_LIM, RADIAL_LIM),
                                qy.clamp(-RADIAL_LIM, RADIAL_LIM),
                            );
                            radial_t(qx, qy, ex, ey, r, a)
                        }
                    };
                    let c = match t {
                        Some(t) => lut_at(lut, t, extend),
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
                let (du, dv) = (i64::from(inv.a.raw()), i64::from(inv.b.raw()));
                let (w, h) = (i64::from(image.w), i64::from(image.h));
                // SAFE-ARITH: `|u|, |v| ≤ 2^62` at the row start (`map_center`) and each of at
                // most `2^16` steps adds less than 2^31: they stay below 2^62 + 2^47, and the
                // half-texel offsets and neighbour columns fit.
                for px in out.chunks_exact_mut(4) {
                    let c = if aa {
                        let (su, sv) = (u.wrapping_sub(32_768), v.wrapping_sub(32_768));
                        let (x, y) = (su >> 16, sv >> 16);
                        let (x1, y1) = (x.wrapping_add(1), y.wrapping_add(1));
                        let (fx, fy) = (((su >> 8) & 0xFF) as u32, ((sv >> 8) & 0xFF) as u32);
                        let get = |x: i64, y: i64| {
                            image.texel(wrap(x, w, extend) as u16, wrap(y, h, extend) as u16)
                        };
                        if fx == 0 && fy == 0 {
                            get(x, y)
                        } else {
                            bilinear([get(x, y), get(x1, y), get(x, y1), get(x1, y1)], fx, fy)
                        }
                    } else {
                        image.texel(wrap(u >> 16, w, extend) as u16, wrap(v >> 16, h, extend) as u16)
                    };
                    px.copy_from_slice(&c.to_le_bytes());
                    u = u.wrapping_add(du);
                    v = v.wrapping_add(dv);
                }
            }
        }
    }
}

/// Radial `t` (16.16) of the point `q` (relative to the focal point, 24.8) for the end circle
/// offset `e` and radius `r` (24.8), `a = |e|² − r²`.
#[inline]
fn radial_t(qx: i64, qy: i64, ex: i64, ey: i64, r: i64, a: i64) -> Option<i64> {
    // SAFE-ARITH (this function): `|qx|, |qy| ≤ 2^40` (clamped by the caller), `|ex|, |ey| ≤
    // r < 2^17` and `|a| < 2^34` (`Sampler::new`): `|b| < 2^58`, `c ≤ 2^81`, `b² < 2^116`,
    // `|a·c| < 2^115`, so the discriminant fits; `(s − b) · 2^16 < 2^82`.
    let m = |x: i64, y: i64| i128::from(x).wrapping_mul(i128::from(y));
    let b = m(qx, ex).wrapping_add(m(qy, ey));
    let c = m(qx, qx).wrapping_add(m(qy, qy));
    let a = i128::from(a);
    if a > 0 {
        return None;
    }
    if a == 0 {
        // Zero radius and focal = center: everything is past the end.
        return (r == 0).then_some(1 << 17);
    }
    let disc = b.wrapping_mul(b).wrapping_sub(a.wrapping_mul(c));
    if disc < 0 {
        return None;
    }
    let s = i128::from(isqrt128(disc as u128));
    // a < 0 always (focal inside the circle): the larger root is (s − b) / (−a).
    // `a != 0` (returned above): the `?` is never taken.
    let t = (s.wrapping_sub(b) << 16).checked_div(a.wrapping_neg())?;
    Some(t.clamp(i128::from(i64::MIN / 2), i128::from(i64::MAX / 2)) as i64)
}

/// Most steps [`RadialTrack`] moves from its prediction before it computes `t` directly.
const TRACK_STEPS: u32 = 8;

/// [`radial_t`] for one draw with a focal point inside the circle (`a < 0`), evaluated with
/// 64-bit arithmetic and tracked from pixel to pixel: the same values, bit for bit, without
/// the per-pixel 128-bit root and division (costly on 32-bit cores).
///
/// `radial_t` is `T = ⌊(⌊√D⌋ − b) · 2^16 / A⌋` with `b = q·e`, `c = q·q`, `A = −a` and
/// `D = b² + A·c`. For `k ≥ 0`: `T ≤ k` ⇔ `(⌊√D⌋ − b) · 2^16 < (k + 1)·A` ⇔
/// `⌊√D⌋ < M(k)` with `M(k) = b + ⌈(k + 1)·A / 2^16⌉` ⇔ `M(k) > 0 ∧ D < M(k)²` — a test with
/// two multiplications and no root or division. `t` changes smoothly along a row, so the
/// previous pixel's `t` plus its last change predicts the next one and a few tests find the
/// exact value; where the prediction is off by more than [`TRACK_STEPS`] (next to the focal
/// point) `t` is computed directly: [`isqrt64`] and one division.
///
/// The evaluation needs `D < 2^64`: it covers the points with `|qx|, |qy| ≤ qlim`, the others
/// take `radial_t`. For a covered point `|b| < 2^32` and `⌊√D⌋ < 2^32` (`b² ≤ D`), so
/// `0 ≤ ⌊√D⌋ − b < 2^33` and `T < 2^49 / A`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RadialTrack {
    /// `A = −a`.
    a: NonZeroU32,
    /// The end circle's offset `e` (24.8).
    ex: i32,
    /// See `ex`.
    ey: i32,
    /// Largest `|qx|`, `|qy|` covered: `qlim² · ((|ex| + |ey|)² + 2A) < 2^64`, and `< 2^31`.
    qlim: u64,
    /// `⌊2^49 / A⌋ + 1`: above `T` of every covered point.
    t_cap: u64,
}

impl RadialTrack {
    /// The constants for the end circle offset `e` and `a = |e|² − r²` (see [`radial_t`]);
    /// `None` when `a ≥ 0` or the values do not fit the 64-bit evaluation.
    fn new(ex: i64, ey: i64, a: i64) -> Option<Self> {
        if a >= 0 {
            return None;
        }
        let a = NonZeroU32::new(u32::try_from(a.unsigned_abs()).ok()?)?;
        let (ex, ey) = (i32::try_from(ex).ok()?, i32::try_from(ey).ok()?);
        // For `|qx|, |qy| ≤ Q`: `|b| ≤ Q·(|ex| + |ey|)` and `A·c ≤ 2A·Q²`, so `D ≤ Q²·k`.
        // SAFE-ARITH: `|ex| + |ey| < 2^32`, so `k < 2^64 + 2^33` in `u128`.
        let el = u128::from(ex.unsigned_abs()).wrapping_add(u128::from(ey.unsigned_abs()));
        let k = el
            .wrapping_mul(el)
            .wrapping_add(u128::from(a.get()).wrapping_mul(2));
        // `k ≥ 2A ≥ 2`: the fallback is never taken; the quotient is below 2^63.
        let qmax = u64::try_from(u128::from(u64::MAX).checked_div(k).unwrap_or(0)).ok()?;
        Some(Self {
            a,
            ex,
            ey,
            qlim: u64::from(isqrt64(qmax)).min(i32::MAX as u64),
            // DIV: `a` is a `NonZeroU32`.
            t_cap: ((1u64 << 49) / NonZeroU64::from(a)).wrapping_add(1),
        })
    }

    /// Whether `q` (24.8, relative to the focal point) is covered.
    #[inline]
    fn covers(&self, qx: i64, qy: i64) -> bool {
        qx.unsigned_abs() <= self.qlim && qy.unsigned_abs() <= self.qlim
    }

    /// `radial_t` of the covered point `q`; `prev` is the previous pixel's `t` and its change
    /// from the pixel before (the prediction).
    #[inline]
    fn t(&self, qx: i64, qy: i64, prev: Option<(u64, i64)>) -> u64 {
        let (d, b) = self.disc(qx, qy);
        prev.and_then(|(t, dt)| self.search(d, b, t.saturating_add_signed(dt).min(self.t_cap)))
            .unwrap_or_else(|| self.direct(d, b))
    }

    /// `(D, b)` of the covered point `q`.
    #[inline]
    fn disc(&self, qx: i64, qy: i64) -> (u64, i64) {
        // SAFE-ARITH (this function): `|qx|, |qy| ≤ qlim < 2^31` (`covers`), so the products
        // of `i32`s fit, `|b| ≤ qlim·(|ex| + |ey|) < 2^32` and `b² + A·c ≤ qlim²·k < 2^64`
        // (`new`); each square is below 2^62.
        let (x, y) = (i64::from(qx as i32), i64::from(qy as i32));
        let b = x
            .wrapping_mul(i64::from(self.ex))
            .wrapping_add(y.wrapping_mul(i64::from(self.ey)));
        let c = (x.wrapping_mul(x) as u64).wrapping_add(y.wrapping_mul(y) as u64);
        let bm = u64::from(b.unsigned_abs() as u32);
        let d = bm
            .wrapping_mul(bm)
            .wrapping_add(u64::from(self.a.get()).wrapping_mul(c));
        (d, b)
    }

    /// Whether `T ≤ k` (`k ≤ t_cap`).
    #[inline]
    fn at_most(&self, d: u64, b: i64, k: u64) -> bool {
        // SAFE-ARITH: `(k + 1)·A ≤ (t_cap + 1)·A ≤ 2^49 + 2A < 2^51` and `|b| < 2^32`, so
        // `M(k)` is within `±2^36`; `M(k) < 2^32` squares below 2^64.
        let p = k.wrapping_add(1).wrapping_mul(u64::from(self.a.get()));
        let m = b.wrapping_add((p.wrapping_add(0xFFFF) >> 16) as i64);
        if m <= 0 {
            return false;
        }
        let m = m as u64;
        m > u64::from(u32::MAX) || d < m.wrapping_mul(m)
    }

    /// `T` found from `guess` (`≤ t_cap`) in at most [`TRACK_STEPS`] steps.
    #[inline]
    fn search(&self, d: u64, b: i64, guess: u64) -> Option<u64> {
        let mut k = guess;
        if self.at_most(d, b, k) {
            // Up to `TRACK_STEPS` moves down, each followed by the check of the next one.
            for _ in 0..=TRACK_STEPS {
                // SAFE-ARITH: `k > 0` (checked first).
                if k == 0 || !self.at_most(d, b, k.wrapping_sub(1)) {
                    return Some(k);
                }
                k = k.wrapping_sub(1);
            }
        } else {
            for _ in 0..TRACK_STEPS {
                // SAFE-ARITH: `T ≤ k` fails, and `T < t_cap`: `k + 1 ≤ t_cap`.
                k = k.wrapping_add(1);
                if self.at_most(d, b, k) {
                    return Some(k);
                }
            }
        }
        None
    }

    /// `T` computed directly.
    fn direct(&self, d: u64, b: i64) -> u64 {
        // SAFE-ARITH: `|b| ≤ ⌊√D⌋ < 2^32` (`b² ≤ D`), so `0 ≤ ⌊√D⌋ − b < 2^33` and the
        // shifted value is below 2^49.
        let s = i64::from(isqrt64(d));
        let num = s.wrapping_sub(b) as u64;
        // DIV: `a` is a `NonZeroU32`.
        (num << 16) / NonZeroU64::from(self.a)
    }
}

/// Bilinear blend of four packed straight-alpha texels, weights `fx`, `fy` in `0..256`.
fn bilinear(t: [u32; 4], fx: u32, fy: u32) -> u32 {
    // SAFE-ARITH: `fx, fy < 256`: the four weights are at most 2^16 and sum to 2^16, so the
    // alpha sum is below 2^24 and each channel sum below 2^40 (in `u64`).
    let (ix, iy) = (256u32.wrapping_sub(fx), 256u32.wrapping_sub(fy));
    let w = [
        ix.wrapping_mul(iy),
        fx.wrapping_mul(iy),
        ix.wrapping_mul(fy),
        fx.wrapping_mul(fy),
    ];
    let a = t
        .iter()
        .zip(&w)
        .fold(0u32, |s, (&v, &wi)| s.wrapping_add((v >> 24).wrapping_mul(wi)));
    if a == 0 {
        return 0;
    }
    let out_a = (a.wrapping_add(32_768) >> 16).min(255);
    let ch = |s: u32| -> u32 {
        let v = t.iter().zip(&w).fold(0u64, |sum, (&v, &wi)| {
            // SHIFT: `s` is one of the literal channel offsets 16, 8, 0 passed to `ch` below.
            let c = u64::from((v >> s) & 0xFF);
            sum.wrapping_add(c.wrapping_mul(u64::from((v >> 24).wrapping_mul(wi))))
        });
        // `a != 0` (returned above): the division's fallback is never taken.
        (v.wrapping_add(u64::from(a) / 2)
            .checked_div(u64::from(a))
            .unwrap_or(0))
        .min(255) as u32
    };
    (out_a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Fills `lut` with the color map of `stops`, through the render caches' gradient cache when
/// the stops fit a render [`Gradient`] (≤ [`MAX_STOPS`]).
pub(crate) fn load_lut(p: &mut Painter<'_>, stops: &[GradStop], lut: &mut [u32; 256]) {
    if stops.len() <= MAX_STOPS {
        let g = Gradient::new(GradKind::Hor, stops);
        // The cached map is always 256 entries; any other length is rebuilt here.
        match <&[u32; 256]>::try_from(p.caches().gradient_color_map(&g)) {
            Ok(map) => *lut = *map,
            Err(_) => build_color_map(stops, lut),
        }
    } else {
        build_color_map(stops, lut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use proptest::prelude::*;
    use twine_core::Angle;

    /// The radial row of `Sampler::fill` with `radial_t` at every pixel (the evaluation
    /// `RadialTrack` must reproduce).
    fn fill_radial_reference(s: &Sampler, lut: &[u32; 256], y: i32, x0: i32, out: &mut [u8]) {
        let Sampler::Radial {
            inv,
            fx,
            fy,
            ex,
            ey,
            r,
            a,
            extend,
            ..
        } = *s
        else {
            panic!("radial sampler");
        };
        let (x2, y2) = (i128::from(x0) * 2 + 1, i128::from(y) * 2 + 1);
        let row = |a: i64, c: i64, t: i64| -> i64 {
            let v = (i128::from(a) * x2 + i128::from(c) * y2) / 2 + i128::from(t);
            v.clamp(i128::from(i64::MIN / 4), i128::from(i64::MAX / 4)) as i64
        };
        let [i0, i1, i2, i3, i4, i5] = inv;
        let (mut u, mut v) = (row(i0, i2, i4), row(i1, i3, i5));
        for px in out.chunks_exact_mut(4) {
            let qx = ((u >> 8) - fx).clamp(-RADIAL_LIM, RADIAL_LIM);
            let qy = ((v >> 8) - fy).clamp(-RADIAL_LIM, RADIAL_LIM);
            let c = radial_t(qx, qy, ex, ey, r, a).map_or(0, |t| lut_at(lut, t, extend));
            px.copy_from_slice(&c.to_le_bytes());
            u = u.saturating_add(i0);
            v = v.saturating_add(i1);
        }
    }

    /// Distinct entries, so equal output means equal color map indices.
    fn index_lut() -> [u32; 256] {
        core::array::from_fn(|i| 0xFF00_0000 | i as u32)
    }

    fn radial(c: (i32, i32), r: i32, focal: Option<(i32, i32)>, extend: GradExtend) -> Paint {
        Paint::Radial {
            center: FxPoint::new(Fx::from_raw(c.0), Fx::from_raw(c.1)),
            radius: Fx::from_raw(r),
            focal: focal.map(|f| FxPoint::new(Fx::from_raw(f.0), Fx::from_raw(f.1))),
            stops: Stops::Owned(Vec::new()),
            extend,
            transform: Transform::IDENTITY,
        }
    }

    /// Asserts that `fill` matches the reference on row `y` from `x0` (`n` pixels).
    fn assert_row_matches(p: &Paint, t: &Transform, y: i32, x0: i32, n: usize) {
        let s = Sampler::new(p, t).expect("invertible transform");
        assert!(
            matches!(s, Sampler::Radial { track: Some(_), .. }),
            "tracked evaluation set up"
        );
        let lut = index_lut();
        let (mut got, mut want) = (vec![0u8; n * 4], vec![0u8; n * 4]);
        s.fill(&lut, y, x0, &mut got);
        fill_radial_reference(&s, &lut, y, x0, &mut want);
        assert_eq!(got, want, "row {y} from {x0}: {p:?} under {t:?}");
    }

    #[test]
    fn radial_rows_match_reference_around_the_focal_point() {
        // Rows through and next to the center (where `t` has a kink and the tracking falls
        // back), small and large radii, focal points near the circle.
        let ext = [GradExtend::Pad, GradExtend::Repeat, GradExtend::Reflect];
        for (r_px, focal) in [
            (1, None),
            (40, None),
            (206, None),
            (3000, None),
            (80, Some((50, 0))),
        ] {
            for e in ext {
                let p = radial(
                    (300 << 16, 200 << 16),
                    r_px << 16,
                    focal.map(|(dx, dy): (i32, i32)| ((300 + dx) << 16, (200 + dy) << 16)),
                    e,
                );
                for y in [-50, 0, 150, 199, 200, 201, 260, 480] {
                    assert_row_matches(&p, &Transform::IDENTITY, y, -40, 720);
                }
            }
        }
    }

    #[test]
    fn radial_search_finds_t_from_a_nearby_guess() {
        // r = 206 px scaled as `Sampler::new` does (≈ 2^14 in 24.8), focal point off center.
        let (ex, ey, r) = (3_000i64, -1_200i64, 13_184i64);
        let a = ex * ex + ey * ey - r * r;
        let k = RadialTrack::new(ex, ey, a).expect("a < 0");
        for (qx, qy) in [(0, 0), (5_000, 7), (-13_000, 2_500), (20_000, -30_000), (1, -1)] {
            let want = radial_t(qx, qy, ex, ey, r, a).expect("inside") as u64;
            let (d, b) = k.disc(qx, qy);
            assert_eq!(k.direct(d, b), want, "direct at ({qx}, {qy})");
            for off in [-8i64, -3, -1, 0, 1, 3, 8] {
                let guess = want.saturating_add_signed(off).min(k.t_cap);
                assert_eq!(k.search(d, b, guess), Some(want), "guess {off:+} at ({qx}, {qy})");
            }
            assert_eq!(k.search(d, b, want + 9), None, "too far: computed directly");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn radial_rows_match_reference(
            cx in -400i32..1200, cy in -400i32..1200,
            r in 1i32..(4000 << 16),
            focal in proptest::option::of((-1.0f64..1.0, -1.0f64..1.0)),
            ext in 0usize..3,
            angle in -180i32..180, sx in 1i32..400, sy in 1i32..400,
            y in -300i32..900, x0 in -300i32..900, n in 1usize..600,
        ) {
            let f = focal.map(|(dx, dy)| {
                let (rx, ry) = (f64::from(r) * dx, f64::from(r) * dy);
                ((cx << 16).saturating_add(rx as i32), (cy << 16).saturating_add(ry as i32))
            });
            let extend = [GradExtend::Pad, GradExtend::Repeat, GradExtend::Reflect][ext];
            let p = radial((cx << 16, cy << 16), r, f, extend);
            let t = Transform::scale(Fx::from_ratio(sx, 100), Fx::from_ratio(sy, 100))
                .then(Transform::rotate(Angle::deg(angle)));
            assert_row_matches(&p, &t, y, x0, n);
        }
    }

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
