//! [`RenderConfig`] and [`RenderCaches`]: all scratch memory and caches of the renderer,
//! allocated once with fixed budgets. Drawing never allocates; caches evict instead of growing.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use crate::circle::CircleCache;
use crate::gradient::{Gradient, GradientCache};
use crate::shadow::ShadowCache;

/// Budgets of the renderer's caches and scratch buffers.
///
/// ```
/// use twine_render::{RenderCaches, RenderConfig};
/// let cfg = RenderConfig { layer_buf_bytes: 8 * 1024, ..RenderConfig::default() };
/// let caches = RenderCaches::new(&cfg);
/// assert!(caches.bytes_reserved() >= 8 * 1024);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RenderConfig {
    /// Quarter-circle coverage entries (default 4).
    pub circle_cache_entries: u8,
    /// Blurred shadow corners (default 1; 4 KiB each).
    pub shadow_cache_entries: u8,
    /// Gradient color maps (default 4; 1 KiB each).
    pub gradient_cache_entries: u8,
    /// Widest span in pixels processed at once (default 480). Wider spans are split.
    pub max_span: u16,
    /// Bytes of the ARGB8888 layer buffer (default 24 KiB).
    pub layer_buf_bytes: u32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            circle_cache_entries: 4,
            shadow_cache_entries: 1,
            gradient_cache_entries: 4,
            max_span: 480,
            layer_buf_bytes: 24 * 1024,
        }
    }
}

/// Hit and miss counters of one cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u32,
    /// Lookups that computed the entry.
    pub misses: u32,
}

/// Counters of every cache (for tests and the performance monitor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RenderCacheStats {
    /// Quarter-circle coverage.
    pub circle: CacheStats,
    /// Blurred shadow corners.
    pub shadow: CacheStats,
    /// Gradient color maps.
    pub gradient: CacheStats,
    /// Image rows drawn by the same-format copy path of
    /// [`Painter::image`](crate::Painter::image).
    pub image_copy_rows: u32,
}

/// All scratch memory and caches of the renderer, created once (e.g. when the engine is built)
/// with [`RenderConfig`] budgets and reused by every [`Painter`](crate::Painter).
#[derive(Debug)]
pub struct RenderCaches {
    config: RenderConfig,
    /// Coverage of the primitive being drawn (`max_span`).
    pub(crate) cov: Vec<u8>,
    /// Coverage produced by masks when the primitive has none (`max_span`).
    pub(crate) mask: Vec<u8>,
    /// `Argb8888` row samples (gradients, transformed images; `4 × max_span`).
    pub(crate) span: Vec<u8>,
    /// 16-bit accumulators (polygon coverage, shadow blur; `max_span`).
    pub(crate) acc: Vec<u16>,
    pub(crate) circle: CircleCache,
    pub(crate) shadow: ShadowCache,
    pub(crate) gradient: GradientCache,
    /// The layer buffer (taken out while a layer is being drawn).
    pub(crate) layer_buf: LayerBuf,
    /// Debug counter: image rows copied with the same-format fast path.
    pub(crate) image_copy_rows: u32,
    /// Scratch state of a higher-level drawing crate (see [`take_extension`](Self::take_extension)).
    ext: Option<Box<dyn Any>>,
}

/// Where the layer buffer lives: on the heap ([`RenderCaches::new`]) or in caller memory
/// ([`RenderCaches::with_layer_buf`]). Dereferences to the bytes; `Default` (an empty buffer,
/// no allocation) is what `mem::take` leaves behind while a layer is drawn.
#[derive(Debug, Default)]
pub(crate) enum LayerBuf {
    /// Taken out (a layer is being drawn), or a zero-byte budget.
    #[default]
    Empty,
    /// Allocated by [`RenderCaches::new`].
    Heap(Vec<u8>),
    /// Caller memory (a `static`), never freed.
    Static(&'static mut [u8]),
}

impl core::ops::Deref for LayerBuf {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            LayerBuf::Empty => &[],
            LayerBuf::Heap(v) => v,
            LayerBuf::Static(s) => s,
        }
    }
}

impl core::ops::DerefMut for LayerBuf {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            LayerBuf::Empty => &mut [],
            LayerBuf::Heap(v) => v,
            LayerBuf::Static(s) => s,
        }
    }
}

impl RenderCaches {
    /// Allocates every buffer and cache arena of `cfg`, the layer buffer
    /// (`cfg.layer_buf_bytes`) included.
    #[must_use]
    pub fn new(cfg: &RenderConfig) -> Self {
        let layer_buf = if cfg.layer_buf_bytes == 0 {
            LayerBuf::Empty
        } else {
            LayerBuf::Heap(vec![0; cfg.layer_buf_bytes as usize])
        };
        Self::with_layer(cfg, layer_buf)
    }

    /// Like [`new`](Self::new), but the layer buffer is `layer_buf` — caller memory, typically
    /// a `static`, so the largest scratch buffer of the renderer stays out of the heap.
    /// `cfg.layer_buf_bytes` is ignored; [`config`](Self::config) reports `layer_buf.len()`
    /// (saturated to `u32::MAX`). The bytes need no particular alignment or initial content
    /// (every layer clears what it uses).
    ///
    /// Allocates the other scratch buffers and caches of `cfg` as `new` does; never panics.
    ///
    /// ```
    /// use twine_render::{RenderCaches, RenderConfig};
    ///
    /// // On firmware: `static LAYER: TakeOnce<[u8; 8192]>` and `LAYER.take()`.
    /// let layer: &'static mut [u8] = Box::leak(Box::new([0u8; 8192]));
    /// let caches = RenderCaches::with_layer_buf(&RenderConfig::default(), layer);
    /// assert!(caches.layer_buf_is_static());
    /// assert_eq!(caches.layer_buf_bytes(), 8192);
    /// assert_eq!(caches.config().layer_buf_bytes, 8192);
    /// ```
    #[must_use]
    pub fn with_layer_buf(cfg: &RenderConfig, layer_buf: &'static mut [u8]) -> Self {
        let cfg = RenderConfig {
            layer_buf_bytes: u32::try_from(layer_buf.len()).unwrap_or(u32::MAX),
            ..*cfg
        };
        Self::with_layer(&cfg, LayerBuf::Static(layer_buf))
    }

