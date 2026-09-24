//! The widget tree: [`Node`], [`Tree`], its iterators and the invariant checker.
//!
//! Nodes live in a generational arena. Children form an intrusive doubly linked list
//! (`first_child`/`last_child` on the parent, `prev`/`next` on the siblings), so insertion and
//! removal are O(1), no node owns a `Vec` of children, and every iterator walks the links
//! without allocating (not even a stack).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt::{self, Write};

use twine_core::{Arena, Point, Rect};
use twine_style::State;

use crate::handlers::Handlers;
use crate::style_cache::StyleCache;
use crate::style_list::StyleList;
use crate::{
    EngineError, GroupId, InvariantError, LayoutDirty, NodeId, ObjFlags, Widget, WidgetClass, flag_names,
    fmt_node_id,
};

/// One node of the tree: links, the widget, flags, state, styles and geometry.
///
/// Fields are private; read them with the getters and change them through
/// [`Engine`](crate::Engine) methods (which keep invalidation and caches consistent).
pub struct Node {
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    prev: Option<NodeId>,
    next: Option<NodeId>,
    child_count: u16,
    pub(crate) widget: Box<dyn Widget>,
    class: &'static WidgetClass,
    pub(crate) flags: ObjFlags,
    pub(crate) state: State,
    pub(crate) styles: StyleList,
    pub(crate) coords: Rect,
    pub(crate) scroll: Point,
    /// Scroll direction, scrollbar mode and snapping (LVGL `spec_attr` scroll fields).
    pub(crate) scroll_attrs: crate::scroll::ScrollAttrs,
    /// Bounding box of the scrolling children (margins included), relative to the unscrolled
    /// content origin; `None` = not computed since the last change (see `Engine::scroll_bottom`).
    pub(crate) children_bbox: core::cell::Cell<Option<crate::scroll::ChildrenBox>>,
    pub(crate) ext_draw: u16,
    /// The widget's own `ext_draw_size` as last computed (used while the widget is detached).
    pub(crate) widget_ext: core::cell::Cell<u16>,
    pub(crate) layout_dirty: LayoutDirty,
    /// `Engine::align_to` relation.
    pub(crate) align_to: Option<twine_layout::AlignTo<NodeId>>,
    /// Some node was aligned to this one (its moves re-layout them).
    pub(crate) align_base: bool,
    pub(crate) style_cache: StyleCache,
    /// The node was drawn at least once (style transitions start only after that, LVGL
    /// `rendered`).
    pub(crate) rendered: core::cell::Cell<bool>,
    /// User event handlers (allocated on the first handler).
    pub(crate) handlers: Option<Box<Handlers>>,
    /// The focus group the node belongs to.
    pub(crate) group: Option<GroupId>,
    #[cfg(any(debug_assertions, feature = "test-ids"))]
    pub(crate) test_id: Option<&'static str>,
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("class", &self.class.name)
            .field("parent", &self.parent)
            .field("coords", &self.coords)
            .field("flags", &self.flags)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Node {
    fn new(widget: Box<dyn Widget>) -> Self {
        let class = widget.class();
        Self {
            parent: None,
            first_child: None,
            last_child: None,
            prev: None,
            next: None,
            child_count: 0,
            widget,
            class,
            flags: class.default_flags,
            state: State::DEFAULT,
            styles: StyleList::new(),
            coords: Rect::ZERO,
            scroll: Point::ZERO,
            scroll_attrs: crate::scroll::ScrollAttrs::DEFAULT,
            children_bbox: core::cell::Cell::new(None),
            ext_draw: 0,
            widget_ext: core::cell::Cell::new(0),
            layout_dirty: LayoutDirty::empty(),
            align_to: None,
            align_base: false,
            style_cache: StyleCache::default(),
            rendered: core::cell::Cell::new(false),
            handlers: None,
            group: None,
            #[cfg(any(debug_assertions, feature = "test-ids"))]
            test_id: None,
        }
    }

    /// The parent (`None` for screens and layers).
    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// The first child.
    #[must_use]
    pub fn first_child(&self) -> Option<NodeId> {
        self.first_child
    }

    /// The last child (drawn last, topmost).
    #[must_use]
    pub fn last_child(&self) -> Option<NodeId> {
        self.last_child
    }

    /// The next sibling.
    #[must_use]
    pub fn next_sibling(&self) -> Option<NodeId> {
        self.next
    }

    /// The previous sibling.
    #[must_use]
    pub fn prev_sibling(&self) -> Option<NodeId> {
        self.prev
    }

    /// Number of children.
    #[must_use]
    pub fn child_count(&self) -> u16 {
        self.child_count
    }

    /// Flags.
    #[must_use]
    pub fn flags(&self) -> ObjFlags {
        self.flags
    }

    /// State.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// Absolute coordinates.
    #[must_use]
    pub fn coords(&self) -> Rect {
        self.coords
    }

    /// Scroll offset of the children.
    #[must_use]
    pub fn scroll(&self) -> Point {
        self.scroll
    }

    /// Extra draw size around the coordinates (shadow, outline, transform).
    #[must_use]
    pub fn ext_draw(&self) -> u16 {
        self.ext_draw
    }

    /// Pending layout work.
    #[must_use]
    pub fn layout_dirty(&self) -> LayoutDirty {
        self.layout_dirty
    }

    /// The widget class.
    #[must_use]
    pub fn class(&self) -> &'static WidgetClass {
        self.class
    }

