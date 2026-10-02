//! Style resolution: the value of a property for a node's part, exactly as LVGL's
//! `lv_obj_get_style_prop` computes it, over an abstract [`StyleSource`].

use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::{Angle, Color, Opa, Scale};
use twine_text::Font;

use crate::design::{ElementKind, ElementRef};
use crate::prop::{PROP_COUNT, PropId};
use crate::selector::{Part, Selector, State};
use crate::style::StyleRef;
use crate::value::{PropValue, StyleValue};
use crate::value_types::Length;

/// Where a style entry comes from; the order of an entry list is highest priority first:
/// transitions, local, normal (latest added first), theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EntryKind {
    /// A running style transition (LVGL `is_trans`): consulted first, by part only.
    Transition,
    /// The node's local style for one selector (LVGL `is_local`).
    Local,
    /// A style added to the node (LVGL `lv_obj_add_style`).
    Normal,
    /// A style added by the theme (LVGL `is_theme`).
    Theme,
}

/// One style attached to a node.
#[derive(Clone, Debug)]
pub struct StyleEntry {
    /// Part and states the style applies to.
    pub selector: Selector,
    /// The style.
    pub style: StyleRef,
    /// Origin (priority class).
    pub kind: EntryKind,
}

impl StyleEntry {
    /// An entry.
    #[must_use]
    pub fn new(selector: Selector, style: impl Into<StyleRef>, kind: EntryKind) -> Self {
        Self {
            selector,
            style: style.into(),
            kind,
        }
    }
}

/// A tree of nodes with style entries, as the resolver sees it (the engine's widget tree, or
/// a toy tree in tests).
pub trait StyleSource {
    /// Node handle.
    type Id: Copy;
    /// The node's entries in priority order (highest first): transitions, local, normal
    /// (latest added first), theme. The order is the contract; the resolver does not sort.
    fn entries(&self, id: Self::Id) -> &[StyleEntry];
    /// The node's current state.
    fn state(&self, id: Self::Id) -> State;
    /// The parent node, `None` for a root.
    fn parent(&self, id: Self::Id) -> Option<Self::Id>;
    /// The DPI that density-independent lengths ([`Length::Dp`]) of `id` resolve with (the
    /// engine: the DPI of the node's display). The resolver calls it only when the winning
    /// value is a `Dp` length, so it may be slow (e.g. walk to the root). Default:
    /// [`REFERENCE_DPI`](crate::REFERENCE_DPI), i.e. 1 dp = 1 px.
    fn dpi(&self, id: Self::Id) -> u16 {
        let _ = id;
        crate::REFERENCE_DPI
    }
    /// The value the theme of `id` gives the [design element](crate::design) `element` (the
    /// engine: the element table of the node's display), `None` if it does not define it.
    /// The resolver calls it only when the winning value is an element, so a style without
    /// elements never pays for it. Default: `None` (the element resolves to the property's
    /// default, see [`resolve`]).
    fn design_value(&self, id: Self::Id, element: ElementRef) -> Option<StyleValue> {
        let _ = (id, element);
        None
    }
}

/// Defaults that depend on the environment rather than on the property (LVGL
/// `LV_FONT_DEFAULT`): the engine passes the theme's normal font.
#[derive(Clone, Copy, Debug)]
pub struct StyleDefaults {
    /// Default `Font`.
    pub font: &'static Font,
}

impl Default for StyleDefaults {
    /// `twine_text::EMPTY_FONT`.
    fn default() -> Self {
        Self {
            font: &twine_text::EMPTY_FONT,
        }
    }
}

/// Options of [`resolve_with`] for the queried node (not its ancestors).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolveOptions {
    /// Resolve as if the node were in this state (LVGL sets `obj->state` temporarily to
    /// compare the old and the new state of a transition).
    pub state: Option<State>,
    /// Ignore the node's transition entries (LVGL `obj->skip_trans`).
    pub skip_transitions: bool,
}

/// One entry examined during a traced resolution (see [`resolve_traced`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryTrace {
    /// Number of parent steps from the queried node (0 = the node itself).
    pub depth: u16,
    /// Part looked up at this node (`Main` once inheritance started).
    pub part: Part,
    /// Index in the node's entry list.
    pub entry_index: usize,
    /// Entry kind.
    pub kind: EntryKind,
    /// Entry selector.
    pub selector: Selector,
    /// The selector applies (for transitions: the part matches and transitions are not
    /// skipped).
    pub matched: bool,
    /// The entry's style sets the property.
    pub had_prop: bool,
    /// Selector weight ([`Selector::weight`]: the precedence mask of its states).
    pub weight: u16,
    /// This entry provided the resolved value.
    pub chosen: bool,
}

/// A step of a traced resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceEvent<Id> {
    /// An entry was examined.
    Entry(EntryTrace),
    /// Not found; the inherited property continues at `node`'s `part` (first the node's own
    /// `Main` part when the query was for another part, then each parent's `Main` part).
    Inherited {
        /// Node looked at next.
        node: Id,
        /// Part looked at.
        part: Part,
        /// Parent steps from the queried node.
        depth: u16,
    },
    /// No entry set the property: the default is used.
    Default(StyleValue),
}

/// LVGL `get_prop_core`: the winning entry of one node, as `(index, value)`.
///
/// Transition entries match by part only and win immediately; other entries must match part
/// and state; the highest weight wins, ties go to the earlier entry; an entry whose state
/// equals the node state exactly ends the search.
fn own_value(
    entries: &[StyleEntry],
    state: State,
    part: Part,
    prop: PropId,
    skip_transitions: bool,
) -> Option<(usize, StyleValue)> {
    let group = prop.group_bit();
    let mut best = None;
    let mut weight: i32 = -1;
    for (i, e) in entries.iter().enumerate() {
        if e.style.has_group() & group == 0 {
            continue;
        }
        if e.kind == EntryKind::Transition {
            if skip_transitions || !e.selector.part_matches(part) {
                continue;
            }
            if let Some(v) = e.style.get(prop) {
                return Some((i, v));
            }
            continue;
        }
        if !e.selector.matches(part, state) {
            continue;
        }
        let w = i32::from(e.selector.weight());
        if w <= weight {
            continue; // only better candidates
        }
        if let Some(v) = e.style.get(prop) {
            // LVGL: `state_act == state`. No matching entry can weigh more (the weight of a
            // state set is its bits, see `Selector::weight`).
            if w == i32::from(state.bits()) {
                return Some((i, v));
            }
            best = Some((i, v));
            weight = w;
        }
    }
    best
}

fn default_value(prop: PropId, defaults: StyleDefaults) -> StyleValue {
    if prop == PropId::Font {
        StyleValue::Font(defaults.font)
    } else {
        prop.meta().default
    }
}

