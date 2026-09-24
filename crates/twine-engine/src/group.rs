//! Focus groups (LVGL `lv_group`): [`GroupId`], [`RefocusPolicy`] and the group API of
//! [`Engine`].
//!
//! A group is an ordered list of nodes of which at most one is focused. Keypads and encoders
//! attached to a group ([`Engine::set_input_group`]) send their keys to the focused node and
//! move the focus with `Next`/`Prev` (keypad) or rotation (encoder). The focused node gets
//! [`State::FOCUSED`](crate::State::FOCUSED), plus `FOCUS_KEY` when a keypad or encoder moved
//! the focus and `EDITED` while the group is in edit mode.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use crate::{
    Engine, EngineError, EventCode, EventParam, GroupDef, InputId, NodeId, ObjFlags, State, fmt_node_id,
};

/// Most groups alive at once.
pub const MAX_GROUPS: usize = 16;

/// Handle of a focus group created with [`Engine::create_group`]. Printed as `g0`, `g1`, ….
///
/// Slots of deleted groups are reused by later groups.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct GroupId(u8);

impl GroupId {
    /// The group's slot index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "g{}", self.0)
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "g{}", self.0)
    }
}

/// Which node gets the focus when the focused node is removed from its group (LVGL
/// `lv_group_refocus_policy_t`; LVGL's default is `Prev`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RefocusPolicy {
    /// The next node.
    Next,
    /// The previous node.
    #[default]
    Prev,
}

/// Called after the focus moved to a node.
pub type FocusCb = Box<dyn FnMut(&mut Engine, NodeId)>;
/// Called when `focus_next` (`true`) or `focus_prev` (`false`) hit the end of a group that does
/// not wrap.
pub type EdgeCb = Box<dyn FnMut(&mut Engine, bool)>;

/// A focus group.
pub(crate) struct Group {
    nodes: Vec<NodeId>,
    focused: Option<NodeId>,
    editing: bool,
    wrap: bool,
    frozen: bool,
    refocus_policy: RefocusPolicy,
    focus_cb: Option<FocusCb>,
    edge_cb: Option<EdgeCb>,
}

impl Group {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            focused: None,
            editing: false,
            wrap: true,
            frozen: false,
            refocus_policy: RefocusPolicy::Prev,
            focus_cb: None,
            edge_cb: None,
        }
    }
}

impl Engine {
    fn group_ref(&self, g: GroupId) -> Option<&Group> {
        self.groups.get(g.index()).and_then(Option::as_ref)
    }

    fn group_mut(&mut self, g: GroupId) -> Option<&mut Group> {
        self.groups.get_mut(g.index()).and_then(Option::as_mut)
    }

    fn warn_group(what: &str, g: GroupId) {
        let _ = what;
        twine_core::warn!(target: "twine::input", "{}: group {} not found", what, g);
    }

