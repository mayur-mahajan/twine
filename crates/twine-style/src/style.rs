//! Style containers: flash-resident [`Style`], heap [`StyleBuf`] and the shared [`StyleRef`].

use alloc::rc::Rc;
use alloc::vec::Vec;

use crate::prop::{PropId, StyleProp};
use crate::tracks::{GridTracks, OwnedSlots, TracksRef};
use crate::transition::{Transition, TransitionId, TransitionRef, TransitionValue};
use crate::value::StyleValue;
use crate::value_types::GridTrack;

#[cfg(test)]
std::thread_local! {
    /// Number of property scans (`get` calls that passed the group check), for tests.
    static LOOKUPS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// Number of property scans done by this thread so far (tests only).
#[cfg(test)]
pub(crate) fn lookups() -> u32 {
    LOOKUPS.with(core::cell::Cell::get)
}

#[inline]
fn count_lookup() {
    #[cfg(test)]
    LOOKUPS.with(|c| c.set(c.get() + 1));
}

/// Scans `props` from the end (the last duplicate wins).
#[inline]
fn find(props: &[StyleProp], has_group: u16, id: PropId) -> Option<StyleValue> {
    if has_group & id.group_bit() == 0 {
        return None;
    }
    count_lookup();
    props.iter().rev().find(|p| p.id() == id).map(StyleProp::value)
}

/// The template of grid template property `id` in `props` (the last occurrence wins).
fn find_ref(props: &[StyleProp], has_group: u16, id: PropId) -> Option<TracksRef> {
    if has_group & id.group_bit() == 0 {
        return None;
    }
    props.iter().rev().find(|p| p.id() == id)?.tracks_ref()
}

/// The `transition` property in `props` (the last occurrence wins).
#[inline]
fn find_transition(props: &[StyleProp], has_group: u16) -> Option<TransitionRef> {
    if has_group & PropId::Transition.group_bit() == 0 {
        return None;
    }
    props.iter().rev().find_map(|p| match *p {
        StyleProp::Transition(t) => Some(t),
        _ => None,
    })
}

const fn mask_of(props: &[StyleProp]) -> u16 {
    let mut mask = 0u16;
    let mut i = 0;
    while i < props.len() {
        mask |= props[i].id().group_bit();
        i += 1;
    }
    mask
}

/// Placeholder entry of [`merge_props`]'s output array (every slot is overwritten).
const FILLER: StyleProp = StyleProp::Opacity(crate::design::DesignValue::Fixed(twine_core::Opa::COVER));

/// Number of distinct properties in `parts` (the length of [`merge_props`]'s result). Used by
/// the `style!` spread at compile time; not a public API.
#[doc(hidden)]
#[must_use]
pub const fn merged_len(parts: &[&[StyleProp]]) -> usize {
    let mut seen = [false; crate::PROP_COUNT + 1];
    let mut n = 0;
    let mut i = 0;
    while i < parts.len() {
        let mut j = 0;
        while j < parts[i].len() {
            let id = parts[i][j].id() as usize;
            if !seen[id] {
                seen[id] = true;
                n += 1;
            }
            j += 1;
        }
        i += 1;
    }
    n
}

/// The `style!` spread, at compile time: the properties of `parts` in order, each property
/// once, at the position of its first occurrence with the value of its last (the semantics of
/// [`StyleBuf::set`]). `N` must be [`merged_len`]`(parts)` (a compile error otherwise). Not a
/// public API.
#[doc(hidden)]
#[must_use]
pub const fn merge_props<const N: usize>(parts: &[&[StyleProp]]) -> [StyleProp; N] {
    let mut out = [FILLER; N];
    // Index into `out` of each property id (`usize::MAX`: not yet seen).
    let mut slot = [usize::MAX; crate::PROP_COUNT + 1];
    let mut n = 0;
    let mut i = 0;
    while i < parts.len() {
        let mut j = 0;
        while j < parts[i].len() {
            let p = parts[i][j];
            let id = p.id() as usize;
            if slot[id] == usize::MAX {
                assert!(n < N, "merge_props: N is smaller than merged_len");
                slot[id] = n;
                out[n] = p;
                n += 1;
            } else {
                out[slot[id]] = p;
            }
            j += 1;
        }
        i += 1;
    }
    assert!(n == N, "merge_props: N is larger than merged_len");
    out
}

/// An immutable, `const`-constructible style that lives in flash (LVGL `LV_STYLE_CONST_INIT`).
///
/// Usually written with the [`style!`](crate::style!) macro. Lookups scan the properties from
/// the end, so a later duplicate wins; the precomputed group mask ([`Style::has_group`]) lets
/// resolution skip styles that cannot contain a property.
///
/// ```
/// use twine_core::Color;
/// use twine_style::design::{self, DesignValue};
/// use twine_style::{Length, PropId, Radius, StyleProp, Style, StyleValue};
///
/// // Written out (`style!` does the same): fixed values and design elements.
/// static S: Style = Style::new(&[
///     StyleProp::BgColor(DesignValue::Fixed(Color::RED)),
///     StyleProp::Radius(DesignValue::Fixed(Radius::Px(4))),
///     StyleProp::BorderColor(DesignValue::Element(design::OUTLINE)),
/// ]);
/// assert_eq!(S.get(PropId::Radius), Some(StyleValue::Length(Length::Px(4))));
/// assert_eq!(S.get(PropId::BorderColor), Some(StyleValue::Element(design::OUTLINE.erase())));
/// assert_eq!(S.get(PropId::Width), None);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Style {
    props: &'static [StyleProp],
    has_group: u16,
}

