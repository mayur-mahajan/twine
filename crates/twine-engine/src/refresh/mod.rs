//! The refresh pipeline: per display a [`Refresher`] collects dirty areas and, at most once per
//! `refr_period`, renders them — merged, rounded to the display alignment and split into
//! buffer-sized chunks — through one of the buffer strategies (partial single/double with DMA
//! pipelining, full double framebuffer with area sync, direct).

mod areas;
mod framebuffer;
mod partial;
mod rotate;
pub(crate) mod traverse;

use alloc::boxed::Box;

use twine_core::{Color, ColorFormat, Duration, Instant, Opa, Rect, RectSet};
use twine_hal::{DisplayInfo, DrawBufferMem};

use twine_render::{DrawBuf, Painter};

use crate::display::Backend;
use crate::{Engine, EngineError, RefreshStats, Wake};

pub(crate) use framebuffer::{DirectState, FullState};
pub(crate) use partial::PartialState;

/// Maximum areas of one frame: the dirty areas plus the performance overlay's area.
const MAX_FRAME_AREAS: usize = 34;

/// How a display's frames are rendered and flushed.
pub(crate) enum Strategy {
    Partial(PartialState),
    Full(Box<FullState>),
    Direct(DirectState),
}

/// A frame in progress: its areas and how far rendering got.
pub(crate) struct FrameJob {
    areas: heapless::Vec<Rect, MAX_FRAME_AREAS>,
    /// `areas[..counted]` count in the statistics (the rest is the performance overlay).
    counted: usize,
    idx: usize,
    next_y: i32,
    vsync_pending: bool,
    started_us: Option<u64>,
    stats: RefreshStats,
    /// Time spent on uncounted areas (subtracted from the frame time).
    excluded_us: u64,
    /// When the frame started (engine time).
    now: Instant,
}

/// Per-display refresh state.
pub(crate) struct Refresher {
    pub(crate) dirty: RectSet<32>,
    pub(crate) last_refresh: Option<Instant>,
    pub(crate) strategy: Strategy,
    pub(crate) frame: u32,
    pub(crate) job: Option<FrameJob>,
    pub(crate) vsync: bool,
    /// Rendered or flushed since the driver was last told it is idle.
    pub(crate) busy: bool,
    pub(crate) stats: RefreshStats,
    /// The performance overlay's own dirty area (kept out of the statistics).
    pub(crate) overlay_dirty: Option<Rect>,
    /// Alignment of every rendered area (display alignment; 8 for I1 framebuffers).
    pub(crate) align: u8,
}

/// A frame started by [`Engine::refresh_begin`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    /// The display the frame is for.
    pub display: crate::DisplayId,
    /// The display's frame counter.
    pub frame: u32,
    /// Number of (merged, aligned) areas to render.
    pub areas: u16,
    /// Pixels of the areas.
    pub px: u32,
}

/// Result of [`Engine::refresh`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshOutcome {
    /// At least one frame was rendered (completely or partly).
    pub rendered: bool,
    /// Earliest instant a display needs another refresh (pending areas waiting for
    /// `refr_period`, or a framebuffer swap to wait for).
    pub next_due: Option<Instant>,
    /// A frame is only partly rendered (cooperative flush): call again as soon as a draw
    /// buffer can be free.
    pub in_progress: bool,
}

impl Refresher {
    fn new(info: &DisplayInfo, strategy: Strategy, align: u8) -> Self {
        Self {
            dirty: RectSet::new(info.area()),
            last_refresh: None,
            strategy,
            frame: 0,
            job: None,
            vsync: false,
            busy: false,
            stats: RefreshStats::default(),
            overlay_dirty: None,
            align,
        }
    }

    /// A refresher over one or two partial draw buffers (validated).
    pub(crate) fn new_partial(
        info: &DisplayInfo,
        a: DrawBufferMem,
        b: Option<DrawBufferMem>,
    ) -> Result<Self, EngineError> {
        let st = PartialState::new(info, a, b)?;
        Ok(Self::new(info, Strategy::Partial(st), info.align.max(1)))
    }

    /// A refresher for an external display (chunk-level refresh API).
    pub(crate) fn new_external(info: &DisplayInfo, chunk_bytes: usize) -> Result<Self, EngineError> {
        let st = PartialState::new_external(info, chunk_bytes)?;
        Ok(Self::new(info, Strategy::Partial(st), info.align.max(1)))
    }

