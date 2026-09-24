//! Pointer processing (touch panels and mice): LVGL `indev_pointer_proc`,
//! `indev_proc_press`, `indev_proc_release`, `indev_click_focus` and object search.

use twine_core::{Instant, Point};
use twine_hal::PointerData;
use twine_style::{Dir, State};

use super::{ClickCounter, Forget, InputId, Timing, stopped};
use crate::{DisplayId, Engine, EventCode, EventParam, EventResult, MeasureCx, NodeId, ObjFlags};

/// The state of one pointer (LVGL `lv_indev_t::pointer`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PointerProc {
    /// The point of the current read.
    pub(crate) act_point: Point,
    /// The point of the previous read.
    pub(crate) last_point: Point,
    /// Whether the previous read was pressed (LVGL `prev_state`).
    pub(crate) pressed: bool,
    /// The node being pressed.
    pub(crate) act_obj: Option<NodeId>,
    /// The node pressed last (for click focus).
    pub(crate) last_obj: Option<NodeId>,
    /// The node under a released (mouse) pointer.
    pub(crate) hovered: Option<NodeId>,
    press_time: Instant,
    long_pr_sent: bool,
    longpr_rep_time: Instant,
    /// Movement since the previous read.
    pub(crate) vect: Point,
    /// Movement since the press until a scroll starts, then the scrolled distance.
    pub(crate) scroll_sum: Point,
    /// The node being scrolled by this pointer (dragged, then thrown after the release).
    pub(crate) scroll_obj: Option<NodeId>,
    /// The locked scroll direction (`HOR`, `VER` or `NONE`).
    pub(crate) scroll_dir: Dir,
    /// The momentum of a throw (decays every read after the release).
    pub(crate) scroll_throw_vect: Point,
    /// The throw vector at the release (for snap predictions).
    pub(crate) scroll_throw_vect_ori: Point,
    /// How far the scroll may go (`scroll_sum` limits, for `SCROLL_ONE`).
    pub(crate) scroll_area: crate::scroll_drag::ScrollLimits,
    /// The last movements and their times (the throw vector is their time-decayed sum).
    pub(crate) vect_hist: [(Point, Instant); crate::scroll_drag::VECT_HIST_SIZE],
    pub(crate) vect_hist_index: u8,
    pub(crate) gesture_sum: Point,
    pub(crate) gesture_sent: bool,
    pub(crate) gesture_dir: Option<Dir>,
    clicks: ClickCounter,
    /// The press started on `act_obj` (LVGL `pointer.pressed`): only then a release clicks.
    press_started_here: bool,
    /// Ignore the pointer until it is released.
    pub(crate) wait_until_release: bool,
}

impl PointerProc {
    pub(crate) fn new() -> Self {
        Self {
            act_point: Point::ZERO,
            last_point: Point::ZERO,
            pressed: false,
            act_obj: None,
            last_obj: None,
            hovered: None,
            press_time: Instant::ZERO,
            long_pr_sent: false,
            longpr_rep_time: Instant::ZERO,
            vect: Point::ZERO,
            scroll_sum: Point::ZERO,
            scroll_obj: None,
            scroll_dir: Dir::NONE,
            scroll_throw_vect: Point::ZERO,
            scroll_throw_vect_ori: Point::ZERO,
            scroll_area: crate::scroll_drag::ScrollLimits::NONE,
            vect_hist: [(Point::ZERO, Instant::ZERO); crate::scroll_drag::VECT_HIST_SIZE],
            vect_hist_index: 0,
            gesture_sum: Point::ZERO,
            gesture_sent: false,
            gesture_dir: None,
            clicks: ClickCounter::default(),
            press_started_here: false,
            wait_until_release: false,
        }
    }

    /// LVGL `indev_proc_reset_query_handler`.
    pub(crate) fn reset(&mut self, forget: Forget) {
        self.act_obj = None;
        self.long_pr_sent = false;
        self.scroll_sum = Point::ZERO;
        self.scroll_obj = None;
        self.scroll_dir = Dir::NONE;
        self.scroll_throw_vect = Point::ZERO;
        self.gesture_sum = Point::ZERO;
        match forget {
            Forget::Nothing => {}
            Forget::All => {
                self.last_obj = None;
                self.hovered = None;
            }
            Forget::Node(n) => {
                if self.last_obj == Some(n) {
                    self.last_obj = None;
                }
                if self.hovered == Some(n) {
                    self.hovered = None;
                }
            }
        }
    }

