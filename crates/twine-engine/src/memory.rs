//! [`EngineMemory`]: what the engine's memory is used for ([`Engine::memory_report`]).

use crate::Engine;

/// The memory an [`Engine`] holds, by part — from [`Engine::memory_report`].
///
/// **Units:** every `usize` field except `nodes` is in **bytes**, counted as what was asked of
/// the allocator (vector capacities, box sizes), without allocator overhead. Heap and caller
/// memory are kept apart: [`heap_bytes`](Self::heap_bytes) is what the engine takes from the
/// heap, [`static_bytes`](Self::static_bytes) what the application gave it (statics).
///
/// **Not included:** what widgets allocate themselves (a label's text, a list's items),
/// shared style buffers (`Rc<StyleBuf>`, themes), framebuffers (owned by their
/// `FramebufferDisplay` driver), the draw buffers of chunked displays (owned by their caller,
/// e.g. `AsyncUi`, which reports them), and small bookkeeping vectors (focus groups, input
/// devices, animation, transition and layout work lists, posted events). The reactive
/// runtime, the engine command queues and the waker pool are reported by `twine-view`'s
/// `Ui::memory_report`, which wraps this report.
///
/// ```
/// use twine_engine::{Engine, EngineConfig, Obj};
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let before = e.memory_report();
/// assert_eq!(before.layer_buf, 24 * 1024); // the default `layer_buf_bytes`, on the heap
/// assert!(!before.layer_buf_static);
/// e.create_root(Box::new(Obj)).unwrap();
/// let after = e.memory_report();
/// assert_eq!(after.nodes, 1);
/// assert!(after.tree > before.tree);
/// assert!(after.heap_bytes() >= after.tree + after.layer_buf);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EngineMemory {
    /// Live nodes (a count, not bytes): widgets, screens and display layers.
    pub nodes: usize,
    /// The widget tree: the node arena (every slot, by capacity, and its free lists), the
    /// root and style-link vectors, and per node its boxed widget, style entry vector, layout
    /// extension and event handlers.
    pub tree: usize,
    /// Node slots of the arena (a count: live, free and retired slots, by capacity).
    pub node_slots: usize,
    /// The renderer's per-span scratch buffers (`8 ×` the widest span; see
    /// `RenderCaches::scratch_bytes`).
    pub render_scratch: usize,
    /// The renderer's quarter-circle, shadow and gradient caches (fixed arenas).
    pub render_caches: usize,
    /// The glyph cache with the text renderer's scratch rows
    /// ([`EngineConfig::glyph_cache_bytes`](crate::EngineConfig::glyph_cache_bytes) plus
    /// tables).
    pub glyph_cache: usize,
    /// Decoded images in the image cache and its entry table (bounded by
    /// [`EngineConfig::image_cache_bytes`](crate::EngineConfig::image_cache_bytes) plus the
    /// table).
    pub image_cache: usize,
    /// The ARGB8888 layer buffer (opacity groups, transforms, blend modes), wherever it
    /// lives.
    pub layer_buf: usize,
    /// The layer buffer is caller memory ([`Engine::with_layer_buf`]) instead of heap.
    pub layer_buf_static: bool,
    /// Partial draw buffers the engine allocated (`BufferMode::Alloc`; alignment padding
    /// included). Heap draw buffers live as long as the program, like statics.
    pub draw_buffers_heap: usize,
    /// Partial draw buffers in caller memory (`BufferMode::Partial`, e.g. `draw_buffers!`
    /// statics).
    pub draw_buffers_static: usize,
}

impl EngineMemory {
    /// Heap bytes of the engine: the tree, the renderer's scratch buffers and caches, the
    /// glyph and image caches, a heap layer buffer and heap draw buffers.
    ///
    /// # Panics
    /// Never for a report from [`Engine::memory_report`]; a hand-built value whose parts sum
    /// past `usize::MAX` overflows (a panic in debug builds).
    ///
    /// ```
    /// use twine_engine::EngineMemory;
    /// let m = EngineMemory { tree: 100, layer_buf: 4096, draw_buffers_static: 9600, ..EngineMemory::default() };
    /// assert_eq!(m.heap_bytes(), 4196);
    /// assert_eq!(m.static_bytes(), 9600);
    /// ```
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let layer = if self.layer_buf_static { 0 } else { self.layer_buf };
        self.tree
            + self.render_scratch
            + self.render_caches
            + self.glyph_cache
            + self.image_cache
            + layer
            + self.draw_buffers_heap
    }

    /// Caller memory the engine uses (a static layer buffer and static draw buffers): RAM
    /// the linker placed, not the heap. See [`heap_bytes`](Self::heap_bytes).
    ///
    /// # Panics
    /// As [`heap_bytes`](Self::heap_bytes): never for a report from
    /// [`Engine::memory_report`].
    ///
    /// ```
    /// use twine_engine::EngineMemory;
    /// let m = EngineMemory { layer_buf: 2048, layer_buf_static: true, draw_buffers_static: 4096, ..EngineMemory::default() };
    /// assert_eq!(m.static_bytes(), 6144);
    /// assert_eq!(m.heap_bytes(), 0);
    /// ```
    #[must_use]
    pub fn static_bytes(&self) -> usize {
        let layer = if self.layer_buf_static { self.layer_buf } else { 0 };
        layer + self.draw_buffers_static
    }
}

impl Engine {
    /// What the engine's memory is used for, by part ([`EngineMemory`]: units, and what is
    /// not included). For budgeting a device (which part to shrink), for a debug console, and
    /// for tests that pin the memory of a scene.
    ///
    /// Allocates nothing; never panics; O(nodes) (per-node boxes are summed), so call it on
    /// demand, not every frame.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    ///
    /// let cfg = EngineConfig { glyph_cache_bytes: 4096, ..EngineConfig::default() };
    /// let e = Engine::new(cfg).unwrap();
    /// let m = e.memory_report();
    /// assert!(m.glyph_cache >= 4096);
    /// assert_eq!((m.nodes, m.draw_buffers_heap), (0, 0));
    /// ```
    #[must_use]
    pub fn memory_report(&self) -> EngineMemory {
        let mut m = EngineMemory {
            nodes: self.tree.len(),
            tree: self.tree.heap_bytes(),
            node_slots: self.tree.slot_capacity(),
            ..EngineMemory::default()
        };
        if let Some(res) = &self.res {
            m.render_scratch = res.caches.scratch_bytes();
            m.render_caches = res.caches.cache_bytes();
            m.layer_buf = res.caches.layer_buf_bytes();
            m.layer_buf_static = res.caches.layer_buf_is_static();
            m.glyph_cache = res.aux.glyphs.bytes_reserved();
            m.image_cache = res.aux.images.bytes_reserved();
        }
        for d in &self.displays {
            m.draw_buffers_heap += d.draw_buffers.heap;
            m.draw_buffers_static += d.draw_buffers.caller;
        }
        m
    }
}