    /// A refresher over a display's framebuffers (`full`: two, double buffered).
    pub(crate) fn new_framebuffer(
        info: &DisplayInfo,
        fbs: (DrawBufferMem, Option<DrawBufferMem>),
        full: bool,
    ) -> Result<Self, EngineError> {
        let need = info.bytes_per_row() * usize::from(info.height);
        let check = |m: &DrawBufferMem| {
            if m.len() == need {
                Ok(())
            } else {
                Err(EngineError::BufferTooSmall {
                    needed: need,
                    got: m.len(),
                })
            }
        };
        check(&fbs.0)?;
        let align = if info.format == ColorFormat::I1 {
            info.align.max(8)
        } else {
            info.align.max(1)
        };
        let strategy = if full {
            let b = fbs.1.ok_or(EngineError::BufferModeMismatch)?;
            check(&b)?;
            Strategy::Full(Box::new(FullState::new(fbs.0, b)))
        } else {
            Strategy::Direct(DirectState::new(fbs.0))
        };
        Ok(Self::new(info, strategy, align))
    }

    /// Adds a dirty area; more than `max` areas collapse into their bounding box.
    pub(crate) fn add_dirty(&mut self, area: Rect, max: usize) {
        self.dirty.add(area);
        if self.dirty.len() > max {
            let bbox = self.dirty.bounding_box();
            self.dirty.clear();
            self.dirty.add(bbox);
        }
    }

    /// Framebuffer `index` (framebuffer strategies).
    pub(crate) fn framebuffer(&self, index: u8) -> Option<&[u8]> {
        match &self.strategy {
            Strategy::Full(f) => f.fb[usize::from(index & 1)].as_ref().map(DrawBufferMem::as_slice),
            Strategy::Direct(d) if index == 0 => d.fb.as_ref().map(DrawBufferMem::as_slice),
            _ => None,
        }
    }

    /// Whether the driver still holds a buffer or a present is not done.
    pub(crate) fn flush_pending(&self) -> bool {
        match &self.strategy {
            Strategy::Partial(p) => !p.in_flight.is_empty(),
            Strategy::Full(f) => f.present_pending,
            Strategy::Direct(_) => false,
        }
    }
}

/// Renders `clip` of display `d` into `buf` (pixels of `buf_area` in `format`, rows `stride`
/// bytes apart), tinting it for the refresh debug overlay of `frame` when enabled.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_into(
    engine: &Engine,
    res: &mut crate::engine::RenderRes,
    d: usize,
    buf: &mut [u8],
    format: ColorFormat,
    stride: usize,
    buf_area: Rect,
    clip: Rect,
    frame: u32,
) {
    let crate::engine::RenderRes { caches, aux } = res;
    let db = match DrawBuf::new(buf, format, stride, buf_area) {
        Ok(b) => b,
        Err(e) => {
            twine_core::error!(target: "twine::refresh", "cannot draw {} into buffer: {:?}", buf_area, e);
            return;
        }
    };
    let mut p = Painter::new(db, caches);
    p.with_clip(clip, |p| {
        traverse::draw_area(engine, p, aux, d, clip);
        if engine.config.debug_refresh {
            p.fill(clip, debug_color(frame), Opa::P30);
        }
    });
}

/// The refresh-debug tint of `frame`: hue `frame × 37 mod 360` at full saturation.
fn debug_color(frame: u32) -> Color {
    let h = (frame.wrapping_mul(37) % 360) as i32;
    let x = (255 * (60 - (h % 120 - 60).abs()) / 60) as u8;
    match h / 60 {
        0 => Color::new(255, x, 0),
        1 => Color::new(x, 255, 0),
        2 => Color::new(0, 255, x),
        3 => Color::new(0, x, 255),
        4 => Color::new(x, 0, 255),
        _ => Color::new(255, 0, x),
    }
}

/// What one display's refresh did.
enum DisplayOutcome {
    Idle,
    Due(Instant),
    Rendered,
    InProgress,
}

impl Engine {
    /// The high-resolution time in µs (`None` without [`EngineConfig::hires_timer`](crate::EngineConfig::hires_timer)).
    pub(crate) fn hires_us(&self) -> Option<u64> {
        self.config.hires_timer.map(|f| f().as_micros())
    }

