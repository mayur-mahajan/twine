//! Display integrity (`docs/safety/profile.md` § 11): [`FrameObserver`] (an independent monitor
//! sees every chunk sent to a panel), the off-screen re-render of a region
//! ([`Engine::render_region_to`]), panel read-back ([`Engine::read_back`]) and the check that
//! compares both ([`Engine::check_region`]), safety-critical nodes
//! ([`ObjFlags::SAFETY_CRITICAL`], [`Engine::critical_report`]), the heartbeat
//! ([`Engine::heartbeat`]) and the fail-safe screen ([`Engine::set_fail_safe_screen`],
//! [`Engine::enter_fail_safe`]).
//!
//! Twine provides the mechanisms; the application owns the policy — what to check, how often,
//! and what a mismatch leads to (a retry, the fail-safe screen, a reset).

use alloc::boxed::Box;
use core::cell::Cell;

use twine_core::alloc_guard::{self, AllocSite};
use twine_core::crc::Crc32;
use twine_core::fault::FaultKind;
use twine_core::{ColorFormat, Opa, Rect, Rotation};
use twine_render::{DrawBuf, Painter};

use crate::display::Backend;
use crate::refresh::{Strategy, traverse};
use crate::{DisplayId, Engine, EngineError, FaultRecord, InvalidateReason, NodeId, ObjFlags, RefreshStats};

/// An independent monitor of what Twine sends to its displays: it sees every chunk of
/// pixels right before the driver gets it (or, for a framebuffer display, right after it was
/// rendered into the framebuffer) and the end of every frame — e.g. to compute checksums of
/// the regions that show safety-critical values and compare them with expected ones on
/// another core, or to log what was drawn.
///
/// Registered once, at start-up ([`Engine::set_frame_observer`],
/// `UiBuilder::frame_observer`); the engine calls it from its refresh path, so it must be fast
/// and must not allocate (a [`Crc32`] over the chunk's rows is a few cycles per byte). Without
/// an observer the refresh path pays one branch per chunk.
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
/// use twine_core::{ColorFormat, Instant};
/// use twine_core::crc::Crc32;
/// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, FrameChunk, FrameObserver, FrameReport};
/// use twine_hal::DisplayInfo;
/// use twine_testing::MemoryDisplay;
///
/// /// Counts the chunks and remembers each frame's checksum.
/// #[derive(Default)]
/// struct Monitor { chunks: u32, crc: Crc32, frames: Vec<u32> }
/// struct Shared(Rc<RefCell<Monitor>>);
/// impl FrameObserver for Shared {
///     fn chunk(&mut self, chunk: &FrameChunk<'_>) {
///         let mut m = self.0.borrow_mut();
///         m.chunks += 1;
///         for row in chunk.rows() { m.crc.update(row); }
///     }
///     fn frame_done(&mut self, report: &FrameReport) {
///         let mut m = self.0.borrow_mut();
///         let v = m.crc.value();
///         m.frames.push(v);
///         m.crc = Crc32::new();
///         assert_eq!(report.stats.chunks, 4);
///     }
/// }
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
/// e.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
/// let seen = Rc::new(RefCell::new(Monitor::default()));
/// e.set_frame_observer(Shared(seen.clone()));
/// e.step(Instant::from_millis(0));
/// assert_eq!(seen.borrow().chunks, 4); // the 32-row screen in 8-row chunks
/// assert_eq!(seen.borrow().frames.len(), 1);
/// ```
pub trait FrameObserver {
    /// A chunk of a frame: `chunk.area` of `chunk.display` will show `chunk`'s pixels. Called
    /// once per chunk, in rendering order, before the chunk is flushed (partial and chunked
    /// displays) or after it was rendered into the framebuffer (framebuffer displays).
    fn chunk(&mut self, chunk: &FrameChunk<'_>);

    /// The frame of `report.display` is complete (all its chunks were seen). Called once per
    /// frame, after its last chunk.
    fn frame_done(&mut self, report: &FrameReport);
}

/// The pixels of one chunk seen by a [`FrameObserver`].
///
/// The pixels are in the display's flush layout: `format` is what the panel receives (`I1`
/// for mono panels, converted from the engine's `L8`), `area` is in flush coordinates
/// (physical for software-rotated displays, see `twine_hal::display`). Read them with
/// [`rows`](Self::rows) (each row of `area`, packed) or check them with [`crc`](Self::crc).
#[derive(Clone, Copy, Debug)]
pub struct FrameChunk<'a> {
    /// The display the chunk is for.
    pub display: DisplayId,
    /// The area the chunk covers (flush coordinates).
    pub area: Rect,
    /// The pixel format of the bytes.
    pub format: ColorFormat,
    /// The bytes, starting at the first byte of the area's first row.
    pixels: &'a [u8],
    /// Bytes from one row of the area to the next.
    stride: usize,
}

