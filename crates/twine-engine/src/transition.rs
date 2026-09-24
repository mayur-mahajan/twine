//! Style transitions on state changes (LVGL `lv_obj_style.c`: `obj_transition_states`,
//! `lv_obj_style_create_transition`, `trans_anim_cb`, `trans_anim_start_cb`,
//! `trans_anim_completed_cb`, `remove_trans_styles`; LVGL 9.6).
//!
//! When a rendered node changes state, every style entry that applies in the new state and
//! sets the `Transition` property contributes its [`TransitionDsc`] (for equal properties of
//! the same part, the entry with the higher or equal state weight found first wins). For each
//! property whose value differs between the old and the new state (transitions ignored), the
//! current value (transitions included, so a running transition continues smoothly) is written
//! into the node's transition style of that part — one [`EntryKind::Transition`] entry per
//! part, consulted before every other style — and an animation `0 → 255` (the descriptor's
//! duration, delay and easing, no early apply) mixes it towards the new value with
//! [`interpolate`]. When the animation starts (after its delay) it re-reads the current value
//! and cancels older transitions of the same property; when it completes the property leaves
//! the transition style, whose entry is removed once empty.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;

use twine_anim::{Anim, AnimId};
use twine_style::{
    EntryKind, Part, PropId, ResolveOptions, Selector, State, StyleBuf, StyleEntry, StyleProp, StyleRef,
    StyleValue, TransitionDsc, interpolate, resolve_with,
};

use crate::anim::{Deferred, Op};
use crate::{Engine, NodeId, fmt_node_id};

/// Maximum number of properties that start transitions in one state change (LVGL
/// `STYLE_TRANSITION_MAX`).
const STYLE_TRANSITION_MAX: usize = 32;

/// Events of a transition's animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransOp {
    /// The delay is over (LVGL `trans_anim_start_cb`).
    Start,
    /// New progress `0..=255` (LVGL `trans_anim_cb`).
    Value(i32),
    /// Completed (LVGL `trans_anim_completed_cb`).
    Done,
}

/// A running transition (LVGL `trans_t`).
#[derive(Clone, Copy, Debug)]
struct Transition {
    /// Creation order (newer transitions have larger serials).
    serial: u32,
    node: NodeId,
    part: Part,
    prop: PropId,
    start: StyleValue,
    end: StyleValue,
    anim: AnimId,
}

/// The engine's running transitions.
#[derive(Debug, Default)]
pub(crate) struct TransState {
    list: Vec<Transition>,
    next_serial: u32,
}

fn push_op(ctx: &mut dyn Any, op: Op) {
    if let Some(q) = ctx.downcast_mut::<Deferred>() {
        q.ops.push(op);
    }
}

impl Engine {
    /// Starts the transitions of a state change of `id` from `prev` to `new` (the node is
    /// already in `new`). Nodes that were never drawn change instantly (LVGL 9.6).
    pub(crate) fn start_transitions(&mut self, id: NodeId, prev: State, new: State) {
        let Some(n) = self.tree.node(id) else { return };
        if !n.rendered.get() {
            return;
        }
        let mut ts: heapless::Vec<(Part, State, PropId, &'static TransitionDsc), STYLE_TRANSITION_MAX> =
            heapless::Vec::new();
        'entries: for e in n.styles.entries() {
            if e.kind == EntryKind::Transition || e.selector.state.bits() & !new.bits() != 0 {
                continue;
            }
            let Some(StyleValue::Transition(tr)) = e.style.get(PropId::Transition) else {
                continue;
            };
            let (part, state) = (e.selector.part, e.selector.state);
            for &p in tr.props {
                // A property of the same part from an entry with a higher or equal state
                // weight is already there (LVGL keeps the first such entry).
                if ts.iter().any(|&(tp, ts_state, tprop, _)| {
                    tprop == p && tp == part && ts_state.bits() >= state.bits()
                }) {
                    continue;
                }
                if ts.push((part, state, p, tr)).is_err() {
                    break 'entries;
                }
            }
        }
        for (part, _, prop, dsc) in ts {
            self.create_transition(id, part, prev, new, prop, dsc);
        }
    }