    /// Renders and flushes every display whose dirty areas are due at `now` (at most one frame
    /// per `refr_period` per display). Called by [`step`](Self::step).
    pub fn refresh(&mut self, now: Instant) -> RefreshOutcome {
        let mut out = RefreshOutcome::default();
        for d in 0..self.displays.len() {
            match self.refresh_display(d, now) {
                DisplayOutcome::Idle => {}
                DisplayOutcome::Due(t) => out.next_due = Some(out.next_due.map_or(t, |n| n.min(t))),
                DisplayOutcome::Rendered => out.rendered = true,
                DisplayOutcome::InProgress => {
                    out.rendered = true;
                    out.in_progress = true;
                }
            }
        }
        out
    }

    /// One step of the engine (the update cycle): [`read_inputs`](Self::read_inputs),
    /// [`run_timers`](Self::run_timers), [`run_anims`](Self::run_anims), the layout pass
    /// ([`update_layout`](Self::update_layout), only dirty subtrees), then
    /// [`refresh`](Self::refresh) every display that is due. Returns when to call again: the
    /// earliest of the next frame, the next animation frame (at most one per `refr_period`
    /// while an animation plays), the end of an animation delay, the next due timer, the next
    /// input read (polled devices, held devices, long press timers) and pending flushes;
    /// [`Wake::Idle`] when nothing is pending — an idle UI costs no CPU at all.
    pub fn step(&mut self, now: Instant) -> Wake {
        self.begin_step(now);
        self.read_inputs(now);
        self.run_timers(now);
        self.run_anims(now);
        self.finish_step(now)
    }

    /// Starts an update at `now` (the first part of [`step`](Self::step), for callers that run
    /// the steps one by one — the view layer's `Ui::update`): records the time used by
    /// animations and timers started before the next animation step.
    pub fn begin_step(&mut self, now: Instant) {
        self.set_anim_now(now);
    }

    /// Finishes an update (the last part of [`step`](Self::step), after
    /// [`read_inputs`](Self::read_inputs), [`run_timers`](Self::run_timers) and
    /// [`run_anims`](Self::run_anims)): the layout pass, the refresh of every due display and
    /// the performance bookkeeping. Returns when to call again (see [`step`](Self::step)).
    pub fn finish_step(&mut self, now: Instant) -> Wake {
        self.update_layout();
        let out = self.refresh(now);
        self.after_refresh(now, out.rendered);
        // The performance overlay may have changed its size: lay it out for the next frame.
        self.update_layout();
        if out.in_progress {
            return Wake::Now;
        }
        if self.config.cooperative_flush && self.flush_pending() {
            return Wake::Now;
        }
        let input = self.input_deadline().map_or(Wake::Idle, Wake::At);
        out.next_due
            .map_or(Wake::Idle, Wake::At)
            .min(input)
            .min(self.anim_wake(now))
    }

    /// Whether a display driver still holds a draw buffer or a framebuffer swap is pending.
    #[must_use]
    pub fn flush_pending(&self) -> bool {
        self.displays.iter().any(|d| d.refresher.flush_pending())
    }

    /// Statistics of the last frame of `display` (default values for unknown displays).
    #[must_use]
    pub fn last_stats(&self, display: crate::DisplayId) -> RefreshStats {
        self.displays
            .get(display.index())
            .map_or_else(RefreshStats::default, |d| d.refresher.stats)
    }

    fn refresh_display(&mut self, d: usize, now: Instant) -> DisplayOutcome {
        if matches!(self.displays[d].backend, Backend::External(())) {
            // Rendered by the caller through `refresh_begin` / `render_chunk`: only report when
            // the next frame is due (a frame due now is rendered right after this update).
            let r = &self.displays[d].refresher;
            if r.job.is_none() && (!r.dirty.is_empty() || r.overlay_dirty.is_some()) {
                if let Some(last) = r.last_refresh {
                    let due = last + self.config.refr_period;
                    if now < due {
                        return DisplayOutcome::Due(due);
                    }
                }
            }
            return DisplayOutcome::Idle;
        }
        self.reclaim_buffers(d);
        if self.displays[d].refresher.job.is_none() {
            let r = &mut self.displays[d].refresher;
            if r.dirty.is_empty() && r.overlay_dirty.is_none() {
                if r.busy && !r.flush_pending() {
                    r.busy = false;
                    twine_core::debug!(target: "twine::refresh", "display {} idle", self.displays[d].id);
                    if let Backend::Flush(b) = &mut self.displays[d].backend {
                        b.idle();
                    }
                }
                return DisplayOutcome::Idle;
            }
            if let Some(last) = r.last_refresh {
                let due = last + self.config.refr_period;
                if now < due {
                    return DisplayOutcome::Due(due);
                }
            }
            if !self.present_done(d) {
                return if self.config.cooperative_flush {
                    DisplayOutcome::InProgress
                } else {
                    DisplayOutcome::Due(now + Duration::ms(1))
                };
            }
            self.start_job(d, now);
        }
        if self.run_job(d) {
            DisplayOutcome::Rendered
        } else {
            DisplayOutcome::InProgress
        }
    }