impl<'a> FrameChunk<'a> {
    /// A chunk over `pixels` (`stride` bytes per row; the first byte is the area's top-left
    /// pixel). For observers' tests and custom refresh paths.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Rect};
    /// use twine_engine::{Engine, EngineConfig, FrameChunk};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(4, 4, ColorFormat::L8), 16).unwrap();
    /// let px = [1u8, 2, 3, 4, 5, 6];
    /// let c = FrameChunk::new(d, Rect::from_xywh(0, 0, 2, 2), ColorFormat::L8, &px, 3);
    /// let rows: Vec<&[u8]> = c.rows().collect();
    /// assert_eq!(rows, [&[1u8, 2][..], &[4, 5][..]]);
    /// ```
    #[must_use]
    pub const fn new(
        display: DisplayId,
        area: Rect,
        format: ColorFormat,
        pixels: &'a [u8],
        stride: usize,
    ) -> Self {
        FrameChunk {
            display,
            area,
            format,
            pixels,
            stride,
        }
    }

    /// Bytes of one row of the area (sub-byte formats round up).
    #[must_use]
    pub fn row_bytes(&self) -> usize {
        self.format
            .stride(u32::try_from(self.area.width().max(0)).unwrap_or(0)) as usize
    }

    /// The rows of the area, top to bottom, each [`row_bytes`](Self::row_bytes) long (a row
    /// the buffer does not hold completely is left out: never panics).
    pub fn rows(&self) -> impl Iterator<Item = &'a [u8]> + 'a {
        let (pixels, stride, len) = (self.pixels, self.stride.max(1), self.row_bytes());
        let height = usize::try_from(self.area.height().max(0)).unwrap_or(0);
        (0..height).map_while(move |r| {
            let start = r.checked_mul(stride)?;
            pixels.get(start..start.checked_add(len)?)
        })
    }

    /// The [`Crc32`] of [`rows`](Self::rows) (the area's packed pixels).
    #[must_use]
    pub fn crc(&self) -> Crc32 {
        let mut c = Crc32::new();
        for row in self.rows() {
            c.update(row);
        }
        c
    }
}

/// The end of a frame, as a [`FrameObserver`] sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameReport {
    /// The display.
    pub display: DisplayId,
    /// The frame's statistics (frame number, areas, pixels, chunks, timings).
    pub stats: RefreshStats,
}

/// A node's region on its display: what [`Engine::render_region_to`] renders and
/// [`Engine::check_region`] checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Region {
    /// The display showing the node.
    pub display: DisplayId,
    /// The node's visible area: its coordinates plus extra draw size, clipped by its
    /// ancestors (logical coordinates).
    pub rect: Rect,
    /// Bytes a buffer needs for [`Engine::render_region_to`] (rows of `rect` in the display's
    /// format; `width × height` for `I1` displays, which are rendered as `L8` and packed).
    pub buffer_bytes: usize,
}

/// What [`Engine::critical_report`] tells about a safety-critical node, for an independent
/// cross-check of what it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CriticalReport {
    /// The node.
    pub node: NodeId,
    /// Its region (`None` when it is not visible: hidden, clipped away, not on a shown screen).
    pub region: Option<Region>,
    /// The digest of the value its widget shows ([`Widget::value_digest`](crate::Widget::value_digest):
    /// a label's text, a bar's value and range…).
    pub value: Crc32,
    /// The display frame the node was last drawn in (`None`: not drawn since it became
    /// critical).
    pub drawn_frame: Option<u32>,
}

/// The result of [`Engine::check_region`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct IntegrityCheck {
    /// The node whose region was checked.
    pub node: NodeId,
    /// Its region.
    pub region: Region,
    /// Checksum of the independent off-screen re-render.
    pub expected: Crc32,
    /// Checksum of what the display holds (read back).
    pub actual: Crc32,
}

impl IntegrityCheck {
    /// Whether the display shows what was rendered.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.expected == self.actual
    }
}

/// A safety-critical node and when it was last drawn.
#[derive(Debug)]
pub(crate) struct CriticalEntry {
    pub(crate) node: NodeId,
    pub(crate) drawn_frame: Cell<Option<u32>>,
}

/// A display's fail-safe screen ([`Engine::set_fail_safe_screen`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FailSafe {
    /// The registered screen.
    pub(crate) screen: Option<NodeId>,
    /// The reason code while the display is in the fail-safe state.
    pub(crate) active: Option<u32>,
}

impl Engine {
    // ---- Frame observer ------------------------------------------------------------------

    /// Registers `observer` (replacing a previous one): from now on it sees every chunk sent
    /// to every display and the end of every frame (see [`FrameObserver`]). Register it at
    /// start-up: this boxes the observer (the only allocation; noted as an
    /// [`EngineScratch`](AllocSite::EngineScratch) growth after a seal). Never panics.
    pub fn set_frame_observer(&mut self, observer: impl FrameObserver + 'static) {
        alloc_guard::note(AllocSite::EngineScratch);
        self.observer = Some(Box::new(observer));
    }

