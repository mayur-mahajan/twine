//! [`ImageCache`] (decoded / decompressed pixels, LRU within a byte budget) and
//! [`ImageHeaderCache`] (probed headers, 8 entries round-robin).

use alloc::vec::Vec;

use twine_render::ImagePixels;

use crate::{Error, ImageHeader, MAX_PATH_LEN, pixels_of};

/// Identity of an image source in the caches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceKey {
    /// Address of static data (an `Image` or encoded bytes).
    Ptr(usize),
    /// A file path.
    File(heapless::String<MAX_PATH_LEN>),
}

impl SourceKey {
    /// The key of static data at `p`.
    #[must_use]
    pub fn of_ptr<T: ?Sized>(p: &'static T) -> Self {
        SourceKey::Ptr(core::ptr::from_ref(p).cast::<u8>() as usize)
    }
}

/// Counters of an [`ImageCache`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u32,
    /// Lookups that decoded.
    pub misses: u32,
    /// Entries evicted to make room.
    pub evictions: u32,
    /// Bytes of pixel data held.
    pub bytes_used: usize,
}

#[derive(Debug)]
struct Entry {
    key: SourceKey,
    header: ImageHeader,
    data: Vec<u8>,
    last_use: u32,
}

/// Decoded pixels borrowed from an [`ImageCache`].
#[derive(Clone, Copy, Debug)]
pub struct CachedImage<'a> {
    /// Header of `data`.
    pub header: ImageHeader,
    /// Uncompressed pixel data (layout of [`crate::Image`]).
    pub data: &'a [u8],
}

impl<'a> CachedImage<'a> {
    /// Drawable pixels (`None` if the data does not match the header).
    #[must_use]
    pub fn pixels(&self) -> Option<ImagePixels<'a>> {
        pixels_of(&self.header, self.data)
    }
}

/// Decoded images, least-recently-used first out, within a byte budget.
///
/// An image larger than the budget is returned as a **transient** entry, valid until the next
/// cache call (then freed) — it is decoded again every time it is drawn, which is slow; a
/// warning is logged once per source. A budget of 0 makes every entry transient.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{ImageCache, ImageHeader, SourceKey};
///
/// let mut cache = ImageCache::new(1024, 4);
/// let decode = |out: &mut Vec<u8>| {
///     out.resize(16, 7);
///     Ok(ImageHeader::new(ColorFormat::L8, 4, 4))
/// };
/// assert_eq!(cache.get_or_decode(SourceKey::Ptr(1), decode).unwrap().data.len(), 16);
/// cache.get_or_decode(SourceKey::Ptr(1), decode).unwrap();
/// let s = cache.stats();
/// assert_eq!((s.hits, s.misses, s.bytes_used), (1, 1, 16));
/// ```
#[derive(Debug)]
pub struct ImageCache {
    budget: usize,
    max_entries: usize,
    used: usize,
    entries: Vec<Entry>,
    clock: u32,
    transient: Option<Entry>,
    warned: heapless::Vec<SourceKey, 8>,
    stats: CacheStats,
}

impl ImageCache {
    /// A cache holding at most `max_entries` images and `budget_bytes` bytes of pixels.
    #[must_use]
    pub fn new(budget_bytes: usize, max_entries: usize) -> Self {
        Self {
            budget: budget_bytes,
            max_entries,
            used: 0,
            entries: Vec::with_capacity(max_entries),
            clock: 0,
            transient: None,
            warned: heapless::Vec::new(),
            stats: CacheStats::default(),
        }
    }