/// Resolves `prop` for `part` of node `id`: the LVGL `lv_obj_get_style_prop` algorithm.
///
/// 1. The node's entries, highest priority first. Transition entries apply when their part
///    matches and win at once. Other entries apply when part and state match
///    ([`Selector::matches`]); among them the highest [`Selector::weight`] wins (the
///    documented [state precedence](State#precedence)), and on equal weight the earlier
///    (higher priority) entry. An entry whose state equals the node's state
///    ends the search. Entries whose group mask cannot contain the property are skipped.
/// 2. Not found and the property is inherited: the node's own `Main` part (if `part` was
///    another part), then each ancestor's `Main` part in its own state.
/// 3. Otherwise the property's default ([`PropMeta::default`](crate::PropMeta::default); the
///    `Font` default is `defaults.font`).
///
/// A [design element](crate::design) comes back as the value the theme gives it
/// ([`StyleSource::design_value`] of `id`); an element the theme does not define gives the
/// property's default (step 3, without inheritance) and logs a warning the first time per
/// element kind. A density-independent length ([`Length::Dp`], also as an element's value)
/// comes back in pixels, converted with [`StyleSource::dpi`] of `id`: callers see neither
/// elements nor `Dp`. The common path costs one inlined test of the found value; elements and
/// `Dp` are finished out of line.
///
/// Worked example (same as the test `pressed_state_overrides_default_when_pressed`): a button
/// has a theme style `bg_color: BLUE` and a normal style for `PRESSED` with
/// `bg_color: DARK_BLUE`. In `DEFAULT` state only the theme entry matches → blue. When pressed,
/// both match; the pressed entry outranks the default one (`PRESSED` > no state) → dark blue.
///
/// ```
/// use twine_core::Color;
/// use twine_style::*;
///
/// struct Node { entries: Vec<StyleEntry>, state: State }
/// struct Tree(Vec<Node>);
/// impl StyleSource for Tree {
///     type Id = usize;
///     fn entries(&self, id: usize) -> &[StyleEntry] { &self.0[id].entries }
///     fn state(&self, id: usize) -> State { self.0[id].state }
///     fn parent(&self, _: usize) -> Option<usize> { None }
/// }
///
/// static THEME: Style = style! { bg_color: Color::BLUE };
/// static PRESSED: Style = style! { bg_color: Color::hex(0x000080) };
/// let mut tree = Tree(vec![Node {
///     entries: vec![
///         StyleEntry::new(Selector::state(State::PRESSED), &PRESSED, EntryKind::Normal),
///         StyleEntry::new(Selector::MAIN, &THEME, EntryKind::Theme),
///     ],
///     state: State::DEFAULT,
/// }]);
/// let d = StyleDefaults::default();
/// assert_eq!(resolve(&tree, 0, Part::Main, PropId::BgColor, &d), StyleValue::Color(Color::BLUE));
/// tree.0[0].state = State::PRESSED;
/// assert_eq!(resolve(&tree, 0, Part::Main, PropId::BgColor, &d), StyleValue::Color(Color::hex(0x000080)));
/// ```
#[must_use]
pub fn resolve<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
    defaults: &StyleDefaults,
) -> StyleValue {
    resolve_with(src, id, part, prop, defaults, ResolveOptions::default())
}

/// [`resolve`] with options for the queried node (state override, skipping transitions).
#[must_use]
pub fn resolve_with<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
    defaults: &StyleDefaults,
    opts: ResolveOptions,
) -> StyleValue {
    let state = opts.state.unwrap_or_else(|| src.state(id));
    let v = 'found: {
        if let Some((_, v)) = own_value(src.entries(id), state, part, prop, opts.skip_transitions) {
            break 'found v;
        }
        if prop.meta().inherited {
            if part != Part::Main {
                if let Some((_, v)) =
                    own_value(src.entries(id), state, Part::Main, prop, opts.skip_transitions)
                {
                    break 'found v;
                }
            }
            let mut node = src.parent(id);
            while let Some(n) = node {
                if let Some((_, v)) = own_value(src.entries(n), src.state(n), Part::Main, prop, false) {
                    break 'found v;
                }
                node = src.parent(n);
            }
        }
        // Defaults are never elements or `Dp`.
        return default_value(prop, *defaults);
    };
    finish(src, id, prop, defaults, v)
}

/// The tracks of grid template property `prop` (`GridColumnTracks` / `GridRowTracks`) for
/// `part` of `id`: the template of the style entry that wins the cascade ([`resolve`]'s rules;
/// templates are not inherited), borrowed from that entry's style. A `'static` list or a
/// run-time template held by a [`StyleBuf`](crate::StyleBuf) is read through the style that
/// holds it, so it is always the winning style's own, live list. `None` when no style sets the
/// property.
///
/// ```
/// use twine_style::*;
///
/// struct Node(Vec<StyleEntry>);
/// impl StyleSource for Node {
///     type Id = ();
///     fn entries(&self, _: ()) -> &[StyleEntry] { &self.0 }
///     fn state(&self, _: ()) -> State { State::PRESSED }
///     fn parent(&self, _: ()) -> Option<()> { None }
/// }
/// let main = StyleBuf::new().grid_column_tracks(grid_tracks![fr(1)]);
/// let pressed = StyleBuf::new().grid_column_tracks(grid_tracks![fr(1), fr(1)]);
/// let n = Node(vec![
///     StyleEntry::new(Selector::state(State::PRESSED), pressed, EntryKind::Normal),
///     StyleEntry::new(Selector::MAIN, main, EntryKind::Normal),
/// ]);
/// let t = resolve_grid_tracks(&n, (), Part::Main, PropId::GridColumnTracks, ResolveOptions::default());
/// assert_eq!(t.map(<[_]>::len), Some(2));
/// ```
#[must_use]
pub fn resolve_grid_tracks<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
    opts: ResolveOptions,
) -> Option<&[crate::GridTrack]> {
    let state = opts.state.unwrap_or_else(|| src.state(id));
    let entries = src.entries(id);
    let (i, _) = own_value(entries, state, part, prop, opts.skip_transitions)?;
    entries[i].style.grid_tracks(prop)
}

/// `v` with a design element replaced by the source's value and a [`Length::Dp`] converted
/// with the source's DPI for `id`; any other value unchanged (one inlined test; the rest is out
/// of line).
#[inline(always)]
fn finish<S: StyleSource>(
    src: &S,
    id: S::Id,
    prop: PropId,
    defaults: &StyleDefaults,
    v: StyleValue,
) -> StyleValue {
    if v.needs_source() {
        finish_with_source(src, id, prop, *defaults, v)
    } else {
        v
    }
}