    /// Removes the frame observer and returns it (`None` if there was none).
    pub fn take_frame_observer(&mut self) -> Option<Box<dyn FrameObserver>> {
        self.observer.take()
    }

    /// Shows a chunk to the observer (one branch when there is none). `stride`: bytes per row
    /// of `pixels`, `None` for packed rows of `area`.
    #[inline]
    pub(crate) fn observe_chunk(&mut self, d: usize, area: Rect, pixels: &[u8], stride: Option<usize>) {
        if let Some(o) = self.observer.as_deref_mut() {
            notify_chunk(o, &self.displays, d, area, pixels, stride);
        }
    }

    /// Tells the observer that the frame of display `d` with `stats` is complete.
    #[inline]
    pub(crate) fn observe_frame(&mut self, d: usize, stats: &RefreshStats) {
        if let (Some(o), Some(disp)) = (self.observer.as_deref_mut(), self.displays.get(d)) {
            o.frame_done(&FrameReport {
                display: disp.id,
                stats: *stats,
            });
        }
    }

    // ---- Off-screen re-render and read-back ------------------------------------------------

    /// The region `node` occupies on its display ([`Region`]).
    ///
    /// # Errors
    /// [`EngineError::NodeNotFound`] for an unknown node; [`EngineError::NotVisible`] when it
    /// is hidden, clipped away or not on a shown screen or layer.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Rect};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, Obj};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.set_pos(n, 4, 4);
    /// e.set_size(n, 10, 6);
    /// e.update_layout();
    /// let r = e.region(n).unwrap();
    /// assert_eq!((r.display, r.rect), (d, Rect::from_xywh(4, 4, 10, 6)));
    /// assert_eq!(r.buffer_bytes, 10 * 6 * 2);
    /// ```
    pub fn region(&self, node: NodeId) -> Result<Region, EngineError> {
        if !self.tree.contains(node) {
            return Err(EngineError::NodeNotFound(node));
        }
        let (d, rect) = self.visible_area(node).ok_or(EngineError::NotVisible(node))?;
        let disp = self.displays.get(d).ok_or(EngineError::NotVisible(node))?;
        Ok(Region {
            display: disp.id,
            rect,
            buffer_bytes: region_bytes(disp.info.format, rect),
        })
    }

