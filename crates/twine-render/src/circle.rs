//! Anti-aliased quarter-circle coverage and the [`CircleCache`].
//!
//! A quarter circle of radius `r` is the top-left corner of a `2r × 2r` rounded square: the
//! circle's center is at `(r, r)` in pixel-corner coordinates. For pixel `(px, py)` of the
//! quarter (`0 ≤ px, py < r`) the coverage is computed at 4 sub-rows `ys = py + (2k+1)/8`: the
//! circle's x-intersection `xe = r − √(r² − (r − ys)²)` (24.8 fixed point, integer square root)
//! gives the horizontal coverage `clamp(px + 1 − xe, 0, 1)`; the pixel's opacity is the average
//! of the 4 sub-row coverages (capped at 255). Coverage grows with `px` and `py`.
//!
//! A row is described by `aa_start` (coverage is 0 before it), `full_from` (coverage is 255 from
//! it on) and the anti-aliased bytes in between. The cache stores these rows for radii up to
//! [`MAX_CACHED_RADIUS`] in one flat byte arena reserved up front; larger radii (or radii that
//! do not fit the arena) are computed on the fly per row.

use alloc::vec::Vec;

use twine_core::math::isqrt64;

use crate::caches::CacheStats;

/// Largest radius the cache stores; larger radii are computed per row.
pub const MAX_CACHED_RADIUS: i32 = 256;

/// Largest radius drawn exactly (2^22 px); larger radii are clamped to it (every radius of
/// the renderer goes through [`eff_radius`](crate::rrect::eff_radius) or this clamp). It keeps
/// the coverage arithmetic in range: `(8r)² · 2^10 < 2^64` and `256 · (r + 1) < 2^31`.
pub(crate) const MAX_RADIUS: i32 = 1 << 22;

/// Arena bytes reserved per cache entry (an entry of radius `r` takes about `6·r + 2·r` bytes;
/// radius 64 fits `circle_cache_entries` times, one radius-256 entry fits in 4 entries' space).
pub(crate) const ARENA_BYTES_PER_ENTRY: usize = 576;

/// Row header size in the arena: `aa_start`, `full_from`, data offset (`u16` LE each).
const HDR: usize = 6;

/// The x-intersections (1/256 px from the quarter's outer edge) of the 4 sub-rows of row `py`.
#[inline]
pub(crate) fn row_edges(r: i32, py: i32) -> [i32; 4] {
    let r = r.clamp(0, MAX_RADIUS);
    let py = py.clamp(0, r);
    // SAFE-ARITH: `0 ≤ py ≤ r ≤ 2^22` (clamped above) and `k < 4`: `r8, ys8 ≤ 2^25 + 7`, so
    // `|dy8| ≤ 2^26`, the squares are below 2^52 and `inner · 2^10 < 2^62`; `256 · r ≤ 2^30`
    // and the root `√(inner · 2^10) ≤ 2^31` is at most `256 · r` (`inner ≤ r8²`).
    let r8 = 8i64.wrapping_mul(i64::from(r));
    let r256 = 256i32.wrapping_mul(r);
    core::array::from_fn(|k| {
        let ys8 = 8i64
            .wrapping_mul(i64::from(py))
            .wrapping_add((k as i64).wrapping_mul(2).wrapping_add(1));
        let dy8 = r8.wrapping_sub(ys8);
        let inner = r8.wrapping_mul(r8).wrapping_sub(dy8.wrapping_mul(dy8)); // (r² − (r − ys)²) · 64
        if inner <= 0 {
            r256
        } else {
            r256.wrapping_sub(isqrt64((inner as u64) << 10) as i32)
        }
    })
}

/// Coverage of pixel `px` given the sub-row edges.
#[inline(always)]
pub(crate) fn edge_cov(e: &[i32; 4], px: i32) -> u8 {
    // SAFE-ARITH: `0 ≤ px < r ≤ MAX_RADIUS` and the edges are in `0..=256 · r` (`row_edges`),
    // so `edge ≤ 2^30`, `|edge − x| ≤ 2^30` and the four clamped terms sum to at most 1024.
    let edge = 256i32.wrapping_mul(px.wrapping_add(1));
    let sum = e
        .iter()
        .fold(0i32, |s, &x| s.wrapping_add(edge.wrapping_sub(x).clamp(0, 256)));
    (sum / 4).min(255) as u8
}