impl Style {
    /// A style of `props` (the group mask is computed at compile time in `const` context).
    #[must_use]
    pub const fn new(props: &'static [StyleProp]) -> Self {
        Self {
            props,
            has_group: mask_of(props),
        }
    }

    /// The value of `id`, if set (the last occurrence wins).
    #[inline]
    #[must_use]
    pub fn get(&self, id: PropId) -> Option<StyleValue> {
        find(self.props, self.has_group, id)
    }

    /// The tracks of the grid template property `id` (`GridColumnTracks` / `GridRowTracks`),
    /// if set.
    #[must_use]
    pub fn grid_tracks(&self, id: PropId) -> Option<&'static [GridTrack]> {
        find_ref(self.props, self.has_group, id)?.as_static()
    }

    /// The set of properties this style sets (O(properties); used by transitions that animate
    /// what a state change alters).
    #[must_use]
    pub fn keys(&self) -> crate::Props {
        self.props.iter().map(StyleProp::id).collect()
    }

    /// The `transition` property, if set (a static style holds `'static` transitions only).
    #[must_use]
    pub fn get_transition(&self) -> Option<&'static Transition> {
        match find_transition(self.props, self.has_group)? {
            TransitionRef::Static(t) => Some(t),
            TransitionRef::Local(_) => None,
        }
    }

    /// The properties, in declaration order (`const`, so a `style!` spread can read them at
    /// compile time).
    #[must_use]
    pub const fn props(&self) -> &'static [StyleProp] {
        self.props
    }

    /// Bit `g` is set iff a property of group `g` (`PropId::group`) is present.
    #[must_use]
    pub const fn has_group(&self) -> u16 {
        self.has_group
    }

    /// Whether the style sets no property.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.props.is_empty()
    }

    /// Composes at run time: a heap copy of this style with every property of `over` set on
    /// top (later wins, like [`StyleBuf::set`]). `over` may be a [`Style`], a [`StyleBuf`] or
    /// a [`StyleRef`]; a run-time grid template `over` holds is shared by the result
    /// ([`StyleBuf::extend_from`]).
    ///
    /// For two `static` styles prefer the compile-time spread of [`style!`](crate::style!)
    /// (`style! { ..BASE, ..OVER }`): it stays in flash and allocates nothing. `merge` is for
    /// values only known at run time, e.g. a theme style plus an application's override.
    /// Allocates the result once; never panics.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::{PropId, Style, StyleBuf, StyleValue, Length, style};
    ///
    /// static BASE: Style = style! { bg_color: Color::WHITE, radius: 8, padding: 4 };
    /// let accent = StyleBuf::new().bg_color(Color::RED).width(100);
    /// let s = BASE.merge(&accent);
    /// assert_eq!(s.get(PropId::BgColor), Some(StyleValue::Color(Color::RED))); // `over` wins
    /// assert_eq!(s.get(PropId::PaddingTop), Some(StyleValue::Length(Length::Px(4)))); // kept
    /// assert_eq!(s.get(PropId::Width), Some(StyleValue::Length(Length::Px(100)))); // added
    /// ```
    #[must_use]
    pub fn merge(&self, over: &impl StyleContainer) -> StyleBuf {
        let mut b = StyleBuf::from(self);
        b.extend_from(over);
        b
    }
}

mod sealed {
    /// Only the style containers of this crate.
    pub trait Sealed {}
    impl Sealed for super::Style {}
    impl Sealed for super::StyleBuf {}
    impl Sealed for super::StyleRef {}
}

/// A style container whose properties can be composed into a [`StyleBuf`]: a flash
/// [`Style`], a heap [`StyleBuf`] or a [`StyleRef`] (either). Taken by
/// [`StyleBuf::extend_from`] and [`Style::merge`]; sealed (implemented by these three only).
pub trait StyleContainer: sealed::Sealed {
    /// Sets every property of `self` on `dst` (in order; a later value replaces an earlier
    /// one), sharing run-time grid templates. Returns whether `dst` changed.
    #[doc(hidden)]
    fn __extend_into(&self, dst: &mut StyleBuf) -> bool;
}

impl StyleContainer for Style {
    fn __extend_into(&self, dst: &mut StyleBuf) -> bool {
        // A static style holds `'static` templates only: every property is a plain `set`.
        let mut changed = false;
        for p in self.props {
            changed |= dst.set(*p);
        }
        changed
    }
}

impl StyleContainer for StyleBuf {
    fn __extend_into(&self, dst: &mut StyleBuf) -> bool {
        let mut changed = false;
        for p in &self.props {
            changed |= match *p {
                // A run-time value belongs to its container: `dst` takes its own reference
                // (released with `dst`'s property), never a bare `Local` value.
                StyleProp::GridColumnTracks(TracksRef::Local(_))
                | StyleProp::GridRowTracks(TracksRef::Local(_)) => self.copy_tracks_to(p.id(), dst),
                StyleProp::Transition(TransitionRef::Local(_)) => match self.held.transition() {
                    Some(t) => dst.set_transition(TransitionValue::Shared(t.clone())),
                    None => false,
                },
                _ => dst.set(*p),
            };
        }
        changed
    }
}