    /// Creates an empty focus group (wrapping, not editing, [`RefocusPolicy::Prev`]).
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj, State};
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// let (a, b) = (e.create(root, Box::new(Obj)).unwrap(), e.create(root, Box::new(Obj)).unwrap());
    /// let g = e.create_group().unwrap();
    /// e.group_add(g, a);
    /// e.group_add(g, b);
    /// assert_eq!(e.focused(g), Some(a)); // the first node added gets the focus
    /// e.focus_next(g);
    /// assert_eq!(e.focused(g), Some(b));
    /// assert!(e.tree().node(b).unwrap().state().contains(State::FOCUSED));
    /// ```
    pub fn create_group(&mut self) -> Result<GroupId, EngineError> {
        let idx = match self.groups.iter().position(Option::is_none) {
            Some(i) => i,
            None if self.groups.len() < MAX_GROUPS => {
                self.groups.push(None);
                self.groups.len() - 1
            }
            None => {
                twine_core::warn!(target: "twine::input", "create_group: more than {} groups", MAX_GROUPS);
                return Err(EngineError::TooManyGroups);
            }
        };
        self.groups[idx] = Some(Group::new());
        let g = GroupId(idx as u8);
        twine_core::debug!(target: "twine::input", "group {} created", g);
        Ok(g)
    }

    /// Deletes a group: the focused node is defocused, the members leave the group, inputs
    /// using it get no group, and it stops being the default group.
    pub fn delete_group(&mut self, g: GroupId) {
        let Some(focused) = self.group_ref(g).map(|gr| gr.focused) else {
            Self::warn_group("delete_group", g);
            return;
        };
        if let Some(f) = focused {
            self.send_event(f, EventCode::Defocused, EventParam::None);
        }
        let Some(group) = self.groups.get_mut(g.index()).and_then(Option::take) else {
            return;
        };
        for n in group.nodes {
            if let Some(node) = self.tree.node_mut(n) {
                node.group = None;
            }
        }
        for st in self.inputs.iter_mut().flatten() {
            if st.group == Some(g) {
                st.group = None;
            }
        }
        if self.default_group == Some(g) {
            self.default_group = None;
        }
    }

    /// Sets the group new widgets of classes with [`GroupDef::True`] join automatically.
    pub fn set_default_group(&mut self, g: Option<GroupId>) {
        if let Some(g) = g {
            if self.group_ref(g).is_none() {
                Self::warn_group("set_default_group", g);
                return;
            }
        }
        self.default_group = g;
    }

    /// The default group.
    #[must_use]
    pub fn default_group(&self) -> Option<GroupId> {
        self.default_group
    }

    /// Appends `id` to group `g` (leaving its previous group first). The first node of a group
    /// gets the focus.
    pub fn group_add(&mut self, g: GroupId, id: NodeId) {
        if self.group_ref(g).is_none() {
            Self::warn_group("group_add", g);
            return;
        }
        if !self.tree.contains(id) {
            twine_core::warn!(target: "twine::input", "group_add: node {} not found", fmt_node_id(id));
            return;
        }
        self.group_remove(id);
        let Some(group) = self.group_mut(g) else {
            return;
        };
        group.nodes.push(id);
        let first = group.nodes.len() == 1;
        if let Some(n) = self.tree.node_mut(id) {
            n.group = Some(g);
        }
        twine_core::trace!(target: "twine::input", "group {}: add {}", g, fmt_node_id(id));
        if first {
            self.refocus(g);
        }
    }

    /// Removes `id` from its group. If it was focused, the focus moves on according to the
    /// group's [`RefocusPolicy`] (or the group ends up without focus).
    pub fn group_remove(&mut self, id: NodeId) {
        let Some(g) = self.group_of(id) else {
            return;
        };
        let Some(group) = self.group_mut(g) else {
            return;
        };
        if group.focused == Some(id) {
            group.frozen = false;
            if group.nodes.len() > 1 {
                self.refocus(g);
            }
        }
        if self.group_ref(g).is_some_and(|gr| gr.focused == Some(id)) {
            if let Some(gr) = self.group_mut(g) {
                gr.focused = None;
            }
            if self.tree.contains(id) {
                self.send_event(id, EventCode::Defocused, EventParam::None);
            }
        }
        if let Some(gr) = self.group_mut(g) {
            gr.nodes.retain(|n| *n != id);
        }
        if let Some(n) = self.tree.node_mut(id) {
            n.group = None;
        }
    }

    /// Removes every node from group `g` (the focused one is defocused).
    pub fn group_remove_all(&mut self, g: GroupId) {
        let Some(focused) = self.group_ref(g).map(|gr| gr.focused) else {
            Self::warn_group("group_remove_all", g);
            return;
        };
        if let Some(f) = focused {
            if let Some(gr) = self.group_mut(g) {
                gr.focused = None;
            }
            self.send_event(f, EventCode::Defocused, EventParam::None);
        }
        let nodes = self
            .group_mut(g)
            .map(|gr| core::mem::take(&mut gr.nodes))
            .unwrap_or_default();
        for n in nodes {
            if let Some(node) = self.tree.node_mut(n) {
                node.group = None;
            }
        }
    }

    /// Focuses `id` in its group (LVGL `lv_group_focus_obj`): leaves edit mode, defocuses the
    /// previous node. Ignored when the group is frozen or `id` is in no group.
    pub fn focus(&mut self, id: NodeId) {
        let Some(g) = self.group_of(id) else {
            twine_core::debug!(target: "twine::input", "focus: {} is in no group", fmt_node_id(id));
            return;
        };
        if self.group_ref(g).is_none_or(|gr| gr.frozen) {
            return;
        }
        self.set_editing(g, false);
        let Some(prev) = self.group_ref(g).map(|gr| gr.focused) else {
            return;
        };
        if prev == Some(id) {
            return;
        }
        if let Some(p) = prev {
            self.send_event(p, EventCode::Defocused, EventParam::None);
        }
        let Some(gr) = self.group_mut(g) else {
            return;
        };
        if !gr.nodes.contains(&id) {
            return;
        }
        gr.focused = Some(id);
        twine_core::debug!(target: "twine::input", "group {}: focus {}", g, fmt_node_id(id));
        self.run_focus_cb(g, id);
        // `Focused` scrolls the node into view when it has `SCROLL_ON_FOCUS`.
        if self.tree.contains(id) {
            self.send_event(id, EventCode::Focused, EventParam::None);
        }
    }

    /// Focuses the next focusable node of `g` (skipping hidden and disabled nodes; wrapping if
    /// enabled, else calling the edge callback at the end).
    pub fn focus_next(&mut self, g: GroupId) {
        if !self.focus_next_core(g, true) {
            self.run_edge_cb(g, true);
        }
    }

    /// Focuses the previous focusable node of `g` (see [`focus_next`](Self::focus_next)).
    pub fn focus_prev(&mut self, g: GroupId) {
        if !self.focus_next_core(g, false) {
            self.run_edge_cb(g, false);
        }
    }

    /// Freezes (`true`) the focus of `g`: focus changes are ignored until unfrozen.
    pub fn focus_freeze(&mut self, g: GroupId, frozen: bool) {
        match self.group_mut(g) {
            Some(gr) => gr.frozen = frozen,
            None => Self::warn_group("focus_freeze", g),
        }
    }

    /// Enters or leaves edit mode (encoder). The focused node receives `Focused` again, which
    /// adds or removes its `EDITED` state.
    pub fn set_editing(&mut self, g: GroupId, editing: bool) {
        let Some(gr) = self.group_mut(g) else {
            Self::warn_group("set_editing", g);
            return;
        };
        if gr.editing == editing {
            return;
        }
        gr.editing = editing;
        let focused = gr.focused;
        twine_core::debug!(target: "twine::input", "group {}: editing {}", g, editing);
        if let Some(f) = focused {
            self.send_event(f, EventCode::Focused, EventParam::None);
        }
    }

    /// Whether `g` is in edit mode.
    #[must_use]
    pub fn group_editing(&self, g: GroupId) -> bool {
        self.group_ref(g).is_some_and(|gr| gr.editing)
    }

    /// Whether `focus_next`/`focus_prev` wrap around at the ends (default `true`).
    pub fn set_wrap(&mut self, g: GroupId, wrap: bool) {
        match self.group_mut(g) {
            Some(gr) => gr.wrap = wrap,
            None => Self::warn_group("set_wrap", g),
        }
    }

    /// Sets the callback run after the focus moved (it may use the engine).
    pub fn set_focus_cb(&mut self, g: GroupId, cb: Option<FocusCb>) {
        match self.group_mut(g) {
            Some(gr) => gr.focus_cb = cb,
            None => Self::warn_group("set_focus_cb", g),
        }
    }

    /// Sets the callback run when a non-wrapping group hits its end.
    pub fn set_edge_cb(&mut self, g: GroupId, cb: Option<EdgeCb>) {
        match self.group_mut(g) {
            Some(gr) => gr.edge_cb = cb,
            None => Self::warn_group("set_edge_cb", g),
        }
    }

    /// Sets where the focus goes when the focused node leaves the group.
    pub fn set_refocus_policy(&mut self, g: GroupId, policy: RefocusPolicy) {
        match self.group_mut(g) {
            Some(gr) => gr.refocus_policy = policy,
            None => Self::warn_group("set_refocus_policy", g),
        }
    }

    /// The focused node of `g`.
    #[must_use]
    pub fn focused(&self, g: GroupId) -> Option<NodeId> {
        self.group_ref(g).and_then(|gr| gr.focused)
    }

    /// The group `id` belongs to.
    #[must_use]
    pub fn group_of(&self, id: NodeId) -> Option<GroupId> {
        self.tree.node(id).and_then(|n| n.group)
    }

    /// Number of nodes in `g`.
    #[must_use]
    pub fn group_count(&self, g: GroupId) -> usize {
        self.group_ref(g).map_or(0, |gr| gr.nodes.len())
    }

    /// The nodes of `g` in focus order.
    #[must_use]
    pub fn group_nodes(&self, g: GroupId) -> &[NodeId] {
        self.group_ref(g).map_or(&[], |gr| &gr.nodes)
    }

    /// Attaches a keypad or encoder to group `g` (`None` detaches it).
    pub fn set_input_group(&mut self, input: InputId, g: Option<GroupId>) {
        if let Some(g) = g {
            if self.group_ref(g).is_none() {
                Self::warn_group("set_input_group", g);
                return;
            }
        }
        match self.input_state_mut(input) {
            Some(st) => st.group = g,
            None => twine_core::warn!(target: "twine::input", "set_input_group: input {} not found", input),
        }
    }

    /// Adds a new node to the default group if its class asks for it (`GroupDef::True`).
    pub(crate) fn group_auto_add(&mut self, id: NodeId) {
        let Some(g) = self.default_group else {
            return;
        };
        if self
            .tree
            .node(id)
            .is_some_and(|n| n.class().group_def == GroupDef::True)
        {
            self.group_add(g, id);
        }
    }

    /// LVGL `lv_group_refocus`: moves the focus per policy, temporarily wrapping.
    fn refocus(&mut self, g: GroupId) {
        let Some(gr) = self.group_mut(g) else {
            return;
        };
        let wrap = gr.wrap;
        gr.wrap = true;
        let next = gr.refocus_policy == RefocusPolicy::Next;
        if next {
            self.focus_next(g);
        } else {
            self.focus_prev(g);
        }
        if let Some(gr) = self.group_mut(g) {
            gr.wrap = wrap;
        }
    }

    /// Whether `id` can take the focus: not disabled, neither it nor an ancestor hidden.
    fn focusable(&self, id: NodeId) -> bool {
        let Some(n) = self.tree.node(id) else {
            return false;
        };
        if n.state.contains(State::DISABLED) || n.is_hidden() {
            return false;
        }
        !self.tree.ancestors(id).any(|a| {
            self.tree
                .node(a)
                .is_some_and(|x| x.flags.contains(ObjFlags::HIDDEN))
        })
    }

    /// Port of LVGL `focus_next_core`. Returns whether the focus changed.
    fn focus_next_core(&mut self, g: GroupId, forward: bool) -> bool {
        let Some(gr) = self.group_ref(g) else {
            Self::warn_group(if forward { "focus_next" } else { "focus_prev" }, g);
            return false;
        };
        if gr.frozen {
            return false;
        }
        let len = gr.nodes.len();
        let begin = || if forward { Some(0) } else { len.checked_sub(1) };
        let step = |i: usize| -> Option<usize> {
            if forward {
                (i + 1 < len).then_some(i + 1)
            } else {
                i.checked_sub(1)
            }
        };
        let current = gr.focused.and_then(|f| gr.nodes.iter().position(|n| *n == f));
        let mut next = current;
        let mut sentinel: Option<usize> = None;
        let (mut can_move, mut can_begin) = (true, true);
        loop {
            if next.is_none() {
                if gr.wrap || sentinel.is_none() {
                    if !can_begin {
                        return false;
                    }
                    next = if len == 0 { None } else { begin() };
                    can_move = false;
                    can_begin = false;
                } else {
                    return false;
                }
            }
            if sentinel.is_none() {
                sentinel = next;
                if sentinel.is_none() {
                    return false; // empty group
                }
            }
            if can_move {
                next = next.and_then(step);
                if next == sentinel {
                    return false;
                }
            }
            can_move = true;
            let Some(i) = next else {
                continue;
            };
            if self.focusable(gr.nodes[i]) {
                break;
            }
        }
        if next == current {
            return false;
        }
        let Some(new) = next.map(|i| gr.nodes[i]) else {
            return false;
        };
        let old = gr.focused;
        if let Some(o) = old {
            self.send_event(o, EventCode::Defocused, EventParam::None);
        }
        if !self.tree.contains(new) {
            return false;
        }
        let Some(gr) = self.group_mut(g) else {
            return false;
        };
        if !gr.nodes.contains(&new) {
            return false;
        }
        gr.focused = Some(new);
        twine_core::debug!(target: "twine::input", "group {}: focus {}", g, fmt_node_id(new));
        // `Focused` scrolls the node into view when it has `SCROLL_ON_FOCUS`.
        self.send_event(new, EventCode::Focused, EventParam::None);
        self.run_focus_cb(g, new);
        true
    }

    fn run_focus_cb(&mut self, g: GroupId, node: NodeId) {
        let Some(mut cb) = self.group_mut(g).and_then(|gr| gr.focus_cb.take()) else {
            return;
        };
        cb(self, node);
        if let Some(gr) = self.group_mut(g) {
            if gr.focus_cb.is_none() {
                gr.focus_cb = Some(cb);
            }
        }
    }

    fn run_edge_cb(&mut self, g: GroupId, next: bool) {
        let Some(mut cb) = self.group_mut(g).and_then(|gr| gr.edge_cb.take()) else {
            return;
        };
        cb(self, next);
        if let Some(gr) = self.group_mut(g) {
            if gr.edge_cb.is_none() {
                gr.edge_cb = Some(cb);
            }
        }
    }
}
