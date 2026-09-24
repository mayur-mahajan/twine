//! Partial draw buffers: one buffer (render, flush, wait) or two with DMA pipelining (render
//! chunk N+1 into one buffer while the driver still transfers chunk N from the other).
//!
//! A buffer handed to the driver with `begin_flush` is out of the engine's hands (its slot is
//! `None`) until `poll_flush` returns it; slots are reclaimed in submission order. The engine
//! therefore can never render into a buffer the driver owns.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::{ColorFormat, Rect, Rotation};
use twine_hal::{DisplayInfo, DrawBufferMem};

use super::areas;
use crate::display::Backend;
use crate::{Engine, EngineError};

/// State of a partial-buffer display.
pub(crate) struct PartialState {
    /// Draw buffers; `None` while lent to the driver (never when rotating: then the scratch
    /// buffers are lent out instead).
    pub(crate) bufs: [Option<DrawBufferMem>; 2],
    /// Rotation targets, one per draw buffer (software rotation only).
    pub(crate) scratch: [Option<DrawBufferMem>; 2],
    /// Number of usable slots (1 or 2).
    pub(crate) count: usize,
    /// Slots whose memory the driver holds, oldest first.
    pub(crate) in_flight: heapless::Deque<u8, 2>,
    /// Rows per chunk.
    pub(crate) rows: i32,
    /// Software rotation (`info.rotation != Deg0 && !info.hw_rotation`).
    pub(crate) rotation: Rotation,
    /// `L8` shadow chunk for `I1` displays (and its rotated copy).
    pub(crate) l8: Vec<u8>,
    pub(crate) l8_rot: Vec<u8>,
    /// Logical render buffer of a rotating external display (the caller's buffer receives the
    /// rotated chunk).
    pub(crate) ext_src: Option<DrawBufferMem>,
}

static WARN_ROWS: AtomicBool = AtomicBool::new(false);

/// Leaks a zeroed, 4-byte aligned buffer of `len` bytes (allocated once per display, like a
/// `'static` MCU buffer).
fn leak_buffer(len: usize) -> DrawBufferMem {
    let v: &'static mut [u8] = Box::leak(vec![0u8; len + 3].into_boxed_slice());
    let off = v.as_ptr().align_offset(4).min(3);
    DrawBufferMem::new(&mut v[off..off + len])
}

impl PartialState {
    /// Validates the buffers (whole rows, 4-byte aligned, at least `align` rows) and allocates
    /// the rotation / mono scratch memory.
    pub(crate) fn new(
        info: &DisplayInfo,
        a: DrawBufferMem,
        b: Option<DrawBufferMem>,
    ) -> Result<Self, EngineError> {
        let row = info.bytes_per_row();
        if row == 0 || info.height == 0 {
            return Err(EngineError::InvalidConfig("display has no pixels"));
        }
        let mut rows = usize::MAX;
        for m in core::iter::once(&a).chain(b.as_ref()) {
            if m.len() < row {
                return Err(EngineError::BufferTooSmall {
                    needed: row,
                    got: m.len(),
                });
            }
            if !m.is_aligned(4) {
                return Err(EngineError::BufferMisaligned);
            }
            if m.len() % row != 0 && !WARN_ROWS.load(Ordering::Relaxed) {
                WARN_ROWS.store(true, Ordering::Relaxed);
                twine_core::warn!(
                    target: "twine::refresh",
                    "draw buffer of {} bytes is not a multiple of a row ({} bytes); the rest is unused",
                    m.len(),
                    row
                );
            }
            rows = rows.min(m.len() / row);
        }
        let rows = rows.min(usize::from(info.height)) as i32;
        let align = i32::from(info.align.max(1));
        if rows < align {
            return Err(EngineError::BufferTooSmall {
                needed: row * align as usize,
                got: a.len(),
            });
        }
        let rows = areas::chunk_rows(rows, info.align);
        let rotation = if info.hw_rotation {
            Rotation::Deg0
        } else {
            info.rotation
        };
        let count = if b.is_some() { 2 } else { 1 };
        let chunk_px = rows as usize * usize::from(info.width);
        let mono = info.format == ColorFormat::I1;
        let mut scratch = [None, None];
        if rotation != Rotation::Deg0 {
            // The rotated chunk is `rows` wide and `width` tall: its packed size can exceed the
            // chunk's for sub-byte formats, so size it for the rotated layout.
            let rot_len = info.format.stride(rows as u32) as usize * usize::from(info.width);
            for s in scratch.iter_mut().take(count) {
                *s = Some(leak_buffer(rot_len.max(a.len())));
            }
        }
        Ok(Self {
            bufs: [Some(a), b],
            scratch,
            count,
            in_flight: heapless::Deque::new(),
            rows,
            rotation,
            l8: if mono { vec![0; chunk_px] } else { Vec::new() },
            l8_rot: if mono && rotation != Rotation::Deg0 {
                vec![0; chunk_px]
            } else {
                Vec::new()
            },
            ext_src: None,
        })
    }

