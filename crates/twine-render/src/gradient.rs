//! Gradients: [`Gradient`], [`GradStop`], [`GradKind`], [`GradExtend`], the color-map
//! [`GradientCache`] and per-pixel sampling.
//!
//! Geometry (points, centers, radii) is relative to the top-left of the drawn area, in pixels;
//! a gradient's parameter `t` is 0 at the start and 1 at the end. Stops map `t` (as
//! `frac / 255`) to colors and opacities; the 256-entry color map of the stops is cached.

use alloc::vec::Vec;

use twine_core::math::{atan2, isqrt64};
use twine_core::{Angle, Color, Opa, Point, Rect};

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
    /// Position, `0..=255` along the gradient.
    pub frac: u8,
}

impl GradStop {
    /// An opaque stop.
    #[must_use]
    pub const fn new(color: Color, frac: u8) -> Self {
        Self {
            color,
            opa: Opa::COVER,
            frac,
        }
    }

    /// A stop with opacity.
    #[must_use]
    pub const fn with_opa(color: Color, opa: Opa, frac: u8) -> Self {
        Self { color, opa, frac }
    }
}

/// Gradient geometry (relative to the drawn area's top-left).
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
/// use twine_core::Color;
/// use twine_render::{GradExtend, GradKind, GradStop, Gradient};
///
/// const G: Gradient = Gradient::new(GradKind::Ver, &[GradStop::new(Color::RED, 0), GradStop::new(Color::BLUE, 255)])
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
            frac: 0,
        }; MAX_STOPS];
        let n = if stops.len() > MAX_STOPS {
            MAX_STOPS
        } else {
            stops.len()
        };
        let mut i = 0;
        while i < n {
            s[i] = stops[i];
            i += 1;
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
        &self.stops[..usize::from(self.count)]
    }
}