    /// The byte budget.
    #[must_use]
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Returns the cached pixels of `key`, or runs `decode` (which fills the buffer it gets
    /// and returns the header) and caches the result, evicting least recently used entries
    /// as needed. Cache hits do not allocate.
    pub fn get_or_decode(
        &mut self,
        key: SourceKey,
        decode: impl FnOnce(&mut Vec<u8>) -> Result<ImageHeader, Error>,
    ) -> Result<CachedImage<'_>, Error> {
        self.transient = None;
        self.clock = self.clock.wrapping_add(1);
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            self.stats.hits += 1;
            let e = &mut self.entries[i];
            e.last_use = self.clock;
            return Ok(CachedImage {
                header: e.header,
                data: &e.data,
            });
        }
        self.stats.misses += 1;
        let mut data = Vec::new();
        let header = decode(&mut data)?;
        let size = data.len();
        let entry = Entry {
            key,
            header,
            data,
            last_use: self.clock,
        };
        if size > self.budget || self.max_entries == 0 {
            if !self.warned.contains(&entry.key) {
                twine_core::warn!(target: "twine::image", "image larger than cache budget; decoding every frame (slow) ({} > {} bytes)", size, self.budget);
                let _ = self.warned.push(entry.key.clone());
            }
            let e = self.transient.insert(entry);
            return Ok(CachedImage {
                header: e.header,
                data: &e.data,
            });
        }
        while self.used + size > self.budget || self.entries.len() >= self.max_entries {
            self.evict_lru();
        }
        self.used += size;
        self.stats.bytes_used = self.used;
        self.entries.push(entry);
        let e = &self.entries[self.entries.len() - 1];
        Ok(CachedImage {
            header: e.header,
            data: &e.data,
        })
    }

    fn evict_lru(&mut self) {
        let clock = self.clock;
        let Some(i) = (0..self.entries.len()).max_by_key(|&i| clock.wrapping_sub(self.entries[i].last_use)) else {
            return;
        };
        let e = self.entries.swap_remove(i);
        self.used -= e.data.len();
        self.stats.evictions += 1;
        self.stats.bytes_used = self.used;
        twine_core::debug!(target: "twine::image", "image cache: evicted {} bytes", e.data.len());
    }

    /// Whether `key` is cached (does not count as a use).
    #[must_use]
    pub fn contains(&self, key: &SourceKey) -> bool {
        self.entries.iter().any(|e| &e.key == key)
    }

    /// Drops the entry of `key` (e.g. after the file changed).
    pub fn invalidate(&mut self, key: &SourceKey) {
        if let Some(i) = self.entries.iter().position(|e| &e.key == key) {
            let e = self.entries.swap_remove(i);
            self.used -= e.data.len();
            self.stats.bytes_used = self.used;
        }
    }

    /// Drops every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.transient = None;
        self.used = 0;
        self.stats.bytes_used = 0;
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.stats
    }
}

/// Number of entries of an [`ImageHeaderCache`].
pub const HEADER_CACHE_ENTRIES: usize = 8;

/// Recently probed image headers (8 entries, replaced round-robin), so widgets can ask for
/// image sizes without decoding.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{ImageHeader, ImageHeaderCache, SourceKey};
/// let mut hc = ImageHeaderCache::new();
/// hc.insert(SourceKey::Ptr(8), ImageHeader::new(ColorFormat::L8, 3, 3));
/// assert_eq!(hc.get(&SourceKey::Ptr(8)).map(|h| h.w), Some(3));
/// ```
#[derive(Debug, Default)]
pub struct ImageHeaderCache {
    entries: heapless::Vec<(SourceKey, ImageHeader), HEADER_CACHE_ENTRIES>,
    next: usize,
}

impl ImageHeaderCache {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: heapless::Vec::new(),
            next: 0,
        }
    }

    /// The header of `key`, if cached.
    #[must_use]
    pub fn get(&self, key: &SourceKey) -> Option<ImageHeader> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, h)| *h)
    }

    /// Caches `header` for `key` (replacing the oldest entry when full).
    pub fn insert(&mut self, key: SourceKey, header: ImageHeader) {
        if let Some(e) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            e.1 = header;
            return;
        }
        if let Err(e) = self.entries.push((key, header)) {
            self.entries[self.next] = e;
            self.next = (self.next + 1) % HEADER_CACHE_ENTRIES;
        }
    }

    /// Drops every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next = 0;
    }
}
