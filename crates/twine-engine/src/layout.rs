//! The layout pass: dirty scheduling ([`LayoutDirty`]), the [`LayoutTree`] view of the widget
//! tree used by `twine-layout`, delta moves, invalidation of old and new areas, and the
//! `SizeChanged` / `LayoutChanged` events. Also the LVGL-like imperative layout setters
//! (`set_size`, `set_pos`, `align`, `set_flex_flow`, `set_grid_cell`…).

use alloc::vec::Vec;

use twine_core::{Point, Rect, Size};
use twine_layout::{AlignTo, LayoutFlags, LayoutScratch, LayoutTree, layout_children_with};
use twine_style::{
    Align, FlexAlign, FlexFlow, GridAlign, GridTrack, LayoutKind, Length, Part, PropId, Selector, StyleProp,
    StyleValue,
};

use crate::{
    Engine, EventCode, EventParam, InvalidateReason, LayoutDirty, MeasureCx, NodeId, ObjFlags, fmt_node_id,
};

/// How many times [`Engine::update_layout`] lays out again when event handlers changed the
/// layout (LVGL's limit as well).
pub const MAX_LAYOUT_ITERATIONS: u8 = 4;

/// What the last [`Engine::update_layout`] did (see [`Engine::layout_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LayoutStats {
    /// Subtrees laid out (summed over the iterations).
    pub roots: u32,
    /// Nodes whose rectangle changed (summed over the iterations).
    pub moved: u32,
    /// Layout iterations (0 when nothing was dirty; more than 1 when event handlers changed
    /// the layout again).
    pub iterations: u8,
    /// Whether the pass stopped at [`MAX_LAYOUT_ITERATIONS`] with work left.
    pub converged: bool,
}

/// State of the layout pass kept in the engine: pending flag, scratch buffers reused across
/// passes (no allocation in steady state) and statistics.
#[derive(Debug, Default)]
pub(crate) struct LayoutState {
    /// Some node has pending layout work.
    pub(crate) pending: bool,
    scratch: LayoutScratch<NodeId>,
    /// `(node, old, new)` for every node whose rectangle changed in the current iteration.
    moved: Vec<(NodeId, Rect, Rect)>,
    /// Nodes whose children are laid out in the current iteration.
    targets: Vec<NodeId>,
    /// Parents that receive `LayoutChanged`.
    parents: Vec<NodeId>,
    /// Wrappers (`LAYOUT_PASSTHROUGH`) whose box is recomputed, with their depth.
    wrappers: Vec<(NodeId, usize)>,
    /// Nodes whose scroll position is checked after the pass (resized, or a child deleted).
    pub(crate) readjust: Vec<NodeId>,
    /// A pass is running (nested calls, e.g. from event handlers, return at once).
    running: bool,
    stats: LayoutStats,
    /// Nodes whose children the layout enumerated in the last pass (feature `debug-checks`).
    #[cfg(feature = "debug-checks")]
    visited: core::cell::RefCell<Vec<NodeId>>,
}

/// The widget tree as seen by `twine-layout`: styles of the `Main` part, widget content
/// sizes, flags, and coordinates. `set_coords` moves the node's subtree along (children
/// coordinates are absolute) and records the change; invalidation and events happen after
/// the layout of all dirty subtrees.
struct EngineLayout<'a> {
    engine: &'a mut Engine,
}

