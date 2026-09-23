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

/// Arena bytes reserved per cache entry (an entry of radius `r` takes about `6·r + 2·r` bytes;
/// radius 64 fits `circle_cache_entries` times, one radius-256 entry fits in 4 entries' space).
pub(crate) const ARENA_BYTES_PER_ENTRY: usize = 576;

/// Row header size in the arena: `aa_start`, `full_from`, data offset (`u16` LE each).
const HDR: usize = 6;

/// The x-intersections (1/256 px from the quarter's outer edge) of the 4 sub-rows of row `py`.
#[inline]
pub(crate) fn row_edges(r: i32, py: i32) -> [i32; 4] {
    let r8 = 8 * i64::from(r);
    core::array::from_fn(|k| {
        let ys8 = 8 * i64::from(py) + 2 * k as i64 + 1;
        let dy8 = r8 - ys8;
        let inner = r8 * r8 - dy8 * dy8; // (r² − (r − ys)²) · 64
        if inner <= 0 {
            256 * r
        } else {
            (256 * i64::from(r) - i64::from(isqrt64((inner as u64) << 10))) as i32
        }
    })
}

/// Coverage of pixel `px` given the sub-row edges.
#[inline(always)]
pub(crate) fn edge_cov(e: &[i32; 4], px: i32) -> u8 {
    let edge = 256 * (px + 1);
    let sum: i32 = e.iter().map(|&x| (edge - x).clamp(0, 256)).sum();
    (sum / 4).min(255) as u8
}

/// `(aa_start, full_from)` of a row from its edges.
#[inline]
pub(crate) fn edge_bounds(e: &[i32; 4], r: i32) -> (i32, i32) {
    let min = e.iter().copied().min().unwrap_or(0);
    let aa_start = (min >> 8).clamp(0, r);
    let mut full = aa_start;
    while full < r && edge_cov(e, full) < 255 {
        full += 1;
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
                RowAa::Table(t) => t[(px - self.aa_start) as usize],
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
                let h = py as usize * HDR;
                let rd = |i: usize| i32::from(u16::from_le_bytes([bytes[h + i], bytes[h + i + 1]]));
                let (aa_start, full_from, off) = (rd(0), rd(2), rd(4) as usize);
                let data = r as usize * HDR + off;
                CircleRow {
                    aa_start,
                    full_from,
                    aa: RowAa::Table(&bytes[data..data + (full_from - aa_start) as usize]),
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
        }
    }

    /// Reserved bytes.
    pub(crate) fn bytes_reserved(&self) -> usize {
        self.arena.capacity() + self.entries.capacity() * core::mem::size_of::<Entry>()
    }

    /// Hit/miss counters.
    pub(crate) fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Exact arena bytes needed for radius `r`.
    fn entry_size(r: i32) -> usize {
        let aa: usize = (0..r)
            .map(|py| {
                let e = row_edges(r, py);
                let (a, f) = edge_bounds(&e, r);
                (f - a) as usize
            })
            .sum();
        r as usize * HDR + aa
    }

    fn find(&self, r: i32) -> Option<usize> {
        self.entries.iter().position(|e| i32::from(e.r) == r)
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
        for other in &mut self.entries {
            if other.off > e.off {
                other.off -= e.len;
            }
        }
    }

    /// Makes sure radius `r` is cached if possible (counts a hit or a miss).
    pub(crate) fn ensure(&mut self, r: i32) {
        if r <= 0 {
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        if let Some(i) = self.find(r) {
            self.entries[i].last_use = self.tick;
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
        while self.entries.len() >= self.max_entries || self.arena.capacity() - self.arena.len() < size {
            if self.entries.is_empty() {
                return;
            }
            self.evict_lru();
        }
        let off = self.arena.len();
        // Headers first, then the AA bytes (within the reserved capacity: no allocation).
        self.arena.resize(off + r as usize * HDR, 0);
        let mut data_off = 0usize;
        for py in 0..r {
            let e = row_edges(r, py);
            let (a, f) = edge_bounds(&e, r);
            let h = off + py as usize * HDR;
            self.arena[h..h + 2].copy_from_slice(&(a as u16).to_le_bytes());
            self.arena[h + 2..h + 4].copy_from_slice(&(f as u16).to_le_bytes());
            self.arena[h + 4..h + 6].copy_from_slice(&(data_off as u16).to_le_bytes());
            for px in a..f {
                self.arena.push(edge_cov(&e, px));
            }
            data_off += (f - a) as usize;
        }
        debug_assert_eq!(self.arena.len() - off, size);
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
        match self.find(r) {
            Some(i) => {
                let e = self.entries[i];
                Quarter::Cached {
                    r,
                    bytes: &self.arena[e.off..e.off + e.len],
                }
            }
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
