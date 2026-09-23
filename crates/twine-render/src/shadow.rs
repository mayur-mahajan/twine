//! Box shadows: [`ShadowDsc`], [`shadow_ext_size`] and the cached blurred corner
//! ([`ShadowCache`]).
//!
//! The shadow shape is the object's area moved by the offsets and grown by `spread`
//! (radius grows by `spread` too when the object is rounded). Blurring follows LVGL's box
//! shadow: the shape's coverage is box-blurred twice horizontally and twice vertically with a
//! window of about `width / 2`, so the shadow fades over `width` pixels centered on the shape's
//! edge (50 % at the edge, reaching `width / 2` outside). Only one quarter of the result — the
//! "corner map" — is computed and cached; the four corners are drawn by mirroring it, the edges
//! repeat its last row/column and the center is solid.

use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;

use twine_core::{Color, Opa, Rect};

use crate::caches::CacheStats;
use crate::circle::Quarter;
use crate::dispatch::warn_once;
use crate::rrect::{RowSpan, eff_radius};

/// Box shadow parameters (the default draws no shadow).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ShadowDsc {
    /// Blur width in pixels (0 = sharp).
    pub width: i32,
    /// Horizontal offset.
    pub ofs_x: i32,
    /// Vertical offset.
    pub ofs_y: i32,
    /// Growth of the shape (negative shrinks).
    pub spread: i32,
    /// Shadow color.
    pub color: Color,
    /// Shadow opacity.
    pub opa: Opa,
}

impl Default for ShadowDsc {
    fn default() -> Self {
        Self {
            width: 0,
            ofs_x: 0,
            ofs_y: 0,
            spread: 0,
            color: Color::BLACK,
            opa: Opa::TRANSP,
        }
    }
}

impl ShadowDsc {
    /// Whether drawing this shadow produces anything.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        (self.width > 0 || self.spread != 0 || self.ofs_x != 0 || self.ofs_y != 0)
            && !self.opa.is_transparent()
    }
}

/// How far a shadow can reach outside its object (for the engine's extra draw area):
/// `width / 2 + max(|ofs_x|, |ofs_y|) + spread` (never negative).
///
/// ```
/// use twine_render::{ShadowDsc, shadow_ext_size};
/// let s = ShadowDsc { width: 20, ofs_x: 5, ofs_y: -8, spread: 3, ..Default::default() };
/// assert_eq!(shadow_ext_size(&s), 10 + 8 + 3);
/// ```
#[must_use]
pub fn shadow_ext_size(dsc: &ShadowDsc) -> i32 {
    (dsc.width.max(0) / 2 + dsc.ofs_x.abs().max(dsc.ofs_y.abs()) + dsc.spread).max(0)
}

/// Bytes of corner map reserved per shadow cache entry (a 64 × 64 corner).
pub(crate) const MAP_BYTES_PER_ENTRY: usize = 64 * 64;

/// Resolved shadow geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShadowGeo {
    /// The (unblurred) shadow shape.
    pub shape: Rect,
    /// Its radius (effective).
    pub radius: i32,
    /// Radius of one box-blur pass (window `2·p + 1`); the blur reaches `2·p` pixels.
    pub p: i32,
    /// Bounding box of the blurred shadow (`shape` grown by `2·p`).
    pub bounds: Rect,
    /// Corner map width and height.
    pub qw: i32,
    pub qh: i32,
}

impl ShadowGeo {
    pub fn new(area: Rect, radius: i32, dsc: &ShadowDsc, p: i32) -> Self {
        let shape = area.translate(dsc.ofs_x, dsc.ofs_y).expand(dsc.spread);
        let r = if radius > 0 {
            radius.saturating_add(dsc.spread)
        } else {
            0
        };
        let r = eff_radius(shape, r);
        let e = 2 * p;
        let bounds = shape.expand(e);
        let cs = r + 2 * e + 1;
        let qw = cs.min((bounds.width() + 1) / 2);
        let qh = cs.min((bounds.height() + 1) / 2);
        Self {
            shape,
            radius: r,
            p,
            bounds,
            qw,
            qh,
        }
    }

    /// Scratch `u16` values needed to compute the corner map.
    pub fn scratch_needed(&self) -> usize {
        let e = 2 * self.p as usize;
        (2 * self.qw as usize + self.qh as usize + 3 * e) + 4
    }

    /// Corner map bytes.
    pub fn map_bytes(&self) -> usize {
        (self.qw.max(0) * self.qh.max(0)) as usize
    }