    /// The next long press (repeat) instant while a node is pressed.
    pub(crate) fn deadline(&self, t: &Timing) -> Option<Instant> {
        if !self.pressed || self.act_obj.is_none() || self.scroll_obj.is_some() || self.wait_until_release {
            return None;
        }
        Some(if self.long_pr_sent {
            self.longpr_rep_time + t.long_press_repeat
        } else {
            self.press_time + t.long_press
        })
    }

    /// One sample (LVGL `indev_pointer_proc`).
    pub(crate) fn process(
        &mut self,
        e: &mut Engine,
        id: InputId,
        display: DisplayId,
        d: PointerData,
        now: Instant,
        t: &Timing,
    ) {
        self.act_point = d.point;
        e.input_point = Some(d.point);
        if d.pressed {
            self.proc_press(e, id, display, now, t);
        } else {
            self.proc_release(e, id, display, now, t);
        }
        self.pressed = d.pressed;
        self.last_point = self.act_point;
    }

    /// Forgets nodes that were deleted. A deleted pressed node makes the pointer wait for
    /// the release (LVGL: deleting the pressed object resets the device).
    fn drop_dead(&mut self, e: &Engine) {
        let alive = |n: Option<NodeId>| n.filter(|n| e.tree.contains(*n));
        if self.act_obj.is_some() && alive(self.act_obj).is_none() {
            self.reset(Forget::Nothing);
            if self.pressed {
                self.wait_until_release = true;
            }
        }
        self.last_obj = alive(self.last_obj);
        self.hovered = alive(self.hovered);
        if self.scroll_obj.is_some() && alive(self.scroll_obj).is_none() {
            self.scroll_obj = None;
            self.scroll_dir = Dir::NONE;
            self.scroll_throw_vect = Point::ZERO;
        }
    }

    /// LVGL `indev_proc_press`.
    pub(crate) fn proc_press(
        &mut self,
        e: &mut Engine,
        id: InputId,
        display: DisplayId,
        now: Instant,
        t: &Timing,
    ) {
        self.drop_dead(e);
        if self.wait_until_release {
            return;
        }
        // Search again unless the pressed node keeps the press (PRESS_LOCK) or is scrolled.
        let (found, searched) = match self.act_obj {
            Some(a) if self.scroll_obj.is_some() || e.has_flag(a, ObjFlags::PRESS_LOCK) => (Some(a), false),
            _ => (e.search_obj(display, self.act_point), true),
        };
        if stopped_any(e, id) {
            return;
        }
        // A new press during a throw stops it (LVGL stops the throw animation).
        if searched && self.scroll_obj.is_some() {
            self.scroll_throw_vect = Point::ZERO;
            if !self.throw_handler(e, id, t) {
                return;
            }
        }
        if found != self.act_obj {
            if self.act_obj.is_none() {
                self.last_point = self.act_point;
            }
            self.press_started_here = !self.pressed;
            // A pressed mouse dragged onto another node leaves the hovered one.
            if let Some(h) = self.hovered.filter(|h| found != Some(*h)) {
                self.hovered = None;
                if !e.input_send(id, h, EventCode::HoverLeave, EventParam::None) && stopped_any(e, id) {
                    return;
                }
            }
            if let Some(old) = self.act_obj {
                // The node lost the press to another one (no PRESS_LOCK).
                self.act_obj = None;
                if !e.input_send(id, old, EventCode::PressLost, EventParam::None) && stopped_any(e, id) {
                    return;
                }
            }
            self.act_obj = found;
            if let Some(obj) = found {
                self.press_time = now;
                self.long_pr_sent = false;
                self.scroll_sum = Point::ZERO;
                self.set_scroll_obj(e, id, None, Dir::NONE);
                self.gesture_dir = None;
                self.gesture_sent = false;
                self.gesture_sum = Point::ZERO;
                self.vect = Point::ZERO;
                self.vect_hist = [(Point::ZERO, now); crate::scroll_drag::VECT_HIST_SIZE];
                // Dragged onto a new node while pressed: no `Pressed` for it.
                if !self.pressed
                    && enabled(e, obj)
                    && !e.input_send(id, obj, EventCode::Pressed, EventParam::None)
                {
                    return;
                }
                if self.wait_until_release {
                    return;
                }
                self.click_focus(e, id, obj);
                if stopped(e, id, obj) {
                    return;
                }
            }
        }
        self.vect = self.act_point - self.last_point;
        self.record_vect(now);
        let Some(obj) = self.act_obj else {
            return;
        };
        let en = enabled(e, obj);
        if en && !e.input_send(id, obj, EventCode::Pressing, EventParam::None) {
            return;
        }
        if self.wait_until_release {
            return;
        }
        if let Some(s) = self.scroll_obj {
            e.stop_scroll_anim(s);
        }
        if !self.scroll_handler(e, id, t) {
            return;
        }
        if self.scroll_obj.is_none() && !self.gesture(e, id, t) {
            return;
        }
        // Long presses only while the press is not scrolling (LVGL).
        if self.scroll_obj.is_some() || !e.tree.contains(obj) {
            return;
        }
        if !self.long_pr_sent {
            if now.saturating_duration_since(self.press_time) >= t.long_press {
                self.long_pr_sent = true;
                self.longpr_rep_time = now;
                if en {
                    e.input_send(id, obj, EventCode::LongPressed, EventParam::None);
                }
            }
        } else if now.saturating_duration_since(self.longpr_rep_time) >= t.long_press_repeat {
            self.longpr_rep_time = now;
            if en {
                e.input_send(id, obj, EventCode::LongPressedRepeat, EventParam::None);
            }
        }
    }

