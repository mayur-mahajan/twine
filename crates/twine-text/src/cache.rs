//! [`GlyphCache`]: an LRU cache of decoded A8 glyph bitmaps with a fixed byte budget, plus the
//! scratch memory text decoding and drawing need.
//!
//! # Arena layout
//!
//! The budget is one byte arena split into fixed 64-byte blocks. A glyph occupies a contiguous
//! run of blocks found by **first fit**; when no run is free, least recently used entries are
//! evicted until one is. No compaction is performed: glyphs are small (a 14 px glyph needs 2–3
//! blocks) and all entries are evicted in LRU order anyway, so fragmentation costs at most a few
//! extra evictions, while the scheme needs no allocation and no moving of data.
//!
//! Everything is allocated in [`GlyphCache::new`]; afterwards the cache never allocates, except
//! that the glyph scratch used by providers without incremental decoding grows to the largest
//! glyph seen (once).

use alloc::vec;
use alloc::vec::Vec;

/// Size of one arena block in bytes.
pub const BLOCK_BYTES: usize = 64;
/// Maximum number of cached glyphs.
pub const MAX_ENTRIES: usize = 256;
/// Width of the row scratch buffers: the widest glyph row (in coverage values) that is
/// decoded row by row.
pub const MAX_ROW: usize = 255;
/// Largest glyph (in coverage bytes) the glyph scratch buffer grows to.
pub const MAX_GLYPH_BYTES: usize = 255 * 255;
/// Default budget (`EngineConfig::glyph_cache_bytes`).
pub const DEFAULT_BUDGET: usize = 8 * 1024;
/// Size of the "already warned" ring for missing glyphs.
pub const WARN_RING: usize = 16;

/// Cache key: `(provider address, glyph id)`.
pub type GlyphKey = (usize, u32);

/// Statistics of a [`GlyphCache`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CacheStats {
    /// Lookups that found the glyph.
    pub hits: u64,
    /// Lookups that did not.
    pub misses: u64,
    /// Entries evicted to make room.
    pub evictions: u64,
    /// Bytes of the arena currently occupied (whole blocks).
    pub bytes_used: usize,
    /// Glyph bitmaps drawn by the text renderer (cached or not).
    pub glyph_renders: u64,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: GlyphKey,
    block: u16,
    blocks: u16,
    len: u32,
    stamp: u64,
}

/// LRU cache of decoded glyphs and the scratch buffers of the text renderer.
///
/// ```
/// use twine_text::GlyphCache;
///
/// let mut c = GlyphCache::new(1024);
/// c.insert((1, 7), 4, 2).unwrap().copy_from_slice(&[9; 8]);
/// assert_eq!(c.get((1, 7)), Some(&[9u8; 8][..]));
/// assert_eq!(c.stats().hits, 1);
/// ```
pub struct GlyphCache {
    arena: Vec<u8>,
    used: Vec<bool>,
    entries: Vec<Entry>,
    max_entries: usize,
    tick: u64,
    stats: CacheStats,
    pub(crate) row_a: Vec<u8>,
    pub(crate) row_b: Vec<u8>,
    glyph: Vec<u8>,
    pub(crate) lcd_row: Vec<u8>,
    warned: [GlyphKey; WARN_RING],
    warned_len: usize,
    warned_pos: usize,
}

impl core::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("budget", &self.arena.len())
            .field("entries", &self.entries.len())
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Default for GlyphCache {
    /// A cache with the [`DEFAULT_BUDGET`] (8 KiB).
    fn default() -> Self {
        Self::new(DEFAULT_BUDGET)
    }
}

impl GlyphCache {
    /// A cache of `budget_bytes` (rounded down to whole 64-byte blocks; 0 disables caching).
    ///
    /// Allocates the arena, an entry table of `budget_bytes / 64` (at most 256) entries and two
    /// 255-byte row buffers.
    #[must_use]
    pub fn new(budget_bytes: usize) -> Self {
        let blocks = (budget_bytes / BLOCK_BYTES).min(usize::from(u16::MAX));
        let max_entries = blocks.min(MAX_ENTRIES);
        Self {
            arena: vec![0; blocks * BLOCK_BYTES],
            used: vec![false; blocks],
            entries: Vec::with_capacity(max_entries),
            max_entries,
            tick: 0,
            stats: CacheStats::default(),
            row_a: vec![0; MAX_ROW],
            row_b: vec![0; MAX_ROW],
            glyph: Vec::new(),
            lcd_row: Vec::new(),
            warned: [(0, 0); WARN_RING],
            warned_len: 0,
            warned_pos: 0,
        }
    }