/// `(aa_start, full_from)` of a row from its edges.
#[inline]
pub(crate) fn edge_bounds(e: &[i32; 4], r: i32) -> (i32, i32) {
    let min = e.iter().copied().min().unwrap_or(0);
    let aa_start = (min >> 8).clamp(0, r);
    let mut full = aa_start;
    while full < r && edge_cov(e, full) < 255 {
        // SAFE-ARITH: `full < r` (the loop condition).
        full = full.wrapping_add(1);
    }
    (aa_start, full)
}

/// The anti-aliased part of a quarter-circle row.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RowAa<'c> {
    /// Precomputed bytes for `[aa_start, full_from)`.
    Table(&'c [u8]),
    /// Computed per pixel from the sub-row edges.
    Edges([i32; 4]),
}

/// One row of a quarter circle.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CircleRow<'c> {
    /// Coverage is 0 for `px < aa_start`.
    pub aa_start: i32,
    /// Coverage is 255 for `px >= full_from`.
    pub full_from: i32,
    aa: RowAa<'c>,
}

impl CircleRow<'_> {
    /// A row that is fully covered (radius 0).
    pub const FULL: CircleRow<'static> = CircleRow {
        aa_start: 0,
        full_from: 0,
        aa: RowAa::Table(&[]),
    };

    /// Coverage of pixel `px` (distance from the quarter's outer edge).
    #[inline(always)]
    pub fn cov(&self, px: i32) -> u8 {
        if px < self.aa_start {
            0
        } else if px >= self.full_from {
            255
        } else {
            match &self.aa {
                // SAFE-ARITH: `aa_start ≤ px` (the branch above), so the offset is not negative.
                // SAFE-INDEX: `px < full_from` and the table holds the `full_from − aa_start`
                // values of `[aa_start, full_from)`; the fallback is never taken.
                RowAa::Table(t) => t
                    .get(px.wrapping_sub(self.aa_start) as usize)
                    .copied()
                    .unwrap_or(0),
                RowAa::Edges(e) => edge_cov(e, px),
            }
        }
    }
}

/// A quarter circle, cached or computed on the fly.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Quarter<'c> {
    /// Radius 0: every row is fully covered.
    Square,
    /// Rows stored in the cache arena (entry bytes).
    Cached {
        /// Radius.
        r: i32,
        /// Entry bytes (headers, then AA bytes).
        bytes: &'c [u8],
    },
    /// Rows computed per request.
    Computed {
        /// Radius.
        r: i32,
    },
}

impl<'c> Quarter<'c> {
    /// The radius.
    pub fn radius(&self) -> i32 {
        match *self {
            Quarter::Square => 0,
            Quarter::Cached { r, .. } | Quarter::Computed { r } => r,
        }
    }