    /// LVGL `indev_proc_release`.
    pub(crate) fn proc_release(
        &mut self,
        e: &mut Engine,
        id: InputId,
        display: DisplayId,
        now: Instant,
        t: &Timing,
    ) {
        self.drop_dead(e);
        // Hover (mice): the node under a released pointer that moved.
        if self.wait_until_release || self.last_point != self.act_point {
            let under = e.search_obj(display, self.act_point);
            if stopped_any(e, id) {
                return;
            }
            if under != self.hovered {
                let old = self.hovered;
                self.hovered = under;
                if let Some(h) = under {
                    e.input_send(id, h, EventCode::HoverOver, EventParam::None);
                    if stopped_any(e, id) {
                        return;
                    }
                }
                if let Some(o) = old.filter(|o| e.tree.contains(*o)) {
                    e.input_send(id, o, EventCode::HoverLeave, EventParam::None);
                    if stopped_any(e, id) {
                        return;
                    }
                }
            }
        }
        if self.wait_until_release {
            if let Some(a) = self.act_obj.take() {
                e.input_send(id, a, EventCode::PressLost, EventParam::None);
            }
            self.wait_until_release = false;
            self.long_pr_sent = false;
            if stopped_any(e, id) {
                return;
            }
        }
        let scroll_obj = self.scroll_obj;
        let Some(obj) = self.act_obj else {
            // Released: a throw in progress continues every read.
            if scroll_obj.is_some() {
                self.throw_handler(e, id, t);
            }
            return;
        };
        let en = enabled(e, obj);
        if en && !e.input_send(id, obj, EventCode::Released, EventParam::None) {
            self.act_obj = None;
            return;
        }
        if en {
            match scroll_obj {
                None if self.press_started_here => {
                    if !self.long_pr_sent && !self.short_click(e, id, obj, now, t) {
                        self.act_obj = None;
                        return;
                    }
                    if !e.input_send(id, obj, EventCode::Clicked, EventParam::None) {
                        self.act_obj = None;
                        return;
                    }
                }
                None => {}
                Some(s) => {
                    // The momentum (and the snapping or elastic return) follow in the next reads.
                    e.input_send(id, s, EventCode::ScrollThrowBegin, EventParam::None);
                    if stopped_any(e, id) {
                        self.act_obj = None;
                        return;
                    }
                }
            }
        }
        self.act_obj = None;
        self.long_pr_sent = false;
    }

    /// `ShortClicked` followed by `SingleClicked` / `DoubleClicked` / `TripleClicked`.
    fn short_click(&mut self, e: &mut Engine, id: InputId, obj: NodeId, now: Instant, t: &Timing) -> bool {
        let multi = self.clicks.click(now, Some(self.act_point), t);
        e.input_send(id, obj, EventCode::ShortClicked, EventParam::None)
            && e.input_send(id, obj, multi, EventParam::None)
    }

    /// LVGL `indev_click_focus`: a pressed `CLICK_FOCUSABLE` node takes the focus (in its group
    /// if it has one; otherwise it just receives `Focused`, and the previously pressed node
    /// `Defocused` or `Leave`).
    fn click_focus(&mut self, e: &mut Engine, id: InputId, obj: NodeId) {
        if !e.has_flag(obj, ObjFlags::CLICK_FOCUSABLE) || !enabled(e, obj) {
            return;
        }
        let last = self.last_obj.filter(|l| e.tree.contains(*l));
        let g_act = e.group_of(obj);
        let g_prev = last.and_then(|l| e.group_of(l));
        if g_act == g_prev {
            if g_act.is_some() {
                e.focus(obj);
            } else if last != Some(obj) {
                if let Some(l) = last {
                    e.input_send(id, l, EventCode::Defocused, EventParam::None);
                }
                if stopped(e, id, obj) {
                    return;
                }
                e.input_send(id, obj, EventCode::Focused, EventParam::None);
            }
        } else {
            if let Some(l) = last {
                let code = if g_prev.is_none() {
                    EventCode::Defocused
                } else {
                    EventCode::Leave
                };
                e.input_send(id, l, code, EventParam::None);
                if stopped(e, id, obj) {
                    return;
                }
            }
            if g_act.is_some() {
                e.focus(obj);
            } else {
                e.input_send(id, obj, EventCode::Focused, EventParam::None);
            }
        }
        self.last_obj = Some(obj);
    }
}

