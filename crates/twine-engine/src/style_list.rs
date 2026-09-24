//! Styles on nodes: [`StyleList`], local properties, resolution with inheritance, and the
//! engine's style API (add/remove styles, local properties, states, cached hot properties).

use alloc::rc::Rc;
use alloc::vec::Vec;

use twine_core::{Color, Insets, Opa};
use twine_style::{
    EntryKind, Length, Part, PropId, Selector, State, StyleBuf, StyleDefaults, StyleEntry, StyleProp,
    StyleRef, StyleSource, StyleValue, resolve,
};
use twine_text::Font;

use crate::{Engine, InvalidateReason, LayoutDirty, MainStyle, NodeId, Tree, fmt_node_id};

/// The style entries of a node, kept sorted by priority (highest first): transitions, local
/// styles (one per selector), normal styles (latest added first), theme styles (latest added
/// first). This is the order the resolver expects.
#[derive(Clone, Debug, Default)]
pub struct StyleList(Vec<StyleEntry>);

const fn rank(k: EntryKind) -> u8 {
    match k {
        EntryKind::Transition => 0,
        EntryKind::Local => 1,
        EntryKind::Normal => 2,
        EntryKind::Theme => 3,
    }
}

impl StyleList {
    /// An empty list (does not allocate).
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The entries, highest priority first.
    #[must_use]
    pub fn entries(&self) -> &[StyleEntry] {
        &self.0
    }

    /// The entries, mutably (the order must be kept).
    pub(crate) fn entries_mut(&mut self) -> &mut [StyleEntry] {
        &mut self.0
    }

    /// Inserts `e` before every entry of the same or a lower priority class (so the latest
    /// added style of a class comes first).
    pub(crate) fn insert(&mut self, e: StyleEntry) {
        let r = rank(e.kind);
        let pos = self
            .0
            .iter()
            .position(|x| rank(x.kind) >= r)
            .unwrap_or(self.0.len());
        self.0.insert(pos, e);
    }

    /// The local style of `sel`.
    pub(crate) fn local(&self, sel: Selector) -> Option<&StyleBuf> {
        self.0.iter().find_map(|e| match &e.style {
            StyleRef::Shared(b) if e.kind == EntryKind::Local && e.selector == sel => Some(&**b),
            _ => None,
        })
    }

    /// The local style of `sel`, created when missing.
    pub(crate) fn local_mut(&mut self, sel: Selector) -> &mut StyleBuf {
        let find = |l: &Self| {
            l.0.iter().position(|e| {
                e.kind == EntryKind::Local && e.selector == sel && matches!(e.style, StyleRef::Shared(_))
            })
        };
        let pos = if let Some(p) = find(self) {
            p
        } else {
            self.insert(StyleEntry::new(sel, Rc::new(StyleBuf::new()), EntryKind::Local));
            find(self).unwrap_or(0)
        };
        match &mut self.0[pos].style {
            StyleRef::Shared(rc) => Rc::make_mut(rc),
            StyleRef::Static(_) => unreachable!("local entries are shared buffers"),
        }
    }

    /// Removes and returns the entry at `i`.
    pub(crate) fn remove_at(&mut self, i: usize) -> StyleEntry {
        self.0.remove(i)
    }

    /// Removes entries for which `f` is true; returns how many.
    pub(crate) fn remove_where(&mut self, mut f: impl FnMut(&StyleEntry) -> bool) -> usize {
        let before = self.0.len();
        self.0.retain(|e| !f(e));
        before - self.0.len()
    }
}

impl StyleSource for Tree {
    type Id = NodeId;

    fn entries(&self, id: NodeId) -> &[StyleEntry] {
        self.node(id).map_or(&[], |n| n.styles.entries())
    }

    fn state(&self, id: NodeId) -> State {
        self.node(id).map_or(State::DEFAULT, crate::Node::state)
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        Tree::parent(self, id)
    }
}