/// Finishes a value read straight from a style ([`Style::get`](crate::Style::get),
/// [`StyleBuf::get`](crate::StyleBuf::get)) the way [`resolve`] finishes the value it found: a
/// [design element](crate::design) becomes the value [`StyleSource::design_value`] of `id`
/// gives it (the property's default if none), a `Dp` length becomes pixels for
/// [`StyleSource::dpi`] of `id`; anything else is returned unchanged. For code that keeps its
/// own small styles outside the cascade (e.g. the spans of a span group).
///
/// ```
/// use twine_core::Color;
/// use twine_style::design::{self, ElementRef};
/// use twine_style::*;
///
/// struct Themed;
/// impl StyleSource for Themed {
///     type Id = ();
///     fn entries(&self, _: ()) -> &[StyleEntry] { &[] }
///     fn state(&self, _: ()) -> State { State::DEFAULT }
///     fn parent(&self, _: ()) -> Option<()> { None }
///     fn design_value(&self, _: (), e: ElementRef) -> Option<StyleValue> {
///         (e == design::ON_SURFACE.erase()).then_some(StyleValue::Color(Color::hex(0x0021_2121)))
///     }
/// }
/// let span = StyleBuf::new().text_color(design::ON_SURFACE);
/// let raw = span.get(PropId::TextColor).unwrap();
/// let d = StyleDefaults::default();
/// assert_eq!(resolve_value(&Themed, (), PropId::TextColor, &d, raw), StyleValue::Color(Color::hex(0x0021_2121)));
/// ```
#[must_use]
pub fn resolve_value<S: StyleSource>(
    src: &S,
    id: S::Id,
    prop: PropId,
    defaults: &StyleDefaults,
    v: StyleValue,
) -> StyleValue {
    finish(src, id, prop, defaults, v)
}

/// The out-of-line part of [`finish`]. Not `#[cold]`: in a themed UI most colors are design
/// elements, so this runs often; it is one call into the source and one table read.
#[inline(never)]
fn finish_with_source<S: StyleSource>(
    src: &S,
    id: S::Id,
    prop: PropId,
    defaults: StyleDefaults,
    v: StyleValue,
) -> StyleValue {
    let v = match v {
        StyleValue::Element(e) => match src.design_value(id, e) {
            Some(x) => x,
            None => return missing_element(prop, e, defaults),
        },
        v => v,
    };
    if v.is_dp() { v.with_dpi(src.dpi(id)) } else { v }
}

static MISSING_ELEMENT: [AtomicBool; ElementKind::COUNT] =
    [const { AtomicBool::new(false) }; ElementKind::COUNT];

/// An element the source does not define: the property's default, with a warning the first
/// time per element kind (never panics: a theme without the element is a configuration
/// issue, not a bug).
#[cold]
#[inline(never)]
fn missing_element(prop: PropId, e: ElementRef, defaults: StyleDefaults) -> StyleValue {
    let flag = &MISSING_ELEMENT[e.kind() as usize];
    // Load + store instead of a swap: thumbv6m has no atomic read-modify-write; a rare
    // duplicate warning is harmless.
    if !flag.load(Ordering::Relaxed) {
        flag.store(true, Ordering::Relaxed);
        twine_core::warn!(
            target: "twine::style",
            "{} of {} is not defined by the theme; using the property's default (further missing {} elements are not reported)",
            e,
            prop,
            e.kind().name()
        );
    }
    default_value(prop, defaults)
}

/// The value set by the node's own entries for `part` in its current state, without
/// inheritance or default (LVGL `get_prop_core`; e.g. "does any style set this property").
/// Design elements and `Dp` lengths are finished like in [`resolve`] (an element the source
/// does not define gives the property's default with an empty default font).
#[must_use]
pub fn resolve_local_only<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
) -> Option<StyleValue> {
    own_value(src.entries(id), src.state(id), part, prop, false)
        .map(|(_, v)| finish(src, id, prop, &StyleDefaults::default(), v))
}

fn trace_node<Id>(
    entries: &[StyleEntry],
    state: State,
    part: Part,
    prop: PropId,
    skip_transitions: bool,
    depth: u16,
    trace: &mut dyn FnMut(TraceEvent<Id>),
) -> Option<StyleValue> {
    let found = own_value(entries, state, part, prop, skip_transitions);
    for (i, e) in entries.iter().enumerate() {
        let matched = if e.kind == EntryKind::Transition {
            !skip_transitions && e.selector.part_matches(part)
        } else {
            e.selector.matches(part, state)
        };
        trace(TraceEvent::Entry(EntryTrace {
            depth,
            part,
            entry_index: i,
            kind: e.kind,
            selector: e.selector,
            matched,
            had_prop: e.style.get(prop).is_some(),
            weight: e.selector.weight(),
            chosen: found.is_some_and(|(w, _)| w == i),
        }));
    }
    found.map(|(_, v)| v)
}

/// [`resolve`] reporting every examined entry, inheritance step and the default to `trace`
/// (for debugging and the `style_resolve` example; also logs at `trace` level to
/// `"twine::style"`). Returns the same value as [`resolve`].
pub fn resolve_traced<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
    defaults: &StyleDefaults,
    trace: &mut dyn FnMut(TraceEvent<S::Id>),
) -> StyleValue {
    let mut report = |ev: TraceEvent<S::Id>| {
        match &ev {
            TraceEvent::Entry(e) => twine_core::trace!(
                target: "twine::style",
                "{} depth {} entry#{} {:?} {:?} w={} matched={} had_prop={} chosen={}",
                prop,
                e.depth,
                e.entry_index,
                e.kind,
                e.selector,
                e.weight,
                e.matched,
                e.had_prop,
                e.chosen
            ),
            TraceEvent::Inherited { depth, part, .. } => {
                twine_core::trace!(target: "twine::style", "{} inherited: depth {} {:?}", prop, depth, part);
            }
            TraceEvent::Default(v) => {
                twine_core::trace!(target: "twine::style", "{} default {:?}", prop, v);
            }
        }
        trace(ev);
    };
    let state = src.state(id);
    if let Some(v) = trace_node(src.entries(id), state, part, prop, false, 0, &mut report) {
        return finish(src, id, prop, defaults, v);
    }
    if prop.meta().inherited {
        if part != Part::Main {
            report(TraceEvent::Inherited {
                node: id,
                part: Part::Main,
                depth: 0,
            });
            if let Some(v) = trace_node(src.entries(id), state, Part::Main, prop, false, 0, &mut report) {
                return finish(src, id, prop, defaults, v);
            }
        }
        let mut node = src.parent(id);
        let mut depth = 0u16;
        while let Some(n) = node {
            depth = depth.saturating_add(1);
            report(TraceEvent::Inherited {
                node: n,
                part: Part::Main,
                depth,
            });
            if let Some(v) = trace_node(
                src.entries(n),
                src.state(n),
                Part::Main,
                prop,
                false,
                depth,
                &mut report,
            ) {
                return finish(src, id, prop, defaults, v);
            }
            node = src.parent(n);
        }
    }
    let d = default_value(prop, *defaults);
    report(TraceEvent::Default(d));
    d
}

static WARNED: [AtomicBool; PROP_COUNT + 1] = [const { AtomicBool::new(false) }; PROP_COUNT + 1];