    /// Takes back finished buffers without blocking.
    fn reclaim_buffers(&mut self, d: usize) {
        let disp = &mut self.displays[d];
        if let (Strategy::Partial(p), Backend::Flush(b)) = (&mut disp.refresher.strategy, &mut disp.backend) {
            while !p.in_flight.is_empty() {
                match b.poll_flush() {
                    Some(buf) => p.reclaim(buf),
                    None => break,
                }
            }
        }
    }

    /// Whether a framebuffer swap requested by the previous frame has happened (always true for
    /// other strategies).
    fn present_done(&mut self, d: usize) -> bool {
        let disp = &mut self.displays[d];
        match (&mut disp.refresher.strategy, &mut disp.backend) {
            (Strategy::Full(f), Backend::Framebuffer(b)) => {
                if f.present_pending && b.present_done() {
                    f.present_pending = false;
                }
                !f.present_pending
            }
            _ => true,
        }
    }

    fn start_job(&mut self, d: usize, now: Instant) {
        let started_us = self.hires_us();
        let bounds = self.displays[d].area();
        let r = &mut self.displays[d].refresher;
        r.dirty.merge();
        let mut areas: heapless::Vec<Rect, MAX_FRAME_AREAS> = heapless::Vec::new();
        let mut px = 0u64;
        for a in r.dirty.iter() {
            let a = areas::round_area(*a, r.align, bounds);
            if !a.is_empty() && areas.push(a).is_ok() {
                px += a.area();
            }
        }
        let counted = areas.len();
        if let Some(o) = r.overlay_dirty.take() {
            let o = areas::round_area(o, r.align, bounds);
            if !o.is_empty() {
                let _ = areas.push(o);
            }
        }
        r.dirty.clear();
        r.frame = r.frame.wrapping_add(1);
        r.last_refresh = Some(now);
        r.busy = true;
        let stats = RefreshStats {
            frame: r.frame,
            dirty_areas: counted as u16,
            dirty_px: px.min(u64::from(u32::MAX)) as u32,
            ..RefreshStats::default()
        };
        let first_y = areas.first().map_or(0, |a| a.y0);
        let vsync = r.vsync;
        r.job = Some(FrameJob {
            areas,
            counted,
            idx: 0,
            next_y: first_y,
            vsync_pending: vsync,
            started_us,
            stats,
            excluded_us: 0,
            now,
        });
        #[cfg(feature = "debug-checks")]
        {
            core::mem::swap(&mut self.invalidations, &mut self.frame_invalidations);
            self.invalidations.clear();
        }
        self.nodes_drawn.set(0);
        if matches!(self.displays[d].refresher.strategy, Strategy::Full(_)) {
            self.sync_areas(d);
        }
    }

    /// Continues the frame of display `d`. Returns `true` when the frame is complete, `false`
    /// when it had to stop because no draw buffer was free (cooperative mode).
    fn run_job(&mut self, d: usize) -> bool {
        while let Some((area, y, counted)) = self.job_next(d) {
            let t0 = self.hires_us();
            let next_y = match self.displays[d].refresher.strategy {
                Strategy::Partial(_) => match self.partial_chunk(d, area, y) {
                    Some(next_y) => next_y,
                    None => return false,
                },
                Strategy::Full(_) | Strategy::Direct(_) => {
                    self.framebuffer_area(d, area);
                    area.y1
                }
            };
            if !counted {
                if let (Some(a), Some(b)) = (t0, self.hires_us()) {
                    self.job_mut(d).excluded_us += b.saturating_sub(a);
                }
            }
            self.job_advance(d, next_y);
        }
        self.finish_job(d);
        true
    }

