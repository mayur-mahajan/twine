//! Style transitions on state changes (LVGL `lv_obj_style.c`: `obj_transition_states`,
//! `lv_obj_style_create_transition`, `trans_anim_cb`, `trans_anim_start_cb`,
//! `trans_anim_completed_cb`, `remove_trans_styles`; LVGL 9.6).
//!
//! When a rendered node changes state, every style entry that applies in the new state and
//! sets the `transition` property contributes its [`Transition`]: its [`Props`], or — for
//! [`Transition::all`] — the interpolable properties set by the entries of the part that start
//! or stop applying with the change (bitsets: O(properties) per state change). For a property
//! of a part covered by several transitions, the entry with the higher state weight wins (the
//! first found among equal weights). For each property whose value differs between the old and
//! the new state (transitions ignored), the
//! current value (transitions included, so a running transition continues smoothly) is written
//! into the node's transition style of that part — one [`EntryKind::Transition`] entry per
//! part, consulted before every other style — and an animation `0 → 255` (the transition's
//! [`AnimSpec`](twine_anim::AnimSpec) forward play, adjusted to the engine's
//! [`Motion`](twine_anim::Motion) preference like every animation; with `Motion::None` no
//! transition is created) mixes it towards the new value with
//! [`interpolate`]. When the animation starts (after its delay) it re-reads the current value
//! and cancels older transitions of the same property; when it completes the property leaves
//! the transition style, whose entry is removed once empty.
//!
//! Steady state allocates nothing (P4): the animations carry the transition's serial as an
//! [`AnimTarget::Custom`] target (no boxed closures or callbacks; the start is noticed at the
//! first value, the end when the animation has left the timeline), the emptied transition
//! style buffers go to a small pool that the next transition reuses, and a transition built at
//! run time (`.transition(Transition::all(..))` on a view) was created once at build time and
//! is only read here.

use alloc::rc::Rc;
use alloc::vec::Vec;

use twine_anim::{Anim, AnimId, AnimSpec, AnimTarget, Motion, Repeat};
use twine_style::{
    EntryKind, Part, PropId, Props, ResolveOptions, Selector, State, StyleBuf, StyleEntry, StyleProp,
    StyleRef, StyleValue, TracksRef, Transition, interpolate, resolve_with,
};

use crate::{Engine, NodeId, fmt_node_id};

/// Maximum number of properties that start transitions in one state change (LVGL
/// `STYLE_TRANSITION_MAX`).
const STYLE_TRANSITION_MAX: usize = 32;

/// Emptied transition style buffers kept for reuse (more are freed).
const TRANS_POOL_MAX: usize = 8;

/// Maximum number of style entries with a `transition` property considered in one state
/// change (more are ignored with a warning; a theme uses two, a view adds one or two).
const TRANSITION_ENTRIES_MAX: usize = 8;

/// Maximum number of parts tracked in one state change.
const PARTS_MAX: usize = 8;

/// A set of properties per part (no allocation; at most [`PARTS_MAX`] parts).
#[derive(Default)]
struct PartProps(heapless::Vec<(Part, Props), PARTS_MAX>);

impl PartProps {
    fn get(&self, part: Part) -> Props {
        self.0
            .iter()
            .find(|(p, _)| *p == part)
            .map_or(Props::EMPTY, |(_, s)| *s)
    }

    fn add(&mut self, part: Part, props: Props) {
        if let Some((_, s)) = self.0.iter_mut().find(|(p, _)| *p == part) {
            *s |= props;
        } else if self.0.push((part, props)).is_err() {
            twine_core::warn!(target: "twine::style", "transition: more than {} parts in one state change", PARTS_MAX);
        }
    }
}

/// A running transition (LVGL `trans_t`).
#[derive(Clone, Copy, Debug)]
struct Running {
    /// Creation order (newer transitions have larger serials).
    serial: u32,
    node: NodeId,
    part: Part,
    prop: PropId,
    start: StyleValue,
    end: StyleValue,
    anim: AnimId,
    /// Whether the delay is over (the first value arrived).
    started: bool,
}

/// The engine's running transitions.
#[derive(Debug, Default)]
pub(crate) struct TransState {
    list: Vec<Running>,
    next_serial: u32,
    /// Empty transition style buffers for reuse (no allocation per state change).
    pool: Vec<Rc<StyleBuf>>,
}

