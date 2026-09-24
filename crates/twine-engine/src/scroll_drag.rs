//! Scrolling with a pointer (LVGL `lv_indev_scroll.c`): finding the node to scroll when a
//! drag starts (scroll chains, direction lock), dragging with elastic edges and `SCROLL_ONE`
//! limits, and the momentum throw after the release with elastic return and snapping.
//!
//! Everything works on the integer vectors of the pointer reads and allocates nothing.

use twine_core::{Instant, Point};
use twine_style::{Dir, ScrollSnap, State};

use crate::input::{InputId, PointerProc, Timing};
use crate::scroll::{Axis, COORD_MAX, COORD_MIN, SCROLL_ELASTIC_FACTOR};
use crate::{Engine, EventCode, EventParam, NodeId, ObjFlags, fmt_node_id};

/// Number of pointer movements the throw vector is computed from (LVGL
/// `LV_INDEV_VECT_HIST_SIZE`).
pub(crate) const VECT_HIST_SIZE: usize = 8;

/// Movements older than this (in ms) do not count for the throw (LVGL
/// `indev_scroll_throw_decay`).
const THROW_DECAY_MS: i64 = 99;

/// Limits of `scroll_sum` while dragging (LVGL `pointer.scroll_area`): with `SCROLL_ONE` the
/// scroll stops at the neighbouring snap points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScrollLimits {
    pub(crate) x1: i32,
    pub(crate) y1: i32,
    pub(crate) x2: i32,
    pub(crate) y2: i32,
}

impl ScrollLimits {
    /// No limits.
    pub(crate) const NONE: ScrollLimits = ScrollLimits {
        x1: COORD_MIN,
        y1: COORD_MIN,
        x2: COORD_MAX,
        y2: COORD_MAX,
    };
}

/// A movement component decayed by its age (LVGL `indev_scroll_throw_decay`): full at 0 ms,
/// nothing from 99 ms on.
fn throw_decay(x: i32, t_ms: i64) -> i32 {
    if t_ms <= 0 {
        return x;
    }
    if t_ms >= THROW_DECAY_MS {
        return 0;
    }
    (i64::from(x) * (512 - (512 * t_ms) / THROW_DECAY_MS) / 512) as i32
}

/// LVGL `elastic_diff`: the part of `diff` applied to `obj` given the room at the start and
/// the end of the axis. Without `SCROLL_ELASTIC` the content stops at the edges; with it the
/// movement is [`SCROLL_ELASTIC_FACTOR`] times slower while the content is scrolled out
/// (with snapping: while there is no further snap point on one side).
pub(crate) fn elastic_diff(e: &Engine, obj: NodeId, diff: i32, start: i32, end: i32, axis: Axis) -> i32 {
    if diff == 0 {
        return 0;
    }
    if !e.has_flag(obj, ObjFlags::SCROLL_ELASTIC) {
        // Stop at the edge; content already beyond it (e.g. by center snapping) stays.
        // LVGL compares `ended - diff`, which never clamps a negative `diff`; the clamp is
        // applied symmetrically here.
        let ended = if diff > 0 { start } else { end };
        return if ended <= 0 {
            0
        } else if ended < diff.abs() {
            ended * diff.signum()
        } else {
            diff
        };
    }
    let slow = |d: i32| {
        let d = if d < 0 {
            d - SCROLL_ELASTIC_FACTOR / 2
        } else {
            d + SCROLL_ELASTIC_FACTOR / 2
        };
        d / SCROLL_ELASTIC_FACTOR
    };
    if e.snap_of(obj, axis) == ScrollSnap::None {
        if end < 0 || start < 0 { slow(diff) } else { diff }
    } else {
        let (has_start, has_end) = e.has_more_snap_points(obj, axis);
        if !has_start || !has_end { slow(diff) } else { diff }
    }
}

impl PointerProc {
    /// Sets the scrolled node (also recorded in the engine for `is_scrolling` and `Active`
    /// scrollbars).
    pub(crate) fn set_scroll_obj(&mut self, e: &mut Engine, id: InputId, obj: Option<NodeId>, dir: Dir) {
        self.scroll_obj = obj;
        self.scroll_dir = if obj.is_some() { dir } else { Dir::NONE };
        e.set_indev_scroll(id, obj.map(|o| (o, dir)));
    }