    /// The next piece of the frame of display `d`: `(area, first row, counted in the
    /// statistics)`, or `None` when the frame is complete (or no frame runs).
    fn job_next(&self, d: usize) -> Option<(Rect, i32, bool)> {
        let job = self.displays[d].refresher.job.as_ref()?;
        job.areas
            .get(job.idx)
            .map(|a| (*a, job.next_y, job.idx < job.counted))
    }

    /// Records that the frame of display `d` is rendered up to row `next_y` of its current
    /// area (moving to the next area when that one is done).
    fn job_advance(&mut self, d: usize, next_y: i32) {
        let job = self.job_mut(d);
        let Some(area) = job.areas.get(job.idx).copied() else {
            return;
        };
        job.next_y = next_y;
        if next_y >= area.y1 {
            job.idx += 1;
            if let Some(next) = job.areas.get(job.idx) {
                job.next_y = next.y0;
            }
        }
    }

    /// Starts the next frame of a display added with
    /// [`add_chunked_display`](Self::add_chunked_display), if one is due at `now` (dirty areas
    /// and at least `refr_period` since the previous frame). Render it with
    /// [`render_chunk`](Self::render_chunk) until that returns `None`, then call
    /// [`refresh_end`](Self::refresh_end). Call it after the update step (`Engine::step` /
    /// `finish_step`), which lays out and reports when the next frame is due.
    ///
    /// The buffered refresh of [`add_display`](Self::add_display) displays runs through the same
    /// frame job and chunk renderer, so both paths produce identical pixels.
    pub fn refresh_begin(&mut self, now: Instant) -> Option<FrameInfo> {
        if let Some(d) = self.chunk_display {
            twine_core::warn!(target: "twine::refresh", "refresh_begin: frame of display {} still open", d);
            return None;
        }
        let d = (0..self.displays.len()).find(|&d| {
            let disp = &self.displays[d];
            let r = &disp.refresher;
            matches!(disp.backend, Backend::External(()))
                && r.job.is_none()
                && (!r.dirty.is_empty() || r.overlay_dirty.is_some())
                && r.last_refresh
                    .is_none_or(|last| now >= last + self.config.refr_period)
        })?;
        self.start_job(d, now);
        self.chunk_display = Some(d);
        let job = self.displays[d].refresher.job.as_ref()?;
        let info = FrameInfo {
            display: self.displays[d].id,
            frame: job.stats.frame,
            areas: job.stats.dirty_areas,
            px: job.stats.dirty_px,
        };
        twine_core::trace!(target: "twine::refresh", "frame {} begins: {} areas", info.frame, info.areas);
        Some(info)
    }

    /// Renders the next chunk of the frame started by [`refresh_begin`](Self::refresh_begin)
    /// into `buf` (at least the display's `chunk_bytes`), in the layout to flush: rows of the
    /// returned area, rotated and format-converted as the display needs. Returns the area to
    /// flush, or `None` when the frame is complete.
    pub fn render_chunk(&mut self, buf: &mut [u8]) -> Option<Rect> {
        let d = self.chunk_display?;
        let (area, y, counted) = self.job_next(d)?;
        let rows = match &self.displays[d].refresher.strategy {
            Strategy::Partial(p) => p.rows,
            _ => return None,
        };
        let chunk = areas::chunk_at(area, y, rows, 1);
        #[cfg(feature = "debug-checks")]
        if let Some(h) = self.render_hook {
            h(0);
        }
        let t0 = self.hires_us();
        let mut src = match &mut self.displays[d].refresher.strategy {
            Strategy::Partial(p) => p.ext_src.take(),
            _ => None,
        };
        let (flush_area, render_us) =
            self.render_chunk_into(d, chunk, buf, src.as_mut().map(DrawBufferMem::as_mut_slice));
        if let Strategy::Partial(p) = &mut self.displays[d].refresher.strategy {
            p.ext_src = src;
        }
        let t1 = self.hires_us();
        let job = self.job_mut(d);
        job.stats.chunks = job.stats.chunks.saturating_add(1);
        job.stats.render_us = job
            .stats
            .render_us
            .saturating_add(render_us.min(u64::from(u32::MAX)) as u32);
        if !counted {
            if let (Some(a), Some(b)) = (t0, t1) {
                job.excluded_us += b.saturating_sub(a);
            }
        }
        self.job_advance(d, chunk.y1);
        twine_core::trace!(target: "twine::refresh", "chunk {} rendered", flush_area);
        Some(flush_area)
    }

