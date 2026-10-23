//! Gradients: [`Gradient`], [`GradStop`], [`GradKind`], [`GradExtend`], the color-map
//! [`GradientCache`] and per-pixel sampling.
//!
//! Geometry (points, centers, radii) is relative to the top-left of the drawn area, in pixels;
//! a gradient's parameter `t` is 0 at the start and 1 at the end. Stops map `t` (as
//! `frac`, a `Fraction`) to colors and opacities; the 256-entry color map of the stops is cached.

use alloc::vec::Vec;

use twine_core::math::{atan2, div_rem_euclid_i128, div_rem_i128, isqrt64};
use twine_core::{Angle, Color, Fraction, Opa, Point, Rect};

use crate::caches::CacheStats;

/// Maximum number of stops of a [`Gradient`].
pub const MAX_STOPS: usize = 8;

/// One color stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct GradStop {
    /// Color at the stop.
    pub color: Color,
    /// Opacity at the stop.
    pub opa: Opa,
    /// Position along the gradient (`Fraction::ZERO` = start, `Fraction::ONE` = end).
    pub frac: Fraction,
}

impl GradStop {
    /// An opaque stop.
    #[must_use]
    pub const fn new(color: Color, frac: Fraction) -> Self {
        Self {
            color,
            opa: Opa::COVER,
            frac,
        }
    }

    /// A stop with opacity.
    #[must_use]
    pub const fn with_opa(color: Color, opa: Opa, frac: Fraction) -> Self {
        Self { color, opa, frac }
    }
}

/// Gradient geometry (relative to the drawn area's top-left). Coordinates and radii are
/// clamped (never panicking) to `±2^26` px when the gradient is drawn (far beyond any screen;
/// it keeps the sampling arithmetic exact).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GradKind {
    /// Left to right over the area's width.
    Hor,
    /// Top to bottom over the area's height.
    Ver,
    /// Along the line from `start` to `end`.
    Linear {
        /// Where `t = 0`.
        start: Point,
        /// Where `t = 1`.
        end: Point,
    },
    /// Two-point radial gradient: `t = 0` on the focal circle (`focal`, `focal_radius`),
    /// `t = 1` on the end circle (`center`, `radius`).
    Radial {
        /// Center of the end circle.
        center: Point,
        /// Radius of the end circle.
        radius: i32,
        /// Center of the start (focal) circle.
        focal: Point,
        /// Radius of the start circle.
        focal_radius: i32,
    },
    /// Sweep around `center` from `start_angle` clockwise to `end_angle`.
    Conical {
        /// Sweep center.
        center: Point,
        /// Where `t = 0`.
        start_angle: Angle,
        /// Where `t = 1`.
        end_angle: Angle,
    },
}

/// What happens outside `t ∈ [0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GradExtend {
    /// The end colors continue.
    #[default]
    Pad,
    /// The gradient repeats.
    Repeat,
    /// The gradient repeats mirrored.
    Reflect,
}

/// A gradient with up to [`MAX_STOPS`] stops.
///
/// ```
/// use twine_core::{Color, Fraction};
/// use twine_render::{GradExtend, GradKind, GradStop, Gradient};
///
/// const G: Gradient = Gradient::new(
///     GradKind::Ver,
///     &[GradStop::new(Color::RED, Fraction::ZERO), GradStop::new(Color::BLUE, Fraction::ONE)],
/// )
///     .extend(GradExtend::Pad)
///     .dither(true);
/// assert_eq!(G.stops().len(), 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Gradient {
    /// Geometry.
    pub kind: GradKind,
    stops: [GradStop; MAX_STOPS],
    count: u8,
    /// Behaviour outside `[0, 1]`.
    pub extend: GradExtend,
    /// Ordered 4×4 dithering when drawing into RGB565 buffers.
    pub dither: bool,
}

impl Gradient {
    /// A gradient of `stops` (more than [`MAX_STOPS`] are truncated; stops should be sorted by
    /// `frac`).
    #[must_use]
    pub const fn new(kind: GradKind, stops: &[GradStop]) -> Self {
        let mut s = [GradStop {
            color: Color::BLACK,
            opa: Opa::COVER,
            frac: Fraction::ZERO,
        }; MAX_STOPS];
        let n = if stops.len() > MAX_STOPS {
            MAX_STOPS
        } else {
            stops.len()
        };
        // Copies the first `n` stops (slice patterns: no index in a `const fn`).
        let (mut src, mut dst): (&[GradStop], &mut [GradStop]) = (stops, &mut s);
        while let ([from, src_rest @ ..], [to, dst_rest @ ..]) = (src, dst) {
            *to = *from;
            (src, dst) = (src_rest, dst_rest);
        }
        Self {
            kind,
            stops: s,
            count: n as u8,
            extend: GradExtend::Pad,
            dither: false,
        }
    }

