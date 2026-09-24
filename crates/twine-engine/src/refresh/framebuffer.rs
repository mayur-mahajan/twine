//! Framebuffer displays: `Full` (two framebuffers, render into the back buffer, present, sync
//! the areas the next back buffer is missing — LVGL `refr_sync_areas`) and `Direct` (one
//! framebuffer rendered in place).

use twine_core::{ColorFormat, Rect};
use twine_hal::DrawBufferMem;

use super::{MAX_FRAME_AREAS, Strategy};
use crate::Engine;
use crate::display::Backend;

/// Double framebuffer state.
pub(crate) struct FullState {
    /// Both framebuffers (taken out only while being drawn into or synced).
    pub(crate) fb: [Option<DrawBufferMem>; 2],
    /// Index of the back buffer (rendered next).
    pub(crate) back: u8,
    /// Areas rendered by the previous frame (in the current front buffer).
    pub(crate) last_areas: heapless::Vec<Rect, MAX_FRAME_AREAS>,
    /// A `present` was issued and has not taken effect yet.
    pub(crate) present_pending: bool,
    /// Pixels copied by the last area sync (statistics / tests).
    pub(crate) sync_px: u64,
}

impl FullState {
    pub(crate) fn new(a: DrawBufferMem, b: DrawBufferMem) -> Self {
        Self {
            fb: [Some(a), Some(b)],
            back: 1,
            last_areas: heapless::Vec::new(),
            present_pending: false,
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
fn copy_rect(src: &[u8], dst: &mut [u8], stride: usize, format: ColorFormat, r: Rect) {
    let bpp = usize::from(format.bpp());
    let bx0 = r.x0 as usize * bpp / 8;
    let bx1 = (r.x1 as usize * bpp).div_ceil(8);
    for y in r.y0..r.y1 {
        let o = y as usize * stride;
        dst[o + bx0..o + bx1].copy_from_slice(&src[o + bx0..o + bx1]);
    }
}

impl Engine {
    /// Area sync at the start of a `Full` frame: every area of the previous frame that the
    /// current frame does not redraw completely is copied from the front into the back buffer.
    pub(crate) fn sync_areas(&mut self, d: usize) {
        let info = self.displays[d].info;
        let r = &mut self.displays[d].refresher;
        let Some(job) = r.job.as_ref() else {
            return;
        };
        let Strategy::Full(st) = &mut r.strategy else {
            return;
        };
        st.sync_px = 0;
        let back = usize::from(st.back);
        let (Some(front_mem), Some(mut back_mem)) = (st.fb[1 - back].take(), st.fb[back].take()) else {
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
                st.sync_px += p.area();
            }
        }
        st.fb[1 - back] = Some(front_mem);
        st.fb[back] = Some(back_mem);
        twine_core::trace!(target: "twine::refresh", "area sync: {} px", st.sync_px);
    }

    /// Renders `area` into the back buffer (`Full`) or the framebuffer (`Direct`).
    pub(crate) fn framebuffer_area(&mut self, d: usize, area: Rect) {
        let info = self.displays[d].info;
        let taken = match &mut self.displays[d].refresher.strategy {
            Strategy::Full(st) => st.fb[usize::from(st.back)].take(),
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
        match &mut self.displays[d].refresher.strategy {
            Strategy::Full(st) => st.fb[usize::from(st.back)] = Some(mem),
            Strategy::Direct(st) => st.fb = Some(mem),
            Strategy::Partial(_) => {}
        }
        let job = self.job_mut(d);
        job.stats.chunks = job.stats.chunks.saturating_add(1);
        job.stats.render_us = job
            .stats
            .render_us
            .saturating_add(render_us.min(u64::from(u32::MAX)) as u32);
    }

    /// Ends a `Full` frame: presents the back buffer and swaps.
    pub(crate) fn present_full(&mut self, d: usize) {
        let disp = &mut self.displays[d];
        let (Strategy::Full(st), Backend::Framebuffer(b)) = (&mut disp.refresher.strategy, &mut disp.backend)
        else {
            return;
        };
        let _ = b.present(st.back);
        st.present_pending = true;
        st.back ^= 1;
    }

    /// Ends a `Direct` frame: `present(0)` (a cache clean or no-op for most drivers).
    pub(crate) fn present_direct(&mut self, d: usize) {
        if let Backend::Framebuffer(b) = &mut self.displays[d].backend {
            let _ = b.present(0);
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
