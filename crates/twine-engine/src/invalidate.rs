//! Invalidation ([`InvalidateReason`], `Engine::invalidate*`), node geometry (`place`,
//! extra draw size, transforms) and flags.

use twine_core::{Insets, Rect};
use twine_render::{LayerTransform, ShadowDsc, shadow_ext_size, transformed_bounds};
use twine_style::{BlendMode, GradDir, Part, PropId};

use crate::style_list::length_px;
use crate::{Engine, LayoutDirty, MeasureCx, NodeId, ObjFlags, fmt_node_id};

/// Why an area was invalidated (visible in `trace` logs and, with `debug-checks`, in
/// [`Engine::invalidation_log`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InvalidateReason {
    /// A style, local property or state changed.
    StyleChange,
    /// The node moved or was resized (layout or `place`).
    Layout,
    /// Scrolling.
    Scroll,
    /// A widget setter (named).
    WidgetSetter(&'static str),
    /// An animation.
    Anim,
    /// The node is being deleted.
    Delete,
    /// The node was created.
    Create,
    /// An explicit `invalidate` call (also: flags, screen loads, overlays).
    Explicit,
}

impl Engine {
    /// The area `id` currently occupies on screen (`coords` + extra draw size, clipped by every
    /// ancestor that does not have `OVERFLOW_VISIBLE`), with the index of its display. `None`
    /// when the node or an ancestor is hidden, its root is not shown (not the active or
    /// previous screen or a layer of a display), or nothing remains after clipping.
    pub(crate) fn visible_area(&self, id: NodeId) -> Option<(usize, Rect)> {
        let n = self.tree.node(id)?;
        self.visible_part(id, n.coords.expand(i32::from(n.ext_draw)))
    }

    /// The part of the absolute `area` drawn by `id` that is visible (see
    /// [`visible_area`](Self::visible_area)), with the index of its display.
    pub(crate) fn visible_part(&self, id: NodeId, area: Rect) -> Option<(usize, Rect)> {
        let n = self.tree.node(id)?;
        if n.is_hidden() || area.is_empty() {
            return None;
        }
        let mut area = area;
        let mut root = id;
        for a in self.tree.ancestors(id) {
            let an = self.tree.node(a)?;
            if an.is_hidden() {
                return None;
            }
            if !an
                .flags
                .intersects(ObjFlags::OVERFLOW_VISIBLE.union(ObjFlags::LAYOUT_PASSTHROUGH))
            {
                area = area.intersection(&an.coords)?;
            }
            root = a;
        }
        if area.is_empty() {
            return None;
        }
        let d = self.displays.iter().position(|d| d.shows_root(root))?;
        Some((d, area))
    }

    /// Invalidates the area of `id` (its coordinates plus extra draw size, clipped by its
    /// ancestors) so it is redrawn in the next frame. Hidden nodes and nodes of inactive
    /// screens are skipped. Unknown ids are ignored.
    pub fn invalidate(&mut self, id: NodeId, reason: InvalidateReason) {
        let Some(n) = self.tree.node(id) else {
            return;
        };
        let area = n.coords.expand(i32::from(n.ext_draw));
        self.invalidate_rect_of(id, area, reason);
    }

    /// Invalidates `area` (absolute, e.g. a former position of `id` plus its extra draw size)
    /// clipped like `id`'s own area.
    pub(crate) fn invalidate_rect_of(&mut self, id: NodeId, area: Rect, reason: InvalidateReason) {
        if let Some((d, area)) = self.visible_part(id, area) {
            #[cfg(feature = "perf-monitor")]
            if self.displays[d].perf_overlay == Some(id) {
                // The overlay's own redraw is kept out of the frame statistics.
                let r = &mut self.displays[d].refresher;
                r.overlay_dirty = Some(r.overlay_dirty.map_or(area, |o| o.union(&area)));
                return;
            }
            twine_core::trace!(
                target: "twine::refresh",
                "invalidate {} {} reason={:?}",
                fmt_node_id(id),
                area,
                reason
            );
            self.add_dirty(d, area, reason);
        }
    }