    /// Cache key.
    fn key(&self) -> Key {
        Key {
            r: self.radius,
            p: self.p,
            qw: self.qw,
            qh: self.qh,
            small_x: self.qw * 2 >= self.bounds.width(),
            small_y: self.qh * 2 >= self.bounds.height(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    r: i32,
    p: i32,
    qw: i32,
    qh: i32,
    small_x: bool,
    small_y: bool,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: Key,
    off: usize,
    len: usize,
    last_use: u32,
}

static WARN_BIG: AtomicBool = AtomicBool::new(false);

/// LRU cache of blurred shadow corner maps (fixed arena, never grows).
#[derive(Debug, Default)]
pub struct ShadowCache {
    entries: Vec<Entry>,
    max_entries: usize,
    arena: Vec<u8>,
    tick: u32,
    stats: CacheStats,
}

impl ShadowCache {
    pub(crate) fn new(entries: u8) -> Self {
        let n = usize::from(entries).max(1);
        Self {
            entries: Vec::with_capacity(n),
            max_entries: n,
            arena: Vec::with_capacity(n * MAP_BYTES_PER_ENTRY),
            tick: 0,
            stats: CacheStats::default(),
        }
    }

    pub(crate) fn bytes_reserved(&self) -> usize {
        self.arena.capacity() + self.entries.capacity() * core::mem::size_of::<Entry>()
    }

    pub(crate) fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Resolves the geometry for drawing: reduces the blur (with a one-time warning) until the
    /// corner map fits the arena and `scratch_len`.
    pub(crate) fn fit(&self, area: Rect, radius: i32, dsc: &ShadowDsc, scratch_len: usize) -> ShadowGeo {
        let mut p = dsc.width.max(0) / 4;
        loop {
            let g = ShadowGeo::new(area, radius, dsc, p);
            if p == 0 || (g.map_bytes() <= self.arena.capacity() && g.scratch_needed() <= scratch_len) {
                if p != dsc.width.max(0) / 4 && warn_once(&WARN_BIG) {
                    twine_core::warn!(
                        target: "twine::render",
                        "shadow width {} with radius {} exceeds the shadow cache budget; blur reduced (raise shadow_cache_entries)",
                        dsc.width,
                        radius
                    );
                }
                return g;
            }
            p /= 2;
        }
    }

    fn evict_lru(&mut self) {
        let Some((i, _)) = self.entries.iter().enumerate().min_by_key(|(_, e)| e.last_use) else {
            return;
        };
        let e = self.entries.swap_remove(i);
        let end = e.off + e.len;
        let tail = self.arena.len() - end;
        self.arena.copy_within(end.., e.off);
        self.arena.truncate(e.off + tail);
        for o in &mut self.entries {
            if o.off > e.off {
                o.off -= e.len;
            }
        }
    }

    /// The corner map of `g` (row-major `qw × qh`), computed on a miss. `g` must come from
    /// [`fit`](Self::fit) and `scratch` must hold `g.scratch_needed()` values.
    pub(crate) fn corner(&mut self, g: &ShadowGeo, scratch: &mut [u16]) -> &[u8] {
        self.tick = self.tick.wrapping_add(1);
        let key = g.key();
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            self.entries[i].last_use = self.tick;
            self.stats.hits = self.stats.hits.saturating_add(1);
            let e = self.entries[i];
            return &self.arena[e.off..e.off + e.len];
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        let size = g.map_bytes();
        while self.entries.len() >= self.max_entries || self.arena.capacity() - self.arena.len() < size {
            if self.entries.is_empty() {
                break;
            }
            self.evict_lru();
        }
        let off = self.arena.len();
        self.arena.resize(off + size, 0);
        build_corner(g, &mut self.arena[off..off + size], scratch);
        self.entries.push(Entry {
            key,
            off,
            len: size,
            last_use: self.tick,
        });
        &self.arena[off..off + size]
    }
}

/// Computes the blurred corner map of `g` into `map` (`qw × qh`, row-major).
fn build_corner(g: &ShadowGeo, map: &mut [u8], scratch: &mut [u16]) {
    let (qw, qh, p) = (g.qw as usize, g.qh as usize, g.p as usize);
    let e = 2 * p;
    let (bw, bh) = (g.bounds.width(), g.bounds.height());
    // The shape in bounds-relative coordinates.
    let shape = g.shape.translate(-g.bounds.x0, -g.bounds.y0);
    let q = if g.radius > 0 {
        Quarter::Computed { r: g.radius }
    } else {
        Quarter::Square
    };
    let win = (2 * p + 1) as u32;
    let (src, rest) = scratch.split_at_mut(qw + 2 * e);
    let (out1, rest) = rest.split_at_mut(qw + e);
    let col = &mut rest[..qh + e];

    // Horizontal passes, one map row at a time. Values carry 8 fractional bits.
    for my in 0..qh {
        let rs = RowSpan::new(shape, &q, my as i32);
        for (i, v) in src.iter_mut().enumerate() {
            *v = u16::from(rs.cov(i as i32 - e as i32)) << 8;
        }
        if p == 0 {
            for x in 0..qw {
                map[my * qw + x] = (src[x] >> 8) as u8;
            }
            continue;
        }
        // Pass 1: out1[j] (x = j − p) = mean of src[j .. j + 2p].
        let mut sum: u32 = src[..2 * p].iter().map(|&v| u32::from(v)).sum();
        for j in 0..qw + e {
            sum += u32::from(src[j + 2 * p]);
            out1[j] = ((sum + win / 2) / win) as u16;
            sum -= u32::from(src[j]);
        }
        // Pass 2: map[x] = mean of out1[x .. x + 2p].
        let mut sum: u32 = out1[..2 * p].iter().map(|&v| u32::from(v)).sum();
        for x in 0..qw {
            sum += u32::from(out1[x + 2 * p]);
            let v = (sum + win / 2) / win;
            map[my * qw + x] = ((v + 128) >> 8).min(255) as u8;
            sum -= u32::from(out1[x]);
        }
    }
    if p == 0 {
        return;
    }
    // Rows of the horizontally blurred image beyond the map: rows below the map repeat the last
    // one (large shapes) or mirror (small shapes, where the map covers half the height).
    let small_y = 2 * g.qh >= bh;
    let h_row = |y: i32| -> Option<usize> {
        if y < 0 || y >= bh {
            None
        } else if (y as usize) < qh {
            Some(y as usize)
        } else if small_y {
            let m = bh - 1 - y;
            (m >= 0).then_some(m as usize)
        } else {
            Some(qh - 1)
        }
    };
    let _ = bw;
    for x in 0..qw {
        let hv = |y: i32| -> u32 { h_row(y).map_or(0, |r| u32::from(map[r * qw + x]) << 8) };
        // Pass 1: col[j] (y = j − p) = mean of rows y − p ..= y + p.
        let mut sum: u32 = (-(e as i32)..0).map(hv).sum();
        for (j, c) in col.iter_mut().enumerate() {
            let y = j as i32 - p as i32;
            sum += hv(y + p as i32);
            *c = ((sum + win / 2) / win) as u16;
            sum -= hv(y - p as i32);
        }
        // Pass 2.
        let mut sum: u32 = col[..2 * p].iter().map(|&v| u32::from(v)).sum();
        for y in 0..qh {
            sum += u32::from(col[y + 2 * p]);
            let v = (sum + win / 2) / win;
            map[y * qw + x] = ((v + 128) >> 8).min(255) as u8;
            sum -= u32::from(col[y]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_ext_size_formula() {
        let s = ShadowDsc {
            width: 15,
            ofs_x: -3,
            ofs_y: 2,
            spread: 4,
            ..Default::default()
        };
        assert_eq!(shadow_ext_size(&s), 7 + 3 + 4);
        let s = ShadowDsc {
            width: 0,
            spread: -10,
            ..Default::default()
        };
        assert_eq!(shadow_ext_size(&s), 0);
        // The blurred bounds never exceed the ext size.
        for w in 0..60 {
            let s = ShadowDsc {
                width: w,
                ofs_x: 3,
                ofs_y: -2,
                spread: 2,
                ..Default::default()
            };
            let area = Rect::from_xywh(100, 100, 50, 40);
            let g = ShadowGeo::new(area, 8, &s, w / 4);
            assert!(area.expand(shadow_ext_size(&s)).contains_rect(&g.bounds), "w={w}");
        }
    }

    #[test]
    fn corner_map_monotonic_toward_inside() {
        let dsc = ShadowDsc {
            width: 20,
            ..Default::default()
        };
        let c = ShadowCache::new(1);
        let g = c.fit(Rect::from_xywh(0, 0, 100, 100), 10, &dsc, 1000);
        assert_eq!(g.p, 5);
        let mut map = alloc::vec![0u8; g.map_bytes()];
        let mut scratch = [0u16; 1000];
        build_corner(&g, &mut map, &mut scratch);
        let (qw, qh) = (g.qw as usize, g.qh as usize);
        for y in 0..qh {
            for x in 1..qw {
                assert!(map[y * qw + x] >= map[y * qw + x - 1], "row {y} x {x}");
            }
        }
        for x in 0..qw {
            for y in 1..qh {
                assert!(map[y * qw + x] >= map[(y - 1) * qw + x], "col {x} y {y}");
            }
        }
        assert_eq!(map[0], 0);
        assert_eq!(map[qw * qh - 1], 255);
        // 50 % at the shape's straight edge (e = 10 px inside the bounds).
        let edge = map[(qh - 1) * qw + 10];
        assert!((110..=145).contains(&edge), "{edge}");
    }
}