    /// The state of an external display (chunk-level refresh API): no buffers of its own; the
    /// caller's buffers hold `chunk_bytes`. Chunks have as many rows as fit (in the rotated
    /// layout too), rounded to the display alignment.
    pub(crate) fn new_external(info: &DisplayInfo, chunk_bytes: usize) -> Result<Self, EngineError> {
        let row = info.bytes_per_row();
        if row == 0 || info.height == 0 {
            return Err(EngineError::InvalidConfig("display has no pixels"));
        }
        let rotation = if info.hw_rotation {
            Rotation::Deg0
        } else {
            info.rotation
        };
        // Rows such that the chunk fits in its flush layout (rotated sub-byte chunks can need
        // more bytes than unrotated ones).
        let fits = |rows: usize| {
            let rot = if rotation == Rotation::Deg0 {
                0
            } else {
                info.format.stride(rows as u32) as usize * usize::from(info.width)
            };
            rows * row <= chunk_bytes && rot <= chunk_bytes
        };
        let mut rows = (chunk_bytes / row).min(usize::from(info.height));
        while rows > 0 && !fits(rows) {
            rows -= 1;
        }
        let align = i32::from(info.align.max(1));
        if (rows as i32) < align {
            return Err(EngineError::BufferTooSmall {
                needed: row * align as usize,
                got: chunk_bytes,
            });
        }
        let rows = areas::chunk_rows(rows as i32, info.align);
        let chunk_px = rows as usize * usize::from(info.width);
        let mono = info.format == ColorFormat::I1;
        let ext_src = (rotation != Rotation::Deg0 && !mono).then(|| leak_buffer(rows as usize * row));
        Ok(Self {
            bufs: [None, None],
            scratch: [None, None],
            count: 0,
            in_flight: heapless::Deque::new(),
            rows,
            rotation,
            l8: if mono { vec![0; chunk_px] } else { Vec::new() },
            l8_rot: if mono && rotation != Rotation::Deg0 {
                vec![0; chunk_px]
            } else {
                Vec::new()
            },
            ext_src,
        })
    }

    fn rotating(&self) -> bool {
        self.rotation != Rotation::Deg0
    }

    /// A slot whose flush memory the engine holds.
    fn free_slot(&self) -> Option<usize> {
        (0..self.count).find(|&s| {
            if self.rotating() {
                self.scratch[s].is_some()
            } else {
                self.bufs[s].is_some()
            }
        })
    }

    /// Puts a buffer returned by `poll_flush` back into the oldest in-flight slot.
    pub(crate) fn reclaim(&mut self, buf: DrawBufferMem) {
        let rotating = self.rotating();
        match self.in_flight.pop_front() {
            Some(s) => {
                let slot = if rotating {
                    &mut self.scratch[usize::from(s)]
                } else {
                    &mut self.bufs[usize::from(s)]
                };
                debug_assert!(slot.is_none(), "reclaimed buffer into an occupied slot");
                *slot = Some(buf);
            }
            None => {
                twine_core::error!(target: "twine::driver", "poll_flush returned a buffer that was not in flight");
            }
        }
    }
}

impl Engine {
    /// Renders and flushes the chunk of `area` starting at row `y`. Returns the row where the
    /// next chunk starts, or `None` when no buffer was free and the flush is cooperative.
    pub(crate) fn partial_chunk(&mut self, d: usize, area: Rect, y: i32) -> Option<i32> {
        let (rows, rotation) = match &self.displays[d].refresher.strategy {
            super::Strategy::Partial(p) => (p.rows, p.rotation),
            _ => return Some(area.y1),
        };
        let chunk = areas::chunk_at(area, y, rows, 1);
        // 1. A free slot (reclaim or wait).
        let t_wait = self.hires_us();
        let slot = self.acquire_slot(d)?;
        let t_render = self.hires_us();
        #[cfg(feature = "debug-checks")]
        if let Some(h) = self.render_hook {
            h(slot as u8);
        }
        // 2. Render (and rotate / convert).
        let (target, src) = {
            let st = self.partial(d);
            if rotation == Rotation::Deg0 {
                (st.bufs[slot].take(), None)
            } else {
                (st.scratch[slot].take(), st.bufs[slot].take())
            }
        };
        let Some(mut mem) = target else {
            twine_core::error!(target: "twine::refresh", "slot {} has no buffer", slot);
            return Some(chunk.y1);
        };
        let mut src = src;
        let (flush_area, render_us) = self.render_chunk_into(
            d,
            chunk,
            mem.as_mut_slice(),
            src.as_mut().map(DrawBufferMem::as_mut_slice),
        );
        if let Some(src) = src {
            self.partial(d).bufs[slot] = Some(src);
        }
        // 3. Flush.
        let t_flush = self.hires_us();
        let disp = &mut self.displays[d];
        let vsync = disp.refresher.job.as_ref().is_some_and(|j| j.vsync_pending);
        if let Backend::Flush(b) = &mut disp.backend {
            if vsync {
                b.wait_vsync();
            }
            // On error the driver hands the buffer back through the next `poll_flush`.
            let _ = b.begin_flush(flush_area, mem);
        }
        if let super::Strategy::Partial(p) = &mut disp.refresher.strategy {
            let _ = p.in_flight.push_back(slot as u8);
        }
        twine_core::trace!(
            target: "twine::refresh",
            "chunk {} buf={} wait={}us",
            flush_area,
            slot,
            t_render.zip(t_wait).map_or(0, |(a, b)| a.saturating_sub(b))
        );
        // 4. Reclaim a finished buffer without blocking before the next chunk.
        self.reclaim_buffers(d);
        let t_end = self.hires_us();
        let job = self.job_mut(d);
        job.vsync_pending = false;
        job.stats.chunks = job.stats.chunks.saturating_add(1);
        let us = |a: Option<u64>, b: Option<u64>| {
            a.zip(b)
                .map_or(0, |(a, b)| b.saturating_sub(a))
                .min(u64::from(u32::MAX)) as u32
        };
        job.stats.flush_wait_us = job.stats.flush_wait_us.saturating_add(us(t_wait, t_render));
        job.stats.render_us = job
            .stats
            .render_us
            .saturating_add(render_us.min(u64::from(u32::MAX)) as u32);
        let post = us(t_render, t_flush).saturating_sub(render_us.min(u64::from(u32::MAX)) as u32);
        job.stats.flush_us = job
            .stats
            .flush_us
            .saturating_add(post.saturating_add(us(t_flush, t_end)));
        Some(chunk.y1)
    }