    /// LVGL `lv_obj_style_create_transition`.
    fn create_transition(
        &mut self,
        id: NodeId,
        part: Part,
        prev: State,
        new: State,
        prop: PropId,
        dsc: &'static TransitionDsc,
    ) {
        let defaults = self.style_defaults();
        let at = |state, skip_transitions| ResolveOptions {
            state: Some(state),
            skip_transitions,
        };
        let v1 = resolve_with(&self.tree, id, part, prop, &defaults, at(prev, true));
        let mut v2 = resolve_with(&self.tree, id, part, prop, &defaults, at(new, true));
        if v1 == v2 {
            return;
        }
        // The value shown right now (a running transition included): no jump.
        let mut v1 = resolve_with(&self.tree, id, part, prop, &defaults, at(prev, false));
        self.trans_style_set(id, part, prop, v1);
        self.refresh_style(id, part, Some(prop));
        if prop == PropId::Radius {
            let c = self.coords(id);
            let circle = StyleValue::Int(twine_style::RADIUS_CIRCLE);
            let r = StyleValue::Int((c.width() / 2 + 1).min(c.height() / 2 + 1));
            if v1 == circle {
                v1 = r;
            }
            if v2 == circle {
                v2 = r;
            }
        }
        let serial = self.trans.next_serial;
        self.trans.next_serial = serial.wrapping_add(1);
        let anim = Anim::new(0, 255)
            .duration(dsc.duration)
            .delay(dsc.delay)
            .easing(dsc.easing)
            .early_apply(false)
            .on_start(move |cx| push_op(cx.ctx(), Op::Trans(serial, TransOp::Start)))
            .on_complete(move |cx| push_op(cx.ctx(), Op::Trans(serial, TransOp::Done)));
        let aid = self.anim_start_internal(anim, move |ctx, v| {
            push_op(ctx, Op::Trans(serial, TransOp::Value(v)));
        });
        twine_core::debug!(
            target: "twine::style",
            "{} transition {:?} {:?}: {:?} -> {:?} ({})",
            fmt_node_id(id),
            part,
            prop,
            v1,
            v2,
            dsc.duration
        );
        self.trans.list.push(Transition {
            serial,
            node: id,
            part,
            prop,
            start: v1,
            end: v2,
            anim: aid,
        });
    }

    /// Handles an event of transition `serial` (ignored when it was cancelled meanwhile).
    pub(crate) fn transition_op(&mut self, serial: u32, op: TransOp) {
        let Some(i) = self.trans.list.iter().position(|t| t.serial == serial) else {
            return;
        };
        let t = self.trans.list[i];
        if !self.tree.contains(t.node) {
            return;
        }
        match op {
            TransOp::Start => {
                let start = self.style_prop(t.node, t.part, t.prop);
                self.trans.list[i].start = start;
                self.remove_transitions(t.node, Some(t.part), Some(t.prop), Some(serial));
                self.trans_style_set(t.node, t.part, t.prop, start);
                self.refresh_style(t.node, t.part, Some(t.prop));
            }
            TransOp::Value(v) => {
                let v = interpolate(t.prop, &t.start, &t.end, v.clamp(0, 255) as u8);
                if self.trans_style_set(t.node, t.part, t.prop, v) {
                    self.refresh_style(t.node, t.part, Some(t.prop));
                }
            }
            TransOp::Done => {
                self.trans.list.remove(i);
                let running = self
                    .trans
                    .list
                    .iter()
                    .any(|o| o.node == t.node && o.part == t.part && o.prop == t.prop);
                if !running {
                    let before = self.style_prop(t.node, t.part, t.prop);
                    self.trans_style_remove(t.node, Some(t.part), t.prop);
                    if self.style_prop(t.node, t.part, t.prop) != before {
                        self.refresh_style(t.node, t.part, Some(t.prop));
                    }
                }
            }
        }
    }