    /// Invalidates `area` (absolute) as far as it is visible through `id`'s clipping.
    pub fn invalidate_node_area(&mut self, id: NodeId, area: Rect, reason: InvalidateReason) {
        if let Some((d, vis)) = self.visible_area(id) {
            if let Some(a) = vis.intersection(&area) {
                self.add_dirty(d, a, reason);
            }
        }
    }

    /// Invalidates an absolute area of a display (clipped to the screen).
    pub fn invalidate_area(&mut self, display: crate::DisplayId, area: Rect, reason: InvalidateReason) {
        let d = display.index();
        if d >= self.displays.len() {
            twine_core::warn!(target: "twine::refresh", "invalidate_area: display {} not found", display);
            return;
        }
        twine_core::trace!(target: "twine::refresh", "invalidate {} {} reason={:?}", display, area, reason);
        self.add_dirty(d, area, reason);
    }

    /// Invalidates `id` and each of its descendants (for content that may overflow).
    pub(crate) fn invalidate_subtree(&mut self, id: NodeId, reason: InvalidateReason) {
        let mut cur = Some(id);
        while let Some(c) = cur {
            self.invalidate(c, reason);
            // Pre-order walk without borrowing the tree across `invalidate`.
            cur = self.next_in_subtree(id, c);
        }
    }

