//! Framebuffer displays: `Full` (two framebuffers, render into the back buffer, present, sync
//! the areas the next back buffer is missing — LVGL `refr_sync_areas`) and `Direct` (one
//! framebuffer rendered in place).

use core::num::NonZeroUsize;

use twine_core::{ColorFormat, Rect};
use twine_hal::DrawBufferMem;

use super::timeout::PendingPresent;
use super::{MAX_FRAME_AREAS, Slot, Strategy};
use crate::Engine;
use crate::display::Backend;

/// Double framebuffer state.
pub(crate) struct FullState {
    /// Both framebuffers (taken out only while being drawn into or synced).
    pub(crate) fb: [Option<DrawBufferMem>; 2],
    /// The back buffer (rendered next).
    pub(crate) back: Slot,
    /// Areas rendered by the previous frame (in the current front buffer).
    pub(crate) last_areas: heapless::Vec<Rect, MAX_FRAME_AREAS>,
    /// A `present` was issued and has not taken effect yet (since when).
    pub(crate) present: Option<PendingPresent>,
    /// Pixels copied by the last area sync (statistics / tests).
    pub(crate) sync_px: u64,
}

impl FullState {
    pub(crate) fn new(a: DrawBufferMem, b: DrawBufferMem) -> Self {
        Self {
            fb: [Some(a), Some(b)],
            back: Slot::B,
            last_areas: heapless::Vec::new(),
            present: None,
            sync_px: 0,
        }
    }
}

/// Direct mode state.
pub(crate) struct DirectState {
    pub(crate) fb: Option<DrawBufferMem>,
}

impl DirectState {
    pub(crate) fn new(fb: DrawBufferMem) -> Self {
        Self { fb: Some(fb) }
    }
}

/// Copies the rows of `r` from `src` to `dst` (full framebuffers with `stride` bytes per row).
/// `r` lies inside the framebuffers (areas are clipped to the display); a part outside them
/// is not copied.
fn copy_rect(src: &[u8], dst: &mut [u8], stride: usize, format: ColorFormat, r: Rect) {
    let (Some(stride), Ok(x0), Ok(x1), Ok(y0), Ok(y1)) = (
        NonZeroUsize::new(stride),
        usize::try_from(r.x0),
        usize::try_from(r.x1),
        usize::try_from(r.y0),
        usize::try_from(r.y1),
    ) else {
        return;
    };
    let bpp = usize::from(format.bpp());
    let bx0 = x0.saturating_mul(bpp) / 8;
    let bx1 = x1.saturating_mul(bpp).div_ceil(8);
    let rows = src
        .chunks_exact(stride.get())
        .zip(dst.chunks_exact_mut(stride.get()));
    for (s, d) in rows.skip(y0).take(y1.saturating_sub(y0)) {
        if let (Some(s), Some(d)) = (s.get(bx0..bx1), d.get_mut(bx0..bx1)) {
            d.copy_from_slice(s);
        }
    }
}

#[cfg(feature = "display-integrity")]
/// Shows the rendered `area` of a framebuffer to the frame observer (rows of the
/// framebuffer, starting at the area's first pixel).
#[inline(never)]
fn observe_fb_area(engine: &mut Engine, d: usize, info: &twine_hal::DisplayInfo, area: Rect, fb: &[u8]) {
    let stride = info.bytes_per_row();
    let first = usize::try_from(area.y0)
        .unwrap_or(0)
        .saturating_mul(stride)
        .saturating_add(
            usize::try_from(area.x0)
                .unwrap_or(0)
                .saturating_mul(usize::from(info.format.bpp()))
                / 8,
        );
    engine.observe_chunk(d, area, fb.get(first..).unwrap_or_default(), Some(stride));
}

