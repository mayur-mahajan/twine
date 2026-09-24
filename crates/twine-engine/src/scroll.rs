//! The scroll model (LVGL `lv_obj_scroll.c`): per-node scroll offset and settings, scrollable
//! extents, programmatic scrolling (instant and animated), scrolling children into view, snap
//! points and the scroll-related object behaviour.
//!
//! # Coordinates and signs
//!
//! A node's scroll offset ([`Engine::scroll_offset`], LVGL `lv_obj_get_scroll_x/y`) is how far
//! its content is scrolled: `0` at the start, positive once the content moved up / left. The
//! children (except `FLOATING` ones) sit at their layout position minus the offset. Scrolling
//! changes the offset and moves the children's coordinates by the difference; it never runs the
//! layout and invalidates only the container's area (P2).
//!
//! The *deltas* of [`Engine::scroll_by`] follow LVGL: they are the movement of the content, so
//! `scroll_by(id, 0, 10, false)` moves the content **down** by 10 pixels and reduces the offset
//! by 10.

use twine_anim::{Anim, AnimProp, AnimTarget, Easing};
use twine_core::{Duration, Point, Rect};
use twine_style::{BaseDir, Dir, Part, PropId, ScrollSnap, ScrollbarMode};

use crate::{Engine, EventCode, EventParam, InvalidateReason, MeasureCx, NodeId, ObjFlags, fmt_node_id};

/// Shortest duration of an animated scroll (LVGL `SCROLL_ANIM_TIME_MIN`).
pub const SCROLL_ANIM_TIME_MIN: Duration = Duration::ms(200);
/// Longest duration of an animated scroll (LVGL `SCROLL_ANIM_TIME_MAX`).
pub const SCROLL_ANIM_TIME_MAX: Duration = Duration::ms(400);
/// Dragging or throwing beyond an edge of an elastic node moves the content this many times
/// slower (LVGL `LV_INDEV_DEF_SCROLL_ELASTIC_FACTOR`).
pub const SCROLL_ELASTIC_FACTOR: i32 = 4;

/// The largest coordinate used as "no limit" (LVGL `LV_COORD_MAX`).
pub(crate) const COORD_MAX: i32 = (1 << 29) - 1;
/// The smallest coordinate used as "no limit" (LVGL `LV_COORD_MIN`).
pub(crate) const COORD_MIN: i32 = -COORD_MAX;

/// Scroll settings of a node (LVGL `spec_attr` scroll fields), 4 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScrollAttrs {
    pub(crate) dir: Dir,
    pub(crate) scrollbar_mode: ScrollbarMode,
    pub(crate) snap_x: ScrollSnap,
    pub(crate) snap_y: ScrollSnap,
}

impl ScrollAttrs {
    /// LVGL's defaults: all directions, `Auto` scrollbars, no snapping.
    pub(crate) const DEFAULT: ScrollAttrs = ScrollAttrs {
        dir: Dir::ALL,
        scrollbar_mode: ScrollbarMode::Auto,
        snap_x: ScrollSnap::None,
        snap_y: ScrollSnap::None,
    };
}

/// The cached bounding box of a node's scrolling children (`None` inside: no such child),
/// relative to the unscrolled content origin (`coords.origin() − scroll`), so neither
/// scrolling nor moving the node changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChildrenBox(pub(crate) Option<Rect>);

/// An axis of scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum Axis {
    X,
    Y,
}

/// LVGL `lv_anim_speed_clamped(speed, min, max)` resolved for a distance `d` (LVGL
/// `lv_anim_resolve_speed`): `speed` is in pixels per second, stored with a resolution of 10.
pub(crate) fn speed_clamped_time(speed: u32, min: Duration, max: Duration, d: u32) -> Duration {
    let speed = (speed.min(10_230) + 5) / 10;
    let min_ms = (min.as_millis().min(10_230) as u32 + 5) / 10 * 10;
    let max_ms = (max.as_millis().min(10_230) as u32 + 5) / 10 * 10;
    if speed == 0 {
        return Duration::ms(u64::from(min_ms));
    }
    let t = d.saturating_mul(100) / speed;
    Duration::ms(u64::from(t.clamp(min_ms, max_ms.max(min_ms))))
}

impl Engine {
    // ---- Settings --------------------------------------------------------------------------

    fn scroll_attrs(&self, id: NodeId) -> ScrollAttrs {
        self.tree
            .node(id)
            .map_or(ScrollAttrs::DEFAULT, |n| n.scroll_attrs)
    }

    fn with_scroll_attrs(&mut self, id: NodeId, what: &str, f: impl FnOnce(&mut ScrollAttrs)) -> bool {
        let Some(n) = self.tree.node_mut(id) else {
            twine_core::warn!(target: "twine::scroll", "{}: node {} not found", what, fmt_node_id(id));
            return false;
        };
        let old = n.scroll_attrs;
        f(&mut n.scroll_attrs);
        old != n.scroll_attrs
    }

