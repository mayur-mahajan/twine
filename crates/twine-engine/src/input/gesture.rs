//! Swipe gestures (LVGL `indev_gesture`).

use twine_style::Dir;

use super::{InputId, PointerProc, Timing};
use crate::{Engine, EventCode, EventParam, ObjFlags};

impl PointerProc {
    /// Accumulates the movement of a press into a gesture and sends `Gesture` once per press
    /// when it exceeds `gesture_limit` (reads slower than `gesture_min_velocity` start the sum
    /// again). The target is the pressed node, or its first ancestor without `GESTURE_BUBBLE`.
    /// Returns whether processing may continue.
    pub(crate) fn gesture(&mut self, e: &mut Engine, id: InputId, t: &Timing) -> bool {
        if self.gesture_sent {
            return true;
        }
        let mut target = self.act_obj;
        while let Some(n) = target.filter(|n| e.has_flag(*n, ObjFlags::GESTURE_BUBBLE)) {
            target = e.tree.parent(n);
        }
        let Some(target) = target else {
            return true;
        };
        if self.vect.x.abs() < t.gesture_min_velocity && self.vect.y.abs() < t.gesture_min_velocity {
            self.gesture_sum = twine_core::Point::ZERO;
        }
        self.gesture_sum += self.vect;
        let s = self.gesture_sum;
        if s.x.abs() <= t.gesture_limit && s.y.abs() <= t.gesture_limit {
            return true;
        }
        self.gesture_sent = true;
        let dir = if s.x.abs() > s.y.abs() {
            if s.x > 0 { Dir::RIGHT } else { Dir::LEFT }
        } else if s.y > 0 {
            Dir::BOTTOM
        } else {
            Dir::TOP
        };
        self.gesture_dir = Some(dir);
        twine_core::debug!(target: "twine::input", "{} gesture {:?}", id, dir);
        e.input_send(id, target, EventCode::Gesture, EventParam::Dir(dir));
        !e.input_reset_pending(id) && self.act_obj.is_some_and(|a| e.tree.contains(a))
    }
}