    /// Row `py` (`0 ≤ py < r`; out-of-range rows are fully covered).
    #[inline]
    pub fn row(&self, py: i32) -> CircleRow<'c> {
        match *self {
            Quarter::Square => CircleRow::FULL,
            Quarter::Cached { r, bytes } => {
                if py < 0 || py >= r {
                    return CircleRow::FULL;
                }
                // The entry layout written by `CircleCache::ensure`: `r` headers of `HDR` bytes,
                // then the anti-aliased bytes of every row.
                // SAFE-ARITH: `0 ≤ py < r ≤ MAX_CACHED_RADIUS = 256`, so the header offsets
                // stay below `256 · HDR` and the row data below the entry size (`u16`
                // offsets plus `u16` lengths).
                let h = (py as usize).wrapping_mul(HDR);
                let header = bytes
                    .get(h..h.wrapping_add(HDR))
                    .and_then(|b| b.first_chunk::<HDR>());
                // SAFE-INDEX: `py < r`, so header `py` exists; the fallback is never taken.
                let Some(&[a0, a1, f0, f1, o0, o1]) = header else {
                    return CircleRow::FULL;
                };
                let (aa_start, full_from) = (u16::from_le_bytes([a0, a1]), u16::from_le_bytes([f0, f1]));
                let data = (r as usize)
                    .wrapping_mul(HDR)
                    .wrapping_add(usize::from(u16::from_le_bytes([o0, o1])));
                let len = usize::from(full_from.saturating_sub(aa_start));
                // SAFE-INDEX: the row's bytes were written at `data` (`ensure`); the fallback is
                // never taken.
                let Some(table) = bytes.get(data..data.wrapping_add(len)) else {
                    return CircleRow::FULL;
                };
                CircleRow {
                    aa_start: i32::from(aa_start),
                    full_from: i32::from(full_from),
                    aa: RowAa::Table(table),
                }
            }
            Quarter::Computed { r } => {
                if py < 0 || py >= r {
                    return CircleRow::FULL;
                }
                let e = row_edges(r, py);
                let (aa_start, full_from) = edge_bounds(&e, r);
                CircleRow {
                    aa_start,
                    full_from,
                    aa: RowAa::Edges(e),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    r: u16,
    off: usize,
    len: usize,
    last_use: u32,
}

/// LRU cache of quarter-circle coverage rows (fixed entry count and arena size, never grows).
#[derive(Debug, Default)]
pub struct CircleCache {
    entries: Vec<Entry>,
    max_entries: usize,
    arena: Vec<u8>,
    tick: u32,
    stats: CacheStats,
    /// Fresh mode ([`RenderCaches::set_fresh`](crate::RenderCaches::set_fresh)): the cache is
    /// neither read nor filled; every quarter is computed per row.
    #[cfg(feature = "display-integrity")]
    fresh: bool,
}

impl CircleCache {
    /// A cache with `entries` slots and its arena reserved.
    pub(crate) fn new(entries: u8) -> Self {
        let n = usize::from(entries);
        Self {
            entries: Vec::with_capacity(n),
            max_entries: n,
            arena: Vec::with_capacity(n * ARENA_BYTES_PER_ENTRY),
            tick: 0,
            stats: CacheStats::default(),
            #[cfg(feature = "display-integrity")]
            fresh: false,
        }
    }

    /// Fresh mode: [`ensure`](Self::ensure) and [`quarter`](Self::quarter) bypass the cache.
    #[cfg(feature = "display-integrity")]
    pub(crate) fn set_fresh(&mut self, on: bool) {
        self.fresh = on;
    }

    /// Reserved bytes.
    pub(crate) fn bytes_reserved(&self) -> usize {
        self.arena.capacity().saturating_add(
            self.entries
                .capacity()
                .saturating_mul(core::mem::size_of::<Entry>()),
        )
    }

    /// Hit/miss counters.
    pub(crate) fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Exact arena bytes needed for radius `r`.
    fn entry_size(r: i32) -> usize {
        let aa = (0..r).fold(0usize, |sum, py| {
            let e = row_edges(r, py);
            let (a, f) = edge_bounds(&e, r);
            sum.saturating_add(crate::num::span(a, f))
        });
        crate::num::span(0, r).saturating_mul(HDR).saturating_add(aa)
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
        for other in &mut self.entries {
            if other.off > e.off {
                other.off = other.off.saturating_sub(e.len);
            }
        }
    }

    /// Makes sure radius `r` is cached if possible (counts a hit or a miss).
    pub(crate) fn ensure(&mut self, r: i32) {
        if r <= 0 {
            return;
        }
        #[cfg(feature = "display-integrity")]
        if self.fresh {
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        if let Some(e) = self.entries.iter_mut().find(|e| i32::from(e.r) == r) {
            e.last_use = self.tick;
            self.stats.hits = self.stats.hits.saturating_add(1);
            return;
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        if r > MAX_CACHED_RADIUS || self.max_entries == 0 {
            return;
        }
        let size = Self::entry_size(r);
        if size > self.arena.capacity() {
            return;
        }
        while self.entries.len() >= self.max_entries
            || self.arena.capacity().saturating_sub(self.arena.len()) < size
        {
            if self.entries.is_empty() {
                return;
            }
            self.evict_lru();
        }
        let off = self.arena.len();
        // Headers first, then the AA bytes (within the reserved capacity: no allocation).
        // SAFE-ARITH: `r ≤ MAX_CACHED_RADIUS` and the entry fits the arena (`size`, checked
        // above), so every offset below is at most `off + size ≤ capacity`.
        self.arena
            .resize(off.wrapping_add((r as usize).wrapping_mul(HDR)), 0);
        let mut data_off = 0usize;
        for py in 0..r {
            let e = row_edges(r, py);
            let (a, f) = edge_bounds(&e, r);
            let h = off.wrapping_add((py as usize).wrapping_mul(HDR));
            let [h0, h1, h2] = [a as u16, f as u16, data_off as u16].map(u16::to_le_bytes);
            // SAFE-INDEX: header `py < r` lies in the `r · HDR` bytes resized above; the
            // fallback is never taken.
            if let Some(header) = self.arena.get_mut(h..h.wrapping_add(HDR)) {
                header.copy_from_slice(&[h0[0], h0[1], h1[0], h1[1], h2[0], h2[1]]);
            }
            for px in a..f {
                self.arena.push(edge_cov(&e, px));
            }
            // SAFE-ARITH: `a ≤ f ≤ r`, and the row's bytes are part of `size`.
            data_off = data_off.wrapping_add(crate::num::span(a, f));
        }
        debug_assert_eq!(self.arena.len(), off.saturating_add(size));
        self.entries.push(Entry {
            r: r as u16,
            off,
            len: size,
            last_use: self.tick,
        });
    }

    /// The quarter circle of radius `r` (cached if present, else computed per row). Call
    /// [`ensure`](Self::ensure) first to populate the cache.
    pub(crate) fn quarter(&self, r: i32) -> Quarter<'_> {
        if r <= 0 {
            return Quarter::Square;
        }
        #[cfg(feature = "display-integrity")]
        if self.fresh {
            return Quarter::Computed { r };
        }
        let cached = self
            .entries
            .iter()
            .find(|e| i32::from(e.r) == r)
            .and_then(|e| self.arena.get(e.off..e.off.saturating_add(e.len)));
        match cached {
            Some(bytes) => Quarter::Cached { r, bytes },
            None => Quarter::Computed { r },
        }
    }

    /// Radii currently cached (test helper).
    #[cfg(test)]
    fn cached(&self) -> Vec<i32> {
        self.entries.iter().map(|e| i32::from(e.r)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_quarter(q: &Quarter<'_>) -> Vec<Vec<u8>> {
        let r = q.radius();
        (0..r)
            .map(|py| (0..r).map(|px| q.row(py).cov(px)).collect())
            .collect()
    }

    #[test]
    #[allow(clippy::needless_range_loop)] // (x, y) and (y, x) are compared
    fn circle_coverage_symmetric() {
        // The quarter is symmetric about its diagonal within the sampling error of 4 sub-rows.
        for r in [1, 2, 5, 16, 40] {
            let q = full_quarter(&Quarter::Computed { r });
            for py in 0..r as usize {
                for px in 0..r as usize {
                    let d = i32::from(q[py][px]) - i32::from(q[px][py]);
                    assert!(d.abs() <= 40, "r={r} ({px},{py}) {} vs {}", q[py][px], q[px][py]);
                }
            }
            // Fully inside near the center, empty at the outer corner (for r ≥ 5).
            if r >= 5 {
                assert_eq!(q[r as usize - 1][r as usize - 1], 255);
                assert_eq!(q[0][0], 0);
            }
        }
    }

    #[test]
    fn circle_full_from_monotonic() {
        for r in 1..=64 {
            let q = Quarter::Computed { r };
            let mut prev = i32::MAX;
            for py in 0..r {
                let row = q.row(py);
                assert!(row.aa_start <= row.full_from && row.full_from <= r);
                assert!(row.full_from <= prev, "r={r} py={py}");
                prev = row.full_from;
                for px in 0..r - 1 {
                    assert!(row.cov(px) <= row.cov(px + 1));
                }
            }
        }
    }

    #[test]
    fn cache_hit_after_warmup() {
        let mut c = CircleCache::new(4);
        c.ensure(10);
        c.ensure(10);
        c.ensure(20);
        c.ensure(10);
        assert_eq!(c.stats(), CacheStats { hits: 2, misses: 2 });
        assert!(matches!(c.quarter(10), Quarter::Cached { .. }));
        let cap = c.arena.capacity();
        for r in [1, 2, 3, 4, 5, 6, 7, 50, 64, 3] {
            c.ensure(r);
        }
        assert_eq!(c.arena.capacity(), cap, "the arena never grows");
        assert!(c.cached().len() <= 4);
        assert!(c.cached().contains(&3));
    }

    #[test]
    fn large_radius_uncached_matches_cached_math() {
        let mut c = CircleCache::new(4);
        c.ensure(200);
        let cached = c.quarter(200);
        assert!(
            matches!(cached, Quarter::Cached { .. }),
            "a single radius 200 fits the arena"
        );
        let computed = Quarter::Computed { r: 200 };
        assert_eq!(full_quarter(&cached), full_quarter(&computed));
        c.ensure(300);
        assert!(matches!(c.quarter(300), Quarter::Computed { .. }));
    }
}