    /// Sets the directions `id` can be scrolled in (default [`Dir::ALL`]). Idempotent.
    pub fn set_scroll_dir(&mut self, id: NodeId, dir: Dir) {
        self.with_scroll_attrs(id, "set_scroll_dir", |a| a.dir = dir);
    }

    /// The directions `id` can be scrolled in.
    #[must_use]
    pub fn scroll_dir(&self, id: NodeId) -> Dir {
        self.scroll_attrs(id).dir
    }

    /// Sets when the scrollbars of `id` are shown (default [`ScrollbarMode::Auto`]). A change
    /// redraws the node; an unchanged mode does nothing.
    pub fn set_scrollbar_mode(&mut self, id: NodeId, mode: ScrollbarMode) {
        if self.with_scroll_attrs(id, "set_scrollbar_mode", |a| a.scrollbar_mode = mode) {
            self.invalidate(id, InvalidateReason::Scroll);
        }
    }

    /// When the scrollbars of `id` are shown.
    #[must_use]
    pub fn scrollbar_mode(&self, id: NodeId) -> ScrollbarMode {
        self.scroll_attrs(id).scrollbar_mode
    }

    /// Sets where `SNAPPABLE` children align horizontally when scrolling stops (default
    /// [`ScrollSnap::None`]). Idempotent; does not scroll by itself (see
    /// [`update_snap`](Self::update_snap)).
    pub fn set_scroll_snap_x(&mut self, id: NodeId, snap: ScrollSnap) {
        self.with_scroll_attrs(id, "set_scroll_snap_x", |a| a.snap_x = snap);
    }

    /// Sets where `SNAPPABLE` children align vertically when scrolling stops.
    pub fn set_scroll_snap_y(&mut self, id: NodeId, snap: ScrollSnap) {
        self.with_scroll_attrs(id, "set_scroll_snap_y", |a| a.snap_y = snap);
    }

    /// The horizontal snapping of `id`.
    #[must_use]
    pub fn scroll_snap_x(&self, id: NodeId) -> ScrollSnap {
        self.scroll_attrs(id).snap_x
    }

    /// The vertical snapping of `id`.
    #[must_use]
    pub fn scroll_snap_y(&self, id: NodeId) -> ScrollSnap {
        self.scroll_attrs(id).snap_y
    }

    // ---- Extents ---------------------------------------------------------------------------

    /// The scroll offset of `id` (LVGL `lv_obj_get_scroll_x/y`): how far the content is
    /// scrolled from its start (positive once it moved up / left).
    #[must_use]
    pub fn scroll_offset(&self, id: NodeId) -> Point {
        self.tree.node(id).map_or(Point::ZERO, crate::Node::scroll)
    }

    /// The scroll offset `id` ends at: the target of a running scroll animation, else the
    /// current offset (LVGL `lv_obj_get_scroll_end`).
    #[must_use]
    pub fn scroll_end(&self, id: NodeId) -> Point {
        let mut p = self.scroll_offset(id);
        let key = id.to_raw();
        for (_, a) in self.anim.timeline.iter() {
            match a.target {
                AnimTarget::Node(k, AnimProp::ScrollX) if k == key => p.x = a.end,
                AnimTarget::Node(k, AnimProp::ScrollY) if k == key => p.y = a.end,
                _ => {}
            }
        }
        p
    }

    /// Whether `id` lays out right-to-left.
    pub(crate) fn is_rtl(&self, id: NodeId, part: Part) -> bool {
        self.style_prop(id, part, PropId::BaseDir).get::<BaseDir>() == Some(BaseDir::Rtl)
    }

    /// Padding plus border width of the `Main` part (LVGL `space_*`), as
    /// `(left, top, right, bottom)`.
    fn space(&self, id: NodeId) -> (i32, i32, i32, i32) {
        let m = self.cached_main(id);
        let b = m.border_width.max(0);
        (m.pad.left + b, m.pad.top + b, m.pad.right + b, m.pad.bottom + b)
    }

    /// The widget's own content size (LVGL `lv_obj_get_self_width/height`).
    fn self_size(&self, id: NodeId) -> twine_core::Size {
        self.tree.node(id).map_or(twine_core::Size::ZERO, |n| {
            n.widget.content_size(&MeasureCx::new(self, id))
        })
    }