    /// Renders the logical `chunk` of display `d` into `target` in the layout the display is
    /// flushed with: rendered directly, or rendered into `src` and rotated into `target`
    /// (software rotation), or rendered as `L8` and converted to `I1` (mono panels). Returns
    /// the flush area (physical for software rotation) and the render time in µs. Shared by the
    /// buffered refresh and the chunk-level API.
    pub(crate) fn render_chunk_into(
        &mut self,
        d: usize,
        chunk: Rect,
        target: &mut [u8],
        src: Option<&mut [u8]>,
    ) -> (Rect, u64) {
        let info = self.displays[d].info;
        let rotation = self.partial(d).rotation;
        let (w, h) = (chunk.width() as usize, chunk.height() as usize);
        let mono = info.format == ColorFormat::I1;
        let rotating = rotation != Rotation::Deg0;
        let phys = if rotating {
            chunk.rotate_in(rotation, i32::from(info.width), i32::from(info.height))
        } else {
            chunk
        };
        let render_us;
        if mono {
            let st = self.partial(d);
            let mut l8 = core::mem::take(&mut st.l8);
            let mut l8_rot = core::mem::take(&mut st.l8_rot);
            render_us = self.render_buffer(d, &mut l8[..w * h], ColorFormat::L8, w, chunk, chunk);
            let (src_l8, pw, ph) = if rotating {
                super::rotate::rotate_chunk(&l8[..w * h], &mut l8_rot[..w * h], w, h, 1, rotation);
                let (pw, ph) = if rotation.swaps_axes() { (h, w) } else { (w, h) };
                (&l8_rot[..w * h], pw, ph)
            } else {
                (&l8[..w * h], w, h)
            };
            if let Err(e) = twine_render::convert_l8_to_i1(src_l8, target, pw, ph) {
                twine_core::error!(target: "twine::refresh", "mono conversion failed: {:?}", e);
            }
            let st = self.partial(d);
            st.l8 = l8;
            st.l8_rot = l8_rot;
        } else if rotating {
            let stride = info.format.stride(w as u32) as usize;
            let Some(src) = src else {
                twine_core::error!(target: "twine::refresh", "rotating display {} has no render buffer", d);
                return (phys, 0);
            };
            render_us = self.render_buffer(d, src, info.format, stride, chunk, chunk);
            let bpp = usize::from(info.format.bpp() / 8);
            super::rotate::rotate_chunk(src, target, w, h, bpp, rotation);
        } else {
            let stride = info.format.stride(w as u32) as usize;
            render_us = self.render_buffer(d, target, info.format, stride, chunk, chunk);
        }
        (phys, render_us)
    }

    fn partial(&mut self, d: usize) -> &mut PartialState {
        match &mut self.displays[d].refresher.strategy {
            super::Strategy::Partial(p) => p,
            _ => unreachable!("partial strategy expected"),
        }
    }

    /// A slot whose memory the engine holds: polls the driver, spinning until a buffer comes
    /// back — or returns `None` in cooperative mode.
    fn acquire_slot(&mut self, d: usize) -> Option<usize> {
        let coop = self.config.cooperative_flush;
        loop {
            if let Some(s) = self.partial(d).free_slot() {
                return Some(s);
            }
            let disp = &mut self.displays[d];
            let (super::Strategy::Partial(p), Backend::Flush(b)) =
                (&mut disp.refresher.strategy, &mut disp.backend)
            else {
                return None;
            };
            if p.in_flight.is_empty() {
                twine_core::error!(target: "twine::refresh", "no draw buffer free and none in flight");
                return None;
            }
            match b.poll_flush() {
                Some(buf) => p.reclaim(buf),
                None if coop => return None,
                None => core::hint::spin_loop(),
            }
        }
    }
}