    /// The widget.
    #[must_use]
    pub fn widget(&self) -> &dyn Widget {
        &*self.widget
    }

    /// The style list.
    #[must_use]
    pub fn styles(&self) -> &StyleList {
        &self.styles
    }

    /// The test id (always `None` in release builds without the `test-ids` feature).
    #[must_use]
    pub fn test_id(&self) -> Option<&'static str> {
        #[cfg(any(debug_assertions, feature = "test-ids"))]
        {
            self.test_id
        }
        #[cfg(not(any(debug_assertions, feature = "test-ids")))]
        {
            None
        }
    }

    /// Whether the node has the `HIDDEN` flag.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.flags.contains(ObjFlags::HIDDEN)
    }
}

/// The widget tree: an arena of [`Node`]s plus the list of roots (screens and layers).
#[derive(Debug, Default)]
pub struct Tree {
    nodes: Arena<Node>,
    roots: Vec<NodeId>,
    pub(crate) epoch: u32,
}

impl Tree {
    /// An empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The style epoch: incremented whenever an inherited style property may have changed
    /// anywhere (invalidates every node's cached inherited values at once).
    #[must_use]
    pub fn style_epoch(&self) -> u32 {
        self.epoch
    }

    /// Creates a node for `widget` as the last child of `parent` (or as a new root), with the
    /// class's default flags, state `DEFAULT` and zero coordinates. O(1).
    pub fn create(&mut self, parent: Option<NodeId>, widget: Box<dyn Widget>) -> Result<NodeId, EngineError> {
        if let Some(p) = parent {
            if !self.contains(p) {
                return Err(EngineError::NodeNotFound(p));
            }
        }
        let class = widget.class().name;
        let mut node = Node::new(widget);
        if parent.is_none() {
            // Like LVGL's `lv_obj` constructor: screens and other roots keep neither the press
            // nor scroll chains nor gestures for a parent they do not have.
            node.flags
                .remove(ObjFlags::PRESS_LOCK | ObjFlags::SCROLL_CHAIN | ObjFlags::GESTURE_BUBBLE);
        }
        let id = self.nodes.insert(node).map_err(|_| EngineError::TooManyNodes)?;
        self.attach(id, parent, None);
        twine_core::trace!(
            target: "twine::engine",
            "create {} class={} parent={:?}",
            fmt_node_id(id),
            class,
            parent
        );
        self.debug_check();
        Ok(id)
    }

    /// Deletes `id` and its whole subtree. Returns the deleted ids in post-order (children
    /// before their parent). O(subtree).
    pub fn delete(&mut self, id: NodeId) -> Result<Vec<NodeId>, EngineError> {
        if !self.contains(id) {
            return Err(EngineError::NodeNotFound(id));
        }
        let order = self.post_order(id);
        self.detach(id);
        for &n in &order {
            self.nodes.remove(n);
        }
        twine_core::trace!(target: "twine::engine", "delete {} ({} nodes)", fmt_node_id(id), order.len());
        self.debug_check();
        Ok(order)
    }