    /// With extend mode `e`.
    #[must_use]
    pub const fn extend(mut self, e: GradExtend) -> Self {
        self.extend = e;
        self
    }

    /// With dithering on or off.
    #[must_use]
    pub const fn dither(mut self, on: bool) -> Self {
        self.dither = on;
        self
    }

    /// The stops.
    #[must_use]
    pub fn stops(&self) -> &[GradStop] {
        // SAFE-INDEX: `count ≤ MAX_STOPS` (`new`, the only constructor); the fallback is never
        // taken.
        self.stops.get(..usize::from(self.count)).unwrap_or(&self.stops)
    }
}

/// Packs `(color, opa)` into an `Argb8888` value.
#[inline(always)]
const fn pack(c: Color, a: u8) -> u32 {
    ((a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// Fills `map` (256 entries) with samples of `stops`, packed `0xAARRGGBB`: entry `i` is the
/// color at gradient position `i / 255` (the color map every gradient is drawn with).
///
/// ```
/// use twine_core::{Color, Fraction};
/// use twine_render::{GradStop, build_color_map};
/// let mut map = [0u32; 256];
/// let stops = [GradStop::new(Color::BLACK, Fraction::ZERO), GradStop::new(Color::WHITE, Fraction::ONE)];
/// build_color_map(&stops, &mut map);
/// assert_eq!((map[0], map[255]), (0xFF00_0000, 0xFFFF_FFFF));
/// ```
pub fn build_color_map(stops: &[GradStop], map: &mut [u32]) {
    build_map(stops, map);
}

/// Fills `map` with 256 samples (packed `0xAARRGGBB`) of the stops.
pub(crate) fn build_map(stops: &[GradStop], map: &mut [u32]) {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        map.fill(0);
        return;
    };
    for (i, out) in (0..=255u8).zip(map.iter_mut()) {
        // The stops before and after position `i`.
        let around = stops
            .iter()
            .position(|s| s.frac.raw() >= i)
            .and_then(|k| stops.split_at_checked(k))
            .map(|(before, after)| (before.last(), after.first()));
        let v = match around {
            None | Some((_, None)) => pack(last.color, last.opa.raw()),
            Some((None, _)) => pack(first.color, first.opa.raw()),
            Some((Some(a), Some(b))) => {
                let (fa, fb) = (u32::from(a.frac.raw()), u32::from(b.frac.raw()));
                if fb <= fa {
                    pack(b.color, b.opa.raw())
                } else {
                    // SAFE-ARITH: `fa < i ≤ fb ≤ 255` (`a` is before the first stop at or
                    // after `i`), so `(i − fa) · 256 < 2^16` and `t ≤ 256`; each mix below is at
                    // most `255 · 256 + 128 < 2^17`. The fallback of the division (`fb > fa`)
                    // is never taken.
                    let t = u32::from(i)
                        .wrapping_sub(fa)
                        .wrapping_mul(256)
                        .checked_div(fb.wrapping_sub(fa))
                        .unwrap_or(256);
                    let l = |x: u8, y: u8| -> u8 {
                        // SAFE-ARITH: see above (`t ≤ 256`).
                        (u32::from(x)
                            .wrapping_mul(256u32.wrapping_sub(t))
                            .wrapping_add(u32::from(y).wrapping_mul(t))
                            .wrapping_add(128)
                            >> 8) as u8
                    };
                    pack(
                        Color::new(
                            l(a.color.r, b.color.r),
                            l(a.color.g, b.color.g),
                            l(a.color.b, b.color.b),
                        ),
                        l(a.opa.raw(), b.opa.raw()),
                    )
                }
            }
        };
        *out = v;
    }
}

#[derive(Clone, Copy, Debug)]
struct MapEntry {
    stops: [GradStop; MAX_STOPS],
    count: u8,
    last_use: u32,
}

/// One 256-entry color map (packed `0xAARRGGBB`); indexed by a `u8`, so a lookup needs no
/// check.
pub(crate) type ColorMap = [u32; 256];

/// LRU cache of gradient color maps (256 samples each), keyed by the stops.
#[derive(Debug, Default)]
pub struct GradientCache {
    entries: Vec<MapEntry>,
    maps: Vec<ColorMap>,
    max_entries: usize,
    tick: u32,
    stats: CacheStats,
    /// Fresh mode ([`RenderCaches::set_fresh`](crate::RenderCaches::set_fresh)): a hit
    /// rebuilds its map from the stops before it is used.
    #[cfg(feature = "display-integrity")]
    fresh: bool,
}

impl GradientCache {
    pub(crate) fn new(entries: u8) -> Self {
        let n = usize::from(entries).max(1);
        Self {
            entries: Vec::with_capacity(n),
            maps: Vec::with_capacity(n),
            max_entries: n,
            tick: 0,
            stats: CacheStats::default(),
            #[cfg(feature = "display-integrity")]
            fresh: false,
        }
    }

    /// Fresh mode: every [`lookup`](Self::lookup) rebuilds the map it returns.
    #[cfg(feature = "display-integrity")]
    pub(crate) fn set_fresh(&mut self, on: bool) {
        self.fresh = on;
    }

    pub(crate) fn bytes_reserved(&self) -> usize {
        self.maps
            .capacity()
            .saturating_mul(core::mem::size_of::<ColorMap>())
            .saturating_add(
                self.entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<MapEntry>()),
            )
    }

    pub(crate) fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Index of the color map of `g`, building it (and evicting the LRU entry) on a miss.
    pub(crate) fn lookup(&mut self, g: &Gradient) -> usize {
        self.tick = self.tick.wrapping_add(1);
        // The unused stops of a `Gradient` are always the default (`Gradient::new`), so the
        // whole arrays compare like the used parts.
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.count == g.count && e.stops == g.stops)
        {
            if let Some(e) = self.entries.get_mut(i) {
                e.last_use = self.tick;
            }
            self.stats.hits = self.stats.hits.saturating_add(1);
            #[cfg(feature = "display-integrity")]
            if self.fresh {
                self.rebuild(i, g);
            }
            return i;
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        let entry = MapEntry {
            stops: g.stops,
            count: g.count,
            last_use: self.tick,
        };
        let i = if self.entries.len() < self.max_entries {
            self.entries.push(entry);
            self.maps.push([0; 256]);
            self.entries.len().saturating_sub(1)
        } else {
            let (i, _) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_use)
                .unwrap_or((0, &entry));
            if let Some(e) = self.entries.get_mut(i) {
                *e = entry;
            }
            i
        };
        self.rebuild(i, g);
        i
    }

    /// Builds the map at `index` from `g`'s stops (a miss, or a hit in fresh mode).
    fn rebuild(&mut self, index: usize, g: &Gradient) {
        if let Some(map) = self.maps.get_mut(index) {
            build_map(g.stops(), map);
        }
    }

    /// The color map at `index` (from [`lookup`](Self::lookup), which always returns a cached
    /// index; `None` for any other index). An `Option` rather than a fallback map: a static
    /// all-transparent map would cost 1 KiB of flash for a branch that is never taken.
    pub(crate) fn map(&self, index: usize) -> Option<&ColorMap> {
        self.maps.get(index)
    }
}

