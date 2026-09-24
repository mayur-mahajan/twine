//! Moving nodes to another parent or position ([`Engine::move_node`], LVGL `lv_obj_set_parent`
//! / `lv_obj_move_to_index`), wired to invalidation and the layout pass.

use crate::{
    Engine, EngineError, EventCode, EventParam, InvalidateReason, LayoutDirty, NodeId, ObjFlags, fmt_node_id,
};

impl Engine {
    /// Moves `id` (with its subtree) under `parent`, right before `parent`'s child `before`
    /// (`None`: as the last child). The old area is invalidated, the old and new parents
    /// are laid out again (the node gets its new rectangle in the next layout pass, which
    /// invalidates the new area), and both parents receive `ChildChanged`.
    ///
    /// Moving a node to where it already is does nothing.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// let a = e.create(root, Box::new(Obj)).unwrap();
    /// let b = e.create(root, Box::new(Obj)).unwrap();
    /// e.move_node(b, root, Some(a)).unwrap();
    /// assert_eq!(e.tree().children(root).collect::<Vec<_>>(), [b, a]);
    /// ```
    ///
    /// # Errors
    /// [`EngineError::NodeNotFound`] for unknown nodes; [`EngineError::InvalidConfig`] when
    /// `before` is not a child of `parent` or `parent` is inside `id`'s subtree (all logged).
    pub fn move_node(
        &mut self,
        id: NodeId,
        parent: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), EngineError> {
        let Some(n) = self.tree.node(id) else {
            twine_core::warn!(target: "twine::engine", "move_node: node {} not found", fmt_node_id(id));
            return Err(EngineError::NodeNotFound(id));
        };
        let old_parent = n.parent();
        if old_parent == Some(parent) {
            let next = n.next_sibling();
            if next == before || before == Some(id) {
                return Ok(());
            }
        }
        let wide = n
            .flags()
            .intersects(ObjFlags::OVERFLOW_VISIBLE.union(ObjFlags::LAYOUT_PASSTHROUGH));
        if wide {
            self.invalidate_subtree(id, InvalidateReason::Layout);
        } else {
            self.invalidate(id, InvalidateReason::Layout);
        }
        self.tree.move_before(id, parent, before)?;
        // Same place in the new stacking order (the layout may keep the rectangle).
        if wide {
            self.invalidate_subtree(id, InvalidateReason::Layout);
        } else {
            self.invalidate(id, InvalidateReason::Layout);
        }
        self.mark_layout(id, LayoutDirty::SELF);
        if let Some(op) = old_parent.filter(|p| *p != parent && self.tree.contains(*p)) {
            self.mark_layout(op, LayoutDirty::CHILDREN);
            self.layout.readjust.push(op);
            self.scrollbar_invalidate_tracks(op);
            self.send_event(op, EventCode::ChildChanged, EventParam::None);
        }
        if self.tree.contains(parent) {
            self.scrollbar_invalidate_tracks(parent);
            self.send_event(parent, EventCode::ChildChanged, EventParam::None);
        }
        Ok(())
    }
}