    /// Renders the region of `node` ([`region`](Self::region): the node's visible area,
    /// with everything that shows there — its ancestors' backgrounds, its siblings above it)
    /// **off-screen** into `buf` and returns the [`Crc32`] of the pixels: the "expected" half
    /// of a display-integrity check, to compare with what the panel shows (a
    /// [`FrameObserver`]'s checksum of the region, or [`read_back`](Self::read_back) —
    /// [`check_region`](Self::check_region) does both).
    ///
    /// The re-render is independent of the on-screen one: it runs in fresh mode (no cached
    /// circle, gradient, shadow or glyph data is used as it is) and without the draw
    /// accelerator, so a corrupted cache or a faulty accelerator shows up as a mismatch.
    /// `buf` receives the rows of the region, packed, in the display's format (`I1` displays:
    /// rendered as `L8` and packed in place, so `buf` needs [`Region::buffer_bytes`] = one
    /// byte per pixel). Nothing is flushed, no statistics change, nothing is allocated.
    ///
    /// The region is in logical coordinates; for a software-rotated display it is not the
    /// layout the panel receives. Rendering with `EngineConfig::debug_refresh` tints nothing
    /// here (the on-screen frames are tinted, so they will not match).
    ///
    /// # Errors
    /// [`EngineError::NodeNotFound`], [`EngineError::NotVisible`] (see
    /// [`region`](Self::region)); [`EngineError::BufferTooSmall`] when `buf` is shorter than
    /// [`Region::buffer_bytes`]; [`EngineError::Busy`] when called from inside drawing.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Instant, Opa};
    /// use twine_core::crc::Crc32;
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, Obj};
    /// use twine_hal::DisplayInfo;
    /// use twine_style::{Selector, StyleProp};
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.set_pos(n, 4, 4);
    /// e.set_size(n, 10, 6);
    /// e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
    /// e.set_local_prop(n, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    /// e.step(Instant::from_millis(0)); // drawn on the panel
    ///
    /// let mut buf = vec![0u8; e.region(n).unwrap().buffer_bytes];
    /// let expected = e.render_region_to(n, &mut buf).unwrap();
    /// assert_eq!(expected, Crc32::of(&buf));
    /// // The panel holds the same pixels.
    /// let panel = e.driver::<MemoryDisplay>(d).unwrap();
    /// let mut actual = Crc32::new();
    /// for y in 4..10 {
    ///     let row = &panel.framebuffer()[y * 128 + 8..y * 128 + 28];
    ///     actual.update(row);
    /// }
    /// assert_eq!(expected, actual);
    /// ```
    pub fn render_region_to(&mut self, node: NodeId, buf: &mut [u8]) -> Result<Crc32, EngineError> {
        let region = self.region(node)?;
        let d = region.display.index();
        let len = self.render_offscreen(d, region.rect, buf)?;
        Ok(Crc32::of(buf.get(..len).unwrap_or_default()))
    }

    /// Renders `rect` of display `d` into `buf` off-screen (fresh mode, no accelerator,
    /// no tint); returns the bytes written (packed rows in the display's format).
    fn render_offscreen(&mut self, d: usize, rect: Rect, buf: &mut [u8]) -> Result<usize, EngineError> {
        let format = self.displays.get(d).ok_or(EngineError::Busy)?.info.format;
        let needed = region_bytes(format, rect);
        let Some(buf) = buf.get_mut(..needed) else {
            return Err(EngineError::BufferTooSmall {
                needed,
                got: buf.len(),
            });
        };
        let (w, h) = (
            usize::try_from(rect.width()).unwrap_or(0),
            usize::try_from(rect.height()).unwrap_or(0),
        );
        let mono = format == ColorFormat::I1;
        let (draw_format, stride) = if mono {
            (ColorFormat::L8, w)
        } else {
            (format, format.stride(u32::try_from(w).unwrap_or(0)) as usize)
        };
        let Some(mut res) = self.res.take() else {
            twine_core::warn!(target: "twine::refresh", "render_region_to: called from inside drawing");
            return Err(EngineError::Busy);
        };
        let accel = res.accel.0.take();
        let fresh = (res.caches.fresh(), res.aux.glyphs.fresh());
        res.caches.set_fresh(true);
        res.aux.glyphs.set_fresh(true);
        self.offscreen.set(true);
        {
            let crate::engine::RenderRes { caches, aux, .. } = &mut res;
            match DrawBuf::new(buf, draw_format, stride, rect) {
                Ok(db) => {
                    let mut p = Painter::new(db, caches);
                    p.with_clip(rect, |p| traverse::draw_area(self, p, aux, d, rect));
                }
                Err(e) => {
                    twine_core::error!(target: "twine::refresh", "render_region_to: cannot draw {}: {:?}", rect, e);
                }
            }
        }
        self.offscreen.set(false);
        res.caches.set_fresh(fresh.0);
        res.aux.glyphs.set_fresh(fresh.1);
        res.accel.0 = accel;
        self.res = Some(res);
        Ok(if mono { pack_l8_to_i1(buf, w, h) } else { needed })
    }

    /// Reads the pixels of `area` of `display` back from the panel into `buf` (rows of
    /// `area`, packed, in the display's format and flush coordinates): from the panel's frame
    /// memory through [`DisplayDriver::read_back`](twine_hal::DisplayDriver::read_back), or
    /// from the framebuffer being scanned out for a framebuffer display. The "actual" half of
    /// [`check_region`](Self::check_region).
    ///
    /// Only between frames: nothing may be in flight on the panel's bus.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`]; [`EngineError::Busy`] while a frame is being rendered
    /// or a flush is in flight; [`EngineError::Unsupported`] when the driver cannot read back
    /// (the default) or for a chunked display (its driver is the caller's);
    /// [`EngineError::BufferTooSmall`]; [`EngineError::InvalidConfig`] for an `area` outside
    /// the display; [`EngineError::Driver`] when reading failed.
    ///
    /// ```
    /// use twine_core::{Color, ColorFormat, Instant, Rect};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(16, 16, ColorFormat::Rgb565));
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// e.step(Instant::from_millis(0)); // the (white) screen is drawn
    /// let mut buf = [0u8; 2 * 2 * 2];
    /// e.read_back(d, Rect::from_xywh(0, 0, 2, 2), &mut buf).unwrap();
    /// assert_eq!(buf, [0xFF; 8]);
    /// ```
    pub fn read_back(&mut self, display: DisplayId, area: Rect, buf: &mut [u8]) -> Result<(), EngineError> {
        let d = display.index();
        if d >= self.displays.len() {
            return Err(EngineError::DisplayNotFound(display));
        }
        // Take back finished flushes and swaps first (without waiting).
        self.reclaim_buffers(d);
        self.present_done(d);
        let disp = self
            .displays
            .get_mut(d)
            .ok_or(EngineError::DisplayNotFound(display))?;
        if disp.refresher.job.is_some() || disp.refresher.flush_pending() {
            return Err(EngineError::Busy);
        }
        let phys = physical_area(&disp.info);
        if area.is_empty() || !phys.contains_rect(&area) {
            return Err(EngineError::InvalidConfig("read_back: area outside the display"));
        }
        let needed = region_bytes(disp.info.format, area);
        let Some(out) = buf.get_mut(..needed) else {
            return Err(EngineError::BufferTooSmall {
                needed,
                got: buf.len(),
            });
        };
        match (&mut disp.backend, &disp.refresher.strategy) {
            (Backend::Flush(b), _) => b.read_back(area, out),
            (Backend::Framebuffer(_), Strategy::Full(f)) => {
                let front = f.fb.get(usize::from(f.back ^ 1)).and_then(Option::as_ref);
                copy_out(front, &disp.info, area, out)
            }
            (Backend::Framebuffer(_), Strategy::Direct(st)) => {
                copy_out(st.fb.as_ref(), &disp.info, area, out)
            }
            _ => Err(EngineError::Unsupported(
                "read_back of a chunked display (the caller owns its driver)",
            )),
        }
    }

    /// Checks that the display shows the region of `node` as rendered: renders it
    /// off-screen ([`render_region_to`](Self::render_region_to)), reads the same region back
    /// from the panel ([`read_back`](Self::read_back)) and compares the checksums. A mismatch
    /// raises [`FaultKind::DisplayIntegrity`] (display, node, `code` = the checksum read
    /// back) and is returned as a check that is not [`ok`](IntegrityCheck::ok); what to do
    /// about it is the application's policy. `buf` (at least
    /// [`Region::buffer_bytes`]) holds the expected pixels, then the actual ones.
    ///
    /// Call it between updates, once the region is drawn: a region with areas still waiting
    /// to be drawn is refused ([`EngineError::Busy`]), so a change that is not on the panel yet
    /// is never reported as a mismatch.
    ///
    /// # Errors
    /// As [`render_region_to`](Self::render_region_to) and [`read_back`](Self::read_back);
    /// [`EngineError::Busy`] while the region is dirty; [`EngineError::Unsupported`] for a
    /// software-rotated display (the panel's layout differs from the logical one).
    ///
    /// ```
    /// use twine_core::{ColorFormat, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, Obj};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(32, 16, ColorFormat::Rgb565));
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// let n = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    /// e.set_size(n, 8, 8);
    /// e.step(Instant::from_millis(0));
    /// let mut buf = vec![0u8; e.region(n).unwrap().buffer_bytes];
    /// assert!(e.check_region(n, &mut buf).unwrap().ok());
    /// ```
    pub fn check_region(&mut self, node: NodeId, buf: &mut [u8]) -> Result<IntegrityCheck, EngineError> {
        let region = self.region(node)?;
        let d = region.display.index();
        let disp = self.displays.get(d).ok_or(EngineError::NotVisible(node))?;
        if disp.info.rotation != Rotation::Deg0 && !disp.info.hw_rotation {
            return Err(EngineError::Unsupported(
                "check_region on a software-rotated display",
            ));
        }
        let r = &disp.refresher;
        if r.job.is_some() || r.dirty.iter().any(|a| a.intersection(&region.rect).is_some()) {
            return Err(EngineError::Busy);
        }
        let len = self.render_offscreen(d, region.rect, buf)?;
        let expected = Crc32::of(buf.get(..len).unwrap_or_default());
        self.read_back(region.display, region.rect, buf)?;
        let actual = Crc32::of(buf.get(..len).unwrap_or_default());
        let check = IntegrityCheck {
            node,
            region,
            expected,
            actual,
        };
        if !check.ok() {
            self.integrity_mismatch(&check);
        }
        Ok(check)
    }

    /// Logs and raises the fault of a failed [`check_region`](Self::check_region).
    #[cold]
    #[inline(never)]
    fn integrity_mismatch(&mut self, check: &IntegrityCheck) {
        twine_core::error!(
            target: "twine::integrity",
            "display {} region {} of node {}: expected {}, panel shows {}",
            check.region.display,
            check.region.rect,
            crate::fmt_node_id(check.node),
            check.expected,
            check.actual
        );
        self.raise_fault(
            FaultRecord::new(FaultKind::DisplayIntegrity)
                .display(check.region.display)
                .node(check.node)
                .code(check.actual.value()),
        );
    }

    // ---- Safety-critical nodes -------------------------------------------------------------

    /// The nodes with [`ObjFlags::SAFETY_CRITICAL`], in the order they became critical.
    /// O(1); never panics.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj, ObjFlags};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// e.set_flag(n, ObjFlags::SAFETY_CRITICAL, true);
    /// assert_eq!(e.critical_nodes().collect::<Vec<_>>(), [n]);
    /// e.set_flag(n, ObjFlags::SAFETY_CRITICAL, false);
    /// assert_eq!(e.critical_nodes().count(), 0);
    /// ```
    pub fn critical_nodes(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.critical.iter().map(|c| c.node)
    }

    /// What a safety-critical node shows, for an independent cross-check: its region, the
    /// digest of its widget's value and the frame it was last drawn in ([`CriticalReport`]).
    /// `None` for a node that is not safety-critical. Allocates nothing; O(critical nodes +
    /// depth).
    ///
    /// ```
    /// use twine_core::crc::Crc32;
    /// use twine_engine::{Engine, EngineConfig, Obj, ObjFlags};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// assert!(e.critical_report(n).is_none());
    /// e.set_flag(n, ObjFlags::SAFETY_CRITICAL, true);
    /// let r = e.critical_report(n).unwrap();
    /// assert_eq!((r.region, r.value, r.drawn_frame), (None, Crc32::new(), None)); // on no display
    /// ```
    #[must_use]
    pub fn critical_report(&self, node: NodeId) -> Option<CriticalReport> {
        let entry = self.critical.iter().find(|c| c.node == node)?;
        let mut value = Crc32::new();
        self.tree.node(node)?.widget().value_digest(&mut value);
        Some(CriticalReport {
            node,
            region: self.region(node).ok(),
            value,
            drawn_frame: entry.drawn_frame.get(),
        })
    }

    /// Registers or unregisters `id` as safety-critical after its flag changed (`on`): a node
    /// that becomes critical stops its style transitions and is redrawn whole.
    pub(crate) fn critical_changed(&mut self, id: NodeId, on: bool) {
        if on {
            if !self.critical.iter().any(|c| c.node == id) {
                alloc_guard::push(
                    &mut self.critical,
                    CriticalEntry {
                        node: id,
                        drawn_frame: Cell::new(None),
                    },
                    AllocSite::EngineScratch,
                );
            }
            self.remove_transitions(id, None, None, None);
        } else {
            self.critical.retain(|c| c.node != id);
        }
        self.invalidate(id, InvalidateReason::Explicit);
    }

    /// Forgets deleted nodes (`ids`) in the critical list.
    pub(crate) fn critical_forget(&mut self, ids: &[NodeId]) {
        if !self.critical.is_empty() {
            self.critical.retain(|c| !ids.contains(&c.node));
        }
    }

    /// Records that critical node `id` was drawn on screen (not by an off-screen re-render).
    pub(crate) fn critical_drawn(&self, id: NodeId) {
        if self.offscreen.get() {
            return;
        }
        if let Some(c) = self.critical.iter().find(|c| c.node == id) {
            let frame = self
                .display_of(id)
                .and_then(|d| self.displays.get(d.index()))
                .map(|d| d.refresher.frame);
            c.drawn_frame.set(frame);
        }
    }

    // ---- Fail-safe screen ----------------------------------------------------------------

    /// Registers `screen` (a screen of `display`, created with
    /// [`create_screen`](Self::create_screen) and built at start-up) as the display's
    /// **fail-safe screen**: the static screen [`enter_fail_safe`](Self::enter_fail_safe)
    /// switches to when the application decides the normal UI can no longer be trusted. A
    /// later call replaces the registration (not while the fail-safe state is active).
    /// Allocates nothing.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`]; [`EngineError::InvalidConfig`] when `screen` is not
    /// an inactive screen of `display`, or while the fail-safe state is active.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(32, 16, ColorFormat::Rgb565));
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })).unwrap();
    /// let safe = e.create_screen(d).unwrap();
    /// e.set_fail_safe_screen(d, safe).unwrap();
    /// e.enter_fail_safe(d, 7).unwrap();
    /// assert_eq!((e.active_screen(d), e.fail_safe(d)), (Some(safe), Some(7)));
    /// ```
    pub fn set_fail_safe_screen(&mut self, display: DisplayId, screen: NodeId) -> Result<(), EngineError> {
        let disp = self
            .displays
            .get_mut(display.index())
            .ok_or(EngineError::DisplayNotFound(display))?;
        if disp.fail_safe.active.is_some() {
            return Err(EngineError::InvalidConfig(
                "set_fail_safe_screen: the fail-safe state is active",
            ));
        }
        if !disp.screens.contains(&screen) || disp.active_screen == screen {
            return Err(EngineError::InvalidConfig(
                "set_fail_safe_screen: not an inactive screen of the display",
            ));
        }
        disp.fail_safe.screen = Some(screen);
        Ok(())
    }

    /// Switches `display` to its fail-safe screen ([`set_fail_safe_screen`](Self::set_fail_safe_screen))
    /// at once — a running screen animation is finished first, the top and system layers
    /// (popups, overlays) are hidden — and keeps it there: loading any other screen is refused
    /// (with a warning) until [`leave_fail_safe`](Self::leave_fail_safe). Raises
    /// [`FaultKind::FailSafe`] with `reason` (the application's code) as the record's `code`.
    /// The screen is drawn by the next step. Idempotent (a second call keeps the first reason).
    ///
    /// Allocates nothing, so it works after `Ui::seal`; never panics.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`]; [`EngineError::InvalidConfig`] when no fail-safe
    /// screen is registered (or it was deleted).
    pub fn enter_fail_safe(&mut self, display: DisplayId, reason: u32) -> Result<(), EngineError> {
        let d = display.index();
        let disp = self
            .displays
            .get(d)
            .ok_or(EngineError::DisplayNotFound(display))?;
        if disp.fail_safe.active.is_some() {
            return Ok(());
        }
        let Some(screen) = disp.fail_safe.screen.filter(|s| disp.screens.contains(s)) else {
            return Err(EngineError::InvalidConfig(
                "enter_fail_safe: no fail-safe screen registered",
            ));
        };
        let (top, sys) = (disp.top_layer, disp.sys_layer);
        twine_core::warn!(target: "twine::integrity", "display {}: fail-safe screen (reason {})", display, reason);
        self.finish_screen_anim(display);
        self.load_screen_now(d, screen);
        if let Some(disp) = self.displays.get_mut(d) {
            disp.fail_safe.active = Some(reason);
        }
        self.set_flag(top, ObjFlags::HIDDEN, true);
        self.set_flag(sys, ObjFlags::HIDDEN, true);
        self.raise_fault(
            FaultRecord::new(FaultKind::FailSafe)
                .display(display)
                .code(reason),
        );
        Ok(())
    }

    /// Ends the fail-safe state of `display`: other screens may be loaded again and the top
    /// and system layers are shown again. The fail-safe screen stays active until the
    /// application loads another one. No-op when the display is not in the fail-safe state.
    /// Allocates nothing; never panics.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`].
    pub fn leave_fail_safe(&mut self, display: DisplayId) -> Result<(), EngineError> {
        let disp = self
            .displays
            .get_mut(display.index())
            .ok_or(EngineError::DisplayNotFound(display))?;
        if disp.fail_safe.active.take().is_none() {
            return Ok(());
        }
        let (top, sys) = (disp.top_layer, disp.sys_layer);
        twine_core::info!(target: "twine::integrity", "display {}: fail-safe state left", display);
        self.set_flag(top, ObjFlags::HIDDEN, false);
        self.set_flag(sys, ObjFlags::HIDDEN, false);
        Ok(())
    }

    /// The reason code `display` entered its fail-safe state with (`None`: not in the
    /// fail-safe state, or unknown display).
    #[must_use]
    pub fn fail_safe(&self, display: DisplayId) -> Option<u32> {
        self.displays.get(display.index())?.fail_safe.active
    }

    /// Whether loading `screen` on display `d` is refused by the fail-safe state (logs it).
    pub(crate) fn fail_safe_refuses(&self, d: usize, screen: NodeId) -> bool {
        let Some(disp) = self.displays.get(d) else {
            return false;
        };
        if disp.fail_safe.active.is_none() || disp.fail_safe.screen == Some(screen) {
            return false;
        }
        twine_core::warn!(
            target: "twine::integrity",
            "display {} is in its fail-safe state: loading screen {} refused",
            disp.id,
            crate::fmt_node_id(screen)
        );
        true
    }

    /// Whether `id` is a registered fail-safe screen (it cannot be deleted).
    pub(crate) fn is_fail_safe_screen(&self, id: NodeId) -> bool {
        self.displays.iter().any(|d| d.fail_safe.screen == Some(id))
    }
}