impl StyleContainer for StyleRef {
    fn __extend_into(&self, dst: &mut StyleBuf) -> bool {
        match self {
            StyleRef::Static(s) => s.__extend_into(dst),
            StyleRef::Shared(s) => s.__extend_into(dst),
        }
    }
}

/// A mutable style on the heap (LVGL `lv_style_t` with `lv_style_set_*`).
///
/// Holds at most one entry per property: [`StyleBuf::set`] replaces in place. Builder methods
/// named after every property (`bg_color`, `radius`, …) make construction concise.
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::{PropId, StyleBuf, StyleProp, StyleValue};
///
/// let mut s = StyleBuf::new().bg_color(Color::BLUE).bg_opacity(Opa::COVER).width(120);
/// assert_eq!(s.len(), 3);
/// assert!(s.set(StyleProp::BgColor(Color::RED.into()))); // changed
/// assert!(!s.set(StyleProp::BgColor(Color::RED.into()))); // same value
/// assert_eq!(s.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
/// assert!(s.remove(PropId::BgOpacity));
/// assert_eq!(s.len(), 2);
/// ```
///
/// A run-time grid template (a `Vec`, see [`set_tracks`](Self::set_tracks)) or transition
/// (see [`set_transition`](Self::set_transition)) is owned by the buffer: it lives as long as
/// the property that refers to it, and the buffer's clones share it.
#[derive(Clone, Debug, Default)]
pub struct StyleBuf {
    props: Vec<StyleProp>,
    has_group: u16,
    held: OwnedSlots,
}