    /// The absolute bounding box of the children that count for scrolling (not hidden, not
    /// `FLOATING`), margins included; `None` without such children. Cached per node (see
    /// [`ChildrenBox`]).
    pub(crate) fn scroll_children_box(&self, id: NodeId) -> Option<Rect> {
        let n = self.tree.node(id)?;
        let origin = Point::new(n.coords.x0 - n.scroll.x, n.coords.y0 - n.scroll.y);
        let rel = if let Some(b) = n.children_bbox.get() {
            b.0
        } else {
            let mut bb: Option<Rect> = None;
            for c in self.tree.children(id) {
                let Some(cn) = self.tree.node(c) else { continue };
                if cn.is_hidden() || cn.flags.contains(ObjFlags::FLOATING) {
                    continue;
                }
                let m = |p| self.style_i32(c, Part::Main, p);
                let r = Rect::new(
                    cn.coords.x0 - m(PropId::MarginLeft),
                    cn.coords.y0 - m(PropId::MarginTop),
                    cn.coords.x1 + m(PropId::MarginRight),
                    cn.coords.y1 + m(PropId::MarginBottom),
                );
                bb = Some(bb.map_or(r, |b| b.union(&r)));
            }
            let rel = bb.map(|b| b.translate(-origin.x, -origin.y));
            n.children_bbox.set(Some(ChildrenBox(rel)));
            rel
        };
        rel.map(|r| r.translate(origin.x, origin.y))
    }

    /// Forgets the cached children bounding box of `id` (a child moved, resized, appeared or
    /// disappeared).
    pub(crate) fn forget_children_box(&self, id: NodeId) {
        if let Some(n) = self.tree.node(id) {
            n.children_bbox.set(None);
        }
    }

    /// How far the content of `id` can be scrolled down, i.e. the part above the top edge
    /// (LVGL `lv_obj_get_scroll_top`) = the vertical scroll offset.
    #[must_use]
    pub fn scroll_top(&self, id: NodeId) -> i32 {
        self.scroll_offset(id).y
    }

    /// How much content is below the bottom of the content area (LVGL
    /// `lv_obj_get_scroll_bottom`): the lowest child edge (plus its bottom margin) plus the
    /// bottom padding and border, minus the node's bottom edge; at least the widget's own
    /// content beyond the content area. Negative when scrolled beyond the end (elastic).
    #[must_use]
    pub fn scroll_bottom(&self, id: NodeId) -> i32 {
        let Some(n) = self.tree.node(id) else {
            return 0;
        };
        let c = n.coords;
        let (_, st, _, sb) = self.space(id);
        let child = self
            .scroll_children_box(id)
            .map_or(COORD_MIN, |b| b.y1 - (c.y1 - sb));
        let self_h = self.self_size(id).h - (c.height() - st - sb) - n.scroll.y;
        child.max(self_h)
    }

    /// How much content is left of the content area (LVGL `lv_obj_get_scroll_left`): the
    /// horizontal offset, or for right-to-left nodes the leftmost child edge beyond the
    /// content area.
    #[must_use]
    pub fn scroll_left(&self, id: NodeId) -> i32 {
        let Some(n) = self.tree.node(id) else {
            return 0;
        };
        if !self.is_rtl(id, Part::Main) {
            return n.scroll.x;
        }
        let c = n.coords;
        let (sl, _, sr, _) = self.space(id);
        let child = self
            .scroll_children_box(id)
            .map_or(COORD_MIN, |b| (c.x0 + sl) - b.x0);
        let self_w = self.self_size(id).w - (c.width() - sr - sl) + n.scroll.x;
        child.max(self_w)
    }

    /// How much content is right of the content area (LVGL `lv_obj_get_scroll_right`); for
    /// right-to-left nodes the negated horizontal offset.
    #[must_use]
    pub fn scroll_right(&self, id: NodeId) -> i32 {
        let Some(n) = self.tree.node(id) else {
            return 0;
        };
        if self.is_rtl(id, Part::Main) {
            return -n.scroll.x;
        }
        let c = n.coords;
        let (sl, _, sr, _) = self.space(id);
        let child = self
            .scroll_children_box(id)
            .map_or(COORD_MIN, |b| b.x1 - (c.x1 - sr));
        let self_w = self.self_size(id).w - (c.width() - sr - sl) - n.scroll.x;
        child.max(self_w)
    }

    // ---- Scrolling ---------------------------------------------------------------------------