    /// Records the movement of this read and recomputes the throw vector (LVGL: the sum of
    /// the last [`VECT_HIST_SIZE`] movements, each decayed by its age).
    pub(crate) fn record_vect(&mut self, now: Instant) {
        let i = usize::from(self.vect_hist_index) % VECT_HIST_SIZE;
        self.vect_hist[i] = (self.vect, now);
        self.vect_hist_index = ((i + 1) % VECT_HIST_SIZE) as u8;
        let mut v = Point::ZERO;
        for &(p, at) in &self.vect_hist {
            let t = now.saturating_duration_since(at).as_millis() as i64;
            v.x += throw_decay(p.x, t);
            v.y += throw_decay(p.y, t);
        }
        self.scroll_throw_vect = v;
        self.scroll_throw_vect_ori = v;
    }

    /// LVGL `lv_indev_scroll_handler`: while pressed, finds the node to scroll once the
    /// movement exceeds `scroll_limit` and then scrolls it by each movement. Returns whether
    /// processing may continue.
    pub(crate) fn scroll_handler(&mut self, e: &mut Engine, id: InputId, t: &Timing) -> bool {
        if self.vect == Point::ZERO {
            return true;
        }
        let obj = if let Some(o) = self.scroll_obj {
            o
        } else {
            let Some(o) = self.find_scroll_obj(e, id, t) else {
                return true;
            };
            self.init_scroll_limits(e);
            if let Some(a) = self.act_obj {
                e.clear_state(a, State::PRESSED);
            }
            twine_core::debug!(
                target: "twine::input",
                "{} scroll {} {:?}",
                id,
                fmt_node_id(o),
                self.scroll_dir
            );
            e.input_send(id, o, EventCode::ScrollBegin, EventParam::None);
            if e.input_reset_pending(id) {
                return false;
            }
            if !e.tree.contains(o) {
                self.set_scroll_obj(e, id, None, Dir::NONE);
                return true;
            }
            o
        };
        let (mut dx, mut dy) = (0, 0);
        if self.scroll_dir == Dir::HOR {
            let (sl, sr) = (e.scroll_left(obj), e.scroll_right(obj));
            dx = elastic_diff(e, obj, self.vect.x, sl, sr, Axis::X);
        } else {
            let (st, sb) = (e.scroll_top(obj), e.scroll_bottom(obj));
            dy = elastic_diff(e, obj, self.vect.y, st, sb, Axis::Y);
        }
        let dir = e.scroll_dir(obj);
        if !dir.contains(Dir::LEFT) && dx > 0 {
            dx = 0;
        }
        if !dir.contains(Dir::RIGHT) && dx < 0 {
            dx = 0;
        }
        if !dir.contains(Dir::TOP) && dy > 0 {
            dy = 0;
        }
        if !dir.contains(Dir::BOTTOM) && dy < 0 {
            dy = 0;
        }
        self.scroll_limit_diff(Some(&mut dx), Some(&mut dy));
        e.scroll_by_raw(obj, dx, dy);
        if e.input_reset_pending(id) {
            return false;
        }
        self.scroll_sum += Point::new(dx, dy);
        true
    }