impl StyleBuf {
    /// An empty style (does not allocate).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            props: Vec::new(),
            has_group: 0,
            held: OwnedSlots::new(),
        }
    }

    /// Sets a property, replacing an existing value of the same property in place. Returns
    /// `true` if the style changed (new property or different value).
    ///
    /// A grid template property with a `'static` list is stored as is (releasing a run-time
    /// template the buffer held for it). One that refers to a run-time template
    /// ([`TracksRef::Local`]) is accepted only if this buffer holds that very template
    /// (otherwise it is ignored with a warning: set run-time templates with
    /// [`set_tracks`](Self::set_tracks)). The same holds for the `transition` property
    /// ([`TransitionRef`], [`set_transition`](Self::set_transition)).
    pub fn set(&mut self, p: StyleProp) -> bool {
        let id = p.id();
        if let Some(r) = p.tracks_ref() {
            match r {
                TracksRef::Static(_) => self.held.clear(id),
                TracksRef::Local(tid) => {
                    if self.held.get(id).is_none_or(|t| t.id() != tid) {
                        foreign_template(id);
                        return false;
                    }
                }
            }
        } else if let StyleProp::Transition(r) = p {
            match r {
                TransitionRef::Static(_) => self.held.clear(id),
                TransitionRef::Local(tid) => {
                    if self.held.transition().is_none_or(|t| TransitionId::of(t) != tid) {
                        foreign_template(id);
                        return false;
                    }
                }
            }
        }
        self.put(p)
    }

    /// Stores `p` (replacing the property's previous value).
    fn put(&mut self, p: StyleProp) -> bool {
        let id = p.id();
        if let Some(slot) = self.props.iter_mut().find(|q| q.id() == id) {
            if slot.value() == p.value() {
                return false;
            }
            *slot = p;
            return true;
        }
        self.props.push(p);
        self.has_group |= id.group_bit();
        true
    }

    /// Sets the grid template property `id` (`GridColumnTracks` / `GridRowTracks`): a `'static`
    /// list is stored as is, a run-time one is held by the buffer (and released when the
    /// property is replaced or removed, or the buffer is dropped). Returns `true` if the style
    /// changed: equal tracks change nothing. Another property is ignored with a warning. The
    /// builder methods `grid_column_tracks` / `grid_row_tracks` call it.
    ///
    /// ```
    /// use twine_style::{GridTrack, PropId, SharedTracks, StyleBuf};
    ///
    /// let cols = SharedTracks::from(vec![GridTrack::Fr(1), GridTrack::Fr(2)]);
    /// let mut s = StyleBuf::new();
    /// assert!(s.set_tracks(PropId::GridColumnTracks, cols.clone().into()));
    /// assert!(!s.set_tracks(PropId::GridColumnTracks, cols.to_vec().into())); // equal tracks
    /// assert_eq!(cols.strong_count(), 2); // the buffer holds it
    /// s.remove(PropId::GridColumnTracks);
    /// assert_eq!(cols.strong_count(), 1); // released with the property
    /// ```
    pub fn set_tracks(&mut self, id: PropId, tracks: GridTracks) -> bool {
        let make = match id {
            PropId::GridColumnTracks => StyleProp::GridColumnTracks,
            PropId::GridRowTracks => StyleProp::GridRowTracks,
            _ => {
                twine_core::warn!(target: "twine::style", "set_tracks: {:?} is not a grid template", id);
                return false;
            }
        };
        if self.grid_tracks(id).is_some_and(|t| t == tracks.tracks()) {
            return false;
        }
        match tracks {
            GridTracks::Static(t) => self.set(make(TracksRef::Static(t))),
            GridTracks::Shared(t) => {
                let r = TracksRef::Local(t.id());
                self.held.set(id, t);
                self.put(make(r))
            }
        }
    }

    /// Sets the `transition` property: a `'static` transition is stored as a reference, a
    /// run-time one is held by the buffer (released when the property is replaced or removed,
    /// or the buffer is dropped; the buffer's clones share it). Returns `true` if the style
    /// changed: an equal transition changes nothing. The `transition` builder method calls it.
    /// Allocates only to hold a run-time transition (at build time); never panics.
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_core::Duration;
    /// use twine_style::{Props, StyleBuf, Transition};
    ///
    /// let t = Rc::new(Transition::of(Props::BG, Duration::ms(100)));
    /// let mut s = StyleBuf::new();
    /// assert!(s.set_transition(t.clone().into()));
    /// assert!(!s.set_transition(Transition::of(Props::BG, Duration::ms(100)).into())); // equal
    /// assert_eq!(Rc::strong_count(&t), 2); // the buffer holds it
    /// assert_eq!(s.get_transition(), Some(&*t));
    /// s.remove(twine_style::PropId::Transition);
    /// assert_eq!(Rc::strong_count(&t), 1); // released with the property
    /// ```
    pub fn set_transition(&mut self, t: TransitionValue) -> bool {
        if self.get_transition() == Some(t.get()) {
            return false;
        }
        match t {
            TransitionValue::Static(t) => self.set(StyleProp::Transition(TransitionRef::Static(t))),
            TransitionValue::Shared(t) => {
                let r = TransitionRef::Local(TransitionId::of(&t));
                self.held.set_transition(t);
                self.put(StyleProp::Transition(r))
            }
        }
    }

    /// The `transition` property, borrowed from the style (a `'static` transition or the
    /// run-time one the buffer holds), if set.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::{StyleBuf, Transition};
    /// let s = StyleBuf::new().transition(Transition::all(Duration::ms(150)).ease_out());
    /// assert_eq!(s.get_transition().map(|t| t.spec.duration), Some(Duration::ms(150)));
    /// assert_eq!(StyleBuf::new().get_transition(), None);
    /// ```
    #[must_use]
    pub fn get_transition(&self) -> Option<&Transition> {
        match find_transition(&self.props, self.has_group)? {
            TransitionRef::Static(t) => Some(t),
            TransitionRef::Local(tid) => self
                .held
                .transition()
                .filter(|t| TransitionId::of(t) == tid)
                .map(|t| &**t),
        }
    }

    /// Removes a property (and the run-time grid template or transition it held). Returns
    /// `true` if it was set.
    pub fn remove(&mut self, id: PropId) -> bool {
        let Some(i) = self.props.iter().position(|q| q.id() == id) else {
            return false;
        };
        self.props.remove(i);
        self.held.clear(id);
        self.has_group = mask_of(&self.props);
        true
    }

    /// Whether this buffer sets the grid template property `id` to the template `r` (a value
    /// resolved from it).
    #[must_use]
    pub fn holds_tracks(&self, id: PropId, r: TracksRef) -> bool {
        find_ref(&self.props, self.has_group, id) == Some(r)
    }

    /// Sets the grid template property `id` of `dst` to this buffer's template for it, sharing a
    /// run-time template (`dst` takes its own reference). Returns whether `dst` changed.
    /// Transitions use it to keep the template they show alive.
    ///
    /// ```
    /// use twine_style::{GridTrack, PropId, SharedTracks, StyleBuf};
    /// let t = SharedTracks::from(vec![GridTrack::Fr(1)]);
    /// let src = StyleBuf::new().grid_row_tracks(t.clone());
    /// let mut dst = StyleBuf::new();
    /// assert!(src.copy_tracks_to(PropId::GridRowTracks, &mut dst));
    /// drop(src);
    /// assert_eq!(dst.grid_tracks(PropId::GridRowTracks), Some(&t[..]));
    /// ```
    pub fn copy_tracks_to(&self, id: PropId, dst: &mut StyleBuf) -> bool {
        match find_ref(&self.props, self.has_group, id) {
            Some(TracksRef::Local(tid)) => self.held.copy_to(id, tid, dst),
            Some(r @ TracksRef::Static(_)) => match id {
                PropId::GridColumnTracks => dst.set(StyleProp::GridColumnTracks(r)),
                _ => dst.set(StyleProp::GridRowTracks(r)),
            },
            None => false,
        }
    }

    /// The value of `id`, if set.
    #[inline]
    #[must_use]
    pub fn get(&self, id: PropId) -> Option<StyleValue> {
        find(&self.props, self.has_group, id)
    }

    /// The tracks of the grid template property `id` (`GridColumnTracks` / `GridRowTracks`),
    /// borrowed from the style, if set.
    ///
    /// ```
    /// use twine_style::{GridTrack, PropId, StyleBuf, grid_tracks};
    /// let s = StyleBuf::new().grid_row_tracks(grid_tracks![content, fr(1)]);
    /// assert_eq!(s.grid_tracks(PropId::GridRowTracks), Some(&[GridTrack::Content, GridTrack::Fr(1)][..]));
    /// assert_eq!(s.grid_tracks(PropId::GridColumnTracks), None);
    /// ```
    #[must_use]
    pub fn grid_tracks(&self, id: PropId) -> Option<&[GridTrack]> {
        match find_ref(&self.props, self.has_group, id)? {
            TracksRef::Static(t) => Some(t),
            TracksRef::Local(tid) => self.held.get(id).filter(|t| t.id() == tid).map(|t| &**t),
        }
    }

    /// Removes every property (keeps the allocation) and releases the run-time templates and
    /// transition.
    pub fn clear(&mut self) {
        self.props.clear();
        self.has_group = 0;
        self.held = OwnedSlots::new();
    }

    /// Number of properties.
    #[must_use]
    pub fn len(&self) -> usize {
        self.props.len()
    }

    /// Whether no property is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.props.is_empty()
    }

    /// The set of properties this style sets (O(properties)).
    #[must_use]
    pub fn keys(&self) -> crate::Props {
        self.props.iter().map(StyleProp::id).collect()
    }

    /// The properties in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &StyleProp> {
        self.props.iter()
    }

    /// Bit `g` is set iff a property of group `g` is present.
    #[must_use]
    pub const fn has_group(&self) -> u16 {
        self.has_group
    }

    /// Composes at run time: sets every property of `src` (a [`Style`], a [`StyleBuf`] or a
    /// [`StyleRef`]) on this buffer, in `src`'s order, with the semantics of
    /// [`set`](Self::set): a property this buffer already has is replaced in place (`src`
    /// wins), the others are appended. Returns `true` if the buffer changed.
    ///
    /// **Grid templates:** a run-time template held by `src` (a `StyleBuf`) is shared, not
    /// moved: this buffer takes its own reference ([`copy_tracks_to`](Self::copy_tracks_to)),
    /// so the template lives as long as either container still sets it and is released with
    /// the property like any template the buffer holds. `'static` templates are copied as
    /// values. **Design elements** are copied as elements (resolved later by the display's
    /// theme, like in `src`).
    ///
    /// Runs at build time (allocates when the buffer grows); nothing of it runs per frame.
    /// Never panics.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::{GridTrack, PropId, SharedTracks, Style, StyleBuf, StyleValue, design, style};
    ///
    /// static BASE: Style = style! { bg: design::SURFACE, radius: 8 };
    /// let cols = SharedTracks::from(vec![GridTrack::Fr(1), GridTrack::Fr(2)]);
    /// let grid = StyleBuf::new().grid_column_tracks(cols.clone()).radius(2);
    ///
    /// let mut s = StyleBuf::new().text_color(Color::BLACK);
    /// assert!(s.extend_from(&BASE));
    /// assert!(s.extend_from(&grid)); // later wins: radius 2
    /// assert!(!s.extend_from(&grid)); // nothing new
    /// assert_eq!(s.get(PropId::BgColor), Some(StyleValue::Element(design::SURFACE.erase())));
    /// drop(grid);
    /// assert_eq!(s.grid_tracks(PropId::GridColumnTracks), Some(&cols[..])); // shared, still alive
    /// assert_eq!(cols.strong_count(), 2);
    /// ```
    pub fn extend_from(&mut self, src: &impl StyleContainer) -> bool {
        src.__extend_into(self)
    }
}