impl Engine {
    /// Starts the transitions of a state change of `id` from `prev` to `new` (the node is
    /// already in `new`). Nodes that were never drawn change instantly (LVGL 9.6).
    pub(crate) fn start_transitions(&mut self, id: NodeId, prev: State, new: State) {
        let Some(n) = self.tree.node(id) else { return };
        if !n.rendered.get() {
            return;
        }
        // Items are drawn in their own states without transitions.
        let items = n.class().item_parts;
        let applies = |e: &StyleEntry, s: State| e.selector.state_matches(s);
        // The entries with a transition that apply in the new state: (part, selector weight =
        // state precedence, style). The style is shared (a reference count), so its transition stays readable
        // while the transitions are created.
        let mut cands: heapless::Vec<(Part, u16, StyleRef), TRANSITION_ENTRIES_MAX> = heapless::Vec::new();
        let mut derived = false;
        for e in n.styles.entries() {
            if e.kind == EntryKind::Transition || !applies(e, new) || items.contains(&e.selector.part) {
                continue;
            }
            let Some(t) = e.style.get_transition() else {
                continue;
            };
            derived |= t.props.is_none();
            if cands
                .push((e.selector.part, e.selector.weight(), e.style.clone()))
                .is_err()
            {
                twine_core::warn!(
                    target: "twine::style",
                    "{}: more than {} transitions in one state change, the rest ignored",
                    fmt_node_id(id),
                    TRANSITION_ENTRIES_MAX
                );
                break;
            }
        }
        if cands.is_empty() {
            return;
        }
        // What the change alters, per part: the keys of the entries that start or stop
        // applying (only needed for `Transition::all`).
        let mut changed = PartProps::default();
        if derived {
            for e in n.styles.entries() {
                if e.kind != EntryKind::Transition
                    && applies(e, prev) != applies(e, new)
                    && !items.contains(&e.selector.part)
                {
                    changed.add(e.selector.part, e.style.keys());
                }
            }
        }
        // Higher state precedence first; equal weights keep the entries' order (insertion sort:
        // a handful of entries, stable, no allocation).
        for i in 1..cands.len() {
            let mut j = i;
            while j > 0 && cands[j - 1].1 < cands[j].1 {
                cands.swap(j - 1, j);
                j -= 1;
            }
        }
        let motion = self.anim.motion;
        let mut claimed = PartProps::default();
        let mut started = 0;
        for (part, _, style) in &cands {
            let Some(t) = style.get_transition() else { continue };
            let props = t.animated(changed.get(*part)) - claimed.get(*part);
            claimed.add(*part, props);
            // No motion: the new values apply at once (essential transitions still run).
            if motion == Motion::None && !t.spec.essential {
                continue;
            }
            for prop in props.iter() {
                if prop == PropId::Transition {
                    continue;
                }
                if started == STYLE_TRANSITION_MAX {
                    return;
                }
                if self.create_transition(id, *part, prev, new, prop, t) {
                    started += 1;
                }
            }
        }
    }

    /// LVGL `lv_obj_style_create_transition`. Returns whether a transition was started (the
    /// value changes).
    fn create_transition(
        &mut self,
        id: NodeId,
        part: Part,
        prev: State,
        new: State,
        prop: PropId,
        t: &Transition,
    ) -> bool {
        let defaults = self.style_defaults();
        let at = |state, skip_transitions| ResolveOptions {
            state: Some(state),
            skip_transitions,
        };
        // The resolver converts density-independent lengths, so a transition interpolates
        // pixels.
        let v1 = resolve_with(&*self, id, part, prop, &defaults, at(prev, true));
        let mut v2 = resolve_with(&*self, id, part, prop, &defaults, at(new, true));
        if v1 == v2 {
            return false;
        }
        // The value shown right now (a running transition included): no jump.
        let mut v1 = resolve_with(&*self, id, part, prop, &defaults, at(prev, false));
        self.trans_style_set(id, part, prop, v1);
        self.refresh_style(id, part, Some(prop));
        if prop == PropId::Radius {
            let c = self.coords(id);
            let circle = twine_style::PropValue::into_value(twine_style::Radius::Circle);
            let r = StyleValue::Length(twine_style::Length::Px(
                (c.width() / 2 + 1).min(c.height() / 2 + 1),
            ));
            if v1 == circle {
                v1 = r;
            }
            if v2 == circle {
                v2 = r;
            }
        }
        let serial = self.trans.next_serial;
        self.trans.next_serial = serial.wrapping_add(1);
        // A transition plays its forward play once.
        let spec = AnimSpec {
            repeat: Repeat::ONCE,
            playback: None,
            ..t.spec
        };
        let anim = Anim::with_spec(0, 255, spec)
            .early_apply(false)
            .target(AnimTarget::Custom(serial));
        let aid = self.anim_start_target(anim);
        twine_core::debug!(
            target: "twine::style",
            "{} transition {:?} {:?}: {:?} -> {:?} ({})",
            fmt_node_id(id),
            part,
            prop,
            v1,
            v2,
            spec.duration
        );
        self.trans.list.push(Running {
            serial,
            node: id,
            part,
            prop,
            start: v1,
            end: v2,
            anim: aid,
            started: false,
        });
        true
    }