/// Signed ordered 4×4 dither offsets (−4..=3): the 4×4 Bayer matrix
/// `[[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]` halved, minus 4; one row
/// per `y & 3`.
const DITHER: [[i32; 4]; 4] = [[-4, 0, -3, 1], [2, -2, 3, -1], [-3, 1, -4, 0], [3, -1, 2, -2]];

/// Largest gradient geometry value (points, radii) in pixels: coordinates beyond `±2^26` are
/// clamped (shapes that large are clipped long before; the clamp keeps the sampling
/// arithmetic exact, see [`GradSampler`]).
const GEO_MAX: i64 = 1 << 26;

/// Clamps a gradient coordinate (pixels) to `±GEO_MAX`.
#[inline]
fn geo(v: i32) -> i64 {
    i64::from(v).clamp(-GEO_MAX, GEO_MAX)
}

/// Per-kind precomputed sampling parameters.
#[derive(Clone, Copy, Debug)]
enum Geo {
    Hor {
        k: i64,
    },
    Ver {
        k: i64,
    },
    Linear {
        sx: i64,
        sy: i64,
        dx: i64,
        dy: i64,
        len2: i64,
    },
    Radial {
        fx: i64,
        fy: i64,
        cdx: i64,
        cdy: i64,
        r0: i64,
        dr: i64,
        a: i64,
    },
    Conical {
        cx: i32,
        cy: i32,
        start: i32,
        span: i32,
    },
}