/// Whether the node is not `DISABLED` (disabled nodes block the pointer but get no events).
fn enabled(e: &Engine, obj: NodeId) -> bool {
    e.tree
        .node(obj)
        .is_some_and(|n| !n.state.contains(State::DISABLED))
}

/// Whether a reset of the device is pending.
fn stopped_any(e: &Engine, id: InputId) -> bool {
    e.input_reset_pending(id)
}

impl Engine {
    /// The node a press at `p` on `display` goes to (LVGL `pointer_search_obj`): the system
    /// layer, the top layer, the active screen and the bottom layer are searched in this order;
    /// see [`search_in`](Self::search_in). Disabled nodes are returned (they block the press).
    pub(crate) fn search_obj(&mut self, display: DisplayId, p: Point) -> Option<NodeId> {
        // While a screen load animation runs, neither screen (nor the bottom layer) gets input.
        let blocked = self.screens_input_blocked(display.index());
        let d = self.displays.get(display.index())?;
        let roots = [d.sys_layer, d.top_layer, d.active_screen, d.bottom_layer];
        let n = if blocked { 2 } else { roots.len() };
        roots.into_iter().take(n).find_map(|r| self.search_in(r, p))
    }

    /// LVGL `lv_indev_search_obj`: if `p` lies on `obj` (its extra draw area with
    /// `OVERFLOW_VISIBLE`), the children are searched topmost first; else, or if no child
    /// matched, `obj` itself when it passes [`obj_hit_test`](Self::obj_hit_test). Hidden
    /// subtrees are skipped.
    // Transformed nodes are hit-tested with their untransformed coordinates.
    fn search_in(&mut self, obj: NodeId, p: Point) -> Option<NodeId> {
        let n = self.tree.node(obj)?;
        if n.is_hidden() {
            return None;
        }
        let hit = self.obj_hit_test(obj, p);
        let n = self.tree.node(obj)?;
        let mut area = n.coords;
        if n.flags.contains(ObjFlags::OVERFLOW_VISIBLE) {
            area = area.expand(i32::from(n.ext_draw));
        }
        // Wrappers are transparent: their children are searched wherever they are.
        if area.contains(p) || n.flags.contains(ObjFlags::LAYOUT_PASSTHROUGH) {
            let mut child = n.last_child();
            while let Some(c) = child {
                if let Some(found) = self.search_in(c, p) {
                    return Some(found);
                }
                child = self.tree.node(c).and_then(crate::Node::prev_sibling);
            }
        }
        hit.then_some(obj)
    }

    /// LVGL `lv_obj_hit_test`: the node is `CLICKABLE` and [`Widget::hit_test`](crate::Widget::hit_test)
    /// accepts `p`; with `ADV_HITTEST` a `HitTest` event is sent too, and a handler returning
    /// `Stop` or `Consumed` vetoes the hit.
    pub(crate) fn obj_hit_test(&mut self, obj: NodeId, p: Point) -> bool {
        let Some(n) = self.tree.node(obj) else {
            return false;
        };
        if !n.flags.contains(ObjFlags::CLICKABLE) {
            return false;
        }
        let adv = n.flags.contains(ObjFlags::ADV_HITTEST);
        if !n.widget().hit_test(&MeasureCx::new(self, obj), p) {
            return false;
        }
        if adv {
            let r = self.send_event(obj, EventCode::HitTest, EventParam::Point(p));
            return r == EventResult::Continue && self.tree.contains(obj);
        }
        true
    }

    /// The node a press at `p` on `display` would go to: the topmost `CLICKABLE` node whose
    /// hit test accepts `p` (children of a node are searched only where the node itself
    /// contains `p`, unless it has `OVERFLOW_VISIBLE`); non-clickable nodes pass the press to
    /// their parent. `None` when nothing is hit or the node hit is `DISABLED` (disabled nodes
    /// block the press and receive nothing).
    ///
    /// Sends `HitTest` events to nodes with `ADV_HITTEST`.
    pub fn hit_test(&mut self, display: DisplayId, p: Point) -> Option<NodeId> {
        let n = self.search_obj(display, p)?;
        enabled(self, n).then_some(n)
    }
}