    /// LVGL `lv_indev_find_scroll_obj`: walks from the pressed node up. A scrollable node
    /// that can scroll in the drag direction (the dominant axis of the movement since the
    /// press, once it reaches `scroll_limit`) wins; a node that can at least scroll on that
    /// axis is remembered as a candidate (it shows the elastic effect); nodes without the
    /// scroll chain flag of the axis end the search. The deepest match or the last candidate
    /// is used and the direction is locked.
    fn find_scroll_obj(&mut self, e: &mut Engine, id: InputId, t: &Timing) -> Option<NodeId> {
        let lim = t.scroll_limit;
        let mut candidate: Option<(NodeId, Dir)> = None;
        self.scroll_sum += self.vect;
        let sum = self.scroll_sum;
        let hor_en = sum.x.abs() > sum.y.abs();
        let ver_en = !hor_en;
        let mut cur = self.act_obj;
        while let Some(o) = cur {
            let chain_hor = e.has_flag(o, ObjFlags::SCROLL_CHAIN_HOR);
            let chain_ver = e.has_flag(o, ObjFlags::SCROLL_CHAIN_VER);
            if !e.has_flag(o, ObjFlags::SCROLLABLE) {
                if (!chain_hor && hor_en) || (!chain_ver && ver_en) {
                    break;
                }
                cur = e.tree.parent(o);
                continue;
            }
            let dir = e.scroll_dir(o);
            let mut up_en = ver_en && dir.contains(Dir::TOP);
            let mut down_en = ver_en && dir.contains(Dir::BOTTOM);
            let mut left_en = hor_en && dir.contains(Dir::LEFT);
            let mut right_en = hor_en && dir.contains(Dir::RIGHT);
            let room = |axis: Axis| {
                if e.snap_of(o, axis) == ScrollSnap::None {
                    match axis {
                        Axis::X => (e.scroll_left(o), e.scroll_right(o)),
                        Axis::Y => (e.scroll_top(o), e.scroll_bottom(o)),
                    }
                } else {
                    // Assume room when there are more snap points on that side.
                    let (s, en) = e.has_more_snap_points(o, axis);
                    (if s { 1 } else { -1 }, if en { 1 } else { -1 })
                }
            };
            let (sl, sr) = room(Axis::X);
            let (st, sb) = room(Axis::Y);
            if (st > 0 || sb > 0) && ((up_en && sum.y >= lim) || (down_en && sum.y <= -lim)) {
                candidate = Some((o, Dir::VER));
            }
            if (sl > 0 || sr > 0) && ((left_en && sum.x >= lim) || (right_en && sum.x <= -lim)) {
                candidate = Some((o, Dir::HOR));
            }
            up_en &= st > 0;
            down_en &= sb > 0;
            left_en &= sl > 0;
            right_en &= sr > 0;
            if (left_en && sum.x >= lim)
                || (right_en && sum.x <= -lim)
                || (up_en && sum.y >= lim)
                || (down_en && sum.y <= -lim)
            {
                break;
            }
            if (!chain_hor && hor_en) || (!chain_ver && ver_en) {
                break;
            }
            cur = e.tree.parent(o);
        }
        let (obj, dir) = candidate?;
        self.set_scroll_obj(e, id, Some(obj), dir);
        self.scroll_sum = Point::ZERO;
        Some(obj)
    }

    /// LVGL `init_scroll_limits`: without `SCROLL_ONE` no limit; with it the scroll may reach
    /// the previous and the next snap point only.
    fn init_scroll_limits(&mut self, e: &Engine) {
        let Some(obj) = self.scroll_obj else {
            return;
        };
        let mut a = ScrollLimits::NONE;
        if e.has_flag(obj, ObjFlags::SCROLL_ONE) {
            let c = e.coords(obj);
            // LVGL areas are inclusive: `x2` / `y2` are the last pixel.
            let (x2, y2) = (c.x1 - 1, c.y1 - 1);
            match e.scroll_snap_y(obj) {
                ScrollSnap::Start => {
                    a.y1 = e.find_snap_point_y(obj, c.y0 + 1, COORD_MAX, 0);
                    a.y2 = e.find_snap_point_y(obj, COORD_MIN, c.y0 - 1, 0);
                }
                ScrollSnap::End => {
                    a.y1 = e.find_snap_point_y(obj, y2, COORD_MAX, 0);
                    a.y2 = e.find_snap_point_y(obj, COORD_MIN, y2, 0);
                }
                ScrollSnap::Center => {
                    let mid = c.y0 + c.height() / 2;
                    a.y1 = e.find_snap_point_y(obj, mid + 1, COORD_MAX, 0);
                    a.y2 = e.find_snap_point_y(obj, COORD_MIN, mid - 1, 0);
                }
                ScrollSnap::None => {}
            }
            match e.scroll_snap_x(obj) {
                ScrollSnap::Start => {
                    a.x1 = e.find_snap_point_x(obj, c.x0, COORD_MAX, 0);
                    a.x2 = e.find_snap_point_x(obj, COORD_MIN, c.x0, 0);
                }
                ScrollSnap::End => {
                    a.x1 = e.find_snap_point_x(obj, x2, COORD_MAX, 0);
                    a.x2 = e.find_snap_point_x(obj, COORD_MIN, x2, 0);
                }
                ScrollSnap::Center => {
                    let mid = c.x0 + c.width() / 2;
                    a.x1 = e.find_snap_point_x(obj, mid + 1, COORD_MAX, 0);
                    a.x2 = e.find_snap_point_x(obj, COORD_MIN, mid - 1, 0);
                }
                ScrollSnap::None => {}
            }
        }
        // "No snap point" is `COORD_MAX`, but the lower limits must be small.
        if a.x1 == COORD_MAX {
            a.x1 = COORD_MIN;
        }
        if a.y1 == COORD_MAX {
            a.y1 = COORD_MIN;
        }
        // Allow scrolling on the edges (snapping reverts it anyway).
        if a.x1 == 0 {
            a.x1 = COORD_MIN;
        }
        if a.x2 == 0 {
            a.x2 = COORD_MAX;
        }
        if a.y1 == 0 {
            a.y1 = COORD_MIN;
        }
        if a.y2 == 0 {
            a.y2 = COORD_MAX;
        }
        self.scroll_area = a;
    }