impl LayoutTree for EngineLayout<'_> {
    type Id = NodeId;

    fn children(&self, id: NodeId, out: &mut dyn FnMut(NodeId)) {
        self.engine.layout_children_of(id, out);
    }

    fn style_i32(&self, id: NodeId, prop: PropId) -> i32 {
        // Paddings and the border width come from the node's style cache.
        match prop {
            PropId::PadLeft => self.engine.cached_main(id).pad.left,
            PropId::PadTop => self.engine.cached_main(id).pad.top,
            PropId::PadRight => self.engine.cached_main(id).pad.right,
            PropId::PadBottom => self.engine.cached_main(id).pad.bottom,
            PropId::BorderWidth => self.engine.cached_main(id).border_width,
            _ => self.style_prop(id, prop).as_i32().unwrap_or(0),
        }
    }

    fn style_prop(&self, id: NodeId, prop: PropId) -> StyleValue {
        #[cfg(feature = "std")]
        if matches!(prop, PropId::Width | PropId::Height) && self.engine.tree.parent(id).is_none() {
            // Screens and layers have the size of their display, whatever their style says.
            let c = self.engine.coords(id);
            let v = if prop == PropId::Width {
                c.width()
            } else {
                c.height()
            };
            return StyleValue::Length(Length::Px(v));
        }
        self.engine.style_prop(id, Part::Main, prop)
    }

    fn content_size(&self, id: NodeId) -> Size {
        self.engine.tree.node(id).map_or(Size::ZERO, |n| {
            n.widget.content_size(&MeasureCx::new(self.engine, id))
        })
    }

    fn flags(&self, id: NodeId) -> LayoutFlags {
        let Some(n) = self.engine.tree.node(id) else {
            return LayoutFlags::HIDDEN;
        };
        let f = n.flags;
        let mut out = LayoutFlags::empty();
        out.set(LayoutFlags::HIDDEN, f.contains(ObjFlags::HIDDEN));
        out.set(LayoutFlags::IGNORE_LAYOUT, f.contains(ObjFlags::IGNORE_LAYOUT));
        out.set(LayoutFlags::FLOATING, f.contains(ObjFlags::FLOATING));
        out.set(
            LayoutFlags::FLEX_IN_NEW_TRACK,
            f.contains(ObjFlags::FLEX_IN_NEW_TRACK),
        );
        out.set(LayoutFlags::SCROLLABLE_CONTENT, f.contains(ObjFlags::SCROLLABLE));
        out
    }

    fn set_coords(&mut self, id: NodeId, r: Rect) {
        let e = &mut *self.engine;
        let Some(n) = e.tree.node_mut(id) else {
            return;
        };
        let old = n.coords;
        n.coords = r;
        if let Some(p) = n.parent() {
            e.forget_children_box(p);
        }
        let (dx, dy) = (r.x0 - old.x0, r.y0 - old.y0);
        if dx != 0 || dy != 0 {
            // Children are absolute: move the subtree along. Children the layout does not
            // move any further then keep their (shifted) rectangle and need no work at all.
            let mut cur = e.tree.node(id).and_then(crate::Node::first_child);
            while let Some(c) = cur {
                if let Some(cn) = e.tree.node_mut(c) {
                    cn.coords = cn.coords.translate(dx, dy);
                }
                cur = e.next_in_subtree(id, c);
            }
        }
        e.layout.moved.push((id, old, r));
    }

    fn coords(&self, id: NodeId) -> Rect {
        self.engine.coords(id)
    }

    fn align_to(&self, id: NodeId) -> Option<AlignTo<NodeId>> {
        self.engine
            .tree
            .node(id)
            .and_then(|n| n.align_to)
            .filter(|a| self.engine.tree.contains(a.base))
    }

    fn scroll(&self, id: NodeId) -> Point {
        // `twine-layout` asks for the scroll offset once per node whose children it arranges.
        #[cfg(feature = "debug-checks")]
        self.engine.layout.visited.borrow_mut().push(id);
        self.engine.tree.node(id).map_or(Point::ZERO, crate::Node::scroll)
    }
}

impl Engine {
    /// The children of `id` as the layout sees them: `LAYOUT_PASSTHROUGH` wrappers are
    /// replaced by their own (layout) children; hidden wrappers contribute nothing.
    pub(crate) fn layout_children_of(&self, id: NodeId, out: &mut dyn FnMut(NodeId)) {
        for c in self.tree.children(id) {
            match self.tree.node(c) {
                Some(n) if n.flags.contains(ObjFlags::LAYOUT_PASSTHROUGH) => {
                    if !n.flags.contains(ObjFlags::HIDDEN) {
                        self.layout_children_of(c, out);
                    }
                }
                _ => out(c),
            }
        }
    }

    /// Whether `id` is a `LAYOUT_PASSTHROUGH` wrapper.
    fn is_passthrough(&self, id: NodeId) -> bool {
        self.tree
            .node(id)
            .is_some_and(|n| n.flags.contains(ObjFlags::LAYOUT_PASSTHROUGH))
    }