impl Engine {
    /// Area sync at the start of a `Full` frame: every area of the previous frame that the
    /// current frame does not redraw completely is copied from the front into the back buffer.
    pub(crate) fn sync_areas(&mut self, d: usize) {
        let Some(disp) = self.displays.get_mut(d) else {
            return;
        };
        let info = disp.info;
        let r = &mut disp.refresher;
        let Some(job) = r.job.as_ref() else {
            return;
        };
        let Strategy::Full(st) = &mut r.strategy else {
            return;
        };
        st.sync_px = 0;
        let back = st.back;
        let (back_slot, front_slot) = back.split_mut(&mut st.fb);
        let (Some(front_mem), Some(mut back_mem)) = (front_slot.take(), back_slot.take()) else {
            return;
        };
        let stride = info.bytes_per_row();
        for prev in &st.last_areas {
            // prev \ (union of the current areas), in at most 64 pieces.
            let mut pieces: heapless::Vec<Rect, 64> = heapless::Vec::new();
            let _ = pieces.push(*prev);
            let mut overflow = false;
            for cur in &job.areas {
                let mut next: heapless::Vec<Rect, 64> = heapless::Vec::new();
                for p in &pieces {
                    for q in p.subtract(cur) {
                        if next.push(q).is_err() {
                            overflow = true;
                        }
                    }
                }
                if overflow {
                    break;
                }
                pieces = next;
            }
            if overflow {
                // Too fragmented: copy the whole area (correct, just more copying).
                pieces.clear();
                let _ = pieces.push(*prev);
            }
            for p in &pieces {
                copy_rect(
                    front_mem.as_slice(),
                    back_mem.as_mut_slice(),
                    stride,
                    info.format,
                    *p,
                );
                st.sync_px = st.sync_px.saturating_add(p.area());
            }
        }
        *back.other().pick_mut(&mut st.fb) = Some(front_mem);
        *back.pick_mut(&mut st.fb) = Some(back_mem);
        twine_core::trace!(target: "twine::refresh", "area sync: {} px", st.sync_px);
    }

    /// Renders `area` into the back buffer (`Full`) or the framebuffer (`Direct`).
    pub(crate) fn framebuffer_area(&mut self, d: usize, area: Rect) {
        let Some(disp) = self.displays.get_mut(d) else {
            return;
        };
        let info = disp.info;
        let taken = match &mut disp.refresher.strategy {
            Strategy::Full(st) => st.back.pick_mut(&mut st.fb).take(),
            Strategy::Direct(st) => st.fb.take(),
            Strategy::Partial(_) => None,
        };
        let Some(mut mem) = taken else {
            return;
        };
        let render_us = self.render_buffer(
            d,
            mem.as_mut_slice(),
            info.format,
            info.bytes_per_row(),
            info.area(),
            area,
        );
        #[cfg(feature = "display-integrity")]
        if self.observer.is_some() {
            observe_fb_area(self, d, &info, area, mem.as_slice());
        }
        match self.displays.get_mut(d).map(|x| &mut x.refresher.strategy) {
            Some(Strategy::Full(st)) => *st.back.pick_mut(&mut st.fb) = Some(mem),
            Some(Strategy::Direct(st)) => st.fb = Some(mem),
            _ => {}
        }
        if let Some(job) = self.job_mut(d) {
            job.stats.chunks = job.stats.chunks.saturating_add(1);
            job.stats.render_us = job
                .stats
                .render_us
                .saturating_add(render_us.min(u64::from(u32::MAX)) as u32);
        }
    }

    /// Ends a `Full` frame: presents the back buffer and swaps. Returns whether the present
    /// succeeded. On failure nothing was scanned out: the buffers are not swapped, so the
    /// back buffer — which holds the complete, up-to-date frame — is rendered into again, and
    /// the frame's areas are redrawn according to the flush policy.
    pub(crate) fn present_full(&mut self, d: usize) -> bool {
        let since = self.wait_stamp();
        let Some(disp) = self.displays.get_mut(d) else {
            return true;
        };
        let (Strategy::Full(st), Backend::Framebuffer(b)) = (&mut disp.refresher.strategy, &mut disp.backend)
        else {
            return true;
        };
        match b.present(st.back.number()) {
            Ok(()) => {
                st.present = Some(PendingPresent {
                    since,
                    timed_out: false,
                });
                st.back = st.back.other();
                self.flush_ok(d);
                true
            }
            Err(e) => {
                if self.flush_failed(d, e.driver_code().unwrap_or_default()) {
                    self.requeue_job_areas(d, 0);
                }
                false
            }
        }
    }

    /// Ends a `Direct` frame: `present(0)` (a cache clean or no-op for most drivers). On
    /// failure the frame's areas are redrawn according to the flush policy.
    pub(crate) fn present_direct(&mut self, d: usize) {
        let result = match self.displays.get_mut(d).map(|x| &mut x.backend) {
            Some(Backend::Framebuffer(b)) => b.present(0),
            _ => return,
        };
        match result {
            Ok(()) => self.flush_ok(d),
            Err(e) => {
                if self.flush_failed(d, e.driver_code().unwrap_or_default()) {
                    self.requeue_job_areas(d, 0);
                }
            }
        }
    }

    /// Pixels copied by the last area sync of a `Full` display (0 otherwise).
    #[must_use]
    pub fn last_sync_px(&self, display: crate::DisplayId) -> u64 {
        match self.displays.get(display.index()).map(|d| &d.refresher.strategy) {
            Some(Strategy::Full(st)) => st.sync_px,
            _ => 0,
        }
    }
}