    /// Moves `id` under `new_parent` (`None` = make it a root) at `index` (`None` = last).
    /// Moving a node into its own subtree is rejected (`InvalidConfig("cycle")`).
    pub fn move_to(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        index: Option<usize>,
    ) -> Result<(), EngineError> {
        if !self.contains(id) {
            return Err(EngineError::NodeNotFound(id));
        }
        if let Some(p) = new_parent {
            if !self.contains(p) {
                return Err(EngineError::NodeNotFound(p));
            }
            if p == id || self.ancestors(p).any(|a| a == id) {
                twine_core::warn!(
                    target: "twine::engine",
                    "move_to: {} cannot move into its own subtree ({})",
                    fmt_node_id(id),
                    fmt_node_id(p)
                );
                return Err(EngineError::InvalidConfig("cycle"));
            }
        }
        self.detach(id);
        self.attach(id, new_parent, index);
        twine_core::trace!(
            target: "twine::engine",
            "move {} to parent={:?} index={:?}",
            fmt_node_id(id),
            new_parent,
            index
        );
        self.debug_check();
        Ok(())
    }

    /// Moves `id` under `parent`, right before its child `before` (`None`: as the last child).
    /// Unlike [`move_to`](Self::move_to) this needs no index lookup (O(1) apart from the cycle
    /// check). `before == Some(id)` keeps the node where it is when it already is a child of
    /// `parent`.
    pub fn move_before(
        &mut self,
        id: NodeId,
        parent: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), EngineError> {
        if !self.contains(id) {
            return Err(EngineError::NodeNotFound(id));
        }
        if !self.contains(parent) {
            return Err(EngineError::NodeNotFound(parent));
        }
        if let Some(b) = before {
            if b == id {
                return if self.parent(id) == Some(parent) {
                    Ok(())
                } else {
                    Err(EngineError::InvalidConfig(
                        "move_before: `before` is the node itself",
                    ))
                };
            }
            if self.parent(b) != Some(parent) {
                twine_core::warn!(
                    target: "twine::engine",
                    "move_before: {} is not a child of {}",
                    fmt_node_id(b),
                    fmt_node_id(parent)
                );
                return Err(EngineError::InvalidConfig(
                    "move_before: `before` is not a child of `parent`",
                ));
            }
        }
        if parent == id || self.ancestors(parent).any(|a| a == id) {
            twine_core::warn!(
                target: "twine::engine",
                "move_before: {} cannot move into its own subtree ({})",
                fmt_node_id(id),
                fmt_node_id(parent)
            );
            return Err(EngineError::InvalidConfig("cycle"));
        }
        self.detach(id);
        self.attach_before(id, parent, before);
        twine_core::trace!(
            target: "twine::engine",
            "move {} under {} before {:?}",
            fmt_node_id(id),
            fmt_node_id(parent),
            before
        );
        self.debug_check();
        Ok(())
    }

    /// Moves `id` to position `index` among its siblings (clamped to the last position).
    pub fn set_index(&mut self, id: NodeId, index: usize) -> Result<(), EngineError> {
        let parent = self.node(id).ok_or(EngineError::NodeNotFound(id))?.parent;
        self.move_to(id, parent, Some(index))
    }

    /// Swaps the positions of `a` and `b` in their parents (LVGL `lv_obj_swap`; they may have
    /// different parents). Rejected if one is an ancestor of the other.
    pub fn swap(&mut self, a: NodeId, b: NodeId) -> Result<(), EngineError> {
        if a == b {
            return if self.contains(a) {
                Ok(())
            } else {
                Err(EngineError::NodeNotFound(a))
            };
        }
        let pa = self.node(a).ok_or(EngineError::NodeNotFound(a))?.parent;
        let pb = self.node(b).ok_or(EngineError::NodeNotFound(b))?.parent;
        if self.ancestors(a).any(|x| x == b) || self.ancestors(b).any(|x| x == a) {
            twine_core::warn!(target: "twine::engine", "swap: {} and {} are nested", a, b);
            return Err(EngineError::InvalidConfig("cycle"));
        }
        let ia = self.index(a).unwrap_or(0);
        let ib = self.index(b).unwrap_or(0);
        if pa == pb {
            let (first, i_first, second, i_second) = if ia < ib { (a, ia, b, ib) } else { (b, ib, a, ia) };
            // Move the later one to the earlier position, then the earlier one to the later.
            self.move_to(second, pa, Some(i_first))?;
            self.move_to(first, pa, Some(i_second))?;
        } else {
            self.move_to(a, pb, Some(ib))?;
            self.move_to(b, pa, Some(ia))?;
        }
        Ok(())
    }