    /// Moves the content of `id` by `(dx, dy)` without events other than `Scroll` and without
    /// bounds (LVGL `lv_obj_scroll_by_raw`): the offset decreases by the delta, the
    /// non-floating children's subtrees move by it and the node's area is invalidated once.
    /// Returns `false` when `id` was deleted by a `Scroll` handler.
    pub(crate) fn scroll_by_raw(&mut self, id: NodeId, dx: i32, dy: i32) -> bool {
        if dx == 0 && dy == 0 {
            return self.tree.contains(id);
        }
        let Some(n) = self.tree.node_mut(id) else {
            return false;
        };
        n.scroll = Point::new(n.scroll.x - dx, n.scroll.y - dy);
        let scroll = n.scroll;
        let overflow = n.flags.contains(ObjFlags::OVERFLOW_VISIBLE);
        if overflow {
            // Children may draw outside the node: their old areas too.
            self.invalidate_children_areas(id);
        }
        self.move_children_by(id, dx, dy);
        twine_core::trace!(
            target: "twine::engine",
            "scroll {} by ({}, {}) -> {:?}",
            fmt_node_id(id),
            dx,
            dy,
            scroll
        );
        self.send_event(id, EventCode::Scroll, EventParam::None);
        if !self.tree.contains(id) {
            return false;
        }
        let area = self.coords(id);
        self.invalidate_rect_of(id, area, InvalidateReason::Scroll);
        if overflow {
            self.invalidate_children_areas(id);
        }
        true
    }

    /// Moves the subtrees of the non-floating children of `id` by `(dx, dy)` (LVGL
    /// `lv_obj_move_children_by(.., ignore_floating = true)`).
    fn move_children_by(&mut self, id: NodeId, dx: i32, dy: i32) {
        let mut child = self.tree.node(id).and_then(crate::Node::first_child);
        while let Some(c) = child {
            let next = self.tree.node(c).and_then(crate::Node::next_sibling);
            if !self.has_flag(c, ObjFlags::FLOATING) {
                let mut cur = Some(c);
                while let Some(x) = cur {
                    if let Some(xn) = self.tree.node_mut(x) {
                        xn.coords = xn.coords.translate(dx, dy);
                    }
                    cur = self.next_in_subtree(c, x);
                }
            }
            child = next;
        }
    }

    /// Invalidates the areas (with extra draw size) of every descendant of `id`.
    fn invalidate_children_areas(&mut self, id: NodeId) {
        let mut cur = self.tree.node(id).and_then(crate::Node::first_child);
        while let Some(c) = cur {
            self.invalidate(c, InvalidateReason::Scroll);
            cur = self.next_in_subtree(id, c);
        }
    }

    /// Scrolls the content of `id` by `(dx, dy)` (LVGL `lv_obj_scroll_by`; positive `dy`
    /// moves the content **down**, reducing the offset). Not bounded: see
    /// [`scroll_by_bounded`](Self::scroll_by_bounded).
    ///
    /// Without `anim` the move happens at once, framed by `ScrollBegin` and `ScrollEnd` (and
    /// running scroll animations of `id` stop, each with its `ScrollEnd`). With `anim` each
    /// axis animates with `EaseOut` over a time from the distance at half the display width
    /// (height) per second, clamped to [`SCROLL_ANIM_TIME_MIN`]..=[`SCROLL_ANIM_TIME_MAX`];
    /// `ScrollBegin` is sent at the start and `ScrollEnd` when it completes or is replaced.
    ///
    /// ```
    /// use twine_core::Rect;
    /// use twine_engine::{Engine, EngineConfig, Obj};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let list = e.create_root(Box::new(Obj)).unwrap();
    /// e.place(list, Rect::from_xywh(0, 0, 100, 100));
    /// let item = e.create(list, Box::new(Obj)).unwrap();
    /// e.set_size(item, 100, 300);
    /// e.update_layout();
    /// e.scroll_by(list, 0, -40, false); // content up by 40
    /// assert_eq!(e.scroll_offset(list).y, 40);
    /// assert_eq!(e.coords(item).y0, -40);
    /// assert_eq!(e.scroll_bottom(list), 160);
    /// ```
    pub fn scroll_by(&mut self, id: NodeId, dx: i32, dy: i32, anim: bool) {
        if !self.tree.contains(id) {
            twine_core::warn!(target: "twine::scroll", "scroll_by: node {} not found", fmt_node_id(id));
            return;
        }
        if dx == 0 && dy == 0 {
            return;
        }
        if anim {
            let res = self.display_res(id);
            if dx != 0 && !self.scroll_anim_start(id, Axis::X, dx, res.w) {
                return;
            }
            if dy != 0 {
                self.scroll_anim_start(id, Axis::Y, dy, res.h);
            }
        } else {
            self.stop_scroll_anim(id);
            self.send_event(id, EventCode::ScrollBegin, EventParam::None);
            if !self.tree.contains(id) || !self.scroll_by_raw(id, dx, dy) {
                return;
            }
            self.send_event(id, EventCode::ScrollEnd, EventParam::None);
        }
    }

    /// The resolution of the display showing `id` (the default display, else 320 × 240).
    fn display_res(&self, id: NodeId) -> twine_core::Size {
        self.display_of(id)
            .or(self.default_display)
            .and_then(|d| self.displays.get(d.index()))
            .map_or(twine_core::Size::new(320, 240), |d| d.area().size())
    }