    fn with_layer(cfg: &RenderConfig, layer_buf: LayerBuf) -> Self {
        let span = usize::from(cfg.max_span.max(16));
        Self {
            config: *cfg,
            cov: vec![0; span],
            mask: vec![0; span],
            span: vec![0; 4 * span],
            acc: vec![0; span],
            circle: CircleCache::new(cfg.circle_cache_entries),
            shadow: ShadowCache::new(cfg.shadow_cache_entries),
            gradient: GradientCache::new(cfg.gradient_cache_entries),
            layer_buf,
            image_copy_rows: 0,
            ext: None,
        }
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &RenderConfig {
        &self.config
    }

    /// Widest span processed at once.
    #[must_use]
    pub fn max_span(&self) -> usize {
        self.cov.len()
    }

    /// Heap bytes reserved by the scratch buffers, the caches and a heap layer buffer:
    /// [`scratch_bytes`](Self::scratch_bytes) + [`cache_bytes`](Self::cache_bytes) + the
    /// layer buffer unless it [is static](Self::layer_buf_is_static). The extension slot
    /// ([`put_extension`](Self::put_extension)) is not included.
    ///
    /// Allocates nothing; never panics; O(1).
    #[must_use]
    pub fn bytes_reserved(&self) -> usize {
        let layer = if self.layer_buf_is_static() {
            0
        } else {
            self.layer_buf_bytes()
        };
        self.scratch_bytes() + self.cache_bytes() + layer
    }

    /// Heap bytes of the per-span scratch buffers (coverage, mask, ARGB samples and 16-bit
    /// accumulators: `8 × max_span` bytes by capacity). Allocates nothing; never panics.
    ///
    /// ```
    /// use twine_render::{RenderCaches, RenderConfig};
    /// let c = RenderCaches::new(&RenderConfig { max_span: 100, ..RenderConfig::default() });
    /// assert_eq!(c.scratch_bytes(), 8 * 100);
    /// ```
    #[must_use]
    pub fn scratch_bytes(&self) -> usize {
        self.cov.capacity()
            + self.mask.capacity()
            + self.span.capacity()
            + self.acc.capacity() * core::mem::size_of::<u16>()
    }

    /// Heap bytes of the quarter-circle, shadow and gradient caches (their fixed arenas).
    /// Allocates nothing; never panics.
    #[must_use]
    pub fn cache_bytes(&self) -> usize {
        self.circle.bytes_reserved() + self.shadow.bytes_reserved() + self.gradient.bytes_reserved()
    }

    /// Size of the layer buffer in bytes, wherever it lives (0 while a layer is being drawn,
    /// which is never observable from outside a draw).
    #[must_use]
    pub fn layer_buf_bytes(&self) -> usize {
        self.layer_buf.len()
    }

    /// Whether the layer buffer is caller memory ([`with_layer_buf`](Self::with_layer_buf))
    /// rather than heap.
    #[must_use]
    pub fn layer_buf_is_static(&self) -> bool {
        matches!(self.layer_buf, LayerBuf::Static(_))
    }

    /// The cached 256-entry color map (packed `0xAARRGGBB`, see [`build_color_map`]) of
    /// `g`'s stops, built on a miss (evicting the least recently used map). Counts in
    /// [`stats().gradient`](Self::stats).
    ///
    /// [`build_color_map`]: crate::build_color_map
    pub fn gradient_color_map(&mut self, g: &Gradient) -> &[u32] {
        let i = self.gradient.lookup(g);
        self.gradient.map(i)
    }

    /// Takes the extension scratch state of type `T` out of the caches, if it is stored there.
    ///
    /// Crates that draw through the [`Painter`](crate::Painter) but keep their own scratch
    /// buffers (the vector rasterizer) park them here between draws, so repeated drawing reuses
    /// the buffers without the caller having to pass them around. Taking and putting back moves
    /// a box and never allocates. There is one slot: storing a different type replaces it.
    ///
    /// ```
    /// use twine_render::RenderCaches;
    /// let mut c = RenderCaches::default();
    /// assert!(c.take_extension::<Vec<u8>>().is_none());
    /// c.put_extension(Box::new(vec![1u8, 2]));
    /// let v = c.take_extension::<Vec<u8>>().unwrap();
    /// assert_eq!(*v, [1, 2]);
    /// ```
    pub fn take_extension<T: Any>(&mut self) -> Option<Box<T>> {
        match self.ext.take() {
            Some(b) if b.is::<T>() => b.downcast::<T>().ok(),
            other => {
                self.ext = other;
                None
            }
        }
    }

    /// Stores extension scratch state (see [`take_extension`](Self::take_extension)).
    pub fn put_extension<T: Any>(&mut self, v: Box<T>) {
        self.ext = Some(v);
    }

    /// Hit/miss counters.
    #[must_use]
    pub fn stats(&self) -> RenderCacheStats {
        RenderCacheStats {
            circle: self.circle.stats(),
            shadow: self.shadow.stats(),
            gradient: self.gradient.stats(),
            image_copy_rows: self.image_copy_rows,
        }
    }
}

impl Default for RenderCaches {
    fn default() -> Self {
        Self::new(&RenderConfig::default())
    }
}