    /// Position of `id` among its siblings (or among the roots). O(siblings).
    #[must_use]
    pub fn index(&self, id: NodeId) -> Option<usize> {
        let n = self.node(id)?;
        match n.parent {
            Some(p) => self.children(p).position(|c| c == id),
            None => self.roots.iter().position(|r| *r == id),
        }
    }

    /// Whether `id` refers to a live node.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains(id)
    }

    /// The node `id`.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// The widget of `id` as `W`.
    #[must_use]
    pub fn get<W: Widget>(&self, id: NodeId) -> Option<&W> {
        self.node(id)?.widget.downcast_ref::<W>()
    }

    /// The widget of `id` as `&mut W`. Changing widget data this way does not invalidate
    /// anything; prefer the widget's setters.
    pub fn get_mut<W: Widget>(&mut self, id: NodeId) -> Option<&mut W> {
        self.node_mut(id)?.widget.downcast_mut::<W>()
    }

    /// The widget of `id`.
    #[must_use]
    pub fn widget_dyn(&self, id: NodeId) -> Option<&dyn Widget> {
        self.node(id).map(|n| &*n.widget)
    }

    /// The parent of `id`.
    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.parent
    }

    /// Children of `id`, first to last (paint order). No allocation.
    #[must_use]
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.node(id).and_then(|n| n.first_child),
        }
    }

    /// Children of `id`, last to first (topmost first).
    #[must_use]
    pub fn children_rev(&self, id: NodeId) -> ChildrenRev<'_> {
        ChildrenRev {
            tree: self,
            next: self.node(id).and_then(|n| n.last_child),
        }
    }

    /// Ancestors of `id` from the parent up to the root (excluding `id`).
    #[must_use]
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            tree: self,
            next: self.parent(id),
        }
    }

    /// `id` and all its descendants in pre-order (paint order), walking the links only: no
    /// stack, no allocation.
    #[must_use]
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            tree: self,
            root: id,
            next: self.contains(id).then_some(id),
        }
    }

    /// Every live node, in slot order.
    pub(crate) fn all_nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter()
    }

    /// The `i`-th root.
    pub(crate) fn root_at(&self, i: usize) -> Option<NodeId> {
        self.roots.get(i).copied()
    }

    /// The roots (screens and layers) in creation order.
    pub fn roots(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.roots.iter().copied()
    }

    /// The child at `index`; a negative index counts from the end (`-1` = last), like LVGL.
    #[must_use]
    pub fn child(&self, id: NodeId, index: i32) -> Option<NodeId> {
        if index >= 0 {
            self.children(id).nth(index as usize)
        } else {
            self.children_rev(id).nth((-(i64::from(index)) - 1) as usize)
        }
    }

    /// The root of `id`'s tree (itself for a root).
    #[must_use]
    pub fn root_of(&self, id: NodeId) -> Option<NodeId> {
        if !self.contains(id) {
            return None;
        }
        Some(self.ancestors(id).last().unwrap_or(id))
    }

    /// Post-order ids of `id`'s subtree (children before parents).
    pub(crate) fn post_order(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let leftmost = |mut n: NodeId| {
            while let Some(c) = self.node(n).and_then(|x| x.first_child) {
                n = c;
            }
            n
        };
        let mut n = leftmost(id);
        loop {
            out.push(n);
            if n == id {
                break;
            }
            let node = &self.nodes.get(n).expect("post_order: live node");
            match node.next {
                Some(s) => n = leftmost(s),
                None => match node.parent {
                    Some(p) => n = p,
                    None => break,
                },
            }
        }
        out
    }

    /// Unlinks `id` from its parent's child list (or the roots). Its subtree stays intact.
    fn detach(&mut self, id: NodeId) {
        let Some(n) = self.nodes.get(id) else {
            return;
        };
        let (parent, prev, next) = (n.parent, n.prev, n.next);
        match parent {
            Some(p) => {
                if let Some(pv) = prev {
                    if let Some(x) = self.nodes.get_mut(pv) {
                        x.next = next;
                    }
                }
                if let Some(nx) = next {
                    if let Some(x) = self.nodes.get_mut(nx) {
                        x.prev = prev;
                    }
                }
                if let Some(pn) = self.nodes.get_mut(p) {
                    if pn.first_child == Some(id) {
                        pn.first_child = next;
                    }
                    if pn.last_child == Some(id) {
                        pn.last_child = prev;
                    }
                    pn.child_count = pn.child_count.saturating_sub(1);
                }
            }
            None => self.roots.retain(|r| *r != id),
        }
        if let Some(n) = self.nodes.get_mut(id) {
            n.parent = None;
            n.prev = None;
            n.next = None;
        }
    }

    /// Links a detached `id` under `parent` before its `index`-th child (`None` or past the
    /// end: last).
    fn attach(&mut self, id: NodeId, parent: Option<NodeId>, index: Option<usize>) {
        let Some(p) = parent else {
            let i = index.unwrap_or(usize::MAX).min(self.roots.len());
            self.roots.insert(i, id);
            return;
        };
        let before = index.and_then(|i| self.children(p).nth(i));
        self.attach_before(id, p, before);
    }

    /// Links a detached `id` under `p` before its child `before` (`None`: last).
    fn attach_before(&mut self, id: NodeId, p: NodeId, before: Option<NodeId>) {
        let prev = match before {
            Some(b) => self.nodes.get(b).and_then(|x| x.prev),
            None => self.nodes.get(p).and_then(|x| x.last_child),
        };
        if let Some(n) = self.nodes.get_mut(id) {
            n.parent = Some(p);
            n.prev = prev;
            n.next = before;
        }
        match prev {
            Some(pv) => {
                if let Some(x) = self.nodes.get_mut(pv) {
                    x.next = Some(id);
                }
            }
            None => {
                if let Some(pn) = self.nodes.get_mut(p) {
                    pn.first_child = Some(id);
                }
            }
        }
        match before {
            Some(b) => {
                if let Some(x) = self.nodes.get_mut(b) {
                    x.prev = Some(id);
                }
            }
            None => {
                if let Some(pn) = self.nodes.get_mut(p) {
                    pn.last_child = Some(id);
                }
            }
        }
        if let Some(pn) = self.nodes.get_mut(p) {
            pn.child_count = pn.child_count.saturating_add(1);
        }
    }

    /// Checks every structural invariant: parent/child links are consistent, `child_count`
    /// matches, `prev`/`next` are symmetric, there are no cycles, roots have no parent and every
    /// parentless node is a root.
    pub fn check_invariants(&self) -> Result<(), InvariantError> {
        let limit = self.nodes.len() + 1;
        let err = |node, what, other| Err(InvariantError { node, what, other });
        let mut linked = 0usize;
        for (id, n) in self.nodes.iter() {
            // Parent chain must end at a root within `limit` steps.
            let mut steps = 0;
            let mut cur = n.parent;
            while let Some(p) = cur {
                steps += 1;
                if steps > limit {
                    return err(id, "cycle in parent chain", Some(p));
                }
                match self.nodes.get(p) {
                    Some(pn) => cur = pn.parent,
                    None => return err(id, "parent is not alive", Some(p)),
                }
            }
            match n.parent {
                None => {
                    if !self.roots.contains(&id) {
                        return err(id, "parentless node is not a root", None);
                    }
                }
                Some(p) if self.roots.contains(&id) => return err(id, "root has a parent", Some(p)),
                Some(_) => {}
            }
            // The child list.
            let mut count = 0usize;
            let mut prev: Option<NodeId> = None;
            let mut cur = n.first_child;
            while let Some(c) = cur {
                count += 1;
                if count > limit {
                    return err(id, "cycle in child list", Some(c));
                }
                let Some(cn) = self.nodes.get(c) else {
                    return err(id, "child is not alive", Some(c));
                };
                if cn.parent != Some(id) {
                    return err(c, "child's parent link does not point back", Some(id));
                }
                if cn.prev != prev {
                    return err(c, "prev link is not symmetric", prev);
                }
                prev = Some(c);
                cur = cn.next;
            }
            if prev != n.last_child {
                return err(id, "last_child does not end the child list", n.last_child);
            }
            if count != usize::from(n.child_count) {
                return err(id, "child_count does not match the child list", None);
            }
            linked += count;
        }
        for &r in &self.roots {
            match self.nodes.get(r) {
                None => return err(r, "root is not alive", None),
                Some(n) if n.parent.is_some() => return err(r, "root has a parent", n.parent),
                Some(_) => {}
            }
        }
        if linked + self.roots.len() != self.nodes.len() {
            return err(
                self.roots.first().copied().unwrap_or_default(),
                "node count differs from linked nodes",
                None,
            );
        }
        Ok(())
    }

    /// With `debug-checks`: verifies the invariants after a mutation.
    #[inline]
    #[cfg_attr(not(feature = "debug-checks"), allow(clippy::unused_self))]
    fn debug_check(&self) {
        #[cfg(feature = "debug-checks")]
        if let Err(e) = self.check_invariants() {
            twine_core::error!(target: "twine::engine", "{}", e);
            debug_assert!(false, "{e}");
        }
    }

    /// Writes an indented dump of `root`'s subtree, one node per line:
    /// `obj n3g1 "test_id" [x0,y0 → x1,y1] state=PRESSED flags=CLICKABLE|… text="…"`.
    pub fn dump(&self, root: NodeId, out: &mut dyn Write) -> fmt::Result {
        for id in self.descendants(root) {
            let depth = self.ancestors(id).take_while(|a| *a != root).count() + usize::from(id != root);
            let Some(n) = self.node(id) else { continue };
            for _ in 0..depth {
                out.write_str("  ")?;
            }
            write!(out, "{} {}", n.class.name, fmt_node_id(id))?;
            if let Some(t) = n.test_id() {
                write!(out, " {t:?}")?;
            }
            let c = n.coords;
            write!(
                out,
                " [{},{} → {},{}] state={} flags={}",
                c.x0,
                c.y0,
                c.x1,
                c.y1,
                flag_names(n.state),
                flag_names(n.flags)
            )?;
            if let Some(t) = n.widget.text() {
                write!(out, " text={t:?}")?;
            }
            out.write_char('\n')?;
        }
        Ok(())
    }
}