    /// Starts the scroll animation of one axis. Returns `false` if `id` was deleted.
    fn scroll_anim_start(&mut self, id: NodeId, axis: Axis, delta: i32, res: i32) -> bool {
        let t = speed_clamped_time(
            (res.max(0) as u32) >> 1,
            SCROLL_ANIM_TIME_MIN,
            SCROLL_ANIM_TIME_MAX,
            delta.unsigned_abs(),
        );
        let cur = self.scroll_offset(id);
        let (start, prop) = match axis {
            Axis::X => (cur.x, AnimProp::ScrollX),
            Axis::Y => (cur.y, AnimProp::ScrollY),
        };
        self.send_event(id, EventCode::ScrollBegin, EventParam::None);
        if !self.tree.contains(id) {
            return false;
        }
        // Like `lv_anim_start`, a running animation of the axis is replaced (its end is sent).
        self.stop_scroll_anim_axis(id, prop);
        twine_core::trace!(
            target: "twine::scroll",
            "scroll {} {:?} anim {} -> {} in {:?}",
            fmt_node_id(id),
            axis,
            start,
            start - delta,
            t
        );
        let a = Anim::new(start, start - delta)
            .duration(t)
            .easing(Easing::EaseOut)
            .on_complete(move |cx| {
                crate::defer(cx, move |e| {
                    if e.tree.contains(id) {
                        e.send_event(id, EventCode::ScrollEnd, EventParam::None);
                    }
                });
            });
        self.anim_start(id, prop, a);
        self.tree.contains(id)
    }

    /// Stops the scroll animation of `prop` on `id`, sending `ScrollEnd` if one ran.
    fn stop_scroll_anim_axis(&mut self, id: NodeId, prop: AnimProp) {
        if self.anim.timeline.remove_target_prop(id.to_raw(), prop) > 0 && self.tree.contains(id) {
            self.send_event(id, EventCode::ScrollEnd, EventParam::None);
        }
    }

    /// Stops the scroll animations of `id` where they are (LVGL `lv_obj_stop_scroll_anim`),
    /// sending `ScrollEnd` for each.
    pub fn stop_scroll_anim(&mut self, id: NodeId) {
        self.stop_scroll_anim_axis(id, AnimProp::ScrollY);
        self.stop_scroll_anim_axis(id, AnimProp::ScrollX);
    }

    /// Scrolls by `(dx, dy)` but not further than the content allows (LVGL
    /// `lv_obj_scroll_by_bounded`; the layout is updated first).
    pub fn scroll_by_bounded(&mut self, id: NodeId, dx: i32, dy: i32, anim: bool) {
        if dx == 0 && dy == 0 {
            return;
        }
        self.update_layout();
        let Some(n) = self.tree.node(id) else {
            twine_core::warn!(target: "twine::scroll", "scroll_by_bounded: node {} not found", fmt_node_id(id));
            return;
        };
        // In LVGL's raw terms: the content offset is the negated scroll offset.
        let x_current = -n.scroll.x;
        let mut x_bounded = x_current + dx;
        if self.is_rtl(id, Part::Main) {
            x_bounded = x_bounded.max(0);
            if x_bounded > 0 {
                let max = (self.scroll_left(id) + self.scroll_right(id)).max(0);
                x_bounded = x_bounded.min(max);
            }
        } else {
            x_bounded = x_bounded.min(0);
            if x_bounded < 0 {
                let max = (self.scroll_left(id) + self.scroll_right(id)).max(0);
                x_bounded = x_bounded.max(-max);
            }
        }
        let y_current = -self.scroll_offset(id).y;
        let mut y_bounded = (y_current + dy).min(0);
        if y_bounded < 0 {
            let max = (self.scroll_top(id) + self.scroll_bottom(id)).max(0);
            y_bounded = y_bounded.max(-max);
        }
        let (dx, dy) = (x_bounded - x_current, y_bounded - y_current);
        if dx != 0 || dy != 0 {
            self.scroll_by(id, dx, dy, anim);
        }
    }

    /// Scrolls to the offset `(x, y)`, bounded by the content (LVGL `lv_obj_scroll_to`).
    /// Scrolling to the current position does nothing.
    pub fn scroll_to(&mut self, id: NodeId, x: i32, y: i32, anim: bool) {
        self.scroll_to_x(id, x, anim);
        self.scroll_to_y(id, y, anim);
    }

    /// Scrolls horizontally to offset `x` (bounded; a running horizontal scroll animation
    /// stops first).
    pub fn scroll_to_x(&mut self, id: NodeId, x: i32, anim: bool) {
        self.stop_scroll_anim_axis(id, AnimProp::ScrollX);
        let diff = -x + self.scroll_offset(id).x;
        self.scroll_by_bounded(id, diff, 0, anim);
    }