    /// Applies progress `v` (`0..=255`) of transition `serial` (ignored when it was cancelled
    /// meanwhile). The first value after the delay first re-reads the start value and cancels
    /// older transitions of the property (LVGL `trans_anim_start_cb`); once the animation has
    /// left the timeline the transition completes (LVGL `trans_anim_completed_cb`).
    pub(crate) fn transition_value(&mut self, serial: u32, v: i32) {
        let Some(i) = self.trans.list.iter().position(|t| t.serial == serial) else {
            return;
        };
        let t = self.trans.list[i];
        if !self.tree.contains(t.node) {
            return;
        }
        if !t.started {
            let start = self.style_prop(t.node, t.part, t.prop);
            self.trans.list[i].start = start;
            self.trans.list[i].started = true;
            self.remove_transitions(t.node, Some(t.part), Some(t.prop), Some(serial));
            self.trans_style_set(t.node, t.part, t.prop, start);
            self.refresh_style(t.node, t.part, Some(t.prop));
        }
        // Cancelling older transitions shifted the list.
        let Some(i) = self.trans.list.iter().position(|o| o.serial == serial) else {
            return;
        };
        let t = self.trans.list[i];
        let v = interpolate(t.prop, &t.start, &t.end, v.clamp(0, 255) as u8);
        if self.trans_style_set(t.node, t.part, t.prop, v) {
            self.refresh_style(t.node, t.part, Some(t.prop));
        }
        if self.anim.timeline.get(t.anim).is_none() {
            self.transition_done(i);
        }
    }

    /// The transition at `i` completed: it leaves the list, and its property leaves the
    /// transition style unless a newer transition of it runs.
    fn transition_done(&mut self, i: usize) {
        let t = self.trans.list.remove(i);
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
        if let Some(r @ TracksRef::Local(_)) = value.as_grid_tracks() {
            return self.trans_tracks_set(id, part, prop, r);
        }
        let Some(p) = StyleProp::from_value(prop, value) else {
            twine_core::warn!(target: "twine::style", "transition: {:?} cannot hold {:?}", prop, value);
            return false;
        };
        let Some(n) = self.tree.node_mut(id) else {
            return false;
        };
        let changed = n.styles.transition_mut(part, &mut self.trans.pool).set(p);
        if changed {
            n.style_cache.invalidate();
        }
        changed
    }

    /// [`trans_style_set`](Self::trans_style_set) of a run-time grid template `r`: the
    /// transition style takes its own reference to the template from the style of `id` that
    /// holds it, so the template stays alive while the transition shows it.
    #[cold]
    #[inline(never)]
    fn trans_tracks_set(&mut self, id: NodeId, part: Part, prop: PropId, r: TracksRef) -> bool {
        let Some(n) = self.tree.node_mut(id) else {
            return false;
        };
        let src = n.styles.entries().iter().find_map(|e| match &e.style {
            StyleRef::Shared(b) if b.holds_tracks(prop, r) => Some(b.clone()),
            _ => None,
        });
        // The value was resolved from one of the node's styles, so one of them holds it.
        let Some(src) = src else {
            debug_assert!(false, "transition: {prop:?} template not found");
            return false;
        };
        let changed = src.copy_tracks_to(prop, n.styles.transition_mut(part, &mut self.trans.pool));
        if changed {
            n.style_cache.invalidate();
        }
        changed
    }

    /// Removes `prop` from the transition styles of `id` (of `part`, or of every part), and
    /// drops transition entries that became empty.
    fn trans_style_remove(&mut self, id: NodeId, part: Option<Part>, prop: PropId) {
        let Some(n) = self.tree.node_mut(id) else { return };
        n.styles.transition_remove(part, prop, &mut self.trans.pool);
        n.style_cache.invalidate();
    }
}

impl crate::StyleList {
    /// The transition style of `part`, created when missing (from `pool` when it has a
    /// buffer).
    pub(crate) fn transition_mut(&mut self, part: Part, pool: &mut Vec<Rc<StyleBuf>>) -> &mut StyleBuf {
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
            let buf = pool.pop().unwrap_or_default();
            self.insert(StyleEntry::new(Selector::part(part), buf, EntryKind::Transition));
            find(self).unwrap_or(0)
        };
        match &mut self.entries_mut()[pos].style {
            StyleRef::Shared(rc) => Rc::make_mut(rc),
            StyleRef::Static(_) => unreachable!("transition entries are shared buffers"),
        }
    }

    /// Removes `prop` from the transition entries of `part` (`None` = all parts); empty
    /// transition entries are dropped, their buffers kept in `pool` (up to a few).
    pub(crate) fn transition_remove(
        &mut self,
        part: Option<Part>,
        prop: PropId,
        pool: &mut Vec<Rc<StyleBuf>>,
    ) {
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
        let empty = |e: &StyleEntry| {
            e.kind == EntryKind::Transition && matches!(&e.style, StyleRef::Shared(b) if b.is_empty())
        };
        while let Some(i) = self.entries().iter().position(empty) {
            if let StyleRef::Shared(rc) = self.remove_at(i).style {
                if pool.len() < TRANS_POOL_MAX && Rc::strong_count(&rc) == 1 && Rc::weak_count(&rc) == 0 {
                    pool.push(rc);
                }
            }
        }
    }
}