/// A heap copy of a flash style (one allocation), e.g. to override a few of its properties at
/// run time: `StyleBuf::from(&BASE).radius(4)`. Duplicates in `s` collapse (the last wins).
///
/// ```
/// use twine_style::{PropId, Style, StyleBuf, StyleValue, Length, style};
/// static BASE: Style = style! { radius: 8, padding: 4 };
/// let s = StyleBuf::from(&BASE).radius(4);
/// assert_eq!(s.get(PropId::Radius), Some(StyleValue::Length(Length::Px(4))));
/// assert_eq!(s.len(), 5);
/// ```
impl From<&Style> for StyleBuf {
    fn from(s: &Style) -> Self {
        let mut b = StyleBuf {
            props: Vec::with_capacity(s.props.len()),
            has_group: 0,
            held: OwnedSlots::new(),
        };
        b.extend_from(s);
        b
    }
}

impl<'a> IntoIterator for &'a StyleBuf {
    type Item = &'a StyleProp;
    type IntoIter = core::slice::Iter<'a, StyleProp>;

    fn into_iter(self) -> Self::IntoIter {
        self.props.iter()
    }
}

impl FromIterator<StyleProp> for StyleBuf {
    fn from_iter<I: IntoIterator<Item = StyleProp>>(iter: I) -> Self {
        let mut s = StyleBuf::new();
        for p in iter {
            s.set(p);
        }
        s
    }
}

/// A property refers to a run-time template or transition another container holds (see
/// [`StyleBuf::set`]).
#[cold]
#[inline(never)]
fn foreign_template(id: PropId) {
    twine_core::warn!(
        target: "twine::style",
        "{:?}: a run-time value can only be set with set_tracks / set_transition",
        id
    );
}