/// Tells observer `o` about a chunk of display `d` (out of line: the refresh path keeps one
/// branch).
#[inline(never)]
fn notify_chunk(
    o: &mut dyn FrameObserver,
    displays: &[crate::display::Display],
    d: usize,
    area: Rect,
    pixels: &[u8],
    stride: Option<usize>,
) {
    let Some(disp) = displays.get(d) else {
        return;
    };
    let format = disp.info.format;
    let stride = stride.unwrap_or_else(|| format.stride(u32::try_from(area.width()).unwrap_or(0)) as usize);
    o.chunk(&FrameChunk::new(disp.id, area, format, pixels, stride));
}

/// Bytes of the packed rows of `rect` in `format` (one byte per pixel for `I1`: rendered as
/// `L8` first).
fn region_bytes(format: ColorFormat, rect: Rect) -> usize {
    let w = u32::try_from(rect.width().max(0)).unwrap_or(0);
    let h = usize::try_from(rect.height().max(0)).unwrap_or(0);
    let row = if format == ColorFormat::I1 {
        w as usize
    } else {
        format.stride(w) as usize
    };
    row.saturating_mul(h)
}

/// The display's area in flush coordinates (physical for software rotation).
fn physical_area(info: &twine_hal::DisplayInfo) -> Rect {
    if info.rotation.swaps_axes() && !info.hw_rotation {
        Rect::new(0, 0, i32::from(info.height), i32::from(info.width))
    } else {
        info.area()
    }
}