/// Samples a gradient over a drawn area.
///
/// Range argument of the sampling arithmetic: gradient geometry is clamped to `±2^26` px
/// ([`GEO_MAX`]) when the sampler is built, and a sampled pixel's offset from the area origin
/// is clamped to `±2^27` px (it is inside the drawn area for every caller; pixels further out
/// are far outside every gradient, where only the extend mode matters). The 16.16 parameter
/// `t` is reduced to an equivalent small value per row ([`GradSampler::equiv`]), so the
/// per-pixel steps below cannot overflow.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GradSampler {
    geo: Geo,
    origin: Point,
    extend: GradExtend,
}

/// `t` in 16.16 fixed point (`65536` = 1.0), or `None` where the gradient is undefined
/// (radial gradients outside the cone of circles).
type T16 = Option<i64>;

/// Largest pixel offset from the area origin that is sampled exactly (see [`GradSampler`]).
const OFS_MAX: i64 = 1 << 27;

/// The pixel offset `v − o`, clamped to `±OFS_MAX`.
#[inline]
fn ofs(v: i32, o: i32) -> i64 {
    // SAFE-ARITH: the difference of two `i32`s fits in `i64`.
    i64::from(v).wrapping_sub(i64::from(o)).clamp(-OFS_MAX, OFS_MAX)
}