/// A style attached to a node: a `static` [`Style`] or a shared [`StyleBuf`] (themes share
/// their runtime styles between many nodes).
///
/// ```
/// use std::rc::Rc;
/// use twine_core::Color;
/// use twine_style::{PropId, StyleProp, Style, StyleBuf, StyleRef, style};
///
/// static S: Style = style! { bg_color: Color::RED };
/// let a = StyleRef::Static(&S);
/// let b = StyleRef::Shared(Rc::new(StyleBuf::new().bg_color(Color::RED)));
/// assert_eq!(a.get(PropId::BgColor), b.get(PropId::BgColor));
/// assert!(a.ptr_eq(&a.clone()) && !a.ptr_eq(&b));
/// ```
#[derive(Clone, Debug)]
pub enum StyleRef {
    /// A style in flash.
    Static(&'static Style),
    /// A heap style shared by reference counting.
    Shared(Rc<StyleBuf>),
}

impl StyleRef {
    /// The value of `id`, if set.
    #[inline]
    #[must_use]
    pub fn get(&self, id: PropId) -> Option<StyleValue> {
        match self {
            StyleRef::Static(s) => s.get(id),
            StyleRef::Shared(s) => s.get(id),
        }
    }

    /// The group mask (see [`Style::has_group`]).
    #[inline]
    #[must_use]
    pub fn has_group(&self) -> u16 {
        match self {
            StyleRef::Static(s) => s.has_group(),
            StyleRef::Shared(s) => s.has_group(),
        }
    }

    /// The tracks of the grid template property `id` of this style, borrowed from it (a
    /// `'static` list, or the run-time template a [`StyleBuf`] holds).
    ///
    /// ```
    /// use twine_style::{GridTrack, PropId, StyleBuf, StyleRef, grid_tracks};
    /// let s = StyleRef::from(StyleBuf::new().grid_column_tracks(grid_tracks![fr(1), fr(2)]));
    /// assert_eq!(s.grid_tracks(PropId::GridColumnTracks), Some(&[GridTrack::Fr(1), GridTrack::Fr(2)][..]));
    /// ```
    #[must_use]
    pub fn grid_tracks(&self, id: PropId) -> Option<&[GridTrack]> {
        match self {
            StyleRef::Static(s) => s.grid_tracks(id),
            StyleRef::Shared(b) => b.grid_tracks(id),
        }
    }

    /// The set of properties this style sets (O(properties)).
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::{PropId, Props, StyleBuf, StyleRef};
    /// let s = StyleRef::from(StyleBuf::new().bg(Color::RED).radius(4));
    /// assert_eq!(s.keys(), Props::from_ids(&[PropId::BgColor, PropId::BgOpacity, PropId::Radius]));
    /// ```
    #[must_use]
    pub fn keys(&self) -> crate::Props {
        match self {
            StyleRef::Static(s) => s.keys(),
            StyleRef::Shared(b) => b.keys(),
        }
    }

    /// The `transition` property of this style, borrowed from it (a `'static` transition, or
    /// the run-time one a [`StyleBuf`] holds).
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::{StyleBuf, StyleRef, Transition};
    /// let s = StyleRef::from(StyleBuf::new().transition(Transition::all(Duration::ms(80))));
    /// assert_eq!(s.get_transition(), Some(&Transition::all(Duration::ms(80))));
    /// ```
    #[must_use]
    pub fn get_transition(&self) -> Option<&Transition> {
        match self {
            StyleRef::Static(s) => s.get_transition(),
            StyleRef::Shared(b) => b.get_transition(),
        }
    }