    /// The arena size in bytes (0 when caching is disabled).
    #[must_use]
    pub fn budget(&self) -> usize {
        self.arena.len()
    }

    /// Whether glyphs can be cached at all.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.max_entries > 0
    }

    /// The statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let mut s = self.stats;
        s.bytes_used = self.used.iter().filter(|u| **u).count() * BLOCK_BYTES;
        s
    }

    /// Resets the statistics counters (not the contents).
    pub fn reset_stats(&mut self) {
        self.stats = CacheStats::default();
    }

    /// Number of cached glyphs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no glyph is cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A new LRU stamp (64-bit: never wraps in practice).
    fn next_stamp(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// The cached bitmap of `key` (marks it most recently used), counting a hit or a miss.
    pub fn get(&mut self, key: GlyphKey) -> Option<&[u8]> {
        let Some(i) = self.entries.iter().position(|e| e.key == key) else {
            self.stats.misses += 1;
            return None;
        };
        self.stats.hits += 1;
        let stamp = self.next_stamp();
        let e = &mut self.entries[i];
        e.stamp = stamp;
        let start = usize::from(e.block) * BLOCK_BYTES;
        Some(&self.arena[start..start + e.len as usize])
    }

    /// Reserves space for a `w × h` bitmap of `key` (replacing an existing entry), evicting
    /// least recently used entries until it fits. Returns the zeroed-length-exact slot to fill,
    /// or `None` when caching is disabled or the glyph alone exceeds the budget (the caller then
    /// decodes into its own buffer).
    pub fn insert(&mut self, key: GlyphKey, w: usize, h: usize) -> Option<&mut [u8]> {
        let len = w.checked_mul(h)?;
        let blocks = len.div_ceil(BLOCK_BYTES).max(1);
        if !self.is_enabled() || blocks > self.used.len() {
            return None;
        }
        self.remove(key);
        let block = loop {
            if self.entries.len() < self.max_entries {
                if let Some(b) = self.first_fit(blocks) {
                    break b;
                }
            }
            if !self.evict_lru() {
                return None;
            }
        };
        self.used[block..block + blocks].fill(true);
        let stamp = self.next_stamp();
        self.entries.push(Entry {
            key,
            block: block as u16,
            blocks: blocks as u16,
            len: len as u32,
            stamp,
        });
        let start = block * BLOCK_BYTES;
        Some(&mut self.arena[start..start + len])
    }

    /// Removes the entry of `key` (if cached).
    pub fn remove(&mut self, key: GlyphKey) {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            self.release(i);
        }
    }

    /// Removes every entry (statistics are kept).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.used.fill(false);
    }

    fn release(&mut self, i: usize) {
        let e = self.entries.swap_remove(i);
        let b = usize::from(e.block);
        self.used[b..b + usize::from(e.blocks)].fill(false);
    }

    fn evict_lru(&mut self) -> bool {
        let Some(i) = (0..self.entries.len()).min_by_key(|&i| self.entries[i].stamp) else {
            return false;
        };
        twine_core::trace!(target: "twine::text", "glyph cache evicts id {}", self.entries[i].key.1);
        self.release(i);
        self.stats.evictions += 1;
        true
    }

    fn first_fit(&self, blocks: usize) -> Option<usize> {
        let mut run = 0;
        for (i, &u) in self.used.iter().enumerate() {
            if u {
                run = 0;
            } else {
                run += 1;
                if run == blocks {
                    return Some(i + 1 - blocks);
                }
            }
        }
        None
    }

    /// Counts one drawn glyph bitmap (see [`CacheStats::glyph_renders`]).
    pub(crate) fn count_render(&mut self) {
        self.stats.glyph_renders += 1;
    }

    /// Takes the glyph scratch buffer out, sized to exactly `len` bytes (grown once if needed).
    /// `None` (with a warning) above [`MAX_GLYPH_BYTES`]. Return it with
    /// [`put_glyph_scratch`](Self::put_glyph_scratch).
    pub fn take_glyph_scratch(&mut self, len: usize) -> Option<Vec<u8>> {
        if len > MAX_GLYPH_BYTES {
            if self.first_warning((0, u32::MAX)) {
                twine_core::warn!(
                    target: "twine::text",
                    "glyph of {} bytes exceeds the {} byte glyph scratch; not drawn",
                    len,
                    MAX_GLYPH_BYTES
                );
            }
            return None;
        }
        let mut buf = core::mem::take(&mut self.glyph);
        buf.resize(len, 0);
        Some(buf)
    }

    /// Returns the buffer taken with [`take_glyph_scratch`](Self::take_glyph_scratch).
    pub fn put_glyph_scratch(&mut self, buf: Vec<u8>) {
        self.glyph = buf;
    }

    /// Records `key` in the 16-entry "already warned" ring; `true` if it was not there yet
    /// (the caller should log). Used to rate-limit missing-glyph and corrupt-font warnings.
    pub fn first_warning(&mut self, key: GlyphKey) -> bool {
        if self.warned[..self.warned_len].contains(&key) {
            return false;
        }
        self.warned[self.warned_pos] = key;
        self.warned_pos = (self.warned_pos + 1) % WARN_RING;
        self.warned_len = (self.warned_len + 1).min(WARN_RING);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_after_insert() {
        let mut c = GlyphCache::new(512);
        assert!(c.get((1, 1)).is_none());
        c.insert((1, 1), 10, 10).unwrap().fill(7);
        assert_eq!(c.get((1, 1)).unwrap(), &[7u8; 100][..]);
        let s = c.stats();
        assert_eq!((s.hits, s.misses, s.bytes_used), (1, 1, 128));
    }

    #[test]
    fn cache_evicts_lru_when_full() {
        // 4 blocks; each glyph takes 2 → two fit.
        let mut c = GlyphCache::new(256);
        c.insert((1, 1), 100, 1).unwrap().fill(1);
        c.insert((1, 2), 100, 1).unwrap().fill(2);
        assert!(c.get((1, 1)).is_some()); // 1 is now the most recent
        c.insert((1, 3), 100, 1).unwrap().fill(3);
        assert!(c.get((1, 2)).is_none(), "2 was least recently used");
        assert_eq!(c.get((1, 1)).unwrap()[0], 1);
        assert_eq!(c.get((1, 3)).unwrap()[0], 3);
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn cache_entry_limit_evicts() {
        // 2 blocks → at most 2 entries even for tiny glyphs.
        let mut c = GlyphCache::new(128);
        for id in 0..5 {
            c.insert((9, id), 1, 1).unwrap()[0] = id as u8;
        }
        assert_eq!(c.len(), 2);
        assert_eq!(c.get((9, 4)).unwrap(), &[4]);
        assert_eq!(c.get((9, 3)).unwrap(), &[3]);
    }

    #[test]
    fn cache_zero_budget_disabled() {
        let mut c = GlyphCache::new(0);
        assert!(!c.is_enabled());
        assert!(c.insert((1, 1), 1, 1).is_none());
        assert!(c.get((1, 1)).is_none());
    }

    #[test]
    fn cache_oversize_glyph_not_cached() {
        let mut c = GlyphCache::new(256);
        c.insert((1, 1), 8, 8).unwrap();
        assert!(c.insert((1, 2), 20, 20).is_none());
        assert!(c.get((1, 1)).is_some(), "existing entries survive");
    }

    #[test]
    fn reinsert_replaces_and_clear_empties() {
        let mut c = GlyphCache::new(256);
        c.insert((1, 1), 8, 8).unwrap().fill(1);
        c.insert((1, 1), 4, 4).unwrap().fill(2);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get((1, 1)).unwrap().len(), 16);
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.stats().bytes_used, 0);
    }

    #[test]
    fn warning_ring_rate_limits() {
        let mut c = GlyphCache::new(0);
        assert!(c.first_warning((1, 65)));
        assert!(!c.first_warning((1, 65)));
        for i in 0..WARN_RING as u32 {
            assert!(c.first_warning((2, i)));
        }
        assert!(c.first_warning((1, 65)), "evicted from the ring");
    }
}
