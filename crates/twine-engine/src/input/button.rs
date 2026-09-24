//! Physical buttons mapped to screen points (LVGL `indev_button_proc`).

use twine_core::{Instant, Point};
use twine_hal::ButtonData;

use super::{InputId, PointerProc, Timing};
use crate::{DisplayId, Engine};

/// The state of a button device: a pointer that presses at the mapped points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ButtonProc {
    pub(crate) ptr: PointerProc,
    /// `points[n]` is where button `n` presses (set with `Engine::set_button_points`).
    pub(crate) points: &'static [Point],
}

impl ButtonProc {
    pub(crate) fn new() -> Self {
        Self {
            ptr: PointerProc::new(),
            points: &[],
        }
    }

    /// One sample: a pressed button presses at its point; a different button first releases
    /// the previous one.
    pub(crate) fn process(
        &mut self,
        e: &mut Engine,
        id: InputId,
        display: DisplayId,
        d: ButtonData,
        now: Instant,
        t: &Timing,
    ) {
        let Some(&p) = self.points.get(usize::from(d.id)) else {
            twine_core::warn!(target: "twine::input", "input {}: button {} has no point (set_button_points)", id, d.id);
            return;
        };
        let ptr = &mut self.ptr;
        e.input_point = Some(p);
        if d.pressed && ptr.pressed && ptr.last_point != p {
            ptr.proc_release(e, id, display, now, t);
            // The previous button is released: the new one starts a new press.
            ptr.pressed = false;
            if e.input_reset_pending(id) {
                return;
            }
        }
        ptr.act_point = p;
        if d.pressed {
            ptr.proc_press(e, id, display, now, t);
        } else {
            ptr.proc_release(e, id, display, now, t);
        }
        ptr.pressed = d.pressed;
        ptr.last_point = ptr.act_point;
    }
}