    /// Adds the caller's measured transfer times to the statistics of the frame started by
    /// [`refresh_begin`](Self::refresh_begin): `flush_us` of transfer, of which `wait_us` the CPU
    /// only waited (the rest overlapped rendering).
    pub fn refresh_add_flush_time(&mut self, flush_us: u32, wait_us: u32) {
        let Some(d) = self.chunk_display else {
            return;
        };
        if let Some(job) = self.displays[d].refresher.job.as_mut() {
            job.stats.flush_us = job.stats.flush_us.saturating_add(flush_us);
            job.stats.flush_wait_us = job.stats.flush_wait_us.saturating_add(wait_us);
        }
    }

    /// Ends the frame started by [`refresh_begin`](Self::refresh_begin): statistics, the
    /// performance monitor and the `twine::refresh` log line. Chunks not rendered are dropped
    /// (their areas are redrawn by the next frame).
    pub fn refresh_end(&mut self) {
        let Some(d) = self.chunk_display.take() else {
            return;
        };
        if let Some(job) = self.displays[d].refresher.job.as_ref() {
            if job.idx < job.areas.len() {
                twine_core::warn!(target: "twine::refresh", "refresh_end: frame of display {} not complete", d);
                for a in job
                    .areas
                    .iter()
                    .skip(job.idx)
                    .copied()
                    .collect::<heapless::Vec<Rect, MAX_FRAME_AREAS>>()
                {
                    self.displays[d].refresher.dirty.add(a);
                }
            }
        }
        self.finish_job(d);
        self.displays[d].refresher.busy = false;
    }

    fn job_mut(&mut self, d: usize) -> &mut FrameJob {
        self.displays[d]
            .refresher
            .job
            .as_mut()
            .expect("frame job in progress")
    }

    fn finish_job(&mut self, d: usize) {
        match self.displays[d].refresher.strategy {
            Strategy::Full(_) => self.present_full(d),
            Strategy::Direct(_) => self.present_direct(d),
            Strategy::Partial(_) => {}
        }
        let end = self.hires_us();
        let Some(job) = self.displays[d].refresher.job.take() else {
            return;
        };
        let mut s = job.stats;
        if let (Some(a), Some(b)) = (job.started_us, end) {
            s.frame_time_us = b
                .saturating_sub(a)
                .saturating_sub(job.excluded_us)
                .min(u64::from(u32::MAX)) as u32;
        }
        s.nodes_drawn = self.nodes_drawn.get();
        if let Some(f) = self.config.mem_info {
            let m = f();
            s.mem_used = m.used;
            s.mem_peak = m.peak;
        }
        let r = &mut self.displays[d].refresher;
        if let Strategy::Full(f) = &mut r.strategy {
            f.last_areas.clear();
            for a in &job.areas {
                let _ = f.last_areas.push(*a);
            }
        }
        s.fps = self.perf.fps();
        s.cpu_percent = self.perf.cpu_percent();
        r.stats = s;
        twine_core::debug!(
            target: "twine::refresh",
            "frame {} areas={} px={} chunks={} nodes={} render={}us flush={}us wait={}us",
            s.frame,
            s.dirty_areas,
            s.dirty_px,
            s.chunks,
            s.nodes_drawn,
            s.render_us,
            s.flush_us,
            s.flush_wait_us
        );
        let busy = u64::from(s.render_us) + u64::from(s.flush_wait_us);
        self.perf.frame(job.now, busy);
    }

    /// Draws `clip` of display `d` into a buffer taken out of the engine (so drawing can read
    /// the whole engine). Returns the render time in µs.
    pub(crate) fn render_buffer(
        &mut self,
        d: usize,
        buf: &mut [u8],
        format: ColorFormat,
        stride: usize,
        buf_area: Rect,
        clip: Rect,
    ) -> u64 {
        let Some(mut res) = self.res.take() else {
            twine_core::error!(target: "twine::refresh", "render resources missing (re-entrant refresh?)");
            return 0;
        };
        let t0 = self.hires_us();
        let frame = self.displays[d].refresher.frame;
        render_into(self, &mut res, d, buf, format, stride, buf_area, clip, frame);
        let t1 = self.hires_us();
        self.res = Some(res);
        match (t0, t1) {
            (Some(a), Some(b)) => b.saturating_sub(a),
            _ => 0,
        }
    }
}