/// Converts a resolved value to `T`. A variant mismatch is a bug (a style holds a value of the
/// wrong type for the property): debug builds panic, release builds warn once per property and
/// use the default (P7).
fn typed<T: PropValue>(prop: PropId, v: StyleValue, defaults: StyleDefaults) -> Option<T> {
    if let Some(x) = T::from_value(v) {
        return Some(x);
    }
    if v.is_none() {
        return None; // unset reference property
    }
    debug_assert!(
        false,
        "style value {v:?} does not fit property {prop} ({})",
        prop.type_name()
    );
    // Load + store instead of a swap: thumbv6m has no atomic read-modify-write; a rare duplicate
    // warning is harmless.
    if !WARNED[prop as usize].load(Ordering::Relaxed) {
        WARNED[prop as usize].store(true, Ordering::Relaxed);
        twine_core::warn!(target: "twine::style", "value of {} has the wrong type; using the default", prop);
    }
    T::from_value(default_value(prop, defaults))
}

macro_rules! typed_resolvers {
    ($($(#[$m:meta])* $name:ident -> $t:ty = $fallback:expr;)*) => {$(
        $(#[$m])*
        #[must_use]
        pub fn $name<S: StyleSource>(src: &S, id: S::Id, part: Part, prop: PropId, defaults: &StyleDefaults) -> $t {
            typed(prop, resolve(src, id, part, prop, defaults), *defaults).unwrap_or($fallback)
        }
    )*};
}

/// [`resolve`] for integer and pixel-length properties such as paddings (`0` if unset; a
/// density-independent length in pixels for [`StyleSource::dpi`]).
#[must_use]
pub fn resolve_i32<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
    defaults: &StyleDefaults,
) -> i32 {
    let v = resolve(src, id, part, prop, defaults);
    match v.as_px() {
        Some(px) => px,
        None => typed(prop, v, *defaults).unwrap_or(0),
    }
}

typed_resolvers! {
    /// [`resolve`] for length properties.
    resolve_length -> Length = Length::Px(0);
    /// [`resolve`] for color properties.
    resolve_color -> Color = Color::BLACK;
    /// [`resolve`] for opacity properties.
    resolve_opa -> Opa = Opa::COVER;
    /// [`resolve`] for angle properties.
    resolve_angle -> Angle = Angle::deci_deg(0);
    /// [`resolve`] for scale properties.
    resolve_scale -> Scale = Scale::ONE;
    /// [`resolve`] for boolean properties.
    resolve_bool -> bool = false;
}

/// [`resolve`] for `Font` (`defaults.font` when no style sets it).
#[must_use]
pub fn resolve_font<S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    defaults: &StyleDefaults,
) -> &'static Font {
    typed(
        PropId::Font,
        resolve(src, id, part, PropId::Font, defaults),
        *defaults,
    )
    .unwrap_or(defaults.font)
}