/// Calls `f` for every property set by `style`.
fn for_each_prop(style: &StyleRef, mut f: impl FnMut(PropId)) {
    match style {
        StyleRef::Static(s) => s.props().iter().for_each(|p| f(p.id())),
        StyleRef::Shared(b) => b.iter().for_each(|p| f(p.id())),
    }
}

/// How a state change affects a node (LVGL `lv_style_state_cmp_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum StateCmp {
    Same,
    Redraw,
    ExtDraw,
    Layout,
}

/// A resolved length in pixels: `Px` as is, `Pct` of `basis`, `Content` as 0.
#[must_use]
pub(crate) fn length_px(v: StyleValue, basis: i32) -> i32 {
    match v.as_length() {
        Some(Length::Px(p)) => p,
        Some(Length::Pct(p)) => (i64::from(basis) * i64::from(p) / 100) as i32,
        Some(Length::Content) | None => v.as_i32().unwrap_or(0),
    }
}

fn warn_missing(what: &str, id: NodeId) {
    twine_core::warn!(target: "twine::style", "{}: node {} not found", what, fmt_node_id(id));
}

impl Engine {
    /// The defaults used for resolution: `TextFont` is the default display's theme font, else
    /// [`EngineConfig::default_font`](crate::EngineConfig::default_font), else
    /// [`twine_text::EMPTY_FONT`].
    pub(crate) fn style_defaults(&self) -> StyleDefaults {
        StyleDefaults {
            font: self
                .theme_font
                .or(self.config.default_font)
                .unwrap_or(&twine_text::EMPTY_FONT),
        }
    }

    /// Adds `style` for `selector` to `id` (LVGL `lv_obj_add_style`): it has priority over
    /// every style added before and over theme styles.
    pub fn add_style(&mut self, id: NodeId, style: impl Into<StyleRef>, selector: Selector) {
        self.add_entry(id, StyleEntry::new(selector, style.into(), EntryKind::Normal));
    }

    /// Adds a theme style (lowest priority), used by themes.
    pub fn add_theme_style(&mut self, id: NodeId, style: impl Into<StyleRef>, selector: Selector) {
        self.add_entry(id, StyleEntry::new(selector, style.into(), EntryKind::Theme));
    }

    fn add_entry(&mut self, id: NodeId, e: StyleEntry) {
        let part = e.selector.part;
        if !self.tree.contains(id) {
            warn_missing("add_style", id);
            return;
        }
        // LVGL `lv_obj_add_style`: cancels the running transitions of the selector (compared
        // with the transitions' part, so only selectors without states match).
        if e.selector.state == State::DEFAULT {
            let p = (part != Part::Any).then_some(part);
            self.remove_transitions(id, p, None, None);
        }
        let Some(n) = self.tree.node_mut(id) else { return };
        n.styles.insert(e);
        self.refresh_style(id, part, None);
    }

    /// Removes normal and theme styles (LVGL `lv_obj_remove_style`): those equal to `style`
    /// (`None` = any style) whose selector matches `selector` (`None` = any; a selector with
    /// `Part::Any` / `State::ANY` matches every part / state).
    pub fn remove_style(&mut self, id: NodeId, style: Option<&StyleRef>, selector: Option<Selector>) {
        let Some(n) = self.tree.node_mut(id) else {
            warn_missing("remove_style", id);
            return;
        };
        let removed = n.styles.remove_where(|e| {
            matches!(e.kind, EntryKind::Normal | EntryKind::Theme)
                && style.is_none_or(|s| s.ptr_eq(&e.style))
                && selector.is_none_or(|sel| {
                    (sel.part == Part::Any || sel.part == e.selector.part)
                        && (sel.state == State::ANY || sel.state == e.selector.state)
                })
        });
        if removed > 0 {
            self.refresh_style(id, Part::Any, None);
        }
    }

    /// Removes every style of `id`: normal, theme and local styles, and the running style
    /// transitions (LVGL `lv_obj_remove_style_all`).
    pub fn remove_all_styles(&mut self, id: NodeId) {
        if !self.tree.contains(id) {
            warn_missing("remove_all_styles", id);
            return;
        }
        let trans = self.remove_transitions(id, None, None, None);
        let Some(n) = self.tree.node_mut(id) else { return };
        if n.styles.remove_where(|_| true) > 0 || trans {
            self.refresh_style(id, Part::Any, None);
        }
    }

