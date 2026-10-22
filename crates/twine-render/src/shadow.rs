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
use core::num::NonZeroU32;
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
    // Saturating: exact for every shadow whose reach fits an `i32`.
    (dsc.width.max(0) / 2)
        .saturating_add(dsc.ofs_x.saturating_abs().max(dsc.ofs_y.saturating_abs()))
        .saturating_add(dsc.spread)
        .max(0)
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
        // Saturating: exact for every blur `fit` accepts (it keeps the corner map within the
        // cache arena and the scratch buffer).
        let e = p.saturating_mul(2);
        let bounds = shape.expand(e);
        let cs = r.saturating_add(e.saturating_mul(2)).saturating_add(1);
        let qw = cs.min(bounds.width().saturating_add(1) / 2);
        let qh = cs.min(bounds.height().saturating_add(1) / 2);
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
        let (p, qw, qh) = self.sizes();
        let e = p.saturating_mul(2);
        qw.saturating_mul(2)
            .saturating_add(qh)
            .saturating_add(e.saturating_mul(3))
            .saturating_add(4)
    }

    /// Corner map bytes.
    pub fn map_bytes(&self) -> usize {
        let (_, qw, qh) = self.sizes();
        qw.saturating_mul(qh)
    }

    /// `(p, qw, qh)` as sizes (all non-negative by construction).
    fn sizes(&self) -> (usize, usize, usize) {
        let z = |v: i32| crate::num::span(0, v);
        (z(self.p), z(self.qw), z(self.qh))
    }

    /// Cache key.
    fn key(&self) -> Key {
        Key {
            r: self.radius,
            p: self.p,
            qw: self.qw,
            qh: self.qh,
            small_x: self.qw.saturating_mul(2) >= self.bounds.width(),
            small_y: self.qh.saturating_mul(2) >= self.bounds.height(),
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
    /// Fresh mode ([`RenderCaches::set_fresh`](crate::RenderCaches::set_fresh)): a hit
    /// recomputes its corner map before it is used.
    #[cfg(feature = "display-integrity")]
    fresh: bool,
}

impl ShadowCache {
    pub(crate) fn new(entries: u8) -> Self {
        let n = usize::from(entries).max(1);
        Self {
            entries: Vec::with_capacity(n),
            max_entries: n,
            arena: Vec::with_capacity(n.saturating_mul(MAP_BYTES_PER_ENTRY)),
            tick: 0,
            stats: CacheStats::default(),
            #[cfg(feature = "display-integrity")]
            fresh: false,
        }
    }

    /// Fresh mode: every [`corner`](Self::corner) recomputes the map it returns.
    #[cfg(feature = "display-integrity")]
    pub(crate) fn set_fresh(&mut self, on: bool) {
        self.fresh = on;
    }

    pub(crate) fn bytes_reserved(&self) -> usize {
        self.arena.capacity().saturating_add(
            self.entries
                .capacity()
                .saturating_mul(core::mem::size_of::<Entry>()),
        )
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
        // Entries are packed in the arena (`off + len ≤ arena.len()`): the saturating
        // operations are exact.
        let end = e.off.saturating_add(e.len).min(self.arena.len());
        let tail = self.arena.len().saturating_sub(end);
        self.arena.copy_within(end.., e.off.min(end));
        self.arena.truncate(e.off.saturating_add(tail));
        for o in &mut self.entries {
            if o.off > e.off {
                o.off = o.off.saturating_sub(e.len);
            }
        }
    }

    /// The corner map of `g` (row-major `qw × qh`), computed on a miss. `g` must come from
    /// [`fit`](Self::fit) and `scratch` must hold `g.scratch_needed()` values.
    pub(crate) fn corner(&mut self, g: &ShadowGeo, scratch: &mut [u16]) -> &[u8] {
        self.tick = self.tick.wrapping_add(1);
        let key = g.key();
        let tick = self.tick;
        if let Some(e) = self.entries.iter_mut().find(|e| e.key == key) {
            e.last_use = tick;
            let e = *e;
            self.stats.hits = self.stats.hits.saturating_add(1);
            let range = e.off..e.off.saturating_add(e.len);
            #[cfg(feature = "display-integrity")]
            if self.fresh {
                if let Some(map) = self.arena.get_mut(range.clone()) {
                    build_corner(g, map, scratch);
                }
            }
            // SAFE-INDEX: entries describe packed parts of the arena; the fallback is never
            // taken.
            return self.arena.get(range).unwrap_or_default();
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        let size = g.map_bytes();
        while self.entries.len() >= self.max_entries
            || self.arena.capacity().saturating_sub(self.arena.len()) < size
        {
            if self.entries.is_empty() {
                break;
            }
            self.evict_lru();
        }
        let off = self.arena.len();
        let range = off..off.saturating_add(size);
        self.arena.resize(range.end, 0);
        // SAFE-INDEX: the arena was just resized to `range.end`; the fallbacks are never taken.
        if let Some(map) = self.arena.get_mut(range.clone()) {
            build_corner(g, map, scratch);
        }
        self.entries.push(Entry {
            key,
            off,
            len: size,
            last_use: self.tick,
        });
        self.arena.get(range).unwrap_or_default()
    }
}

/// Largest blur pass radius `p` [`build_corner`] computes (`fit` keeps `6p` below the
/// scratch length, at most `u16::MAX`): bounds the window sums below.
const MAX_P: usize = 1 << 14;

/// Computes the blurred corner map of `g` into `map` (`qw × qh`, row-major). `scratch` must
/// hold `g.scratch_needed()` values and `map` `g.map_bytes()` (else nothing is computed).
fn build_corner(g: &ShadowGeo, map: &mut [u8], scratch: &mut [u16]) {
    let (p, qw, qh) = g.sizes();
    if p > MAX_P || qw == 0 || scratch.len() < g.scratch_needed() || map.len() < g.map_bytes() {
        debug_assert!(false, "build_corner: geometry not from `fit`");
        return;
    }
    // From here every size is bounded by `scratch.len()` or `map.len()`.
    // SAFE-ARITH: `e = 2p ≤ 2^15`; `qw + 2e`, `qw + e` and `qh + e` are parts of
    // `scratch_needed() ≤ scratch.len()`.
    let e = p.wrapping_mul(2);
    let bh = g.bounds.height();
    // The shape in bounds-relative coordinates.
    let shape = g
        .shape
        .translate(g.bounds.x0.saturating_neg(), g.bounds.y0.saturating_neg());
    let q = if g.radius > 0 {
        Quarter::Computed { r: g.radius }
    } else {
        Quarter::Square
    };
    // SAFE-ARITH: `2p + 1 ≤ 2^15 + 1`.
    let Some(win) = NonZeroU32::new((p as u32).wrapping_mul(2).wrapping_add(1)) else {
        return;
    };
    let half = win.get() / 2;
    // Rounded window mean. SAFE-ARITH: a window sums at most `2p + 1 ≤ 2^15 + 1` values below
    // 2^16, so every sum is below 2^32 − 2^16 and `+ half` fits.
    let mean = |sum: u32| sum.wrapping_add(half) / win;
    // SAFE-INDEX: the three buffers are parts of `scratch_needed()` values (checked above);
    // the fallbacks are never taken.
    let Some((src, rest)) = scratch.split_at_mut_checked(qw.wrapping_add(e).wrapping_add(e)) else {
        return;
    };
    let Some((out1, rest)) = rest.split_at_mut_checked(qw.wrapping_add(e)) else {
        return;
    };
    let Some(col) = rest.get_mut(..qh.wrapping_add(e)) else {
        return;
    };
    // `e ≤ 2^15`: the cast is exact.
    let e32 = e as i32;

    // Horizontal passes, one map row at a time. Values carry 8 fractional bits.
    for (my, map_row) in (0..i32::MAX).zip(map.chunks_exact_mut(qw).take(qh)) {
        let rs = RowSpan::new(shape, &q, my);
        for (x, v) in (e32.wrapping_neg()..i32::MAX).zip(src.iter_mut()) {
            *v = u16::from(rs.cov(x)) << 8;
        }
        if p == 0 {
            for (m, &v) in map_row.iter_mut().zip(src.iter()) {
                *m = (v >> 8) as u8;
            }
            continue;
        }
        // Pass 1: out1[j] (x = j − p) = mean of src[j ..= j + 2p] (a sliding window: the value
        // `2p` ahead enters, the value at `j` leaves).
        let mut sum: u32 = src.iter().take(e).map(|&v| u32::from(v)).sum();
        for ((o, &enter), &leave) in out1.iter_mut().zip(src.iter().skip(e)).zip(src.iter()) {
            // SAFE-ARITH: `sum` holds the `2p` values before `enter` (see `mean`); `leave` is
            // one of them.
            sum = sum.wrapping_add(u32::from(enter));
            *o = mean(sum) as u16;
            sum = sum.wrapping_sub(u32::from(leave));
        }
        // Pass 2: map[x] = mean of out1[x ..= x + 2p].
        let mut sum: u32 = out1.iter().take(e).map(|&v| u32::from(v)).sum();
        for ((m, &enter), &leave) in map_row.iter_mut().zip(out1.iter().skip(e)).zip(out1.iter()) {
            // SAFE-ARITH: as in pass 1; `mean(sum) + 128 < 2^17`.
            sum = sum.wrapping_add(u32::from(enter));
            *m = (mean(sum).wrapping_add(128) >> 8).min(255) as u8;
            sum = sum.wrapping_sub(u32::from(leave));
        }
    }
    if p == 0 {
        return;
    }
    // Rows of the horizontally blurred image beyond the map: rows below the map repeat the last
    // one (large shapes) or mirror (small shapes, where the map covers half the height).
    let small_y = g.qh.saturating_mul(2) >= bh;
    let (qh32, p32) = (g.qh, e32 / 2);
    let h_row = |y: i32| -> Option<usize> {
        if y < 0 || y >= bh {
            None
        } else if y < qh32 {
            Some(crate::num::span(0, y))
        } else if small_y {
            // SAFE-ARITH: `0 ≤ y < bh` here.
            let m = bh.wrapping_sub(1).wrapping_sub(y);
            (m >= 0).then_some(crate::num::span(0, m))
        } else {
            Some(qh.saturating_sub(1))
        }
    };
    for x in 0..qw {
        // Map value of row `y` at column `x` (0 outside the image).
        let hv = |y: i32| -> u32 {
            h_row(y)
                .and_then(|r| map.get(r.saturating_mul(qw).saturating_add(x)))
                .map_or(0, |&v| u32::from(v) << 8)
        };
        // Pass 1: col[j] (y = j − p) = mean of rows y − p ..= y + p.
        let mut sum: u32 = (e32.wrapping_neg()..0).map(hv).sum();
        for (y, c) in (p32.wrapping_neg()..i32::MAX).zip(col.iter_mut()) {
            // SAFE-ARITH: `|y| ≤ qh + e < 2^31` and `p ≤ 2^14`; the window sums are bounded
            // as in the horizontal passes.
            sum = sum.wrapping_add(hv(y.wrapping_add(p32)));
            *c = mean(sum) as u16;
            sum = sum.wrapping_sub(hv(y.wrapping_sub(p32)));
        }
        // Pass 2.
        let mut sum: u32 = col.iter().take(e).map(|&v| u32::from(v)).sum();
        let column = map.iter_mut().skip(x).step_by(qw).take(qh);
        for ((m, &enter), &leave) in column.zip(col.iter().skip(e)).zip(col.iter()) {
            // SAFE-ARITH: as in the horizontal passes.
            sum = sum.wrapping_add(u32::from(enter));
            *m = (mean(sum).wrapping_add(128) >> 8).min(255) as u8;
            sum = sum.wrapping_sub(u32::from(leave));
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