    /// LVGL `scroll_limit_diff`: keeps `scroll_sum + diff` inside the scroll limits.
    fn scroll_limit_diff(&self, dx: Option<&mut i32>, dy: Option<&mut i32>) {
        let a = self.scroll_area;
        if let Some(dy) = dy {
            if self.scroll_sum.y + *dy < a.y1 {
                *dy = a.y1 - self.scroll_sum.y;
            }
            if self.scroll_sum.y + *dy > a.y2 {
                *dy = a.y2 - self.scroll_sum.y;
            }
        }
        if let Some(dx) = dx {
            if self.scroll_sum.x + *dx < a.x1 {
                *dx = a.x1 - self.scroll_sum.x;
            }
            if self.scroll_sum.x + *dx > a.x2 {
                *dx = a.x2 - self.scroll_sum.x;
            }
        }
    }

    /// LVGL `lv_indev_scroll_throw_predict`: the distance a throw on `dir` would travel.
    pub(crate) fn throw_predict(&self, dir: Dir, t: &Timing) -> i32 {
        let mut v = if dir == Dir::VER {
            self.scroll_throw_vect_ori.y
        } else if dir == Dir::HOR {
            self.scroll_throw_vect_ori.x
        } else {
            return 0;
        };
        let keep = 100 - i32::from(t.scroll_throw.min(100));
        let mut sum = 0i32;
        while v != 0 {
            sum = sum.saturating_add(v);
            v = v * keep / 100;
        }
        sum
    }

    /// LVGL `lv_indev_scroll_throw_handler`, run once per read after the release while a
    /// scroll is in progress: the throw vector decays by `scroll_throw` percent (faster beyond
    /// an elastic edge) and moves the content; with snapping the predicted end is snapped and
    /// animated at once. When the vector reaches zero, overscrolled content animates back to
    /// the edge, `ScrollEnd` is sent and the scroll is over. Returns whether processing may
    /// continue.
    pub(crate) fn throw_handler(&mut self, e: &mut Engine, id: InputId, t: &Timing) -> bool {
        let Some(obj) = self.scroll_obj else {
            return true;
        };
        if self.scroll_dir == Dir::NONE {
            return true;
        }
        if !e.tree.contains(obj) {
            self.set_scroll_obj(e, id, None, Dir::NONE);
            return true;
        }
        if !e.has_flag(obj, ObjFlags::SCROLL_MOMENTUM) {
            self.scroll_throw_vect = Point::ZERO;
        }
        let keep = 100 - i32::from(t.scroll_throw.min(100));
        let (align_x, align_y) = (e.scroll_snap_x(obj), e.scroll_snap_y(obj));
        if self.scroll_dir == Dir::VER {
            self.scroll_throw_vect.x = 0;
            if align_y == ScrollSnap::None {
                let v = self.scroll_throw_vect.y * keep / 100;
                let (st, sb) = (e.scroll_top(obj), e.scroll_bottom(obj));
                self.scroll_throw_vect.y = elastic_diff(e, obj, v, st, sb, Axis::Y);
                e.scroll_by_raw(obj, 0, self.scroll_throw_vect.y);
            } else {
                let mut dy = self.throw_predict(Dir::VER, t);
                self.scroll_throw_vect.y = 0;
                self.scroll_limit_diff(None, Some(&mut dy));
                let y = e.find_snap_point_y(obj, COORD_MIN, COORD_MAX, dy);
                let y = if y == COORD_MAX { 0 } else { y };
                e.scroll_by(obj, 0, dy + y, true);
            }
        } else if self.scroll_dir == Dir::HOR {
            self.scroll_throw_vect.y = 0;
            if align_x == ScrollSnap::None {
                let v = self.scroll_throw_vect.x * keep / 100;
                let (sl, sr) = (e.scroll_left(obj), e.scroll_right(obj));
                self.scroll_throw_vect.x = elastic_diff(e, obj, v, sl, sr, Axis::X);
                e.scroll_by_raw(obj, self.scroll_throw_vect.x, 0);
            } else {
                let mut dx = self.throw_predict(Dir::HOR, t);
                self.scroll_throw_vect.x = 0;
                self.scroll_limit_diff(Some(&mut dx), None);
                let x = e.find_snap_point_x(obj, COORD_MIN, COORD_MAX, dx);
                let x = if x == COORD_MAX { 0 } else { x };
                e.scroll_by(obj, dx + x, 0, true);
            }
        }
        if e.input_reset_pending(id) {
            return false;
        }
        if !e.tree.contains(obj) {
            self.set_scroll_obj(e, id, None, Dir::NONE);
            return true;
        }
        if self.scroll_throw_vect != Point::ZERO {
            return true;
        }
        // The throw ended: scroll back content that is scrolled in beyond an edge.
        if align_y == ScrollSnap::None {
            let (st, sb) = (e.scroll_top(obj), e.scroll_bottom(obj));
            if st > 0 || sb > 0 {
                if st < 0 {
                    e.scroll_by(obj, 0, st, true);
                } else if sb < 0 {
                    e.scroll_by(obj, 0, -sb, true);
                }
            }
        }
        if align_x == ScrollSnap::None && e.tree.contains(obj) {
            let (sl, sr) = (e.scroll_left(obj), e.scroll_right(obj));
            if sl > 0 || sr > 0 {
                if sl < 0 {
                    e.scroll_by(obj, sl, 0, true);
                } else if sr < 0 {
                    e.scroll_by(obj, -sr, 0, true);
                }
            }
        }
        if e.input_reset_pending(id) {
            return false;
        }
        if e.tree.contains(obj) {
            twine_core::debug!(target: "twine::input", "{} scroll end {}", id, fmt_node_id(obj));
            e.input_send(id, obj, EventCode::ScrollEnd, EventParam::None);
            if e.input_reset_pending(id) {
                return false;
            }
        }
        self.set_scroll_obj(e, id, None, Dir::NONE);
        true
    }
}