/// [`resolve`] converted to the property's payload type `T` (enums, references…); `None` for
/// unset reference properties (LVGL `NULL`).
///
/// ```
/// use twine_style::*;
/// # struct One(Vec<StyleEntry>);
/// # impl StyleSource for One {
/// #     type Id = ();
/// #     fn entries(&self, _: ()) -> &[StyleEntry] { &self.0 }
/// #     fn state(&self, _: ()) -> State { State::DEFAULT }
/// #     fn parent(&self, _: ()) -> Option<()> { None }
/// # }
/// static S: Style = style! { align: Align::Center };
/// let src = One(vec![StyleEntry::new(Selector::MAIN, &S, EntryKind::Normal)]);
/// let d = StyleDefaults::default();
/// assert_eq!(resolve_as::<Align, _>(&src, (), Part::Main, PropId::Align, &d), Some(Align::Center));
/// assert_eq!(resolve_as::<TransitionRef, _>(&src, (), Part::Main, PropId::Transition, &d), None);
/// ```
#[must_use]
pub fn resolve_as<T: PropValue, S: StyleSource>(
    src: &S,
    id: S::Id,
    part: Part,
    prop: PropId,
    defaults: &StyleDefaults,
) -> Option<T> {
    typed(prop, resolve(src, id, part, prop, defaults), *defaults)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use proptest::prelude::*;
    use twine_core::Color;

    use super::*;
    use crate::prop::StyleProp;
    use crate::style::StyleBuf;

    #[derive(Debug)]
    struct Node {
        entries: Vec<StyleEntry>,
        state: State,
        parent: Option<usize>,
    }

    #[derive(Debug, Default)]
    struct Tree(Vec<Node>);

    impl Tree {
        fn add(&mut self, parent: Option<usize>, state: State, entries: Vec<StyleEntry>) -> usize {
            self.0.push(Node {
                entries,
                state,
                parent,
            });
            self.0.len() - 1
        }
    }

    impl StyleSource for Tree {
        type Id = usize;
        fn entries(&self, id: usize) -> &[StyleEntry] {
            &self.0[id].entries
        }
        fn state(&self, id: usize) -> State {
            self.0[id].state
        }
        fn parent(&self, id: usize) -> Option<usize> {
            self.0[id].parent
        }
    }

    fn entry(kind: EntryKind, selector: Selector, props: &[StyleProp]) -> StyleEntry {
        StyleEntry::new(selector, props.iter().copied().collect::<StyleBuf>(), kind)
    }

    const RED: Color = Color::RED;
    const GREEN: Color = Color::GREEN;
    const BLUE: Color = Color::BLUE;
    const D: StyleDefaults = StyleDefaults {
        font: &twine_text::EMPTY_FONT,
    };

    fn bg(t: &Tree, id: usize) -> StyleValue {
        resolve(t, id, Part::Main, PropId::BgColor, &D)
    }

    fn one(state: State, entries: Vec<StyleEntry>) -> (Tree, usize) {
        let mut t = Tree::default();
        let id = t.add(None, state, entries);
        (t, id)
    }

    #[test]
    fn default_state_style_applies() {
        let (t, n) = one(
            State::DEFAULT,
            vec![entry(
                EntryKind::Normal,
                Selector::MAIN,
                &[StyleProp::BgColor(RED.into())],
            )],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(RED));
        // A DEFAULT selector applies in every state.
        let (t, n) = one(
            State::PRESSED | State::CHECKED,
            vec![entry(
                EntryKind::Normal,
                Selector::MAIN,
                &[StyleProp::BgColor(RED.into())],
            )],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(RED));
    }

    #[test]
    fn pressed_state_overrides_default_when_pressed() {
        let entries = vec![
            entry(
                EntryKind::Normal,
                Selector::state(State::PRESSED),
                &[StyleProp::BgColor(Color::hex(0x0000_0080).into())],
            ),
            entry(
                EntryKind::Theme,
                Selector::MAIN,
                &[StyleProp::BgColor(BLUE.into())],
            ),
        ];
        let (mut t, n) = one(State::DEFAULT, entries);
        assert_eq!(bg(&t, n), StyleValue::Color(BLUE));
        t.0[n].state = State::PRESSED;
        assert_eq!(bg(&t, n), StyleValue::Color(Color::hex(0x0000_0080)));
    }

    #[test]
    fn higher_state_weight_wins() {
        let entries = vec![
            entry(
                EntryKind::Normal,
                Selector::state(State::PRESSED),
                &[StyleProp::BgColor(RED.into())],
            ),
            entry(
                EntryKind::Normal,
                Selector::state(State::CHECKED | State::PRESSED),
                &[StyleProp::BgColor(GREEN.into())],
            ),
        ];
        let (mut t, n) = one(State::CHECKED | State::PRESSED, entries);
        assert_eq!(
            bg(&t, n),
            StyleValue::Color(GREEN),
            "later-added but heavier entry wins"
        );
        t.0[n].state = State::PRESSED;
        assert_eq!(
            bg(&t, n),
            StyleValue::Color(RED),
            "CHECKED|PRESSED does not match"
        );
    }

    #[test]
    fn disabled_has_highest_precedence_among_single_states() {
        let singles = [
            State::ALT,
            State::CHECKED,
            State::FOCUSED,
            State::FOCUS_KEY,
            State::EDITED,
            State::HOVERED,
            State::PRESSED,
            State::SCROLLED,
            State::DISABLED,
        ];
        let entries: Vec<_> = singles
            .iter()
            .enumerate()
            .map(|(i, s)| {
                entry(
                    EntryKind::Normal,
                    Selector::state(*s),
                    &[StyleProp::Radius(crate::Radius::Px(i as i32).into())],
                )
            })
            .collect();
        let all = singles.iter().fold(State::DEFAULT, |a, s| a | *s);
        let (t, n) = one(all, entries.clone());
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::Radius, &D),
            StyleValue::Length(Length::Px(8))
        );
        // Order of entries does not matter.
        let rev: Vec<_> = entries.into_iter().rev().collect();
        let (t, n) = one(all, rev);
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::Radius, &D),
            StyleValue::Length(Length::Px(8))
        );
        // DISABLED alone beats PRESSED|CHECKED together.
        let (t, n) = one(
            State::DISABLED | State::PRESSED | State::CHECKED,
            vec![
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED | State::CHECKED),
                    &[StyleProp::BgColor(RED.into())],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::DISABLED),
                    &[StyleProp::BgColor(GREEN.into())],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(GREEN));
    }

    #[test]
    fn tie_goes_to_higher_priority_entry() {
        // Two normal styles with the same selector: the later-added one is earlier in the list.
        let (t, n) = one(
            State::PRESSED | State::FOCUSED,
            vec![
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::BgColor(GREEN.into())],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::BgColor(RED.into())],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(GREEN));
    }

    #[test]
    fn local_beats_normal_same_weight() {
        let (t, n) = one(
            State::DEFAULT,
            vec![
                entry(
                    EntryKind::Local,
                    Selector::MAIN,
                    &[StyleProp::BgColor(GREEN.into())],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[StyleProp::BgColor(RED.into())],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(GREEN));
    }

    #[test]
    fn transition_beats_local_same_weight() {
        let (t, n) = one(
            State::DEFAULT,
            vec![
                entry(
                    EntryKind::Transition,
                    Selector::MAIN,
                    &[StyleProp::BgColor(BLUE.into())],
                ),
                entry(
                    EntryKind::Local,
                    Selector::MAIN,
                    &[StyleProp::BgColor(GREEN.into())],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(BLUE));
    }

    #[test]
    fn transition_wins_regardless_of_weight() {
        // LVGL: transition entries are checked by part only and win immediately.
        let (t, n) = one(
            State::PRESSED,
            vec![
                entry(
                    EntryKind::Transition,
                    Selector::MAIN,
                    &[StyleProp::BgColor(BLUE.into())],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::BgColor(GREEN.into())],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(BLUE));
        // … but only for their part, and not when skipped.
        let knob = resolve(&t, n, Part::Knob, PropId::BgColor, &D);
        assert_eq!(knob, PropId::BgColor.meta().default);
        let skip = ResolveOptions {
            skip_transitions: true,
            ..ResolveOptions::default()
        };
        assert_eq!(
            resolve_with(&t, n, Part::Main, PropId::BgColor, &D, skip),
            StyleValue::Color(GREEN)
        );
        let old = ResolveOptions {
            state: Some(State::DEFAULT),
            skip_transitions: true,
        };
        assert_eq!(
            resolve_with(&t, n, Part::Main, PropId::BgColor, &D, old),
            StyleValue::Color(Color::WHITE)
        );
    }

    #[test]
    fn normal_state_style_beats_local_default() {
        // Weight dominates kind (LVGL behaviour).
        let (t, n) = one(
            State::PRESSED,
            vec![
                entry(
                    EntryKind::Local,
                    Selector::MAIN,
                    &[StyleProp::BgColor(GREEN.into())],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::BgColor(RED.into())],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(RED));
    }

    #[test]
    fn theme_is_lowest_priority() {
        let (t, n) = one(
            State::DEFAULT,
            vec![
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[StyleProp::BgColor(RED.into())],
                ),
                entry(
                    EntryKind::Theme,
                    Selector::MAIN,
                    &[
                        StyleProp::BgColor(BLUE.into()),
                        StyleProp::Radius(crate::Radius::Px(8).into()),
                    ],
                ),
            ],
        );
        assert_eq!(bg(&t, n), StyleValue::Color(RED));
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::Radius, &D),
            StyleValue::Length(Length::Px(8)),
            "theme fills gaps"
        );
    }

    #[test]
    fn part_mismatch_ignored() {
        let (t, n) = one(
            State::DEFAULT,
            vec![entry(
                EntryKind::Normal,
                Selector::part(Part::Knob),
                &[StyleProp::Radius(crate::Radius::Px(5).into())],
            )],
        );
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::Radius, &D),
            StyleValue::Length(Length::Px(0))
        );
        assert_eq!(
            resolve(&t, n, Part::Knob, PropId::Radius, &D),
            StyleValue::Length(Length::Px(5))
        );
    }

    #[test]
    fn any_part_matches() {
        let (t, n) = one(
            State::DEFAULT,
            vec![entry(
                EntryKind::Normal,
                Selector::part(Part::Any),
                &[StyleProp::Radius(crate::Radius::Px(5).into())],
            )],
        );
        for part in [Part::Main, Part::Knob, Part::Items, Part::custom::<0>()] {
            assert_eq!(
                resolve(&t, n, part, PropId::Radius, &D),
                StyleValue::Length(Length::Px(5)),
                "{part:?}"
            );
        }
    }

    #[test]
    fn state_not_subset_ignored() {
        let (t, n) = one(
            State::PRESSED,
            vec![entry(
                EntryKind::Normal,
                Selector::state(State::PRESSED | State::FOCUSED),
                &[StyleProp::Radius(crate::Radius::Px(5).into())],
            )],
        );
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::Radius, &D),
            StyleValue::Length(Length::Px(0))
        );
        assert_eq!(resolve_local_only(&t, n, Part::Main, PropId::Radius), None);
    }

    #[test]
    fn inherited_prop_walks_parents() {
        let mut t = Tree::default();
        let root = t.add(
            None,
            State::DEFAULT,
            vec![entry(
                EntryKind::Theme,
                Selector::MAIN,
                &[StyleProp::TextColor(RED.into())],
            )],
        );
        let mid = t.add(
            Some(root),
            State::DEFAULT,
            vec![entry(
                EntryKind::Normal,
                Selector::state(State::CHECKED),
                &[StyleProp::TextColor(GREEN.into())],
            )],
        );
        let leaf = t.add(Some(mid), State::CHECKED, vec![]);
        // The parent's own state counts (mid is not CHECKED): red from the root.
        assert_eq!(
            resolve(&t, leaf, Part::Main, PropId::TextColor, &D),
            StyleValue::Color(RED)
        );
        t.0[mid].state = State::CHECKED;
        assert_eq!(
            resolve(&t, leaf, Part::Main, PropId::TextColor, &D),
            StyleValue::Color(GREEN)
        );
        // Another part first looks at the node's own Main part.
        t.0[leaf].entries.push(entry(
            EntryKind::Normal,
            Selector::MAIN,
            &[StyleProp::TextColor(BLUE.into())],
        ));
        assert_eq!(
            resolve(&t, leaf, Part::Knob, PropId::TextColor, &D),
            StyleValue::Color(BLUE)
        );
        // Parents are consulted on their Main part only.
        t.0[leaf].entries.clear();
        t.0[mid].entries = vec![entry(
            EntryKind::Normal,
            Selector::part(Part::Knob),
            &[StyleProp::TextColor(BLUE.into())],
        )];
        assert_eq!(
            resolve(&t, leaf, Part::Knob, PropId::TextColor, &D),
            StyleValue::Color(RED)
        );
    }

    #[test]
    fn non_inherited_prop_uses_default() {
        let mut t = Tree::default();
        let root = t.add(
            None,
            State::DEFAULT,
            vec![entry(
                EntryKind::Normal,
                Selector::MAIN,
                &[StyleProp::BgColor(RED.into())],
            )],
        );
        let leaf = t.add(Some(root), State::DEFAULT, vec![]);
        assert_eq!(bg(&t, leaf), StyleValue::Color(Color::WHITE));
        assert_eq!(
            resolve(&t, leaf, Part::Main, PropId::Width, &D),
            StyleValue::Length(Length::Content)
        );
        assert_eq!(
            resolve(&t, leaf, Part::Main, PropId::TextColor, &D),
            StyleValue::Color(Color::BLACK)
        );
    }

    #[test]
    fn text_font_default_from_defaults() {
        static THEME_FONT: Font = crate::test_util::font(1);
        let d = StyleDefaults { font: &THEME_FONT };
        let (t, n) = one(State::DEFAULT, vec![]);
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::Font, &d),
            StyleValue::Font(&THEME_FONT)
        );
        assert!(core::ptr::eq(
            resolve_font(&t, n, Part::Main, &d),
            core::ptr::addr_of!(THEME_FONT)
        ));
        assert!(core::ptr::eq(
            resolve_font(&t, n, Part::Main, &D),
            core::ptr::addr_of!(twine_text::EMPTY_FONT)
        ));
    }

    #[test]
    fn group_mask_skips_irrelevant_styles() {
        // Ten styles without any background property: resolving BgColor never scans them.
        let entries: Vec<_> = (0..10)
            .map(|i| {
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[StyleProp::Width(Length::Px(i).into())],
                )
            })
            .collect();
        let (t, n) = one(State::DEFAULT, entries);
        let before = crate::style::lookups();
        assert_eq!(bg(&t, n), StyleValue::Color(Color::WHITE));
        assert_eq!(
            crate::style::lookups(),
            before,
            "group mask must skip all 10 styles"
        );
        let _ = resolve(&t, n, Part::Main, PropId::Height, &D);
        assert_eq!(
            crate::style::lookups(),
            before + 10,
            "same group: every style scanned"
        );
    }

    #[test]
    fn typed_helpers() {
        let (t, n) = one(
            State::DEFAULT,
            vec![entry(
                EntryKind::Normal,
                Selector::MAIN,
                &[
                    StyleProp::Radius(crate::Radius::Px(4).into()),
                    StyleProp::Width(Length::pct(50).into()),
                    StyleProp::ClipCorner(true),
                ],
            )],
        );
        assert_eq!(resolve_i32(&t, n, Part::Main, PropId::Radius, &D), 4);
        assert_eq!(
            resolve_length(&t, n, Part::Main, PropId::Width, &D),
            Length::Pct(50)
        );
        assert!(resolve_bool(&t, n, Part::Main, PropId::ClipCorner, &D));
        assert_eq!(
            resolve_color(&t, n, Part::Main, PropId::BgColor, &D),
            Color::WHITE
        );
        assert_eq!(resolve_opa(&t, n, Part::Main, PropId::BgOpacity, &D), Opa::TRANSP);
        assert_eq!(
            resolve_scale(&t, n, Part::Main, PropId::TransformScaleX, &D),
            Scale::from_raw_256(256)
        );
        assert_eq!(
            resolve_angle(&t, n, Part::Main, PropId::TransformRotation, &D),
            Angle::deci_deg(0)
        );
        assert_eq!(
            resolve_as::<crate::Align, _>(&t, n, Part::Main, PropId::Align, &D),
            Some(crate::Align::Default)
        );
        assert_eq!(
            resolve_as::<&crate::Gradient, _>(&t, n, Part::Main, PropId::BgGradient, &D),
            None
        );
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn typed_mismatch_panics_in_debug() {
        let (t, n) = one(State::DEFAULT, vec![]);
        let _ = resolve_color(&t, n, Part::Main, PropId::Radius, &D);
    }

    #[test]
    fn traced_reports_decisions() {
        let mut t = Tree::default();
        let root = t.add(
            None,
            State::DEFAULT,
            vec![entry(
                EntryKind::Theme,
                Selector::MAIN,
                &[StyleProp::TextColor(RED.into())],
            )],
        );
        let n = t.add(
            Some(root),
            State::PRESSED,
            vec![
                entry(
                    EntryKind::Local,
                    Selector::MAIN,
                    &[StyleProp::BgOpacity(Opa::COVER.into())],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::BgColor(GREEN.into())],
                ),
                entry(
                    EntryKind::Theme,
                    Selector::MAIN,
                    &[StyleProp::BgColor(BLUE.into())],
                ),
            ],
        );
        let mut events = Vec::new();
        let v = resolve_traced(&t, n, Part::Main, PropId::BgColor, &D, &mut |e| events.push(e));
        assert_eq!(v, bg(&t, n));
        let entries: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let TraceEvent::Entry(e) = e {
                    Some(*e)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].matched && !entries[0].had_prop && !entries[0].chosen);
        assert!(entries[1].matched && entries[1].had_prop && entries[1].chosen && entries[1].weight == 0x80);
        assert!(entries[2].matched && entries[2].had_prop && !entries[2].chosen);

        events.clear();
        let v = resolve_traced(&t, n, Part::Knob, PropId::TextColor, &D, &mut |e| events.push(e));
        assert_eq!(v, StyleValue::Color(RED));
        assert!(events.contains(&TraceEvent::Inherited {
            node: n,
            part: Part::Main,
            depth: 0
        }));
        assert!(events.contains(&TraceEvent::Inherited {
            node: root,
            part: Part::Main,
            depth: 1
        }));

        events.clear();
        let v = resolve_traced(&t, n, Part::Main, PropId::Radius, &D, &mut |e| events.push(e));
        assert_eq!(events.last(), Some(&TraceEvent::Default(v)));
    }

    // ---- Brute-force reference ------------------------------------------------------------

    /// Straightforward restatement of the rules: transitions of the part first (first wins),
    /// then among matching entries with the property the highest weight, earliest on ties.
    fn reference_own(entries: &[StyleEntry], state: State, part: Part, prop: PropId) -> Option<StyleValue> {
        for e in entries {
            if e.kind == EntryKind::Transition && e.selector.part_matches(part) {
                if let Some(v) = e.style.get(prop) {
                    return Some(v);
                }
            }
        }
        // The documented rule, restated from `State::PRECEDENCE` without the bit layout: the
        // highest-ranked differing state decides.
        let ranks = |s: State| -> Vec<usize> {
            if s == State::ANY {
                return Vec::new();
            }
            (0..State::PRECEDENCE.len())
                .rev()
                .filter(|r| s.contains(State::PRECEDENCE[*r]))
                .collect()
        };
        let mut best: Option<(Vec<usize>, StyleValue)> = None;
        for e in entries.iter().filter(|e| e.kind != EntryKind::Transition) {
            let w = ranks(e.selector.state);
            let applies = e.selector.part_matches(part)
                && (e.selector.state == State::ANY || (e.selector.state & !state).is_empty());
            if let (true, Some(v)) = (applies, e.style.get(prop)) {
                if best.as_ref().is_none_or(|(bw, _)| w > *bw) {
                    best = Some((w, v));
                }
            }
        }
        best.map(|(_, v)| v)
    }

    fn reference(t: &Tree, id: usize, part: Part, prop: PropId) -> StyleValue {
        if let Some(v) = reference_own(&t.0[id].entries, t.0[id].state, part, prop) {
            return v;
        }
        if prop.meta().inherited {
            if part != Part::Main {
                if let Some(v) = reference_own(&t.0[id].entries, t.0[id].state, Part::Main, prop) {
                    return v;
                }
            }
            let mut p = t.0[id].parent;
            while let Some(n) = p {
                if let Some(v) = reference_own(&t.0[n].entries, t.0[n].state, Part::Main, prop) {
                    return v;
                }
                p = t.0[n].parent;
            }
        }
        default_value(prop, D)
    }

    const STATES: [State; 7] = [
        State::DEFAULT,
        State::CHECKED,
        State::FOCUSED,
        State::PRESSED,
        State::DISABLED,
        State::custom::<0>(),
        State::custom::<3>(),
    ];
    const PARTS: [Part; 3] = [Part::Main, Part::Knob, Part::Any];
    const KINDS: [EntryKind; 4] = [
        EntryKind::Transition,
        EntryKind::Local,
        EntryKind::Normal,
        EntryKind::Theme,
    ];
    const PROPS: [PropId; 4] = [
        PropId::BgColor,
        PropId::Radius,
        PropId::TextColor,
        PropId::PaddingTop,
    ];

    fn arb_state() -> impl Strategy<Value = State> {
        (0u8..128, any::<bool>()).prop_map(|(bits, any)| {
            if any && bits == 0 {
                return State::ANY;
            }
            STATES
                .iter()
                .enumerate()
                .filter(|(i, _)| bits & (1 << i) != 0)
                .fold(State::DEFAULT, |a, (_, s)| a | *s)
        })
    }

    fn arb_entry() -> impl Strategy<Value = StyleEntry> {
        (
            0usize..4,
            0usize..3,
            arb_state(),
            proptest::collection::vec((0usize..4, 0i32..4), 0..3),
        )
            .prop_map(|(k, p, s, props)| {
                let props: Vec<StyleProp> = props
                    .into_iter()
                    .map(|(which, v)| match PROPS[which] {
                        PropId::BgColor => StyleProp::BgColor(Color::new(v as u8, 0, 0).into()),
                        PropId::Radius => StyleProp::Radius(crate::Radius::Px(v).into()),
                        PropId::TextColor => StyleProp::TextColor(Color::new(0, v as u8, 0).into()),
                        _ => StyleProp::PaddingTop(Length::Px(v).into()),
                    })
                    .collect();
                entry(KINDS[k], Selector::part(PARTS[p]).with_state(s), &props)
            })
    }

    fn arb_tree() -> impl Strategy<Value = Tree> {
        proptest::collection::vec((proptest::collection::vec(arb_entry(), 0..6), arb_state()), 1..4).prop_map(
            |nodes| {
                let mut t = Tree::default();
                for (i, (mut entries, state)) in nodes.into_iter().enumerate() {
                    // The contract: transitions first.
                    entries.sort_by_key(|e| e.kind != EntryKind::Transition);
                    let state = if state == State::ANY {
                        State::DEFAULT
                    } else {
                        state
                    };
                    t.add(i.checked_sub(1), state, entries);
                }
                t
            },
        )
    }

    /// A [`Tree`] whose nodes are on a display of `dpi`.
    struct AtDpi<'a>(&'a Tree, u16);

    impl StyleSource for AtDpi<'_> {
        type Id = usize;
        fn entries(&self, id: usize) -> &[StyleEntry] {
            self.0.entries(id)
        }
        fn state(&self, id: usize) -> State {
            self.0.state(id)
        }
        fn parent(&self, id: usize) -> Option<usize> {
            self.0.parent(id)
        }
        fn dpi(&self, _: usize) -> u16 {
            self.1
        }
    }

    #[test]
    fn dp_resolves_with_the_source_dpi() {
        let (t, n) = one(
            State::DEFAULT,
            vec![
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[
                        StyleProp::PaddingTop(Length::Dp(8).into()),
                        StyleProp::Width(Length::Dp(-5).into()),
                    ],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::PaddingTop(Length::Dp(10).into())],
                ),
            ],
        );
        let pad = StyleValue::Length(Length::Px(16));
        let hi = AtDpi(&t, 320);
        assert_eq!(resolve(&hi, n, Part::Main, PropId::PaddingTop, &D), pad);
        assert_eq!(
            resolve(&hi, n, Part::Main, PropId::Width, &D),
            StyleValue::Length(Length::Px(-10))
        );
        assert_eq!(resolve_i32(&hi, n, Part::Main, PropId::PaddingTop, &D), 16);
        assert_eq!(
            resolve_local_only(&hi, n, Part::Main, PropId::PaddingTop),
            Some(pad)
        );
        let mut sink = |_: TraceEvent<usize>| {};
        assert_eq!(
            resolve_traced(&hi, n, Part::Main, PropId::PaddingTop, &D, &mut sink),
            pad
        );
        let pressed = ResolveOptions {
            state: Some(State::PRESSED),
            skip_transitions: false,
        };
        assert_eq!(
            resolve_with(&hi, n, Part::Main, PropId::PaddingTop, &D, pressed),
            StyleValue::Length(Length::Px(20))
        );
        // Pixels and non-length values pass through; the default source counts 1 dp = 1 px.
        assert_eq!(
            resolve(&hi, n, Part::Main, PropId::PaddingLeft, &D),
            StyleValue::Length(Length::Px(0))
        );
        assert_eq!(
            resolve(&t, n, Part::Main, PropId::PaddingTop, &D),
            StyleValue::Length(Length::Px(8))
        );
    }

    /// A [`Tree`] whose nodes are on a display of `dpi` with a design element table.
    struct Themed<'a>(&'a Tree, u16, &'a crate::design::ElementTable);

    impl StyleSource for Themed<'_> {
        type Id = usize;
        fn entries(&self, id: usize) -> &[StyleEntry] {
            self.0.entries(id)
        }
        fn state(&self, id: usize) -> State {
            self.0.state(id)
        }
        fn parent(&self, id: usize) -> Option<usize> {
            self.0.parent(id)
        }
        fn dpi(&self, _: usize) -> u16 {
            self.1
        }
        fn design_value(&self, _: usize, e: ElementRef) -> Option<StyleValue> {
            self.2.value(e)
        }
    }

    #[test]
    fn design_elements_resolve_with_the_source_table() {
        use crate::design::{self, ColorElement, ElementTable};
        use twine_core::Opa;
        const BRAND: ColorElement = ColorElement::custom(0);
        let (mut t, root) = one(
            State::DEFAULT,
            vec![entry(
                EntryKind::Theme,
                Selector::MAIN,
                &[StyleProp::TextColor(design::ON_SURFACE.into())],
            )],
        );
        let n = t.add(
            Some(root),
            State::DEFAULT,
            vec![
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[
                        StyleProp::BgColor(design::SURFACE.into()),
                        StyleProp::BgOpacity(design::SCRIM_OPACITY.into()),
                        StyleProp::PaddingTop(design::SPACE_M.into()),
                        StyleProp::Radius(design::RADIUS_FULL.into()),
                        StyleProp::BorderColor(BRAND.into()),
                        StyleProp::OutlineColor(ColorElement::custom(9).into()),
                    ],
                ),
                entry(
                    EntryKind::Normal,
                    Selector::state(State::PRESSED),
                    &[StyleProp::BgColor(design::PRIMARY.into())],
                ),
            ],
        );
        let table = ElementTable::new()
            .with(design::SURFACE, Color::WHITE)
            .with(design::PRIMARY, Color::BLUE)
            .with(design::ON_SURFACE, Color::hex(0x0021_2121))
            .with(design::SCRIM_OPACITY, Opa::P50)
            .with(design::SPACE_M, Length::Dp(8))
            .with(design::RADIUS_FULL, crate::Radius::Circle)
            .with(BRAND, Color::hex(0x0062_00EE));
        let src = Themed(&t, 320, &table);
        let r = |p| resolve(&src, n, Part::Main, p, &D);
        assert_eq!(r(PropId::BgColor), StyleValue::Color(Color::WHITE));
        assert_eq!(r(PropId::BgOpacity), StyleValue::Opa(Opa::P50));
        // An element's `Dp` value is converted too (8 dp at 320 DPI).
        assert_eq!(r(PropId::PaddingTop), StyleValue::Length(Length::Px(16)));
        assert_eq!(
            r(PropId::Radius).get::<crate::Radius>(),
            Some(crate::Radius::Circle)
        );
        assert_eq!(r(PropId::BorderColor), StyleValue::Color(Color::hex(0x0062_00EE)));
        // Inherited through an element on the parent.
        assert_eq!(r(PropId::TextColor), StyleValue::Color(Color::hex(0x0021_2121)));
        // Undefined element: the property's default (outline color: black), never a panic.
        assert_eq!(r(PropId::OutlineColor), PropId::OutlineColor.meta().default);
        // A state style's element.
        let pressed = ResolveOptions {
            state: Some(State::PRESSED),
            skip_transitions: false,
        };
        assert_eq!(
            resolve_with(&src, n, Part::Main, PropId::BgColor, &D, pressed),
            StyleValue::Color(Color::BLUE)
        );
        assert_eq!(
            resolve_local_only(&src, n, Part::Main, PropId::BgColor),
            Some(StyleValue::Color(Color::WHITE))
        );
        let mut sink = |_: TraceEvent<usize>| {};
        assert_eq!(
            resolve_traced(&src, n, Part::Main, PropId::TextColor, &D, &mut sink),
            StyleValue::Color(Color::hex(0x0021_2121))
        );
        // The typed resolvers see the value, a raw style value is finished on demand.
        assert_eq!(
            resolve_color(&src, n, Part::Main, PropId::BgColor, &D),
            Color::WHITE
        );
        assert_eq!(resolve_i32(&src, n, Part::Main, PropId::PaddingTop, &D), 16);
        assert_eq!(
            resolve_value(
                &src,
                n,
                PropId::BgColor,
                &D,
                StyleValue::Element(design::PRIMARY.erase())
            ),
            StyleValue::Color(Color::BLUE)
        );
        // A source without a table: every element gives the property's default.
        let plain = AtDpi(&t, 160);
        assert_eq!(
            resolve(&plain, n, Part::Main, PropId::BgColor, &D),
            PropId::BgColor.meta().default
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn resolve_equals_bruteforce_reference(t in arb_tree(), part in 0usize..2) {
            let part = [Part::Main, Part::Knob][part];
            let leaf = t.0.len() - 1;
            for prop in PROPS {
                prop_assert_eq!(resolve(&t, leaf, part, prop, &D), reference(&t, leaf, part, prop), "{:?}", prop);
                let mut sink = |_: TraceEvent<usize>| {};
                prop_assert_eq!(resolve_traced(&t, leaf, part, prop, &D, &mut sink), resolve(&t, leaf, part, prop, &D));
            }
        }
    }
}