    /// Scrolls vertically to offset `y` (bounded; a running vertical scroll animation stops
    /// first).
    pub fn scroll_to_y(&mut self, id: NodeId, y: i32, anim: bool) {
        self.stop_scroll_anim_axis(id, AnimProp::ScrollY);
        let diff = -y + self.scroll_offset(id).y;
        self.scroll_by_bounded(id, 0, diff, anim);
    }

    /// Scrolls the parent of `child` so that `child` becomes visible in its content area, by
    /// the smallest distance; with snapping on the parent the child is aligned to the snap
    /// position instead (LVGL `lv_obj_scroll_to_view`). The layout is updated first.
    pub fn scroll_to_view(&mut self, child: NodeId, anim: bool) {
        self.update_layout();
        let Some(n) = self.tree.node(child) else {
            twine_core::warn!(target: "twine::scroll", "scroll_to_view: node {} not found", fmt_node_id(child));
            return;
        };
        let area = n.coords;
        let mut p = Point::ZERO;
        self.scroll_area_into_view(area, child, &mut p, anim);
    }

    /// Like [`scroll_to_view`](Self::scroll_to_view) for every ancestor, from the parent
    /// outwards, so that `child` becomes visible through nested scrollable containers (LVGL
    /// `lv_obj_scroll_to_view_recursive`).
    pub fn scroll_to_view_recursive(&mut self, child: NodeId, anim: bool) {
        self.update_layout();
        if !self.tree.contains(child) {
            twine_core::warn!(
                target: "twine::scroll",
                "scroll_to_view_recursive: node {} not found",
                fmt_node_id(child)
            );
            return;
        }
        let mut p = Point::ZERO;
        let mut cur = child;
        while let Some(parent) = self.tree.parent(cur) {
            let area = self.coords(child);
            self.scroll_area_into_view(area, cur, &mut p, anim);
            if !self.tree.contains(child) || !self.tree.contains(parent) {
                return;
            }
            cur = parent;
        }
    }

    /// LVGL `scroll_area_into_view`: scrolls the parent of `child` so that `area` (or, with
    /// snapping, `child`'s coordinates aligned to the snap position) is visible. `sv`
    /// accumulates the animated scrolls not applied yet (for the outer levels).
    fn scroll_area_into_view(&mut self, area: Rect, child: NodeId, sv: &mut Point, anim: bool) {
        let Some(parent) = self.tree.parent(child) else {
            return;
        };
        if !self.has_flag(parent, ObjFlags::SCROLLABLE) {
            return;
        }
        let attrs = self.scroll_attrs(parent);
        let pc = self.coords(parent);
        let cc = self.coords(child);
        let (sleft, stop, sright, sbottom) = self.space(parent);

        let a = if attrs.snap_y == ScrollSnap::None {
            area
        } else {
            cc
        };
        let top_diff = pc.y0 + stop - a.y0 - sv.y;
        let bottom_diff = -(pc.y1 - sbottom - a.y1 - sv.y);
        let parent_h = pc.height() - stop - sbottom;
        let mut y_scroll = 0;
        if top_diff >= 0 && bottom_diff >= 0 {
            y_scroll = 0;
        } else if top_diff > 0 {
            y_scroll = top_diff;
            // Do not scroll in beyond the start.
            if self.scroll_top(parent) - y_scroll < 0 {
                y_scroll = 0;
            }
        } else if bottom_diff > 0 {
            y_scroll = -bottom_diff;
            if self.scroll_bottom(parent) + y_scroll < 0 {
                y_scroll = 0;
            }
        }
        match attrs.snap_y {
            ScrollSnap::Start => y_scroll += pc.y0 + stop - (a.y0 + y_scroll),
            ScrollSnap::End => y_scroll += pc.y1 - sbottom - (a.y1 + y_scroll),
            ScrollSnap::Center => {
                y_scroll += pc.y0 + stop + parent_h / 2 - (a.height() / 2 + a.y0 + y_scroll);
            }
            ScrollSnap::None => {}
        }

        let a = if attrs.snap_x == ScrollSnap::None {
            area
        } else {
            cc
        };
        let left_diff = pc.x0 + sleft - a.x0 - sv.x;
        let right_diff = -(pc.x1 - sright - a.x1 - sv.x);
        let mut x_scroll = 0;
        if left_diff >= 0 && right_diff >= 0 {
            x_scroll = 0;
        } else if left_diff > 0 {
            x_scroll = left_diff;
            if self.scroll_left(parent) - x_scroll < 0 {
                x_scroll = 0;
            }
        } else if right_diff > 0 {
            x_scroll = -right_diff;
            if self.scroll_right(parent) + x_scroll < 0 {
                x_scroll = 0;
            }
        }
        let parent_w = pc.width() - sleft - sright;
        match attrs.snap_x {
            ScrollSnap::Start => x_scroll += pc.x0 + sleft - (a.x0 + x_scroll),
            ScrollSnap::End => x_scroll += pc.x1 - sright - (a.x1 + x_scroll),
            ScrollSnap::Center => {
                x_scroll += pc.x0 + sleft + parent_w / 2 - (a.width() / 2 + a.x0 + x_scroll);
            }
            ScrollSnap::None => {}
        }

        // Remove any pending scroll animations.
        self.stop_scroll_anim(parent);
        let dir = attrs.dir;
        if !dir.contains(Dir::LEFT) && x_scroll < 0 {
            x_scroll = 0;
        }
        if !dir.contains(Dir::RIGHT) && x_scroll > 0 {
            x_scroll = 0;
        }
        if !dir.contains(Dir::TOP) && y_scroll < 0 {
            y_scroll = 0;
        }
        if !dir.contains(Dir::BOTTOM) && y_scroll > 0 {
            y_scroll = 0;
        }
        if anim {
            sv.x += x_scroll;
            sv.y += y_scroll;
        }
        self.scroll_by(parent, x_scroll, y_scroll, anim);
    }