    /// LVGL `remove_trans_styles`: cancels the transitions of `id` matching `part` and `prop`
    /// (`None` = any), only those older than `older_than` if given, and removes their
    /// properties from the transition styles. Returns whether any was removed (the caller
    /// refreshes the style).
    pub(crate) fn remove_transitions(
        &mut self,
        id: NodeId,
        part: Option<Part>,
        prop: Option<PropId>,
        older_than: Option<u32>,
    ) -> bool {
        let mut removed = false;
        let mut i = 0;
        while i < self.trans.list.len() {
            let t = self.trans.list[i];
            let hit = t.node == id
                && part.is_none_or(|p| p == t.part)
                && prop.is_none_or(|p| p == t.prop)
                && older_than.is_none_or(|s| (t.serial.wrapping_sub(s) as i32) < 0);
            if !hit {
                i += 1;
                continue;
            }
            self.trans.list.remove(i);
            self.anim.timeline.remove(t.anim);
            self.trans_style_remove(id, part, t.prop);
            removed = true;
        }
        removed
    }

    /// Forgets the transitions of deleted nodes (their animations are stopped; the style
    /// entries went away with the nodes).
    pub(crate) fn transitions_forget_nodes(&mut self, ids: &[NodeId]) {
        let mut i = 0;
        while i < self.trans.list.len() {
            if ids.contains(&self.trans.list[i].node) {
                let t = self.trans.list.remove(i);
                self.anim.timeline.remove(t.anim);
            } else {
                i += 1;
            }
        }
    }

    /// Number of running style transitions (all nodes).
    #[must_use]
    pub fn transition_count(&self) -> usize {
        self.trans.list.len()
    }

    /// Sets `prop = value` in the transition style of `part` of `id` (created when missing).
    /// Returns whether the style changed.
    fn trans_style_set(&mut self, id: NodeId, part: Part, prop: PropId, value: StyleValue) -> bool {
        let Some(p) = StyleProp::from_value(prop, value) else {
            twine_core::warn!(target: "twine::style", "transition: {:?} cannot hold {:?}", prop, value);
            return false;
        };
        let Some(n) = self.tree.node_mut(id) else {
            return false;
        };
        let changed = n.styles.transition_mut(part).set(p);
        if changed {
            n.style_cache.invalidate();
        }
        changed
    }

    /// Removes `prop` from the transition styles of `id` (of `part`, or of every part), and
    /// drops transition entries that became empty.
    fn trans_style_remove(&mut self, id: NodeId, part: Option<Part>, prop: PropId) {
        let Some(n) = self.tree.node_mut(id) else { return };
        n.styles.transition_remove(part, prop);
        n.style_cache.invalidate();
    }
}

impl crate::StyleList {
    /// The transition style of `part`, created when missing.
    pub(crate) fn transition_mut(&mut self, part: Part) -> &mut StyleBuf {
        let find = |l: &Self| {
            l.entries().iter().position(|e| {
                e.kind == EntryKind::Transition
                    && e.selector.part == part
                    && matches!(e.style, StyleRef::Shared(_))
            })
        };
        let pos = if let Some(p) = find(self) {
            p
        } else {
            self.insert(StyleEntry::new(
                Selector::part(part),
                Rc::new(StyleBuf::new()),
                EntryKind::Transition,
            ));
            find(self).unwrap_or(0)
        };
        match &mut self.entries_mut()[pos].style {
            StyleRef::Shared(rc) => Rc::make_mut(rc),
            StyleRef::Static(_) => unreachable!("transition entries are shared buffers"),
        }
    }

    /// Removes `prop` from the transition entries of `part` (`None` = all parts); empty
    /// transition entries are dropped.
    pub(crate) fn transition_remove(&mut self, part: Option<Part>, prop: PropId) {
        for e in self.entries_mut() {
            if e.kind == EntryKind::Transition && part.is_none_or(|p| p == e.selector.part || p == Part::Any)
            {
                if let StyleRef::Shared(rc) = &mut e.style {
                    if rc.get(prop).is_some() {
                        Rc::make_mut(rc).remove(prop);
                    }
                }
            }
        }
        self.remove_where(|e| {
            e.kind == EntryKind::Transition && matches!(&e.style, StyleRef::Shared(b) if b.is_empty())
        });
    }
}