/// Copies the rows of `area` out of a full-screen framebuffer `fb` into `out` (packed).
fn copy_out(
    fb: Option<&twine_hal::DrawBufferMem>,
    info: &twine_hal::DisplayInfo,
    area: Rect,
    out: &mut [u8],
) -> Result<(), EngineError> {
    let fb = fb.ok_or(EngineError::Busy)?.as_slice();
    let stride = info.bytes_per_row();
    let bpp = usize::from(info.format.bpp());
    let x0 = usize::try_from(area.x0).unwrap_or(0);
    let row_bytes = info.format.stride(u32::try_from(area.width()).unwrap_or(0)) as usize;
    let first = x0.saturating_mul(bpp) / 8;
    let rows = usize::try_from(area.y0).unwrap_or(0)..usize::try_from(area.y1).unwrap_or(0);
    for (y, dst) in rows.zip(out.chunks_exact_mut(row_bytes.max(1))) {
        let start = y.saturating_mul(stride).saturating_add(first);
        let src = fb
            .get(start..start.saturating_add(row_bytes))
            .ok_or(EngineError::InvalidConfig(
                "read_back: area outside the framebuffer",
            ))?;
        dst.copy_from_slice(src);
    }
    Ok(())
}

/// Packs `w × h` `L8` pixels at the start of `buf` into `I1` rows in place (the engine's mono
/// conversion: `≥ 128` is a set bit, MSB first); returns the packed length. Each output byte
/// lies at or before the first input byte it is made from, so packing front to back never
/// overwrites unread input.
fn pack_l8_to_i1(buf: &mut [u8], w: usize, h: usize) -> usize {
    let out_stride = w.div_ceil(8);
    for r in 0..h {
        for k in 0..out_stride {
            let src = r.saturating_mul(w).saturating_add(k.saturating_mul(8));
            let n = w.saturating_sub(k.saturating_mul(8)).min(8);
            let mut byte = 0u8;
            for (i, l) in buf
                .get(src..src.saturating_add(n))
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                if *l >= 128 {
                    byte |= 0x80u8 >> (i & 7);
                }
            }
            if let Some(o) = buf.get_mut(r.saturating_mul(out_stride).saturating_add(k)) {
                *o = byte;
            }
        }
    }
    out_stride.saturating_mul(h)
}