    /// Whether both refer to the same style object.
    #[must_use]
    pub fn ptr_eq(&self, other: &StyleRef) -> bool {
        match (self, other) {
            (StyleRef::Static(a), StyleRef::Static(b)) => core::ptr::eq(*a, *b),
            (StyleRef::Shared(a), StyleRef::Shared(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl From<&'static Style> for StyleRef {
    fn from(s: &'static Style) -> Self {
        StyleRef::Static(s)
    }
}

impl From<Rc<StyleBuf>> for StyleRef {
    fn from(s: Rc<StyleBuf>) -> Self {
        StyleRef::Shared(s)
    }
}

impl From<StyleBuf> for StyleRef {
    fn from(s: StyleBuf) -> Self {
        StyleRef::Shared(Rc::new(s))
    }
}

#[cfg(test)]
mod tests {
    use twine_core::{Color, Opa};

    use super::*;
    use crate::value_types::Length;

    #[test]
    fn const_style_in_static() {
        static S: Style = Style::new(&[
            StyleProp::Width(crate::design::DesignValue::Fixed(Length::Px(10))), // group 0
            StyleProp::BgColor(crate::design::DesignValue::Fixed(Color::RED)),   // group 2
            StyleProp::GridCellYAlign(crate::GridAlign::Center),                 // group 8
        ]);
        static E: Style = Style::new(&[]);
        assert_eq!(S.has_group(), (1 << 0) | (1 << 2) | (1 << 8));
        assert_eq!(S.props().len(), 3);
        assert!(!S.is_empty());
        assert_eq!(E.has_group(), 0);
        assert!(E.is_empty());
    }

    #[test]
    fn last_duplicate_wins_in_static_style() {
        static S: Style = Style::new(&[
            StyleProp::Radius(crate::design::DesignValue::Fixed(crate::Radius::Px(1))),
            StyleProp::BgOpacity(crate::design::DesignValue::Fixed(Opa::COVER)),
            StyleProp::Radius(crate::design::DesignValue::Fixed(crate::Radius::Px(2))),
        ]);
        assert_eq!(S.get(PropId::Radius), Some(StyleValue::Length(Length::Px(2))));
    }

    #[test]
    fn stylebuf_set_replaces_and_reports_change() {
        let mut s = StyleBuf::new();
        assert!(s.set(StyleProp::Radius(crate::Radius::Px(3).into())));
        assert!(!s.set(StyleProp::Radius(crate::Radius::Px(3).into())));
        assert!(s.set(StyleProp::Radius(crate::Radius::Px(4).into())));
        assert_eq!(s.len(), 1, "replaced in place, no duplicate");
        assert!(s.set(StyleProp::PaddingTop(Length::Px(1).into())));
        assert_eq!(
            s.iter().map(StyleProp::id).collect::<Vec<_>>(),
            [PropId::Radius, PropId::PaddingTop]
        );
        assert_eq!(s.get(PropId::Radius), Some(StyleValue::Length(Length::Px(4))));
        let collected: StyleBuf = [
            StyleProp::Radius(crate::Radius::Px(1).into()),
            StyleProp::Radius(crate::Radius::Px(9).into()),
        ]
        .into_iter()
        .collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(
            collected.get(PropId::Radius),
            Some(StyleValue::Length(Length::Px(9)))
        );
        assert_eq!((&collected).into_iter().count(), 1);
    }

    #[test]
    fn stylebuf_remove_updates_mask() {
        let mut s = StyleBuf::new().width(5).bg_color(Color::RED).radius(3);
        let g_bg = PropId::BgColor.group_bit();
        assert_ne!(s.has_group() & g_bg, 0);
        assert!(s.remove(PropId::BgColor));
        assert!(!s.remove(PropId::BgColor));
        assert_eq!(s.has_group() & g_bg, 0, "group 2 empty now");
        assert_eq!(s.get(PropId::BgColor), None);
        assert_ne!(s.has_group() & PropId::Width.group_bit(), 0);
        s.clear();
        assert_eq!((s.len(), s.has_group()), (0, 0));
        assert!(s.is_empty());
    }

    #[test]
    fn has_group_skips_lookup() {
        let s = StyleBuf::new().width(5);
        let before = lookups();
        assert_eq!(s.get(PropId::BgColor), None); // group 2 not present: no scan
        assert_eq!(s.get(PropId::GridCellRowSpan), None);
        assert_eq!(lookups(), before);
        assert_eq!(s.get(PropId::Height), None); // same group as Width: scanned
        assert_eq!(lookups(), before + 1);
    }

    #[test]
    fn styleref_shared_and_static_equivalent() {
        static S: Style = Style::new(&[
            StyleProp::BgColor(crate::design::DesignValue::Fixed(Color::BLUE)),
            StyleProp::Radius(crate::design::DesignValue::Fixed(crate::Radius::Px(8))),
        ]);
        let st = StyleRef::from(&S);
        let sh = StyleRef::from(StyleBuf::new().bg_color(Color::BLUE).radius(8));
        for id in PropId::ALL {
            assert_eq!(st.get(id), sh.get(id), "{id:?}");
        }
        assert_eq!(st.has_group(), sh.has_group());
        let sh2 = sh.clone();
        assert!(sh.ptr_eq(&sh2));
        assert!(!sh.ptr_eq(&StyleRef::from(Rc::new(StyleBuf::new()))));
        assert!(st.ptr_eq(&StyleRef::Static(&S)));
    }

    #[test]
    fn a_buffer_owns_its_run_time_templates() {
        use crate::tracks::SharedTracks;
        use crate::value_types::GridTrack;
        static S: [GridTrack; 1] = [GridTrack::Fr(1)];
        let a = SharedTracks::from(alloc::vec![GridTrack::Px(1)]);
        let b = SharedTracks::from(alloc::vec![GridTrack::Px(2), GridTrack::Px(3)]);
        let mut s = StyleBuf::new();
        assert!(s.set_tracks(PropId::GridColumnTracks, a.clone().into()));
        assert_eq!(a.strong_count(), 2);
        // Replacing the template releases the old one; the property refers to the new one.
        assert!(s.set_tracks(PropId::GridColumnTracks, b.clone().into()));
        assert_eq!((a.strong_count(), b.strong_count()), (1, 2));
        assert_eq!(s.grid_tracks(PropId::GridColumnTracks), Some(&b[..]));
        // A `'static` list replaces it too.
        assert!(s.set(StyleProp::GridColumnTracks(TracksRef::Static(&S))));
        assert_eq!(b.strong_count(), 1);
        assert_eq!(s.grid_tracks(PropId::GridColumnTracks), Some(&S[..]));
        // Removing the property releases its template.
        s.set_tracks(PropId::GridRowTracks, a.clone().into());
        assert!(s.remove(PropId::GridRowTracks));
        assert_eq!(a.strong_count(), 1);
        assert_eq!(s.grid_tracks(PropId::GridRowTracks), None);
        // Clones share the template; it is freed with the last container.
        s.set_tracks(PropId::GridRowTracks, b.clone().into());
        let c = s.clone();
        assert_eq!(b.strong_count(), 3);
        drop(s);
        assert_eq!(c.grid_tracks(PropId::GridRowTracks), Some(&b[..]));
        drop(c);
        assert_eq!(b.strong_count(), 1);
    }

    #[test]
    fn a_run_time_template_cannot_move_to_another_container() {
        use crate::tracks::SharedTracks;
        use crate::value_types::GridTrack;
        let a = SharedTracks::from(alloc::vec![GridTrack::Px(1)]);
        let s = StyleBuf::new().grid_column_tracks(a.clone());
        let p = *s.iter().next().unwrap();
        // The property only refers to `s`'s template: another buffer does not take it.
        let mut other = StyleBuf::new();
        assert!(!other.set(p));
        assert_eq!(other.grid_tracks(PropId::GridColumnTracks), None);
        // Not even one that holds another template for the property.
        let mut third = StyleBuf::new().grid_column_tracks(alloc::vec![GridTrack::Px(9)]);
        assert!(!third.set(p));
        assert_eq!(
            third.grid_tracks(PropId::GridColumnTracks),
            Some(&[GridTrack::Px(9)][..])
        );
        // Its own buffer accepts it (unchanged).
        let mut s = s;
        assert!(!s.set(p));
        assert_eq!(s.grid_tracks(PropId::GridColumnTracks), Some(&a[..]));
        // A static style holds `'static` lists only (`style!` takes no `Vec`).
        drop(s);
        assert_eq!(a.strong_count(), 1);
    }

    #[test]
    fn merge_and_extend_follow_set_semantics() {
        use crate::design::{self, DesignValue};
        static BASE: Style = Style::new(&[
            StyleProp::BgColor(DesignValue::Element(design::SURFACE)),
            StyleProp::Radius(DesignValue::Fixed(crate::Radius::Px(8))),
            StyleProp::Radius(DesignValue::Fixed(crate::Radius::Px(9))), // later duplicate wins
        ]);
        // From a static style: duplicates collapse, design elements stay elements.
        let b = StyleBuf::from(&BASE);
        assert_eq!(b.len(), 2);
        assert_eq!(b.get(PropId::Radius), Some(StyleValue::Length(Length::Px(9))));
        assert_eq!(
            b.get(PropId::BgColor),
            Some(StyleValue::Element(design::SURFACE.erase()))
        );
        // `merge`: `over` wins in place, new properties are appended.
        let over = StyleBuf::new().bg_color(design::PRIMARY).width(3);
        let m = BASE.merge(&over);
        assert_eq!(
            m.iter().map(StyleProp::id).collect::<Vec<_>>(),
            [PropId::BgColor, PropId::Radius, PropId::Width]
        );
        assert_eq!(
            m.get(PropId::BgColor),
            Some(StyleValue::Element(design::PRIMARY.erase()))
        );
        // Through a `StyleRef` of either kind; extending with the same values changes nothing.
        let mut e = m.clone();
        assert!(!e.extend_from(&StyleRef::from(Rc::new(over.clone()))));
        assert!(e.extend_from(&StyleRef::Static(&BASE)));
        assert_eq!(
            e.get(PropId::BgColor),
            Some(StyleValue::Element(design::SURFACE.erase()))
        );
        assert!(!StyleBuf::new().extend_from(&Style::new(&[])));
    }

    #[test]
    fn extend_shares_run_time_templates_with_the_target() {
        use crate::tracks::SharedTracks;
        use crate::value_types::GridTrack;
        static ROWS: [GridTrack; 1] = [GridTrack::Content];
        let cols = SharedTracks::from(alloc::vec![GridTrack::Fr(1), GridTrack::Fr(2)]);
        let src = StyleBuf::new()
            .grid_column_tracks(cols.clone())
            .grid_row_tracks(&ROWS[..])
            .radius(2);
        // Into an empty buffer and into one holding another template for the property.
        let mut a = StyleBuf::new();
        let mut b = StyleBuf::new().grid_column_tracks(alloc::vec![GridTrack::Px(9)]);
        assert!(a.extend_from(&src));
        assert!(b.extend_from(&StyleRef::from(Rc::new(src.clone()))));
        assert_eq!(
            cols.strong_count(),
            4,
            "src, a and b each hold it (the Rc clone was dropped)"
        );
        drop(src);
        for s in [&a, &b] {
            assert_eq!(s.grid_tracks(PropId::GridColumnTracks), Some(&cols[..]));
            assert_eq!(s.grid_tracks(PropId::GridRowTracks), Some(&ROWS[..]));
            assert_eq!(s.get(PropId::Radius), Some(StyleValue::Length(Length::Px(2))));
        }
        // Each container releases its own reference.
        assert!(a.remove(PropId::GridColumnTracks));
        assert_eq!(cols.strong_count(), 2);
        // `merge` shares too.
        let m = Style::new(&[]).merge(&b);
        assert_eq!(cols.strong_count(), 3);
        drop((b, m));
        assert_eq!(cols.strong_count(), 1);
    }
}