    /// Whether `id` is being scrolled: dragged or thrown by a pointer, or animated (LVGL
    /// `lv_obj_is_scrolling`).
    #[must_use]
    pub fn is_scrolling(&self, id: NodeId) -> bool {
        if self.indev_scroll_dir(id).is_some() {
            return true;
        }
        let key = id.to_raw();
        self.anim.timeline.iter().any(|(_, a)| {
            matches!(a.target, AnimTarget::Node(k, AnimProp::ScrollX | AnimProp::ScrollY) if k == key)
        })
    }

    /// Scrolls `id` so that its nearest `SNAPPABLE` child inside its area aligns with the snap
    /// position of each snapping axis (LVGL `lv_obj_update_snap`).
    pub fn update_snap(&mut self, id: NodeId, anim: bool) {
        self.update_layout();
        let Some(c) = self.tree.node(id).map(crate::Node::coords) else {
            return;
        };
        let mut x = self.find_snap_point_x(id, c.x0, c.x1 - 1, 0);
        let mut y = self.find_snap_point_y(id, c.y0, c.y1 - 1, 0);
        if x == COORD_MAX || x == COORD_MIN {
            x = 0;
        }
        if y == COORD_MAX || y == COORD_MIN {
            y = 0;
        }
        self.scroll_by(id, x, y, anim);
    }

    /// Scrolls back content that ended up scrolled in beyond its end (e.g. after the content
    /// shrank), for axes without snapping (LVGL `lv_obj_readjust_scroll`).
    pub(crate) fn readjust_scroll(&mut self, id: NodeId, anim: bool) {
        // The offsets are checked first: most nodes are not scrolled at all.
        if self.scroll_snap_y(id) == ScrollSnap::None {
            let st = self.scroll_top(id);
            if st > 0 {
                let sb = self.scroll_bottom(id);
                if sb < 0 {
                    self.scroll_by(id, 0, st.min(-sb), anim);
                }
            }
        }
        if self.scroll_snap_x(id) == ScrollSnap::None {
            if self.is_rtl(id, Part::Main) {
                let sr = self.scroll_right(id);
                if sr > 0 {
                    let sl = self.scroll_left(id);
                    if sl < 0 {
                        self.scroll_by(id, sl, 0, anim);
                    }
                }
            } else {
                let sl = self.scroll_left(id);
                if sl > 0 {
                    let sr = self.scroll_right(id);
                    if sr < 0 {
                        self.scroll_by(id, sl.min(-sr), 0, anim);
                    }
                }
            }
        }
    }

    // ---- Snap points (LVGL `lv_indev_scroll.c`) ----------------------------------------------

    /// LVGL `find_snap_point_x`: the scroll distance that aligns the nearest `SNAPPABLE`
    /// child whose snap point (moved by `ofs`) lies in `min..=max` (absolute), or
    /// [`COORD_MAX`] when there is none.
    pub(crate) fn find_snap_point_x(&self, id: NodeId, min: i32, max: i32, ofs: i32) -> i32 {
        self.find_snap_point(id, Axis::X, min, max, ofs)
    }

    /// LVGL `find_snap_point_y` (see [`find_snap_point_x`](Self::find_snap_point_x)).
    pub(crate) fn find_snap_point_y(&self, id: NodeId, min: i32, max: i32, ofs: i32) -> i32 {
        self.find_snap_point(id, Axis::Y, min, max, ofs)
    }