    /// The pre-order successor of `cur` within `root`'s subtree.
    pub(crate) fn next_in_subtree(&self, root: NodeId, cur: NodeId) -> Option<NodeId> {
        let n = self.tree.node(cur)?;
        if let Some(c) = n.first_child() {
            return Some(c);
        }
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

    fn add_dirty(&mut self, d: usize, area: Rect, reason: InvalidateReason) {
        let max = self.config.max_dirty_areas;
        self.displays[d].refresher.add_dirty(area, max);
        #[cfg(feature = "debug-checks")]
        self.invalidations.push((area, reason));
        #[cfg(not(feature = "debug-checks"))]
        let _ = reason;
    }

    /// The invalidations rendered by the last frame that started (feature `debug-checks`;
    /// empty otherwise): everything invalidated between the start of the frame before it and
    /// its own start.
    #[must_use]
    pub fn frame_invalidation_log(&self) -> &[(Rect, InvalidateReason)] {
        #[cfg(feature = "debug-checks")]
        {
            &self.frame_invalidations
        }
        #[cfg(not(feature = "debug-checks"))]
        {
            &[]
        }
    }

    /// Every invalidation since the last frame started (feature `debug-checks`; empty
    /// otherwise).
    #[must_use]
    pub fn invalidation_log(&self) -> &[(Rect, InvalidateReason)] {
        #[cfg(feature = "debug-checks")]
        {
            &self.invalidations
        }
        #[cfg(not(feature = "debug-checks"))]
        {
            &[]
        }
    }

    /// Absolute coordinates of `id` (`Rect::ZERO` for unknown ids).
    #[must_use]
    pub fn coords(&self, id: NodeId) -> Rect {
        self.tree.node(id).map_or(Rect::ZERO, crate::Node::coords)
    }

    /// Coordinates minus padding and border width of `Part::Main`.
    #[must_use]
    pub fn content_area(&self, id: NodeId) -> Rect {
        let m = self.cached_main(id);
        let b = m.border_width.max(0);
        self.coords(id).inset(Insets::new(
            m.pad.left + b,
            m.pad.top + b,
            m.pad.right + b,
            m.pad.bottom + b,
        ))
    }

    /// Places `id` at the absolute rectangle `rect` right away (low-level placement). The
    /// subtree moves with it. No-op when unchanged (P3); otherwise the old and the new area
    /// are invalidated and a resize sends `SizeChanged`.
    ///
    /// The rectangle is overwritten by the layout pass when the node or its parent is laid
    /// out (after a style, child or flag change): for manual placement use the position and
    /// size setters ([`set_pos`](Self::set_pos), [`set_size`](Self::set_size),
    /// [`align`](Self::align)), with the `IGNORE_LAYOUT` flag inside flex or grid containers.
    pub fn place(&mut self, id: NodeId, rect: Rect) {
        let Some(n) = self.tree.node(id) else {
            twine_core::warn!(target: "twine::engine", "place: node {} not found", fmt_node_id(id));
            return;
        };
        let old = n.coords;
        if old == rect {
            return;
        }
        let overflow = n.flags.contains(ObjFlags::OVERFLOW_VISIBLE);
        if overflow {
            self.invalidate_subtree(id, crate::InvalidateReason::Layout);
        } else {
            self.invalidate(id, crate::InvalidateReason::Layout);
        }
        let (dx, dy) = (rect.x0 - old.x0, rect.y0 - old.y0);
        if dx != 0 || dy != 0 {
            let mut cur = self.tree.node(id).and_then(crate::Node::first_child);
            while let Some(c) = cur {
                if let Some(cn) = self.tree.node_mut(c) {
                    cn.coords = cn.coords.translate(dx, dy);
                }
                cur = self.next_in_subtree(id, c);
            }
        }
        if let Some(n) = self.tree.node_mut(id) {
            n.coords = rect;
        }
        if let Some(p) = self.tree.parent(id) {
            self.forget_children_box(p);
        }
        if old.size() != rect.size() {
            // Percentages (pivot, transform size) and the widget's own extra size may change.
            self.refresh_ext_draw(id);
        }
        if overflow {
            self.invalidate_subtree(id, crate::InvalidateReason::Layout);
        } else {
            self.invalidate(id, crate::InvalidateReason::Layout);
        }
        if old.size() != rect.size() {
            self.send_event(id, crate::EventCode::SizeChanged, crate::EventParam::Area(old));
        }
    }

    /// The layer transform of `id` from its style (`None` when it is the identity).
    pub(crate) fn layer_transform(&self, id: NodeId) -> Option<LayerTransform> {
        let m = Part::Main;
        let rotation = self
            .style_prop(id, m, PropId::TransformRotation)
            .as_angle()
            .unwrap_or_default();
        let scale_x = self
            .style_prop(id, m, PropId::TransformScaleX)
            .as_scale()
            .unwrap_or_default();
        let scale_y = self
            .style_prop(id, m, PropId::TransformScaleY)
            .as_scale()
            .unwrap_or_default();
        let skew_x = self
            .style_prop(id, m, PropId::TransformSkewX)
            .as_angle()
            .unwrap_or_default();
        let skew_y = self
            .style_prop(id, m, PropId::TransformSkewY)
            .as_angle()
            .unwrap_or_default();
        let c = self.coords(id);
        let t = LayerTransform {
            rotation,
            scale_x,
            scale_y,
            skew_x,
            skew_y,
            pivot: twine_core::Point::new(
                length_px(self.style_prop(id, m, PropId::TransformPivotX), c.width()),
                length_px(self.style_prop(id, m, PropId::TransformPivotY), c.height()),
            ),
            antialias: true,
        };
        (!t.is_identity()).then_some(t)
    }

    /// Whether `id` is drawn with a rotation, scale or skew.
    #[must_use]
    pub fn has_transform(&self, id: NodeId) -> bool {
        self.layer_transform(id).is_some()
    }

    /// Whether `id` and its children must be rendered into a layer first: `opa_layered` below
    /// cover, a transform, a blend mode other than normal, or a bitmap mask.
    #[must_use]
    pub fn needs_layer(&self, id: NodeId) -> bool {
        let m = Part::Main;
        !self.style_opa(id, m, PropId::OpaLayered).is_cover()
            || self
                .style_prop(id, m, PropId::BlendMode)
                .get::<BlendMode>()
                .unwrap_or(BlendMode::Normal)
                != BlendMode::Normal
            || !self.style_prop(id, m, PropId::BitmapMaskSrc).is_none()
            || self.has_transform(id)
    }

    /// Whether the background has no translucent gradient stop (the `bg_opa` itself is checked
    /// separately).
    pub(crate) fn bg_is_opaque(&self, id: NodeId) -> bool {
        let m = Part::Main;
        if let Some(g) = self
            .style_prop(id, m, PropId::BgGrad)
            .get::<&twine_render::Gradient>()
        {
            return g.stops().iter().all(|s| s.opa.is_cover());
        }
        match self
            .style_prop(id, m, PropId::BgGradDir)
            .get::<GradDir>()
            .unwrap_or_default()
        {
            GradDir::Ver | GradDir::Hor => {
                self.style_opa(id, m, PropId::BgMainOpa).is_cover()
                    && self.style_opa(id, m, PropId::BgGradOpa).is_cover()
            }
            _ => true,
        }
    }

    /// The extra draw size of `id` from its styles and widget (LVGL
    /// `lv_obj_refresh_ext_draw_size`): the maximum of the shadow extent, outline width + pad,
    /// the growth caused by the transform and transform size, and `Widget::ext_draw_size`.
    #[must_use]
    pub fn compute_ext_draw(&self, id: NodeId) -> u16 {
        self.ext_draw_parts(id, true)
    }

    /// The extra draw size without the transform's growth (the untransformed layer area of a
    /// transformed node).
    pub(crate) fn ext_draw_untransformed(&self, id: NodeId) -> u16 {
        self.ext_draw_parts(id, false)
    }

    fn ext_draw_parts(&self, id: NodeId, with_transform: bool) -> u16 {
        let Some(n) = self.tree.node(id) else {
            return 0;
        };
        let m = Part::Main;
        let mut ext = 0i32;
        let sh = ShadowDsc {
            width: self.style_i32(id, m, PropId::ShadowWidth),
            ofs_x: self.style_i32(id, m, PropId::ShadowOffsetX),
            ofs_y: self.style_i32(id, m, PropId::ShadowOffsetY),
            spread: self.style_i32(id, m, PropId::ShadowSpread),
            color: twine_core::Color::BLACK,
            opa: self.style_opa(id, m, PropId::ShadowOpa),
        };
        if sh.is_visible() {
            ext = ext.max(shadow_ext_size(&sh));
        }
        let ow = self.style_i32(id, m, PropId::OutlineWidth);
        if ow > 0 && !self.style_opa(id, m, PropId::OutlineOpa).is_transparent() {
            ext = ext.max(ow + self.style_i32(id, m, PropId::OutlinePad).max(0));
        }
        let c = n.coords;
        let tw = length_px(self.style_prop(id, m, PropId::TransformWidth), c.width());
        let th = length_px(self.style_prop(id, m, PropId::TransformHeight), c.height());
        ext = ext.max(tw).max(th);
        // While the widget is taken out of its node (in its own setters and event handler),
        // the value it reported last is used.
        let w = if n.widget.is::<crate::obj::Detached>() {
            n.widget_ext.get()
        } else {
            let w = n.widget.ext_draw_size(&MeasureCx::new(self, id));
            n.widget_ext.set(w);
            w
        };
        ext = ext.max(i32::from(w));
        if let Some(t) = self.layer_transform(id).filter(|_| with_transform) {
            let base = c.expand(ext);
            let b = transformed_bounds(base, &t);
            let grow = (base.x0 - b.x0)
                .max(base.y0 - b.y0)
                .max(b.x1 - base.x1)
                .max(b.y1 - base.y1);
            ext += grow.max(0);
        }
        ext.clamp(0, i32::from(u16::MAX)) as u16
    }

    /// Recomputes the extra draw size of `id` with `widget_ext` as the widget's own extra size
    /// (for widgets changing it while they are taken out of their node); returns whether the
    /// node's extra draw size changed.
    pub(crate) fn refresh_ext_draw_with(&mut self, id: NodeId, widget_ext: u16) -> bool {
        if let Some(n) = self.tree.node(id) {
            n.widget_ext.set(widget_ext);
        }
        self.refresh_ext_draw(id)
    }

    /// Recomputes the extra draw size of `id`; returns whether it changed.
    pub(crate) fn refresh_ext_draw(&mut self, id: NodeId) -> bool {
        let ext = self.compute_ext_draw(id);
        match self.tree.node_mut(id) {
            Some(n) if n.ext_draw != ext => {
                n.ext_draw = ext;
                true
            }
            _ => false,
        }
    }

    /// Sets (`on`) or clears flags of `id`. Idempotent. Hiding invalidates the area before,
    /// showing after; both mark the parent's layout.
    pub fn set_flag(&mut self, id: NodeId, flags: ObjFlags, on: bool) {
        let Some(n) = self.tree.node(id) else {
            twine_core::warn!(target: "twine::engine", "set_flag: node {} not found", fmt_node_id(id));
            return;
        };
        let old = n.flags;
        let new = if on { old | flags } else { old & !flags };
        if old == new {
            return;
        }
        let changed = old ^ new;
        if changed.contains(ObjFlags::HIDDEN) && on {
            self.invalidate_subtree(id, crate::InvalidateReason::Explicit);
        }
        if changed.contains(ObjFlags::OVERFLOW_VISIBLE) && !on {
            self.invalidate_subtree(id, crate::InvalidateReason::Explicit);
        }
        if let Some(n) = self.tree.node_mut(id) {
            n.flags = new;
        }
        if changed.contains(ObjFlags::HIDDEN) && !on {
            self.invalidate_subtree(id, crate::InvalidateReason::Explicit);
        }
        let layout_flags = ObjFlags::HIDDEN
            | ObjFlags::IGNORE_LAYOUT
            | ObjFlags::FLOATING
            | ObjFlags::FLEX_IN_NEW_TRACK
            | ObjFlags::LAYOUT_PASSTHROUGH;
        if changed.intersects(layout_flags) {
            if let Some(p) = self.tree.parent(id) {
                self.mark_layout(p, LayoutDirty::CHILDREN);
                // Hidden and floating children do not count for the parent's scroll extents.
                self.scrollbar_invalidate_tracks(p);
            }
        }
        if changed.contains(ObjFlags::SCROLLABLE) {
            // Scrollbars appear or disappear (LVGL `lv_obj_set_scrollable`).
            let old_flags = new ^ changed;
            if let Some(n) = self.tree.node_mut(id) {
                n.flags = old_flags;
            }
            self.scrollbar_invalidate(id);
            if let Some(n) = self.tree.node_mut(id) {
                n.flags = new;
            }
            self.scrollbar_invalidate(id);
        }
        if changed.contains(ObjFlags::OVERFLOW_VISIBLE) && on {
            self.invalidate_subtree(id, crate::InvalidateReason::Explicit);
        }
    }

    /// Whether `id` has all of `flags`.
    #[must_use]
    pub fn has_flag(&self, id: NodeId, flags: ObjFlags) -> bool {
        self.tree.node(id).is_some_and(|n| n.flags.contains(flags))
    }

    /// Sets the test id used by queries and dumps (kept only in debug builds or with the
    /// `test-ids` feature).
    pub fn set_test_id(&mut self, id: NodeId, test_id: &'static str) {
        #[cfg(any(debug_assertions, feature = "test-ids"))]
        if let Some(n) = self.tree.node_mut(id) {
            n.test_id = Some(test_id);
        }
        #[cfg(not(any(debug_assertions, feature = "test-ids")))]
        let _ = (id, test_id);
    }
}