    /// `id`, or its nearest ancestor that is not a `LAYOUT_PASSTHROUGH` wrapper (the node
    /// that really lays out `id`'s children).
    fn layout_owner(&self, id: NodeId) -> NodeId {
        let mut cur = id;
        while self.is_passthrough(cur) {
            match self.tree.parent(cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
        cur
    }

    /// Sets the coordinates of the wrappers around the nodes moved by the last layout to the
    /// bounding box of their children (innermost wrappers first). Wrappers draw nothing, so
    /// nothing is invalidated.
    fn update_passthrough_boxes(&mut self) {
        let mut wrappers = core::mem::take(&mut self.layout.wrappers);
        wrappers.clear();
        for &(id, ..) in &self.layout.moved {
            let mut cur = self.tree.parent(id);
            while let Some(p) = cur.filter(|p| self.is_passthrough(*p)) {
                if !wrappers.iter().any(|(w, _)| *w == p) {
                    wrappers.push((p, self.tree.ancestors(p).count()));
                }
                cur = self.tree.parent(p);
            }
        }
        wrappers.sort_unstable_by_key(|w| core::cmp::Reverse(w.1));
        for &(w, _) in &wrappers {
            self.refresh_passthrough_box(w);
        }
        self.layout.wrappers = wrappers;
    }

    /// Sets the coordinates of wrapper `w` to the bounding box of its visible children (an
    /// empty rectangle at its old origin when it has none).
    pub(crate) fn refresh_passthrough_box(&mut self, w: NodeId) {
        let mut bbox: Option<Rect> = None;
        for c in self.tree.children(w) {
            let Some(n) = self.tree.node(c) else { continue };
            if n.is_hidden() || n.coords.is_empty() {
                continue;
            }
            bbox = Some(bbox.map_or(n.coords, |b| b.union(&n.coords)));
        }
        if let Some(n) = self.tree.node_mut(w) {
            n.coords = bbox.unwrap_or(Rect::new(n.coords.x0, n.coords.y0, n.coords.x0, n.coords.y0));
        }
    }

    /// Whether the `Width` or `Height` of `id` is [`Length::Content`] (its size depends on
    /// its children). Screens and layers are never content-sized.
    fn is_content_sized_any(&self, id: NodeId) -> bool {
        if self.tree.parent(id).is_none() {
            return false;
        }
        let content = |p| self.style_prop(id, Part::Main, p).as_length() == Some(Length::Content);
        content(PropId::Width) || content(PropId::Height)
    }

    /// Marks pending layout work on `id`: `SELF` also marks the parent `CHILDREN`; a node
    /// whose children changed and whose size depends on them (content-sized) is marked
    /// `SELF` too, and so on up the tree. Every ancestor learns that a descendant is dirty.
    pub(crate) fn mark_layout(&mut self, id: NodeId, what: LayoutDirty) {
        if !self.tree.contains(id) {
            return;
        }
        let mut cur = id;
        let mut w = what;
        loop {
            if w.contains(LayoutDirty::CHILDREN)
                && !w.contains(LayoutDirty::SELF)
                && (self.is_passthrough(cur) || self.is_content_sized_any(cur))
            {
                // A wrapper's children are laid out by its parent (as are the children of a
                // content-sized node, whose own size depends on them).
                w |= LayoutDirty::SELF;
            }
            if let Some(n) = self.tree.node_mut(cur) {
                n.layout_dirty |= w;
            }
            // The scroll extents depend on the children's rectangles and margins.
            if w.contains(LayoutDirty::CHILDREN) {
                self.forget_children_box(cur);
            }
            if w.contains(LayoutDirty::SELF) {
                if let Some(p) = self.tree.parent(cur) {
                    self.forget_children_box(p);
                }
            }
            match self.tree.parent(cur) {
                Some(p) if w.contains(LayoutDirty::SELF) => {
                    cur = p;
                    w = LayoutDirty::CHILDREN;
                }
                _ => break,
            }
        }
        // Guide the pass: every ancestor of the topmost marked node has a dirty descendant.
        let mut a = self.tree.parent(cur);
        while let Some(p) = a {
            let Some(n) = self.tree.node_mut(p) else { break };
            if n.layout_dirty.contains(LayoutDirty::DESCENDANTS) {
                break;
            }
            n.layout_dirty |= LayoutDirty::DESCENDANTS;
            a = n.parent();
        }
        self.layout.pending = true;
    }

    /// Schedules a layout of `id` (LVGL `lv_obj_mark_layout_as_dirty`), e.g. after a widget's
    /// content size changed. Layout properties set through styles do this automatically.
    pub fn mark_layout_dirty(&mut self, id: NodeId) {
        self.mark_layout(id, LayoutDirty::SELF);
    }

    /// Whether a layout pass is pending (some node is marked dirty).
    #[must_use]
    pub fn layout_pending(&self) -> bool {
        self.layout.pending
    }

    /// Statistics of the last [`update_layout`](Self::update_layout) that had work to do.
    #[must_use]
    pub fn layout_stats(&self) -> LayoutStats {
        self.layout.stats
    }

    /// The nodes whose children were laid out by the last layout pass (feature
    /// `debug-checks`; empty otherwise). Lets tests check that only dirty subtrees are
    /// processed.
    #[must_use]
    pub fn layout_visited(&self) -> Vec<NodeId> {
        #[cfg(feature = "debug-checks")]
        {
            self.layout.visited.borrow().clone()
        }
        #[cfg(not(feature = "debug-checks"))]
        {
            Vec::new()
        }
    }

    /// Runs the layout pass (update cycle step 7, called by [`step`](Self::step)): every dirty
    /// subtree is laid out once, top-down, from its topmost dirty node (whose own rectangle
    /// is kept: its parent was not affected). Nodes whose rectangle changed then invalidate
    /// their old and new area (their subtree moves along without a relayout of its own),
    /// resized nodes get `SizeChanged` ([`EventParam::Area`] = old coordinates) and their
    /// parents `LayoutChanged`. When event handlers change the layout again the pass repeats,
    /// at most [`MAX_LAYOUT_ITERATIONS`] times (then `warn!` and the rest waits for the next
    /// update). Nothing dirty: returns immediately.
    pub fn update_layout(&mut self) {
        if self.layout.running {
            return;
        }
        if !self.layout.pending {
            self.readjust_scrolls();
            return;
        }
        self.layout.running = true;
        let mut stats = LayoutStats {
            converged: true,
            ..LayoutStats::default()
        };
        #[cfg(feature = "debug-checks")]
        self.layout.visited.borrow_mut().clear();
        while self.layout.pending {
            if stats.iterations == MAX_LAYOUT_ITERATIONS {
                stats.converged = false;
                twine_core::warn!(
                    target: "twine::layout",
                    "layout did not converge after {} iterations (event handlers keep changing it)",
                    MAX_LAYOUT_ITERATIONS
                );
                break;
            }
            stats.iterations += 1;
            self.layout.pending = false;
            self.collect_layout_targets();
            stats.roots += self.layout.targets.len() as u32;
            self.layout_targets();
            stats.moved += self.layout.moved.len() as u32;
            self.apply_moves();
        }
        twine_core::debug!(
            target: "twine::layout",
            "layout roots={} moved={} iterations={}",
            stats.roots,
            stats.moved,
            stats.iterations
        );
        self.layout.stats = stats;
        self.layout.running = false;
        self.readjust_scrolls();
    }

    /// Scrolls back the content of resized nodes (and of nodes that lost a child) that ended
    /// up scrolled in beyond its end (LVGL `readjust_scroll_after_layout`).
    fn readjust_scrolls(&mut self) {
        if self.layout.readjust.is_empty() {
            return;
        }
        let mut list = core::mem::take(&mut self.layout.readjust);
        for &id in &list {
            if self.tree.contains(id) {
                self.readjust_scroll(id, false);
            }
        }
        list.clear();
        if self.layout.readjust.is_empty() {
            self.layout.readjust = list;
        }
    }

    /// Finds the topmost dirty nodes (walking only into subtrees with dirty descendants),
    /// clears the flags of their subtrees and stores the nodes whose children are laid out.
    fn collect_layout_targets(&mut self) {
        let mut targets = core::mem::take(&mut self.layout.targets);
        targets.clear();
        let mut i = 0;
        while let Some(root) = self.tree.root_at(i) {
            i += 1;
            let mut cur = Some(root);
            while let Some(c) = cur {
                let Some(n) = self.tree.node_mut(c) else { break };
                let f = n.layout_dirty;
                if f.intersects(LayoutDirty::SELF | LayoutDirty::CHILDREN) {
                    // A dirty node keeps its rectangle unless its parent is laid out too (its
                    // parent is normally marked `CHILDREN` and would have been found first).
                    let target = match n.parent() {
                        Some(p) if f.contains(LayoutDirty::SELF) => p,
                        _ => c,
                    };
                    // Wrappers do not lay out anything: their parent does.
                    let target = self.layout_owner(target);
                    if targets.last() != Some(&target) {
                        targets.push(target);
                    }
                    self.clear_layout_flags(c);
                    cur = self.skip_subtree(root, c);
                } else if f.contains(LayoutDirty::DESCENDANTS) {
                    n.layout_dirty = LayoutDirty::empty();
                    cur = n.first_child().or_else(|| self.skip_subtree(root, c));
                } else {
                    cur = self.skip_subtree(root, c);
                }
            }
        }
        self.layout.targets = targets;
    }

    /// Clears the layout flags of `id`'s subtree.
    fn clear_layout_flags(&mut self, id: NodeId) {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.tree.node_mut(c) {
                Some(n) => n.layout_dirty = LayoutDirty::empty(),
                None => break,
            }
            cur = self.next_in_subtree(id, c);
        }
    }

    /// The pre-order successor of `cur` within `root`'s subtree, skipping `cur`'s children.
    fn skip_subtree(&self, root: NodeId, cur: NodeId) -> Option<NodeId> {
        let mut x = cur;
        loop {
            if x == root {
                return None;
            }
            let xn = self.tree.node(x)?;
            if let Some(s) = xn.next_sibling() {
                return Some(s);
            }
            x = xn.parent()?;
        }
    }

    /// Lays out the children of every target, recording moved nodes.
    fn layout_targets(&mut self) {
        let targets = core::mem::take(&mut self.layout.targets);
        let mut scratch = core::mem::take(&mut self.layout.scratch);
        self.layout.moved.clear();
        for &t in &targets {
            if !self.tree.contains(t) {
                continue;
            }
            twine_core::trace!(target: "twine::layout", "layout subtree of {}", fmt_node_id(t));
            layout_children_with(&mut EngineLayout { engine: self }, &mut scratch, t);
        }
        self.layout.scratch = scratch;
        self.layout.targets = targets;
        self.update_passthrough_boxes();
    }

    /// Invalidates the old and new areas of the moved nodes, then sends `SizeChanged` and
    /// `LayoutChanged` (handlers may mark new layout work).
    fn apply_moves(&mut self) {
        let moved = core::mem::take(&mut self.layout.moved);
        let mut parents = core::mem::take(&mut self.layout.parents);
        parents.clear();
        // Invalidate everything first, while no handler has changed the tree.
        for &(id, old, new) in &moved {
            let Some(n) = self.tree.node(id) else { continue };
            let ext_old = i32::from(n.ext_draw);
            let overflow = n.flags.contains(ObjFlags::OVERFLOW_VISIBLE);
            let is_base = n.align_base;
            let parent = n.parent();
            if old.size() != new.size() {
                self.refresh_ext_draw(id);
            }
            let ext_new = self.tree.node(id).map_or(0, |n| i32::from(n.ext_draw));
            let (a, b) = (old.expand(ext_old), new.expand(ext_new));
            if overflow {
                // Descendants may draw outside: their old (shifted back) and new areas.
                let (dx, dy) = (new.x0 - old.x0, new.y0 - old.y0);
                let mut cur = self.tree.node(id).and_then(crate::Node::first_child);
                while let Some(c) = cur {
                    if let Some(cn) = self.tree.node(c) {
                        let area = cn.coords.expand(i32::from(cn.ext_draw));
                        self.invalidate_rect_of(c, area.translate(-dx, -dy), InvalidateReason::Layout);
                        self.invalidate_rect_of(c, area, InvalidateReason::Layout);
                    }
                    cur = self.next_in_subtree(id, c);
                }
            }
            let union = a.union(&b);
            if a.intersects(&b) && union.area() <= 2 * (a.area() + b.area()) {
                self.invalidate_rect_of(id, union, InvalidateReason::Layout);
            } else {
                self.invalidate_rect_of(id, a, InvalidateReason::Layout);
                self.invalidate_rect_of(id, b, InvalidateReason::Layout);
            }
            if let Some(p) = parent {
                if !parents.contains(&p) {
                    parents.push(p);
                }
            }
            if is_base {
                self.mark_aligned_to(id);
            }
        }
        for &(id, old, new) in &moved {
            if old.size() != new.size() && self.tree.contains(id) {
                self.layout.readjust.push(id);
                self.send_event(id, EventCode::SizeChanged, EventParam::Area(old));
            }
        }
        // The parents' scroll extents may have changed: redraw their scrollbar tracks.
        for &p in &parents {
            self.scrollbar_invalidate_tracks(p);
        }
        for &p in &parents {
            if self.tree.contains(p) {
                self.send_event(p, EventCode::LayoutChanged, EventParam::None);
            }
        }
        self.layout.moved = moved;
        self.layout.parents = parents;
    }

    /// Marks every node aligned to `base` (its position depends on the base's rectangle).
    fn mark_aligned_to(&mut self, base: NodeId) {
        let mut hit = Vec::new();
        for (id, n) in self.tree.all_nodes() {
            if n.align_to.is_some_and(|a| a.base == base) {
                hit.push(id);
            }
        }
        for id in hit {
            self.mark_layout(id, LayoutDirty::SELF);
        }
    }

    // ---- Imperative layout API (LVGL `lv_obj_set_*` without the prefix) -------------------

    fn set_main(&mut self, id: NodeId, p: StyleProp) {
        self.set_local_prop(id, Selector::MAIN, p);
    }

    /// Sets the width and height (`Px`, `Pct` of the parent's content area or `Content`).
    /// Idempotent, like every setter here: an unchanged value does nothing.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_style::{Align, Length};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// e.place(root, twine_core::Rect::from_xywh(0, 0, 200, 100));
    /// let b = e.create(root, Box::new(Obj)).unwrap();
    /// e.set_size(b, Length::Px(40), Length::pct(50));
    /// e.align(b, Align::Center, 0, 0);
    /// e.update_layout();
    /// assert_eq!(e.coords(b), twine_core::Rect::from_xywh(80, 25, 40, 50));
    /// ```
    pub fn set_size(&mut self, id: NodeId, w: impl Into<Length>, h: impl Into<Length>) {
        self.set_main(id, StyleProp::Width(w.into()));
        self.set_main(id, StyleProp::Height(h.into()));
    }

    /// Sets the width.
    pub fn set_width(&mut self, id: NodeId, w: impl Into<Length>) {
        self.set_main(id, StyleProp::Width(w.into()));
    }

    /// Sets the height.
    pub fn set_height(&mut self, id: NodeId, h: impl Into<Length>) {
        self.set_main(id, StyleProp::Height(h.into()));
    }

    /// Sets the position relative to the alignment point in the parent's content area
    /// (`Pct` of the parent's content size).
    pub fn set_pos(&mut self, id: NodeId, x: impl Into<Length>, y: impl Into<Length>) {
        self.set_main(id, StyleProp::X(x.into()));
        self.set_main(id, StyleProp::Y(y.into()));
    }

    /// Sets the x position.
    pub fn set_x(&mut self, id: NodeId, x: impl Into<Length>) {
        self.set_main(id, StyleProp::X(x.into()));
    }

    /// Sets the y position.
    pub fn set_y(&mut self, id: NodeId, y: impl Into<Length>) {
        self.set_main(id, StyleProp::Y(y.into()));
    }

    /// Sets the alignment in the parent (the position becomes an offset from it).
    pub fn set_align(&mut self, id: NodeId, align: Align) {
        self.set_main(id, StyleProp::Align(align));
    }

    /// Sets the alignment and the offset from it.
    pub fn align(&mut self, id: NodeId, align: Align, x: i32, y: i32) {
        self.set_main(id, StyleProp::Align(align));
        self.set_pos(id, x, y);
    }

    /// Aligns `id` to another node `base` (LVGL `lv_obj_align_to`): inner alignments use the
    /// base's content area, `Out*` alignments place `id` next to the base. Unlike LVGL the
    /// relation is kept: `id` follows `base` when it moves. Flex and grid parents ignore it
    /// for their items. `align_to` a deleted node is ignored.
    pub fn align_to(&mut self, id: NodeId, base: NodeId, align: Align, x: i32, y: i32) {
        let rel = AlignTo { base, align, x, y };
        let Some(n) = self.tree.node_mut(id) else {
            twine_core::warn!(target: "twine::layout", "align_to: node {} not found", fmt_node_id(id));
            return;
        };
        if n.align_to == Some(rel) {
            return;
        }
        n.align_to = Some(rel);
        if let Some(b) = self.tree.node_mut(base) {
            b.align_base = true;
        }
        self.mark_layout(id, LayoutDirty::SELF);
    }

    /// Removes an [`align_to`](Self::align_to) relation (the node is positioned by its
    /// `X`/`Y`/`Align` styles again).
    pub fn clear_align_to(&mut self, id: NodeId) {
        if let Some(n) = self.tree.node_mut(id) {
            if n.align_to.take().is_some() {
                self.mark_layout(id, LayoutDirty::SELF);
            }
        }
    }

    /// Sets the layout of the children (`None`, `Flex`, `Grid`).
    pub fn set_layout(&mut self, id: NodeId, kind: LayoutKind) {
        self.set_main(id, StyleProp::Layout(kind));
    }

    /// Sets the flex flow (direction, wrapping, reverse).
    pub fn set_flex_flow(&mut self, id: NodeId, flow: FlexFlow) {
        self.set_main(id, StyleProp::FlexFlow(flow));
    }

    /// Sets the flex main-axis, cross-axis and track placements.
    pub fn set_flex_align(&mut self, id: NodeId, main: FlexAlign, cross: FlexAlign, track: FlexAlign) {
        self.set_main(id, StyleProp::FlexMainPlace(main));
        self.set_main(id, StyleProp::FlexCrossPlace(cross));
        self.set_main(id, StyleProp::FlexTrackPlace(track));
    }

    /// Sets how much of the free main-axis space a flex item takes (0 = none).
    pub fn set_flex_grow(&mut self, id: NodeId, grow: u8) {
        self.set_main(id, StyleProp::FlexGrow(grow));
    }

    /// Sets the grid column and row templates.
    pub fn set_grid_dsc_array(&mut self, id: NodeId, cols: &'static [GridTrack], rows: &'static [GridTrack]) {
        self.set_main(id, StyleProp::GridColumnDscArray(cols));
        self.set_main(id, StyleProp::GridRowDscArray(rows));
    }

    /// Sets how the grid tracks are placed in the container (columns, rows).
    pub fn set_grid_align(&mut self, id: NodeId, column_align: GridAlign, row_align: GridAlign) {
        self.set_main(id, StyleProp::GridColumnAlign(column_align));
        self.set_main(id, StyleProp::GridRowAlign(row_align));
    }

    /// Places a grid item: column alignment, column, column span, row alignment, row, row
    /// span.
    #[allow(clippy::too_many_arguments)]
    pub fn set_grid_cell(
        &mut self,
        id: NodeId,
        column_align: GridAlign,
        col: i32,
        col_span: i32,
        row_align: GridAlign,
        row: i32,
        row_span: i32,
    ) {
        self.set_main(id, StyleProp::GridCellXAlign(column_align));
        self.set_main(id, StyleProp::GridCellColumnPos(col));
        self.set_main(id, StyleProp::GridCellColumnSpan(col_span));
        self.set_main(id, StyleProp::GridCellYAlign(row_align));
        self.set_main(id, StyleProp::GridCellRowPos(row));
        self.set_main(id, StyleProp::GridCellRowSpan(row_span));
    }
}