/// Draws the standard validity look of a node in `State::STALE` / `State::INVALID` over its
/// area (called from the post drawing when one of the states is set): a sparse diagonal hatch
/// for stale values, a dense cross-hatch for invalid ones, in the theme's `WARNING` /
/// `DANGER` colours (`twine_style::design`).
#[cold]
#[inline(never)]
pub(crate) fn draw_validity_mark(engine: &Engine, p: &mut Painter<'_>, id: NodeId, invalid: bool) {
    use twine_style::design::{self, DesignValue};
    let area = engine.coords(id);
    if area.is_empty() {
        return;
    }
    let (element, fallback, spacing, both) = if invalid {
        (design::DANGER, twine_core::Color::new(0xE5, 0x39, 0x35), 6, true)
    } else {
        (
            design::WARNING,
            twine_core::Color::new(0xFB, 0x8C, 0x00),
            10,
            false,
        )
    };
    let color = engine
        .resolve_design_value(id, DesignValue::Element(element))
        .unwrap_or(fallback);
    let dsc = twine_render::LineDsc {
        color,
        opa: if invalid { Opa::P70 } else { Opa::P50 },
        width: 1,
        ..twine_render::LineDsc::default()
    };
    let (w, h) = (area.width(), area.height());
    p.with_clip(area, |p| {
        // Lines x - y = c (and x + y = c), c every `spacing` pixels across the area.
        let mut c = h.saturating_neg();
        while c < w {
            let a = twine_core::Point::new(area.x0.saturating_add(c), area.y0);
            let b = twine_core::Point::new(area.x0.saturating_add(c).saturating_add(h), area.y1);
            p.line(a, b, &dsc);
            if both {
                let a = twine_core::Point::new(area.x1.saturating_sub(c), area.y0);
                let b = twine_core::Point::new(area.x1.saturating_sub(c).saturating_sub(h), area.y1);
                p.line(a, b, &dsc);
            }
            c = c.saturating_add(spacing);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_like_the_mono_conversion() {
        let w: usize = 10;
        let h: usize = 3;
        let src: alloc::vec::Vec<u8> = (0..w * h).map(|i| if i % 3 == 0 { 200 } else { 10 }).collect();
        let mut expected = alloc::vec![0u8; w.div_ceil(8) * h];
        twine_render::convert_l8_to_i1(&src, &mut expected, w, h).unwrap();
        let mut buf = src.clone();
        let n = pack_l8_to_i1(&mut buf, w, h);
        assert_eq!(&buf[..n], &expected[..]);
    }

    #[test]
    fn region_bytes_per_format() {
        let r = Rect::from_xywh(0, 0, 10, 4);
        assert_eq!(region_bytes(ColorFormat::Rgb565, r), 80);
        assert_eq!(region_bytes(ColorFormat::I1, r), 40);
        assert_eq!(region_bytes(ColorFormat::Rgb565, Rect::ZERO), 0);
    }
}