    /// Sets a local style property for `selector` (LVGL `lv_obj_set_style_*`). Idempotent
    /// (P3): setting the value it already has does nothing (no invalidation, no layout).
    /// A running style transition of the property is stopped (LVGL).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_style::{Part, PropId, Selector, StyleProp};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// e.set_local_prop(root, Selector::MAIN, StyleProp::BgColor(Color::RED));
    /// assert_eq!(e.style_color(root, Part::Main, PropId::BgColor), Color::RED);
    /// ```
    pub fn set_local_prop(&mut self, id: NodeId, selector: Selector, prop: StyleProp) {
        if !self.tree.contains(id) {
            warn_missing("set_local_prop", id);
            return;
        }
        let pid = prop.id();
        if self.transition_count() > 0 {
            let part = (selector.part != Part::Any).then_some(selector.part);
            if self.remove_transitions(id, part, Some(pid), None) {
                self.refresh_style(id, selector.part, Some(pid));
            }
        }
        let Some(n) = self.tree.node_mut(id) else { return };
        if n.styles.local(selector).and_then(|s| s.get(pid)) == Some(prop.value()) {
            return;
        }
        n.styles.local_mut(selector).set(prop);
        twine_core::trace!(target: "twine::style", "{} local {:?} = {:?}", fmt_node_id(id), pid, prop.value());
        self.refresh_style(id, selector.part, Some(pid));
    }

    /// Removes a local property; returns whether it was set.
    pub fn remove_local_prop(&mut self, id: NodeId, prop: PropId, selector: Selector) -> bool {
        let Some(n) = self.tree.node_mut(id) else {
            warn_missing("remove_local_prop", id);
            return false;
        };
        if n.styles.local(selector).and_then(|s| s.get(prop)).is_none() {
            return false;
        }
        n.styles.local_mut(selector).remove(prop);
        self.refresh_style(id, selector.part, Some(prop));
        true
    }

    /// The value of `prop` for `part` of `id` with full LVGL resolution (state weights,
    /// priority, inheritance, defaults). Unknown ids give the default.
    #[must_use]
    pub fn style_prop(&self, id: NodeId, part: Part, prop: PropId) -> StyleValue {
        resolve(&self.tree, id, part, prop, &self.style_defaults())
    }