/// Packs `(color, opa)` into an `Argb8888` value.
#[inline(always)]
const fn pack(c: Color, a: u8) -> u32 {
    ((a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// Fills `map` with 256 samples (packed `0xAARRGGBB`) of the stops.
pub(crate) fn build_map(stops: &[GradStop], map: &mut [u32]) {
    let Some(first) = stops.first() else {
        map.fill(0);
        return;
    };
    let last = stops[stops.len() - 1];
    for (i, out) in map.iter_mut().enumerate().take(256) {
        let i = i as i32;
        let v = match stops.iter().position(|s| i32::from(s.frac) >= i) {
            None => pack(last.color, last.opa.0),
            Some(0) => pack(first.color, first.opa.0),
            Some(k) => {
                let (a, b) = (stops[k - 1], stops[k]);
                let (fa, fb) = (i32::from(a.frac), i32::from(b.frac));
                if fb <= fa {
                    pack(b.color, b.opa.0)
                } else {
                    let t = ((i - fa) * 256 / (fb - fa)) as u32;
                    let l = |x: u8, y: u8| -> u8 {
                        ((u32::from(x) * (256 - t) + u32::from(y) * t + 128) >> 8) as u8
                    };
                    pack(
                        Color::new(
                            l(a.color.r, b.color.r),
                            l(a.color.g, b.color.g),
                            l(a.color.b, b.color.b),
                        ),
                        l(a.opa.0, b.opa.0),
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

/// LRU cache of gradient color maps (256 samples each), keyed by the stops.
#[derive(Debug, Default)]
pub struct GradientCache {
    entries: Vec<MapEntry>,
    maps: Vec<u32>,
    max_entries: usize,
    tick: u32,
    stats: CacheStats,
}

impl GradientCache {
    pub(crate) fn new(entries: u8) -> Self {
        let n = usize::from(entries).max(1);
        Self {
            entries: Vec::with_capacity(n),
            maps: Vec::with_capacity(n * 256),
            max_entries: n,
            tick: 0,
            stats: CacheStats::default(),
        }
    }

    pub(crate) fn bytes_reserved(&self) -> usize {
        self.maps.capacity() * 4 + self.entries.capacity() * core::mem::size_of::<MapEntry>()
    }

    pub(crate) fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Index of the color map of `g`, building it (and evicting the LRU entry) on a miss.
    pub(crate) fn lookup(&mut self, g: &Gradient) -> usize {
        self.tick = self.tick.wrapping_add(1);
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.count == g.count && e.stops[..usize::from(e.count)] == *g.stops())
        {
            self.entries[i].last_use = self.tick;
            self.stats.hits = self.stats.hits.saturating_add(1);
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
            self.maps.resize(self.entries.len() * 256, 0);
            self.entries.len() - 1
        } else {
            let (i, _) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_use)
                .unwrap_or((0, &entry));
            self.entries[i] = entry;
            i
        };
        build_map(g.stops(), &mut self.maps[i * 256..(i + 1) * 256]);
        i
    }

    /// The color map at `index` (from [`lookup`](Self::lookup)).
    pub(crate) fn map(&self, index: usize) -> &[u32] {
        &self.maps[index * 256..(index + 1) * 256]
    }
}

/// 4×4 Bayer matrix, values `0..16`.
const BAYER: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

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
#[derive(Clone, Copy, Debug)]
pub(crate) struct GradSampler {
    geo: Geo,
    origin: Point,
    extend: GradExtend,
}

/// `t` in 16.16 fixed point (`65536` = 1.0), or `None` where the gradient is undefined
/// (radial gradients outside the cone of circles).
type T16 = Option<i64>;

impl GradSampler {
    /// A sampler for `g` drawn over `area`.
    pub fn new(g: &Gradient, area: Rect) -> Self {
        let (w, h) = (i64::from(area.width()), i64::from(area.height()));
        let geo = match g.kind {
            GradKind::Hor => Geo::Hor {
                k: if w > 1 { (1 << 32) / (w - 1) } else { 0 },
            },
            GradKind::Ver => Geo::Ver {
                k: if h > 1 { (1 << 32) / (h - 1) } else { 0 },
            },
            GradKind::Linear { start, end } => {
                let (dx, dy) = (i64::from(end.x - start.x), i64::from(end.y - start.y));
                Geo::Linear {
                    sx: i64::from(start.x),
                    sy: i64::from(start.y),
                    dx,
                    dy,
                    len2: (dx * dx + dy * dy).max(1),
                }
            }
            GradKind::Radial {
                center,
                radius,
                focal,
                focal_radius,
            } => {
                // Coordinates in 1/8 px.
                let (cdx, cdy) = (
                    8 * i64::from(center.x - focal.x),
                    8 * i64::from(center.y - focal.y),
                );
                let r0 = 8 * i64::from(focal_radius.max(0));
                let dr = 8 * i64::from(radius.max(0)) - r0;
                Geo::Radial {
                    fx: 8 * i64::from(focal.x),
                    fy: 8 * i64::from(focal.y),
                    cdx,
                    cdy,
                    r0,
                    dr,
                    a: cdx * cdx + cdy * cdy - dr * dr,
                }
            }
            GradKind::Conical {
                center,
                start_angle,
                end_angle,
            } => {
                let s = start_angle.normalized().0;
                let mut span = end_angle.0 - start_angle.0;
                if span <= 0 || span > 3600 {
                    span = (end_angle.normalized().0 - s).rem_euclid(3600);
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

    #[inline]
    fn t(&self, x: i64, y: i64) -> T16 {
        match self.geo {
            Geo::Hor { k } => Some((x * k) >> 16),
            Geo::Ver { k } => Some((y * k) >> 16),
            Geo::Linear { sx, sy, dx, dy, len2 } => Some((((x - sx) * dx + (y - sy) * dy) << 16) / len2),
            Geo::Radial {
                fx,
                fy,
                cdx,
                cdy,
                r0,
                dr,
                a,
            } => {
                let (px, py) = (8 * x - fx, 8 * y - fy);
                let b = i128::from(px * cdx + py * cdy + r0 * dr);
                let c = i128::from(px * px + py * py - r0 * r0);
                let a128 = i128::from(a);
                let root_ok = |t: i128| -> bool { i128::from(r0) * 65536 + t * i128::from(dr) >= 0 };
                if a == 0 {
                    if b == 0 {
                        return None;
                    }
                    let t = (c << 16) / (2 * b);
                    return root_ok(t).then_some(t as i64);
                }
                let disc = b * b - a128 * c;
                if disc < 0 {
                    return None;
                }
                let s = i128::from(isqrt64(u64::try_from(disc).unwrap_or(u64::MAX)));
                let (t1, t2) = (((b + s) << 16) / a128, ((b - s) << 16) / a128);
                let (hi, lo) = if t1 >= t2 { (t1, t2) } else { (t2, t1) };
                if root_ok(hi) {
                    Some(hi as i64)
                } else if root_ok(lo) {
                    Some(lo as i64)
                } else {
                    None
                }
            }
            Geo::Conical { cx, cy, start, span } => {
                let ang = atan2((y as i32) - cy, (x as i32) - cx).0;
                let rel = (ang - start).rem_euclid(3600);
                let rel = if rel > span && rel > span + (3600 - span) / 2 {
                    rel - 3600
                } else {
                    rel
                };
                Some((i64::from(rel) << 16) / i64::from(span))
            }
        }
    }

    /// Color map index of `t` after the extend mode.
    #[inline]
    fn index(&self, t: i64) -> usize {
        let t = match self.extend {
            GradExtend::Pad => t.clamp(0, 65536),
            GradExtend::Repeat => t.rem_euclid(65536),
            GradExtend::Reflect => {
                let r = t.rem_euclid(131_072);
                if r > 65536 { 131_072 - r } else { r }
            }
        };
        ((t * 255 + 32768) >> 16) as usize
    }

    /// The packed color of pixel `(x, y)` (absolute coordinates).
    #[inline]
    pub fn sample(&self, map: &[u32], x: i32, y: i32) -> u32 {
        match self.t(i64::from(x - self.origin.x), i64::from(y - self.origin.y)) {
            Some(t) => map[self.index(t)],
            None => 0,
        }
    }

    /// Fills `out` (`Argb8888` bytes) with the `n` pixels of row `y` starting at `x0`. With
    /// `dither`, an ordered 4×4 offset (−4..=3) is added to each channel.
    pub fn fill_row(&self, map: &[u32], y: i32, x0: i32, n: usize, dither: bool, out: &mut [u8]) {
        let yr = i64::from(y - self.origin.y);
        let out = &mut out[..n * 4];
        match self.geo {
            Geo::Hor { k } => {
                // Incremental: t = x·k >> 16.
                let mut acc = i64::from(x0 - self.origin.x) * k;
                for px in out.chunks_exact_mut(4) {
                    px.copy_from_slice(&map[self.index(acc >> 16)].to_le_bytes());
                    acc += k;
                }
            }
            Geo::Linear { sx, sy, dx, dy, len2 } => {
                let base = (i64::from(x0 - self.origin.x) - sx) * dx + (yr - sy) * dy;
                // t(x) = (base + i·dx) · 65536 / len2, stepped exactly with a remainder.
                let num = base << 16;
                let (mut q, mut r) = (num.div_euclid(len2), num.rem_euclid(len2));
                let step = dx << 16;
                let (sq, sr) = (step.div_euclid(len2), step.rem_euclid(len2));
                for px in out.chunks_exact_mut(4) {
                    px.copy_from_slice(&map[self.index(q)].to_le_bytes());
                    q += sq;
                    r += sr;
                    if r >= len2 {
                        r -= len2;
                        q += 1;
                    }
                }
            }
            _ => {
                for (x, px) in (i64::from(x0 - self.origin.x)..).zip(out.chunks_exact_mut(4)) {
                    let v = match self.t(x, yr) {
                        Some(t) => map[self.index(t)],
                        None => 0,
                    };
                    px.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        if dither {
            let by = (y & 3) as usize * 4;
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                let d = BAYER[by + ((x0 + i as i32) & 3) as usize] / 2 - 4;
                for c in &mut px[..3] {
                    *c = (i32::from(*c) + d).clamp(0, 255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_map_endpoints_match_stops() {
        let stops = [
            GradStop::new(Color::RED, 10),
            GradStop::with_opa(Color::GREEN, Opa(100), 128),
            GradStop::new(Color::BLUE, 240),
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
            &[GradStop::new(Color::BLACK, 0), GradStop::new(Color::WHITE, 255)],
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
        s.fill_row(&map, 7, 0, 50, false, &mut row);
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
            &[GradStop::new(Color::BLACK, 0), GradStop::new(Color::WHITE, 255)],
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
        let g = |c: Color| Gradient::new(GradKind::Ver, &[GradStop::new(c, 0)]);
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
        assert_eq!(c.map(i)[0], pack(Color::GREEN, 255));
    }
}