    fn find_snap_point(&self, id: NodeId, axis: Axis, min: i32, max: i32, ofs: i32) -> i32 {
        let attrs = self.scroll_attrs(id);
        let align = match axis {
            Axis::X => attrs.snap_x,
            Axis::Y => attrs.snap_y,
        };
        if align == ScrollSnap::None {
            return COORD_MAX;
        }
        let Some(n) = self.tree.node(id) else {
            return COORD_MAX;
        };
        let pad = self.cached_main(id).pad;
        let c = n.coords;
        // (start, end-inclusive, size, pad start, pad end) of the parent on the axis.
        let (p0, p1, ps, pad0, pad1) = match axis {
            Axis::X => (c.x0, c.x1 - 1, c.width(), pad.left, pad.right),
            Axis::Y => (c.y0, c.y1 - 1, c.height(), pad.top, pad.bottom),
        };
        let parent_point = match align {
            ScrollSnap::Start => p0 + pad0,
            ScrollSnap::End => p1 - pad1,
            _ => p0 + pad0 + (ps - pad0 - pad1) / 2,
        };
        let mut dist = COORD_MAX;
        for ch in self.tree.children(id) {
            let Some(cn) = self.tree.node(ch) else { continue };
            if cn.is_hidden()
                || cn.flags.contains(ObjFlags::FLOATING)
                || !cn.flags.contains(ObjFlags::SNAPPABLE)
            {
                continue;
            }
            let cc = cn.coords;
            let (c0, c1, cs) = match axis {
                Axis::X => (cc.x0, cc.x1 - 1, cc.width()),
                Axis::Y => (cc.y0, cc.y1 - 1, cc.height()),
            };
            let child_point = match align {
                ScrollSnap::Start => c0,
                ScrollSnap::End => c1,
                _ => c0 + cs / 2,
            } + ofs;
            if child_point >= min && child_point <= max {
                let d = child_point - parent_point;
                if d.abs() < dist.abs() {
                    dist = d;
                }
            }
        }
        if dist == COORD_MAX { COORD_MAX } else { -dist }
    }

    /// LVGL `has_more_snap_points`: whether there are snap points before (start) and after
    /// (end) the snap position on `axis`.
    pub(crate) fn has_more_snap_points(&self, id: NodeId, axis: Axis) -> (bool, bool) {
        let attrs = self.scroll_attrs(id);
        let snap = match axis {
            Axis::X => attrs.snap_x,
            Axis::Y => attrs.snap_y,
        };
        let c = self.coords(id);
        let pad = self.cached_main(id).pad;
        let (p0, p1, ps, pad0, pad1) = match axis {
            Axis::X => (c.x0, c.x1 - 1, c.width(), pad.left, pad.right),
            Axis::Y => (c.y0, c.y1 - 1, c.height(), pad.top, pad.bottom),
        };
        let v = match snap {
            ScrollSnap::Center => p0 + (ps - pad0 - pad1) / 2 + pad0,
            ScrollSnap::Start => p0 + pad0,
            ScrollSnap::End => p1 - pad1,
            ScrollSnap::None => 0,
        };
        let end = self.find_snap_point(id, axis, v + 1, COORD_MAX, 0) != COORD_MAX;
        let start = self.find_snap_point(id, axis, COORD_MIN, v - 1, 0) != COORD_MAX;
        (start, end)
    }

    /// The snapping of `id` on `axis`.
    pub(crate) fn snap_of(&self, id: NodeId, axis: Axis) -> ScrollSnap {
        match axis {
            Axis::X => self.scroll_snap_x(id),
            Axis::Y => self.scroll_snap_y(id),
        }
    }

    /// Applies an animation value of `AnimProp::ScrollX` / `ScrollY`: moves the content so the
    /// offset becomes `v` (unbounded, LVGL `scroll_x_anim` / `scroll_y_anim`).
    pub(crate) fn anim_scroll_to(&mut self, id: NodeId, axis: Axis, v: i32) {
        let cur = self.scroll_offset(id);
        match axis {
            Axis::X => {
                self.scroll_by_raw(id, cur.x - v, 0);
            }
            Axis::Y => {
                self.scroll_by_raw(id, 0, cur.y - v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_clamped_matches_lvgl() {
        // 320 px wide display: 160 px/s, stored as 16 (10 px/s units).
        let t = |d| speed_clamped_time(160, SCROLL_ANIM_TIME_MIN, SCROLL_ANIM_TIME_MAX, d);
        assert_eq!(t(10), Duration::ms(200));
        assert_eq!(t(40), Duration::ms(250)); // 40 · 100 / 16
        assert_eq!(t(100), Duration::ms(400));
        assert_eq!(
            speed_clamped_time(0, SCROLL_ANIM_TIME_MIN, SCROLL_ANIM_TIME_MAX, 5),
            Duration::ms(200)
        );
    }

    #[test]
    fn scroll_attrs_fit_in_four_bytes() {
        assert!(core::mem::size_of::<ScrollAttrs>() <= 4);
    }
}