impl Engine {
    /// Records (or clears) the node pointer `id` scrolls.
    pub(crate) fn set_indev_scroll(&mut self, id: InputId, v: Option<(NodeId, Dir)>) {
        let old = self.indev_scrolls.iter().position(|(i, _, _)| *i == id);
        match (old, v) {
            (Some(i), Some((n, d))) => self.indev_scrolls[i] = (id, n, d),
            (Some(i), None) => {
                self.indev_scrolls.swap_remove(i);
            }
            (None, Some((n, d))) => {
                // At most one entry per input device: cannot overflow.
                let _ = self.indev_scrolls.push((id, n, d));
            }
            (None, None) => {}
        }
    }

    /// The node input device `id` is scrolling.
    pub(crate) fn indev_scroll_of(&self, id: InputId) -> Option<NodeId> {
        self.indev_scrolls
            .iter()
            .find(|(i, _, _)| *i == id)
            .map(|(_, n, _)| *n)
    }

    /// The direction a pointer scrolls `node` in, if one does.
    pub(crate) fn indev_scroll_dir(&self, node: NodeId) -> Option<Dir> {
        self.indev_scrolls
            .iter()
            .find(|(_, n, _)| *n == node)
            .map(|(_, _, d)| *d)
    }

    /// After a reset of device `id` dropped its scroll: forget it and clear the node's
    /// `SCROLLED` state (LVGL `LV_EVENT_INDEV_RESET`).
    pub(crate) fn sync_indev_scroll(&mut self, id: InputId, scroll_obj: Option<NodeId>) {
        let recorded = self.indev_scroll_of(id);
        if recorded.is_some() && scroll_obj.is_none() {
            self.set_indev_scroll(id, None);
            if let Some(n) = recorded.filter(|n| self.tree.contains(*n)) {
                self.clear_state(n, State::PRESSED | State::SCROLLED);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throw_decay_matches_lvgl() {
        assert_eq!(throw_decay(20, 0), 20);
        assert_eq!(throw_decay(20, 30), 20 * (512 - 155) / 512);
        assert_eq!(throw_decay(20, 99), 0);
        assert_eq!(throw_decay(-20, 60), -(20 * (512 - 310) / 512));
    }
}