    /// An integer property (lengths resolve `Px`; other kinds give 0).
    #[must_use]
    pub fn style_i32(&self, id: NodeId, part: Part, prop: PropId) -> i32 {
        let v = self.style_prop(id, part, prop);
        v.as_i32()
            .or_else(|| match v.as_length() {
                Some(Length::Px(p)) => Some(p),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// A color property (black if the property is not a color).
    #[must_use]
    pub fn style_color(&self, id: NodeId, part: Part, prop: PropId) -> Color {
        self.style_prop(id, part, prop).as_color().unwrap_or(Color::BLACK)
    }

    /// An opacity property (cover if the property is not an opacity).
    #[must_use]
    pub fn style_opa(&self, id: NodeId, part: Part, prop: PropId) -> Opa {
        self.style_prop(id, part, prop).as_opa().unwrap_or(Opa::COVER)
    }

    /// The font of `part` (`TextFont`, inherited, default from the configuration).
    #[must_use]
    pub fn style_font(&self, id: NodeId, part: Part) -> &'static Font {
        self.style_prop(id, part, PropId::TextFont)
            .get::<&'static Font>()
            .unwrap_or(self.style_defaults().font)
    }

    /// The hot `Main` properties of `id` in its current state, from the node's cache (filled
    /// on first use after a style or state change). Unknown ids give the defaults.
    #[must_use]
    pub fn cached_main(&self, id: NodeId) -> MainStyle {
        let Some(n) = self.tree.node(id) else {
            return self.resolve_main(id);
        };
        if let Some(v) = n.style_cache.get(self.tree.epoch) {
            #[cfg(feature = "debug-checks")]
            {
                let full = self.resolve_main(id);
                if full != v {
                    twine_core::error!(target: "twine::style", "stale style cache on {}", fmt_node_id(id));
                    debug_assert!(false, "stale style cache on {id}: {v:?} != {full:?}");
                }
            }
            return v;
        }
        let v = self.resolve_main(id);
        n.style_cache.set(v, self.tree.epoch);
        v
    }

    /// Whether `id`'s style cache holds values for the current epoch (for tests).
    #[must_use]
    pub fn style_cache_valid(&self, id: NodeId) -> bool {
        self.tree
            .node(id)
            .is_some_and(|n| n.style_cache.is_valid(self.tree.epoch))
    }

    fn resolve_main(&self, id: NodeId) -> MainStyle {
        let m = Part::Main;
        MainStyle {
            bg_color: self.style_color(id, m, PropId::BgColor),
            bg_opa: self.style_opa(id, m, PropId::BgOpa),
            radius: self.style_i32(id, m, PropId::Radius),
            border_width: self.style_i32(id, m, PropId::BorderWidth),
            border_color: self.style_color(id, m, PropId::BorderColor),
            opa: self.style_opa(id, m, PropId::Opa),
            pad: Insets::new(
                self.style_i32(id, m, PropId::PadLeft),
                self.style_i32(id, m, PropId::PadTop),
                self.style_i32(id, m, PropId::PadRight),
                self.style_i32(id, m, PropId::PadBottom),
            ),
            text_color: self.style_color(id, m, PropId::TextColor),
            font: self.style_font(id, m),
            recolor: self.style_color(id, m, PropId::Recolor),
            recolor_opa: self.style_opa(id, m, PropId::RecolorOpa),
        }
    }

    /// Adds states to `id` (idempotent).
    pub fn add_state(&mut self, id: NodeId, s: State) {
        self.set_state(id, s, true);
    }

    /// Removes states from `id` (idempotent).
    pub fn clear_state(&mut self, id: NodeId, s: State) {
        self.set_state(id, s, false);
    }

    /// Adds (`on`) or removes states. Idempotent; when the change selects the same style
    /// entries as before nothing is invalidated (LVGL `lv_obj_style_state_compare`).
    /// Otherwise the style transitions of the new state start (see `TransitionDsc`; only on
    /// nodes that were drawn already). Every actual change sends
    /// [`StateChanged`](crate::EventCode::StateChanged) with the previous state (posted: a
    /// busy widget gets it once it is back, see [`post_event`](Self::post_event)).
    pub fn set_state(&mut self, id: NodeId, s: State, on: bool) {
        let Some(n) = self.tree.node(id) else {
            warn_missing("set_state", id);
            return;
        };
        let old = n.state;
        let new = if on { old | s } else { old & !s };
        if old == new {
            return;
        }
        let items = n.class().item_parts;
        let (cmp, inherited) = state_compare(n.styles.entries(), old, new, items);
        twine_core::trace!(target: "twine::style", "{} state {:?} -> {:?}: {:?}", fmt_node_id(id), old, new, cmp);
        if let Some(n) = self.tree.node_mut(id) {
            n.state = new;
        }
        if cmp != StateCmp::Same {
            self.start_transitions(id, old, new);
        }
        match cmp {
            StateCmp::Same => {}
            StateCmp::Redraw => {
                self.clear_style_cache(id);
                self.invalidate(id, InvalidateReason::StyleChange);
            }
            StateCmp::ExtDraw => {
                self.invalidate(id, InvalidateReason::StyleChange);
                self.clear_style_cache(id);
                self.refresh_ext_draw(id);
                self.invalidate(id, InvalidateReason::StyleChange);
            }
            StateCmp::Layout => self.refresh_style(id, Part::Any, None),
        }
        if inherited && cmp != StateCmp::Layout {
            self.bump_epoch_and_invalidate_descendants(id);
        }
        self.post_event(id, crate::EventCode::StateChanged, crate::EventParam::State(old));
    }

    /// Refreshes every node that uses the shared style `style` after it was mutated (scans the
    /// whole tree; rare).
    pub fn report_style_change(&mut self, style: &Rc<StyleBuf>) {
        let target = StyleRef::Shared(style.clone());
        let users: Vec<NodeId> = self
            .tree
            .all_nodes()
            .filter(|(_, n)| n.styles.entries().iter().any(|e| e.style.ptr_eq(&target)))
            .map(|(id, _)| id)
            .collect();
        for id in users {
            self.refresh_style(id, Part::Any, None);
        }
    }

    fn clear_style_cache(&self, id: NodeId) {
        if let Some(n) = self.tree.node(id) {
            n.style_cache.invalidate();
        }
    }

    fn bump_epoch_and_invalidate_descendants(&mut self, id: NodeId) {
        self.tree.epoch = self.tree.epoch.wrapping_add(1);
        let mut next = self.tree.node(id).and_then(crate::Node::first_child);
        // Invalidate every descendant (they may draw outside `id` with OVERFLOW_VISIBLE).
        while let Some(c) = next {
            self.invalidate_subtree(c, InvalidateReason::StyleChange);
            next = self.tree.node(c).and_then(crate::Node::next_sibling);
        }
    }

    /// LVGL `lv_obj_refresh_style`: after a style of `id` changed (`prop = None`: any
    /// property). Invalidates before and after, drops the cache, marks layout, recomputes the
    /// extra draw size and propagates inherited changes to the descendants as needed.
    pub(crate) fn refresh_style(&mut self, id: NodeId, part: Part, prop: Option<PropId>) {
        if !self.tree.contains(id) {
            return;
        }
        let _ = part;
        self.invalidate(id, InvalidateReason::StyleChange);
        self.clear_style_cache(id);
        let meta = prop.map(PropId::meta);
        if meta.is_none_or(|m| m.layout || m.flags.contains(twine_style::PropFlags::PARENT_LAYOUT)) {
            self.mark_layout(id, LayoutDirty::SELF);
        }
        // The pivot moves the transformed bounds, so it matters for the extra draw size too.
        if meta.is_none_or(|m| m.ext_draw)
            || matches!(prop, Some(PropId::TransformPivotX | PropId::TransformPivotY))
        {
            self.refresh_ext_draw(id);
        }
        if meta.is_none_or(|m| m.inherited) {
            self.bump_epoch_and_invalidate_descendants(id);
        }
        self.invalidate(id, InvalidateReason::StyleChange);
        self.send_event(id, crate::EventCode::StyleChanged, crate::EventParam::None);
    }
}

/// Compares which entries apply in `old` and `new` state, ignoring entries of the class's
/// `item_parts` (the widget redraws those items itself). Returns the strongest effect and
/// whether an inherited property is involved.
fn state_compare(entries: &[StyleEntry], old: State, new: State, item_parts: &[Part]) -> (StateCmp, bool) {
    let applies = |sel: &Selector, st: State| sel.state == State::ANY || sel.state.bits() & !st.bits() == 0;
    let mut res = StateCmp::Same;
    let mut inherited = false;
    for e in entries {
        if e.kind == EntryKind::Transition
            || item_parts.contains(&e.selector.part)
            || applies(&e.selector, old) == applies(&e.selector, new)
        {
            continue;
        }
        for_each_prop(&e.style, |p| {
            let m = p.meta();
            let c = if m.layout {
                StateCmp::Layout
            } else if m.ext_draw {
                StateCmp::ExtDraw
            } else {
                StateCmp::Redraw
            };
            res = res.max(c);
            inherited |= m.inherited;
        });
    }
    (res, inherited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use twine_style::Style;

    #[test]
    fn insert_keeps_priority_order() {
        static A: Style = Style::new(&[]);
        static B: Style = Style::new(&[]);
        let mut l = StyleList::new();
        l.insert(StyleEntry::new(Selector::MAIN, &A, EntryKind::Theme));
        l.insert(StyleEntry::new(Selector::MAIN, &A, EntryKind::Normal));
        l.insert(StyleEntry::new(Selector::MAIN, &B, EntryKind::Normal));
        l.insert(StyleEntry::new(Selector::MAIN, &B, EntryKind::Theme));
        l.local_mut(Selector::MAIN).set(StyleProp::Radius(3));
        l.insert(StyleEntry::new(Selector::MAIN, &A, EntryKind::Transition));
        let kinds: Vec<EntryKind> = l.entries().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                EntryKind::Transition,
                EntryKind::Local,
                EntryKind::Normal,
                EntryKind::Normal,
                EntryKind::Theme,
                EntryKind::Theme
            ]
        );
        // Latest added normal/theme first.
        assert!(l.entries()[2].style.ptr_eq(&StyleRef::Static(&B)));
        assert!(l.entries()[4].style.ptr_eq(&StyleRef::Static(&B)));
        assert_eq!(
            l.local(Selector::MAIN).unwrap().get(PropId::Radius),
            Some(StyleValue::Int(3))
        );
        assert!(l.local(Selector::state(State::PRESSED)).is_none());
    }

    #[test]
    fn transition_entry_beats_all() {
        use crate::{Obj, Tree};
        use alloc::boxed::Box;
        static RED: Style = Style::new(&[StyleProp::BgColor(Color::RED)]);
        static BLUE: Style = Style::new(&[StyleProp::BgColor(Color::BLUE)]);
        let mut t = Tree::new();
        let n = t.create(None, Box::new(Obj)).unwrap();
        let list = &mut t.node_mut(n).unwrap().styles;
        list.local_mut(Selector::state(State::PRESSED))
            .set(StyleProp::BgColor(Color::GREEN));
        list.insert(StyleEntry::new(Selector::MAIN, &BLUE, EntryKind::Normal));
        list.insert(StyleEntry::new(Selector::MAIN, &RED, EntryKind::Transition));
        t.node_mut(n).unwrap().state = State::PRESSED;
        let v = twine_style::resolve(&t, n, Part::Main, PropId::BgColor, &StyleDefaults::default());
        assert_eq!(v, StyleValue::Color(Color::RED));
    }

    #[test]
    fn state_compare_classifies() {
        static PRESSED_COLOR: Style = Style::new(&[StyleProp::BgColor(Color::RED)]);
        static PRESSED_PAD: Style = Style::new(&[StyleProp::PadTop(3)]);
        static PRESSED_SHADOW: Style = Style::new(&[StyleProp::ShadowWidth(3)]);
        let e = |s: &'static Style| StyleEntry::new(Selector::state(State::PRESSED), s, EntryKind::Normal);
        let d = State::DEFAULT;
        let p = State::PRESSED;
        assert_eq!(
            state_compare(&[e(&PRESSED_COLOR)], d, p, &[]),
            (StateCmp::Redraw, false)
        );
        assert_eq!(state_compare(&[e(&PRESSED_PAD)], d, p, &[]).0, StateCmp::Layout);
        assert_eq!(
            state_compare(&[e(&PRESSED_SHADOW)], d, p, &[]).0,
            StateCmp::ExtDraw
        );
        assert_eq!(
            state_compare(&[e(&PRESSED_COLOR)], d, State::FOCUSED, &[]).0,
            StateCmp::Same
        );
        // Entries of item parts are the widget's business.
        let items = StyleEntry::new(
            Selector::part(Part::Items).with_state(State::PRESSED),
            &PRESSED_SHADOW,
            EntryKind::Theme,
        );
        assert_eq!(
            state_compare(core::slice::from_ref(&items), d, p, &[]).0,
            StateCmp::ExtDraw
        );
        assert_eq!(state_compare(&[items], d, p, &[Part::Items]).0, StateCmp::Same);
        assert_eq!(length_px(StyleValue::Length(Length::Pct(50)), 30), 15);
        assert_eq!(length_px(StyleValue::Length(Length::Px(7)), 30), 7);
    }
}