impl GradSampler {
    /// A sampler for `g` drawn over `area`.
    pub fn new(g: &Gradient, area: Rect) -> Self {
        let (w, h) = (i64::from(area.width()), i64::from(area.height()));
        let geo = match g.kind {
            GradKind::Hor => Geo::Hor {
                // SAFE-ARITH: `w ≥ 2` here, so `w − 1 ≥ 1`; the fallback is never taken.
                k: if w > 1 {
                    (1i64 << 32).checked_div(w.wrapping_sub(1)).unwrap_or(0)
                } else {
                    0
                },
            },
            GradKind::Ver => Geo::Ver {
                // SAFE-ARITH: `h ≥ 2` here, as above.
                k: if h > 1 {
                    (1i64 << 32).checked_div(h.wrapping_sub(1)).unwrap_or(0)
                } else {
                    0
                },
            },
            GradKind::Linear { start, end } => {
                // SAFE-ARITH: `|dx|, |dy| ≤ 2^27` (coordinates clamped to `±2^26`), so
                // `len2 ≤ 2^55`.
                let (dx, dy) = (
                    geo(end.x).wrapping_sub(geo(start.x)),
                    geo(end.y).wrapping_sub(geo(start.y)),
                );
                Geo::Linear {
                    sx: geo(start.x),
                    sy: geo(start.y),
                    dx,
                    dy,
                    len2: dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)).max(1),
                }
            }
            GradKind::Radial {
                center,
                radius,
                focal,
                focal_radius,
            } => {
                // Coordinates in 1/8 px.
                // SAFE-ARITH: the coordinates are clamped to `±2^26` px (`geo`), so their
                // differences are within `±2^27` px = `±2^30` 1/8 px; `r0` and `dr` too. Each
                // square is at most `2^60`, so `a` is within `±2^61`.
                let (cdx, cdy) = (
                    geo(center.x).wrapping_sub(geo(focal.x)).wrapping_mul(8),
                    geo(center.y).wrapping_sub(geo(focal.y)).wrapping_mul(8),
                );
                let r0 = geo(focal_radius.max(0)).wrapping_mul(8);
                let dr = geo(radius.max(0)).wrapping_mul(8).wrapping_sub(r0);
                Geo::Radial {
                    fx: geo(focal.x).wrapping_mul(8),
                    fy: geo(focal.y).wrapping_mul(8),
                    cdx,
                    cdy,
                    r0,
                    dr,
                    a: cdx
                        .wrapping_mul(cdx)
                        .wrapping_add(cdy.wrapping_mul(cdy))
                        .wrapping_sub(dr.wrapping_mul(dr)),
                }
            }
            GradKind::Conical {
                center,
                start_angle,
                end_angle,
            } => {
                let s = start_angle.normalized().as_deci_deg();
                let mut span = end_angle.as_deci_deg().saturating_sub(start_angle.as_deci_deg());
                if span <= 0 || span > 3600 {
                    // SAFE-ARITH: both normalized angles are in `0..3600`.
                    span = end_angle
                        .normalized()
                        .as_deci_deg()
                        .wrapping_sub(s)
                        .rem_euclid(3600);
                    if span == 0 {
                        span = 3600;
                    }
                }
                Geo::Conical {
                    cx: center.x,
                    cy: center.y,
                    start: s,
                    span,
                }
            }
        };
        Self {
            geo,
            origin: area.origin(),
            extend: g.extend,
        }
    }

    /// Whether every pixel of a row has the same value (vertical gradient).
    pub fn is_row_constant(&self) -> bool {
        matches!(self.geo, Geo::Ver { .. })
    }

    /// `t` of the pixel at offset `(x, y)` from the area origin (`|x|, |y| ≤ 2^27 + 2^16`).
    #[inline]
    fn t(&self, x: i64, y: i64) -> T16 {
        match self.geo {
            // SAFE-ARITH: `|x|, |y| < 2^28` and `k ≤ 2^32`: products below 2^60.
            Geo::Hor { k } => Some(x.wrapping_mul(k) >> 16),
            Geo::Ver { k } => Some(y.wrapping_mul(k) >> 16),
            Geo::Linear { sx, sy, dx, dy, len2 } => {
                // SAFE-ARITH: `|x − sx|, |y − sy| < 2^29` and `|dx|, |dy| ≤ 2^27`: the dot
                // product is within `±2^57`, shifted within `±2^73` in `i128`. `len2 ≥ 1`
                // (`.max(1)`): the fallback is never taken.
                let dot = i128::from(
                    x.wrapping_sub(sx)
                        .wrapping_mul(dx)
                        .wrapping_add(y.wrapping_sub(sy).wrapping_mul(dy)),
                );
                div_rem_i128(dot << 16, i128::from(len2)).map(|(q, _)| sat64(q))
            }
            Geo::Radial {
                fx,
                fy,
                cdx,
                cdy,
                r0,
                dr,
                a,
            } => {
                // SAFE-ARITH: `|x|, |y| < 2^27 + 2^16` and `|fx|, |fy| ≤ 2^29`, so `|px|, |py| <
                // 2^30.6`; `|cdx|, |cdy|, |r0|, |dr| ≤ 2^30` and `|a| ≤ 2^61` (`new`). So `|b| ≤
                // 2·2^60.6 + 2^60 < 2^63` and `|c| ≤ 2·2^61.2 < 2^63` fit in `i64` (as before
                // the clamps), and in `i128` `b² < 2^126`, `|a·c| < 2^124`: the discriminant
                // fits.
                let (px, py) = (
                    x.wrapping_mul(8).wrapping_sub(fx),
                    y.wrapping_mul(8).wrapping_sub(fy),
                );
                let b = i128::from(
                    px.wrapping_mul(cdx)
                        .wrapping_add(py.wrapping_mul(cdy))
                        .wrapping_add(r0.wrapping_mul(dr)),
                );
                let c = i128::from(
                    px.wrapping_mul(px)
                        .wrapping_add(py.wrapping_mul(py))
                        .wrapping_sub(r0.wrapping_mul(r0)),
                );
                let (r0, dr) = (i128::from(r0), i128::from(dr));
                let a128 = i128::from(a);
                // SAFE-ARITH: `|r0| ≤ 2^30`, `|t| < 2^80` (below) and `|dr| ≤ 2^31`: below 2^112.
                let root_ok =
                    |t: i128| -> bool { r0.wrapping_mul(65536).wrapping_add(t.wrapping_mul(dr)) >= 0 };
                if a == 0 {
                    // `b != 0` checked by the division (`None` for `b == 0`).
                    let (t, _) = div_rem_i128(c << 16, b.wrapping_mul(2))?;
                    return root_ok(t).then_some(sat64(t));
                }
                let disc = b.wrapping_mul(b).wrapping_sub(a128.wrapping_mul(c));
                if disc < 0 {
                    return None;
                }
                let s = i128::from(isqrt64(u64::try_from(disc).unwrap_or(u64::MAX)));
                // The roots `(b ± s) · 2^16 / a`: `b + s ≥ b − s` and a truncating division is
                // monotonic (increasing for `a > 0`, decreasing for `a < 0`), so the larger root
                // is known before dividing; the smaller one is only computed when the larger
                // does not qualify (one division per pixel in the common case).
                // `a != 0` (the `a == 0` case returned above): the `?` is never taken.
                // SAFE-ARITH: `|b| < 2^63` and `s < 2^32`, shifted below `2^80`.
                let (plus, minus) = (b.wrapping_add(s) << 16, b.wrapping_sub(s) << 16);
                let (hi_num, lo_num) = if a > 0 { (plus, minus) } else { (minus, plus) };
                let (hi, _) = div_rem_i128(hi_num, a128)?;
                if root_ok(hi) {
                    return Some(sat64(hi));
                }
                let (lo, _) = div_rem_i128(lo_num, a128)?;
                root_ok(lo).then_some(sat64(lo))
            }
            Geo::Conical { cx, cy, start, span } => {
                // SAFE-ARITH: `|x|, |y| < 2^28` and the clamped center is within `±2^26`, so
                // the differences fit in `i32`.
                let (dx, dy) = (x.wrapping_sub(geo(cx)) as i32, y.wrapping_sub(geo(cy)) as i32);
                let ang = atan2(dy, dx).as_deci_deg();
                // SAFE-ARITH: `ang`, `start` are in `0..3600` and `span` in `1..=3600`.
                let rel = ang.wrapping_sub(start).rem_euclid(3600);
                let rel = if rel > span && rel > span.wrapping_add(3600i32.wrapping_sub(span) / 2) {
                    rel.wrapping_sub(3600)
                } else {
                    rel
                };
                // `span ≥ 1` (`new`): the fallback is never taken.
                (i64::from(rel) << 16).checked_div(i64::from(span))
            }
        }
    }

    /// The start value of an incremental row walk, reduced so that the walk's indices are
    /// unchanged and its steps stay in range: `t` has `frac` fraction bits (16 or 32) and the
    /// walk moves it by less than `travel − 2^(frac + 1)` in total. Padded values are clamped
    /// to `±travel` (beyond it the walk stays on one side of `[0, 1]`), repeated and reflected
    /// ones reduced modulo two periods, `2^period_log2` with `period_log2 = frac + 1`.
    #[inline]
    fn equiv(&self, t: i128, period_log2: u32, travel: i128) -> i64 {
        let v = match self.extend {
            // SAFE-ARITH: `travel < 2^64` (the callers' literals).
            GradExtend::Pad => t.clamp(travel.wrapping_neg(), travel),
            // SHIFT: `period_log2` is 17 or 33 (the callers' literals).
            // DIV: `2^period_log2 ≥ 2^17` (as above).
            GradExtend::Repeat | GradExtend::Reflect => t.rem_euclid(1i128 << period_log2),
        };
        sat64(v)
    }

    /// Color map index of `t` after the extend mode.
    #[inline]
    fn index(&self, t: i64) -> u8 {
        let t = match self.extend {
            GradExtend::Pad => t.clamp(0, 65536),
            GradExtend::Repeat => t.rem_euclid(65536),
            GradExtend::Reflect => {
                let r = t.rem_euclid(131_072);
                // SAFE-ARITH: `65_536 < r < 131_072` here.
                if r > 65536 { 131_072i64.wrapping_sub(r) } else { r }
            }
        };
        // SAFE-ARITH: `0 ≤ t ≤ 65_536`, so the index is `0..=255`.
        (t.wrapping_mul(255).wrapping_add(32768) >> 16) as u8
    }

    /// The packed color of pixel `(x, y)` (absolute coordinates).
    #[inline]
    pub fn sample(&self, map: &ColorMap, x: i32, y: i32) -> u32 {
        match self.t(ofs(x, self.origin.x), ofs(y, self.origin.y)) {
            // `map` has 256 entries and the index is a `u8`: no check remains.
            Some(t) => map.get(usize::from(self.index(t))).copied().unwrap_or(0),
            None => 0,
        }
    }

    /// Fills `out` (`Argb8888` bytes, `out.len() / 4` pixels, at most [`ROW_PIXELS`]; callers
    /// pass one span chunk of at most `max_span` pixels) with row `y` starting at `x0`. With
    /// `dither`, an ordered 4×4 offset (−4..=3) is added to each channel.
    pub fn fill_row(&self, map: &ColorMap, y: i32, x0: i32, dither: bool, out: &mut [u8]) {
        // The range arguments below assume at most `ROW_PIXELS` pixels (never more: the
        // fallback keeps the whole chunk).
        let n = out.len().min(4 * ROW_PIXELS);
        let out = out.get_mut(..n).unwrap_or_default();
        let yr = ofs(y, self.origin.y);
        let xr = ofs(x0, self.origin.x);
        let px = |map: &ColorMap, t: i64| map.get(usize::from(self.index(t))).copied().unwrap_or(0);
        match self.geo {
            Geo::Hor { k } => {
                // Incremental: t = x·k >> 16, with `acc` reduced to an equivalent start.
                // SAFE-ARITH: `|acc| ≤ 2^49` after `equiv` and every step adds `k ≤ 2^32` for
                // at most `ROW_PIXELS = 2^16` pixels (`2^48` in total): below 2^50.
                let start = i128::from(xr).wrapping_mul(i128::from(k));
                let mut acc = self.equiv(start, 33, 1 << 49);
                for p in out.chunks_exact_mut(4) {
                    p.copy_from_slice(&px(map, acc >> 16).to_le_bytes());
                    acc = acc.wrapping_add(k);
                }
            }
            Geo::Linear { sx, sy, dx, dy, len2 } => {
                // t(x) = (base + i·dx) · 65536 / len2, stepped exactly with a remainder.
                // SAFE-ARITH: `|xr − sx|, |yr − sy| < 2^29` and `|dx|, |dy| ≤ 2^27`, so
                // `|base| < 2^57` and `num < 2^73` in `i128`. `len2 ≥ 1`: the division
                // fallbacks are never taken.
                let base = i128::from(
                    xr.wrapping_sub(sx)
                        .wrapping_mul(dx)
                        .wrapping_add(yr.wrapping_sub(sy).wrapping_mul(dy)),
                );
                let len = i128::from(len2);
                let num = base << 16;
                let (q0, r) = div_rem_euclid_i128(num, len).unwrap_or((0, 0));
                let mut r = sat64(r);
                // The index of `q` only depends on `q` modulo two periods (or its clamp).
                let mut q = self.equiv(q0, 17, 1 << 33);
                let step = dx << 16;
                let (sq, sr) = (
                    step.checked_div_euclid(len2).unwrap_or(0),
                    step.checked_rem_euclid(len2).unwrap_or(0),
                );
                // SAFE-ARITH: `|q| ≤ 2^33` and `|sq| ≤ 2^16 · |dx| / len2 ≤ 2^16` (`len2 ≥
                // dx²`), so after at most `ROW_PIXELS = 2^16` pixels (travel below `2^32 +
                // 2^16`) `|q| < 2^34`; `0 ≤ r, sr < len2 ≤ 2^55`, so `r + sr < 2^56`.
                for p in out.chunks_exact_mut(4) {
                    p.copy_from_slice(&px(map, q).to_le_bytes());
                    q = q.wrapping_add(sq);
                    r = r.wrapping_add(sr);
                    if r >= len2 {
                        r = r.wrapping_sub(len2);
                        q = q.wrapping_add(1);
                    }
                }
            }
            _ => {
                // `|xr| ≤ 2^27`, plus at most `ROW_PIXELS = 2^16` pixels: the bounded range
                // never reaches its end, and every `x` is below `2^28` (`t`'s range).
                for (x, p) in (xr..i64::MAX).zip(out.chunks_exact_mut(4)) {
                    let v = match self.t(x, yr) {
                        Some(t) => px(map, t),
                        None => 0,
                    };
                    p.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        if dither {
            let row = match y & 3 {
                0 => &DITHER[0],
                1 => &DITHER[1],
                2 => &DITHER[2],
                _ => &DITHER[3],
            };
            let offsets = row.iter().cycle().skip((x0 & 3) as usize);
            for (p, &d) in out.chunks_exact_mut(4).zip(offsets) {
                for c in p.iter_mut().take(3) {
                    // SAFE-ARITH: `|d| ≤ 4`.
                    *c = i32::from(*c).wrapping_add(d).clamp(0, 255) as u8;
                }
            }
        }
    }
}

/// Most pixels [`GradSampler::fill_row`] fills per call (`max_span` is a `u16`).
pub(crate) const ROW_PIXELS: usize = 1 << 16;

/// Saturates an `i128` into the `i64` range.
#[inline]
fn sat64(v: i128) -> i64 {
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_map_endpoints_match_stops() {
        let stops = [
            GradStop::new(Color::RED, Fraction::from_raw(10)),
            GradStop::with_opa(Color::GREEN, Opa::from_raw(100), Fraction::from_raw(128)),
            GradStop::new(Color::BLUE, Fraction::from_raw(240)),
        ];
        let mut map = [0u32; 256];
        build_map(&stops, &mut map);
        assert_eq!(map[0], pack(Color::RED, 255));
        assert_eq!(map[10], pack(Color::RED, 255));
        assert_eq!(map[128], pack(Color::GREEN, 100));
        assert_eq!(map[240], pack(Color::BLUE, 255));
        assert_eq!(map[255], pack(Color::BLUE, 255));
        // Midway red → green: channels near half.
        let mid = map[69];
        assert!(((mid >> 16) & 0xFF).abs_diff(128) < 4);
    }

    #[test]
    fn linear_t_monotonic_along_axis() {
        let g = Gradient::new(
            GradKind::Linear {
                start: Point::new(0, 0),
                end: Point::new(40, 30),
            },
            &[
                GradStop::new(Color::BLACK, Fraction::ZERO),
                GradStop::new(Color::WHITE, Fraction::ONE),
            ],
        );
        let s = GradSampler::new(&g, Rect::from_xywh(0, 0, 50, 40));
        let mut prev = i64::MIN;
        for i in 0..=10 {
            let t = s.t(4 * i, 3 * i).unwrap();
            assert!(t > prev);
            prev = t;
        }
        assert_eq!(s.t(0, 0), Some(0));
        assert_eq!(s.t(40, 30), Some(65536));
        // The incremental row fill equals the direct formula.
        let mut map = [0u32; 256];
        build_map(g.stops(), &mut map);
        let mut row = [0u8; 50 * 4];
        s.fill_row(&map, 7, 0, false, &mut row);
        for x in 0..50 {
            let v = u32::from_le_bytes(row[x * 4..x * 4 + 4].try_into().unwrap());
            assert_eq!(v, s.sample(&map, x as i32, 7), "x={x}");
        }
    }

    #[test]
    fn radial_centered_is_distance() {
        let g = Gradient::new(
            GradKind::Radial {
                center: Point::new(50, 50),
                radius: 40,
                focal: Point::new(50, 50),
                focal_radius: 0,
            },
            &[
                GradStop::new(Color::BLACK, Fraction::ZERO),
                GradStop::new(Color::WHITE, Fraction::ONE),
            ],
        );
        let s = GradSampler::new(&g, Rect::from_xywh(0, 0, 100, 100));
        assert_eq!(s.t(50, 50), Some(0));
        assert_eq!(s.t(90, 50), Some(65536));
        assert_eq!(s.t(50, 70), Some(32768));
    }

    #[test]
    fn extend_modes() {
        let g = Gradient::new(GradKind::Hor, &[]);
        let mut s = GradSampler::new(&g, Rect::from_xywh(0, 0, 10, 10));
        assert_eq!(s.index(-5), 0);
        assert_eq!(s.index(70_000), 255);
        s.extend = GradExtend::Repeat;
        assert_eq!(s.index(65536 + 32768), 128);
        s.extend = GradExtend::Reflect;
        assert_eq!(s.index(65536 + 16384), s.index(65536 - 16384));
    }

    #[test]
    fn cache_lru() {
        let mut c = GradientCache::new(2);
        let g = |c: Color| Gradient::new(GradKind::Ver, &[GradStop::new(c, Fraction::ZERO)]);
        let a = c.lookup(&g(Color::RED));
        assert_eq!(c.lookup(&g(Color::RED)), a);
        c.lookup(&g(Color::GREEN));
        c.lookup(&g(Color::RED));
        c.lookup(&g(Color::BLUE)); // evicts GREEN
        assert_eq!(c.stats(), CacheStats { hits: 2, misses: 3 });
        let cap = c.maps.capacity();
        c.lookup(&g(Color::GREEN));
        assert_eq!(c.maps.capacity(), cap);
        let i = c.lookup(&g(Color::GREEN));
        assert_eq!(c.map(i).map(|m| m[0]), Some(pack(Color::GREEN, 255)));
    }
}