/// Iterator over a node's children (see [`Tree::children`]).
#[derive(Clone, Debug)]
pub struct Children<'a> {
    tree: &'a Tree,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let c = self.next?;
        self.next = self.tree.node(c).and_then(|n| n.next);
        Some(c)
    }
}

/// Iterator over a node's children, last first (see [`Tree::children_rev`]).
#[derive(Clone, Debug)]
pub struct ChildrenRev<'a> {
    tree: &'a Tree,
    next: Option<NodeId>,
}

impl Iterator for ChildrenRev<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let c = self.next?;
        self.next = self.tree.node(c).and_then(|n| n.prev);
        Some(c)
    }
}

/// Iterator over a node's ancestors (see [`Tree::ancestors`]).
#[derive(Clone, Debug)]
pub struct Ancestors<'a> {
    tree: &'a Tree,
    next: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let a = self.next?;
        self.next = self.tree.parent(a);
        Some(a)
    }
}

/// Pre-order iterator over a subtree (see [`Tree::descendants`]).
#[derive(Clone, Debug)]
pub struct Descendants<'a> {
    tree: &'a Tree,
    root: NodeId,
    next: Option<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let cur = self.next?;
        let n = self.tree.node(cur)?;
        self.next = if let Some(c) = n.first_child {
            Some(c)
        } else {
            let mut x = cur;
            loop {
                if x == self.root {
                    break None;
                }
                let xn = self.tree.node(x)?;
                if let Some(s) = xn.next {
                    break Some(s);
                }
                match xn.parent {
                    Some(p) => x = p,
                    None => break None,
                }
            }
        };
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Obj;
    use alloc::string::String;
    use alloc::vec;
    use proptest::prelude::*;

    fn obj() -> Box<dyn Widget> {
        Box::new(Obj)
    }

    fn tree_with(n: usize) -> (Tree, NodeId, Vec<NodeId>) {
        let mut t = Tree::new();
        let root = t.create(None, obj()).unwrap();
        let kids = (0..n).map(|_| t.create(Some(root), obj()).unwrap()).collect();
        (t, root, kids)
    }

    #[test]
    fn create_appends_in_order() {
        let (t, root, kids) = tree_with(3);
        assert_eq!(t.children(root).collect::<Vec<_>>(), kids);
        assert_eq!(
            t.children_rev(root).collect::<Vec<_>>(),
            kids.iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(t.node(root).unwrap().child_count(), 3);
        assert_eq!(t.index(kids[2]), Some(2));
        assert_eq!(t.parent(kids[1]), Some(root));
        assert_eq!(t.roots().collect::<Vec<_>>(), vec![root]);
        assert!(t.node(kids[0]).unwrap().flags().contains(ObjFlags::CLICKABLE));
        t.check_invariants().unwrap();
    }

    #[test]
    fn create_under_missing_parent_fails() {
        let (mut t, _, kids) = tree_with(1);
        t.delete(kids[0]).unwrap();
        assert_eq!(
            t.create(Some(kids[0]), obj()),
            Err(EngineError::NodeNotFound(kids[0]))
        );
    }

    #[test]
    fn delete_subtree_frees_all_and_invalidates_ids() {
        let (mut t, root, kids) = tree_with(2);
        let gc = t.create(Some(kids[0]), obj()).unwrap();
        t.delete(kids[0]).unwrap();
        assert!(!t.contains(kids[0]) && !t.contains(gc));
        assert_eq!(t.len(), 2);
        // Slot reuse must not resurrect the old ids.
        let a = t.create(Some(root), obj()).unwrap();
        let b = t.create(Some(root), obj()).unwrap();
        assert!(!t.contains(kids[0]) && !t.contains(gc));
        assert!(a.index() == kids[0].index() || b.index() == kids[0].index() || a.index() == gc.index());
        assert_eq!(t.children(root).collect::<Vec<_>>(), vec![kids[1], a, b]);
        t.check_invariants().unwrap();
        assert!(t.delete(kids[0]).is_err());
    }

    #[test]
    fn delete_returns_post_order() {
        let (mut t, root, kids) = tree_with(2);
        let a = t.create(Some(kids[0]), obj()).unwrap();
        let b = t.create(Some(kids[0]), obj()).unwrap();
        let aa = t.create(Some(a), obj()).unwrap();
        let order = t.delete(root).unwrap();
        assert_eq!(order, vec![aa, a, b, kids[0], kids[1], root]);
        assert!(t.is_empty());
        assert_eq!(t.roots().count(), 0);
    }

    #[test]
    fn move_to_changes_parent_and_index() {
        let (mut t, root, kids) = tree_with(3);
        t.move_to(kids[2], Some(kids[0]), None).unwrap();
        assert_eq!(t.parent(kids[2]), Some(kids[0]));
        assert_eq!(t.node(root).unwrap().child_count(), 2);
        t.move_to(kids[2], Some(root), Some(0)).unwrap();
        assert_eq!(
            t.children(root).collect::<Vec<_>>(),
            vec![kids[2], kids[0], kids[1]]
        );
        t.move_to(kids[1], None, None).unwrap();
        assert_eq!(t.roots().collect::<Vec<_>>(), vec![root, kids[1]]);
        t.check_invariants().unwrap();
    }

    #[test]
    fn move_into_own_subtree_is_rejected() {
        let (mut t, root, kids) = tree_with(1);
        let gc = t.create(Some(kids[0]), obj()).unwrap();
        assert_eq!(
            t.move_to(root, Some(gc), None),
            Err(EngineError::InvalidConfig("cycle"))
        );
        assert_eq!(
            t.move_to(kids[0], Some(kids[0]), None),
            Err(EngineError::InvalidConfig("cycle"))
        );
        t.check_invariants().unwrap();
    }

    #[test]
    fn set_index_and_negative_child_index() {
        let (mut t, root, kids) = tree_with(4);
        t.set_index(kids[3], 1).unwrap();
        assert_eq!(
            t.children(root).collect::<Vec<_>>(),
            vec![kids[0], kids[3], kids[1], kids[2]]
        );
        assert_eq!(t.child(root, 0), Some(kids[0]));
        assert_eq!(t.child(root, -1), Some(kids[2]));
        assert_eq!(t.child(root, -4), Some(kids[0]));
        assert_eq!(t.child(root, -5), None);
        assert_eq!(t.child(root, 4), None);
        t.set_index(kids[0], 99).unwrap();
        assert_eq!(t.child(root, -1), Some(kids[0]));
        t.check_invariants().unwrap();
    }

    #[test]
    fn swap_across_parents() {
        let (mut t, root, kids) = tree_with(3);
        let x = t.create(Some(kids[0]), obj()).unwrap();
        let y = t.create(Some(kids[0]), obj()).unwrap();
        t.swap(kids[1], y).unwrap();
        assert_eq!(t.children(root).collect::<Vec<_>>(), vec![kids[0], y, kids[2]]);
        assert_eq!(t.children(kids[0]).collect::<Vec<_>>(), vec![x, kids[1]]);
        // Same parent, both orders.
        t.swap(kids[0], kids[2]).unwrap();
        assert_eq!(t.children(root).collect::<Vec<_>>(), vec![kids[2], y, kids[0]]);
        t.swap(kids[0], kids[2]).unwrap();
        assert_eq!(t.children(root).collect::<Vec<_>>(), vec![kids[0], y, kids[2]]);
        assert!(t.swap(kids[0], x).is_err());
        t.check_invariants().unwrap();
    }

    #[test]
    fn ancestors_and_root_of() {
        let (mut t, root, kids) = tree_with(1);
        let gc = t.create(Some(kids[0]), obj()).unwrap();
        assert_eq!(t.ancestors(gc).collect::<Vec<_>>(), vec![kids[0], root]);
        assert_eq!(t.root_of(gc), Some(root));
        assert_eq!(t.root_of(root), Some(root));
    }

    #[test]
    fn dump_lists_every_node_indented() {
        let (mut t, root, kids) = tree_with(2);
        t.node_mut(kids[0]).unwrap().coords = Rect::new(1, 2, 3, 4);
        let mut s = String::new();
        t.dump(root, &mut s).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(
            lines[0].starts_with("obj n0g1 [0,0 → 0,0] state=- flags=CLICKABLE"),
            "{}",
            lines[0]
        );
        assert!(lines[1].starts_with("  obj n1g1 [1,2 → 3,4]"), "{}", lines[1]);
    }

    #[test]
    fn invariant_checker_detects_corruption() {
        let (mut t, root, kids) = tree_with(2);
        t.node_mut(root).unwrap().child_count = 5;
        assert!(t.check_invariants().is_err());
        t.node_mut(root).unwrap().child_count = 2;
        t.node_mut(kids[1]).unwrap().prev = None;
        let e = t.check_invariants().unwrap_err();
        assert_eq!(e.node, kids[1]);
    }

    /// A random tree built from `(parent choice)` values; returns the tree and all ids.
    fn random_tree(choices: &[u16]) -> (Tree, Vec<NodeId>) {
        let mut t = Tree::new();
        let mut ids = vec![t.create(None, obj()).unwrap()];
        for &c in choices {
            let p = ids[usize::from(c) % ids.len()];
            ids.push(t.create(Some(p), obj()).unwrap());
        }
        (t, ids)
    }

    fn recursive_preorder(t: &Tree, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        let kids: Vec<NodeId> = t.children(id).collect();
        for c in kids {
            recursive_preorder(t, c, out);
        }
    }

    proptest! {
        #[test]
        fn descendants_preorder_matches_recursive_reference(choices in proptest::collection::vec(any::<u16>(), 0..60), pick in any::<u16>()) {
            let (t, ids) = random_tree(&choices);
            let start = ids[usize::from(pick) % ids.len()];
            let mut reference = Vec::new();
            recursive_preorder(&t, start, &mut reference);
            prop_assert_eq!(t.descendants(start).collect::<Vec<_>>(), reference);
        }

        #[test]
        fn invariants_hold_after_random_ops(ops in proptest::collection::vec((0u8..4, any::<u16>(), any::<u16>(), any::<u8>()), 200)) {
            let mut t = Tree::new();
            let mut ids: Vec<NodeId> = vec![t.create(None, obj()).unwrap()];
            for (op, a, b, idx) in ops {
                let live: Vec<NodeId> = ids.iter().copied().filter(|i| t.contains(*i)).collect();
                if live.is_empty() {
                    ids.push(t.create(None, obj()).unwrap());
                    continue;
                }
                let x = live[usize::from(a) % live.len()];
                let y = live[usize::from(b) % live.len()];
                match op {
                    0 => ids.push(t.create(Some(x), obj()).unwrap()),
                    1 => {
                        if live.len() > 1 {
                            t.delete(x).unwrap();
                        }
                    }
                    2 => {
                        let _ = t.move_to(x, Some(y), Some(usize::from(idx % 5)));
                    }
                    _ => {
                        let _ = t.swap(x, y);
                    }
                }
                prop_assert!(t.check_invariants().is_ok(), "{:?}", t.check_invariants());
            }
        }
    }
}
