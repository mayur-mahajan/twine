//! [`Engine`]: owns the tree, the displays, the render caches and the configuration.

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::fmt::Write;

use twine_core::Instant;
#[cfg(feature = "debug-checks")]
use twine_core::Rect;
use twine_image::{DecoderRegistry, ImageCache, ImageHeaderCache};
use twine_render::RenderCaches;
use twine_text::GlyphCache;

use crate::display::Display;
use crate::draw_cx::AuxRes;
use crate::{DisplayId, EngineConfig, EngineError, InvalidateReason, NodeId, PerfMonitor, Tree};

/// Everything drawing needs besides the engine itself: taken out of the engine while a chunk
/// is rendered, so the traversal can read the whole engine.
#[derive(Debug)]
pub(crate) struct RenderRes {
    pub(crate) caches: RenderCaches,
    pub(crate) aux: AuxRes,
}

/// The engine: the widget tree, up to [`MAX_DISPLAYS`](crate::MAX_DISPLAYS) displays with their
/// screens, layers and refreshers, the render caches (allocated once) and the configuration.
///
/// The engine is usable on its own (imperative, LVGL-like); the declarative view layer builds
/// on it. Call [`step`](Self::step) whenever it asks to be woken ([`Wake`](crate::Wake)).
///
/// ```
/// use twine_engine::{Engine, EngineConfig, Obj};
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let root = e.create_root(Box::new(Obj)).unwrap();
/// assert_eq!(e.tree().len(), 1);
/// assert!(e.tree().contains(root));
/// ```
pub struct Engine {
    pub(crate) tree: Tree,
    pub(crate) displays: Vec<Display>,
    pub(crate) default_display: Option<DisplayId>,
    pub(crate) config: EngineConfig,
    /// The default display's theme font (the default `TextFont` of every node).
    pub(crate) theme_font: Option<&'static twine_text::Font>,
    pub(crate) res: Option<RenderRes>,
    /// Nodes drawn in the current frame.
    pub(crate) nodes_drawn: Cell<u32>,
    pub(crate) bounds_overlay: bool,
    pub(crate) perf: PerfMonitor,
    last_perf_log: Option<Instant>,
    /// Input devices (slot index = `InputId`).
    pub(crate) inputs: crate::input::InputSlots,
    /// Serial number of the last added input device.
    pub(crate) input_serial: u32,
    /// The device being processed and its kind.
    pub(crate) input_active: Option<(crate::InputId, twine_hal::InputKind)>,
    /// The point of the pointer being processed.
    pub(crate) input_point: Option<twine_core::Point>,
    /// The node each pointer is scrolling (dragged or thrown) and the locked direction.
    pub(crate) indev_scrolls:
        heapless::Vec<(crate::InputId, NodeId, twine_style::Dir), { crate::MAX_INPUTS }>,
    /// Focus groups (slot index = `GroupId`).
    pub(crate) groups: Vec<Option<crate::group::Group>>,
    pub(crate) default_group: Option<crate::GroupId>,
    /// Gridnav containers.
    pub(crate) gridnavs: Vec<crate::gridnav::GridnavDsc>,
    /// Next user handler id.
    pub(crate) next_handler_id: u32,
    /// Next `EventCode::Custom` value.
    pub(crate) next_event_code: u16,
    /// The layout pass.
    pub(crate) layout: crate::layout::LayoutState,
    /// Animations and timers.
    pub(crate) anim: crate::anim::AnimState,
    /// Style transitions.
    pub(crate) trans: crate::transition::TransState,
    #[cfg(feature = "debug-checks")]
    pub(crate) invalidations: Vec<(Rect, InvalidateReason)>,
    /// The invalidations rendered by the last frame that started.
    #[cfg(feature = "debug-checks")]
    pub(crate) frame_invalidations: Vec<(Rect, InvalidateReason)>,
    #[cfg(feature = "debug-checks")]
    pub(crate) render_hook: Option<fn(u8)>,
}

impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Engine")
            .field("nodes", &self.tree.len())
            .field("displays", &self.displays.len())
            .field("inputs", &self.inputs.iter().flatten().count())
            .field("groups", &self.groups.iter().flatten().count())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// An engine with `config` (validated). Allocates the render caches once.
    pub fn new(config: EngineConfig) -> Result<Engine, EngineError> {
        config.validate()?;
        let res = RenderRes {
            caches: RenderCaches::new(&config.render_config()),
            aux: AuxRes {
                glyphs: GlyphCache::new(config.glyph_cache_bytes),
                images: ImageCache::new(config.image_cache_bytes, 8),
                headers: ImageHeaderCache::new(),
                registry: DecoderRegistry::with_defaults(),
                fs: None,
            },
        };
        twine_core::info!(
            target: "twine::engine",
            "engine: refr_period={} max_dirty_areas={} layer_buf={} B glyph_cache={} B cooperative_flush={}",
            config.refr_period,
            config.max_dirty_areas,
            config.layer_buf_bytes,
            config.glyph_cache_bytes,
            config.cooperative_flush
        );
        Ok(Self {
            tree: Tree::new(),
            displays: Vec::new(),
            default_display: None,
            config,
            theme_font: None,
            res: Some(res),
            nodes_drawn: Cell::new(0),
            bounds_overlay: false,
            perf: PerfMonitor::default(),
            last_perf_log: None,
            inputs: Vec::new(),
            input_serial: 0,
            input_active: None,
            input_point: None,
            indev_scrolls: heapless::Vec::new(),
            groups: Vec::new(),
            default_group: None,
            gridnavs: Vec::new(),
            next_handler_id: 0,
            next_event_code: 0,
            layout: crate::layout::LayoutState::default(),
            anim: crate::anim::AnimState::default(),
            trans: crate::transition::TransState::default(),
            // Both logs are swapped at every frame start: allocated once, up front.
            #[cfg(feature = "debug-checks")]
            invalidations: Vec::with_capacity(64),
            #[cfg(feature = "debug-checks")]
            frame_invalidations: Vec::with_capacity(64),
            #[cfg(feature = "debug-checks")]
            render_hook: None,
        })
    }

    /// The widget tree.
    #[must_use]
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The configuration, mutably (cache sizes cannot change after [`new`](Self::new)).
    pub fn config_mut(&mut self) -> &mut EngineConfig {
        &mut self.config
    }

    /// Hit/miss counters of the render caches.
    #[must_use]
    pub fn render_cache_stats(&self) -> twine_render::RenderCacheStats {
        self.res.as_ref().map(|r| r.caches.stats()).unwrap_or_default()
    }

    /// Sets where [`ImageSource::File`](twine_image::ImageSource::File) images are read from
    /// (`None`: file images are not found). The file system crate provides an implementation
    /// over its virtual file system; any [`FileSource`](twine_image::FileSource) works.
    pub fn set_file_source(&mut self, fs: Option<alloc::boxed::Box<dyn twine_image::FileSource>>) {
        if let Some(r) = self.res.as_mut() {
            r.aux.fs = fs;
            r.aux.headers.clear();
        }
    }

    /// The header (size, format) of an image source, read through the header cache and the
    /// decoder registry (encoded data and files are probed, not decoded).
    ///
    /// # Errors
    /// [`twine_image::Error::NotFound`] for missing files (or without a file source),
    /// [`twine_image::Error::InvalidHeader`] for unknown formats,
    /// [`twine_image::Error::UnsupportedSource`] for symbols and SVG (not raster images).
    pub fn image_header(
        &mut self,
        src: &twine_image::ImageSource,
    ) -> Result<twine_image::ImageHeader, twine_image::Error> {
        let Some(r) = self.res.as_mut() else {
            return Err(twine_image::Error::UnsupportedSource("engine is rendering"));
        };
        let crate::draw_cx::AuxRes {
            images,
            headers,
            registry,
            fs,
            ..
        } = &mut r.aux;
        let mut icx = twine_image::ImageContext {
            cache: images,
            header_cache: headers,
            registry,
            fs: fs.as_deref_mut().map(|f| f as &mut dyn twine_image::FileSource),
        };
        twine_image::header_of(src, &mut icx)
    }

    /// The performance monitor (fps and CPU of the last one-second window).
    #[must_use]
    pub fn perf_monitor(&self) -> &PerfMonitor {
        &self.perf
    }

    /// Calls `hook(buffer_index)` right before a chunk is rendered into partial buffer
    /// `buffer_index` (feature `debug-checks`; used by tests to prove DMA overlap).
    #[cfg(feature = "debug-checks")]
    pub fn set_render_hook(&mut self, hook: Option<fn(u8)>) {
        self.render_hook = hook;
    }

    /// Toggles the layout bounds overlay (every node's coordinates outlined in magenta, drawn
    /// above the system layer) and redraws every display.
    pub fn set_bounds_overlay(&mut self, on: bool) {
        if self.bounds_overlay == on {
            return;
        }
        self.bounds_overlay = on;
        self.invalidate_all();
    }

    /// Whether the layout bounds overlay is on.
    #[must_use]
    pub fn bounds_overlay(&self) -> bool {
        self.bounds_overlay
    }

    /// Schedules a full redraw of every display.
    pub fn invalidate_all(&mut self) {
        for i in 0..self.displays.len() {
            let (id, area) = (self.displays[i].id, self.displays[i].area());
            self.invalidate_area(id, area, InvalidateReason::Explicit);
        }
    }

    /// A dump of every root of every display (see [`Tree::dump`]), bottom layer first.
    #[must_use]
    pub fn dump(&self) -> String {
        let mut s = String::new();
        for d in &self.displays {
            let _ = writeln!(s, "display {}:", d.id);
            let mut roots: Vec<(&str, NodeId)> = alloc::vec![("bottom layer", d.bottom_layer)];
            for &sc in &d.screens {
                roots.push((
                    if sc == d.active_screen {
                        "screen (active)"
                    } else {
                        "screen"
                    },
                    sc,
                ));
            }
            roots.push(("top layer", d.top_layer));
            roots.push(("sys layer", d.sys_layer));
            for (name, r) in roots {
                let _ = writeln!(s, "# {name}");
                let _ = self.tree.dump(r, &mut s);
            }
        }
        s
    }

    /// Bookkeeping after a refresh: closes the performance window, logs the periodic
    /// `twine::perf` line and updates the performance overlay.
    pub(crate) fn after_refresh(&mut self, now: Instant, rendered: bool) {
        let _ = rendered;
        if self.perf.window_done(now) {
            #[cfg(feature = "perf-monitor")]
            self.update_perf_overlays();
        }
        let due = self
            .last_perf_log
            .is_none_or(|t| now.saturating_duration_since(t) >= self.config.perf_log_period);
        if due && !self.displays.is_empty() {
            if self.last_perf_log.is_some() {
                let s = self.displays[0].refresher.stats;
                twine_core::info!(
                    target: "twine::perf",
                    "fps={} cpu={}% render={}us flush={}us wait={}us px/frame={} mem={}/{}",
                    self.perf.fps(),
                    self.perf.cpu_percent(),
                    s.render_us,
                    s.flush_us,
                    s.flush_wait_us,
                    s.dirty_px,
                    s.mem_used,
                    s.mem_peak
                );
            }
            self.last_perf_log = Some(now);
        }
    }
}
