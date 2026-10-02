//! Grid templates in styles, and the slots in which a style container holds its run-time
//! values (grid templates and the transition, see [`TransitionRef`](crate::TransitionRef)).
//!
//! A grid template (the `grid_column_tracks` / `grid_row_tracks` properties) is either a
//! `'static` list (in flash, e.g. in a `style!`) or a list built at run time (a `Vec`, e.g.
//! from [`grid_tracks!`](crate::grid_tracks)). A [`StyleProp`](crate::StyleProp) stays `Copy`
//! and 12 bytes, so it cannot own a run-time list: its payload is a [`TracksRef`], either the
//! `'static` list itself or [`TracksRef::Local`], "the template this container holds for the
//! property". The **container** owns the list: every [`StyleBuf`](crate::StyleBuf) (heap
//! normal, class and theme styles, a node's local styles, transition styles) keeps one slot
//! per grid template property, set together with the property and released when the property
//! is replaced or removed or the buffer is dropped. A `static` [`Style`](crate::Style) has no
//! slots, so `style!` only takes `'static` lists.
//!
//! The tracks are always read through the container that holds the property:
//! [`StyleBuf::grid_tracks`](crate::StyleBuf::grid_tracks),
//! [`StyleRef::grid_tracks`](crate::StyleRef::grid_tracks) and
//! [`resolve_grid_tracks`](crate::resolve_grid_tracks) (the template of the style that wins
//! the cascade). A `TracksRef` alone cannot be dereferenced, so no path can read a freed list or
//! another container's list.
//!
//! [`GridTracks`] is the owned value users pass (a `'static` slice or array, a `Vec`, a
//! [`SharedTracks`]); it converts with `From`.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::ops::Deref;

use crate::value_types::GridTrack;

/// A grid template built at run time: an immutable, reference-counted list of tracks (cloning
/// shares it; the last clone frees it). One word.
///
/// ```
/// use twine_style::{GridTrack, SharedTracks};
///
/// let a = SharedTracks::from(vec![GridTrack::Fr(1), GridTrack::Px(40)]);
/// let b = a.clone(); // shared, not copied
/// assert_eq!(a.strong_count(), 2);
/// assert_eq!(&b[..], &[GridTrack::Fr(1), GridTrack::Px(40)]);
/// ```
#[derive(Clone, Debug)]
pub struct SharedTracks(Rc<Box<[GridTrack]>>);

impl SharedTracks {
    /// Number of holders (style containers and your own clones).
    #[must_use]
    pub fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    /// The identity of this list, as a [`TracksRef::Local`] carries it.
    #[inline]
    pub(crate) fn id(&self) -> TrackId {
        TrackId(Rc::as_ptr(&self.0) as usize)
    }
}

impl Deref for SharedTracks {
    type Target = [GridTrack];

    #[inline]
    fn deref(&self) -> &[GridTrack] {
        &self.0
    }
}

impl From<Vec<GridTrack>> for SharedTracks {
    fn from(v: Vec<GridTrack>) -> Self {
        Self(Rc::new(v.into_boxed_slice()))
    }
}

/// Two shared lists are equal when they have the same tracks.
impl PartialEq for SharedTracks {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Eq for SharedTracks {}

/// A grid template to set: a `'static` list (stored as is) or a list built at run time
/// (shared by reference counting; the style that holds it keeps it alive). Every grid template
/// setter takes `impl Into<GridTracks>`: a `'static` slice or array, a `Vec` or a
/// [`SharedTracks`]. Equality compares the tracks.
///
/// ```
/// use twine_style::{GridTrack, GridTracks, PropId, StyleBuf, grid_tracks};
///
/// static COLS: [GridTrack; 2] = [GridTrack::Fr(1), GridTrack::Px(80)];
/// let flash = StyleBuf::new().grid_column_tracks(&COLS);
/// let heap = StyleBuf::new().grid_column_tracks(grid_tracks![fr(1), px(80)]);
/// assert_eq!(flash.grid_tracks(PropId::GridColumnTracks), Some(&COLS[..]));
/// assert_eq!(heap.grid_tracks(PropId::GridColumnTracks), Some(&COLS[..]));
/// assert_eq!(GridTracks::from(&COLS), GridTracks::from(COLS.to_vec()));
/// ```
#[derive(Clone, Debug)]
pub enum GridTracks {
    /// A list in flash (or leaked).
    Static(&'static [GridTrack]),
    /// A list built at run time.
    Shared(SharedTracks),
}

impl GridTracks {
    /// A `'static` template (usable in `const` context).
    ///
    /// ```
    /// use twine_style::{GridTrack, GridTracks};
    /// static COLS: [GridTrack; 2] = [GridTrack::Fr(1), GridTrack::Px(40)];
    /// const T: GridTracks = GridTracks::from_static(&COLS);
    /// assert_eq!(T.tracks(), &COLS[..]);
    /// ```
    #[must_use]
    pub const fn from_static(tracks: &'static [GridTrack]) -> Self {
        Self::Static(tracks)
    }

    /// The tracks.
    #[inline]
    #[must_use]
    pub fn tracks(&self) -> &[GridTrack] {
        match self {
            Self::Static(t) => t,
            Self::Shared(s) => s,
        }
    }
}

impl PartialEq for GridTracks {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.tracks(), other.tracks());
        core::ptr::eq(a, b) || a == b
    }
}

impl Eq for GridTracks {}

impl From<&'static [GridTrack]> for GridTracks {
    fn from(t: &'static [GridTrack]) -> Self {
        Self::Static(t)
    }
}

impl<const N: usize> From<&'static [GridTrack; N]> for GridTracks {
    fn from(t: &'static [GridTrack; N]) -> Self {
        Self::Static(t)
    }
}

impl From<Vec<GridTrack>> for GridTracks {
    fn from(v: Vec<GridTrack>) -> Self {
        Self::Shared(v.into())
    }
}

impl From<SharedTracks> for GridTracks {
    fn from(s: SharedTracks) -> Self {
        Self::Shared(s)
    }
}

/// The identity of a run-time template held by a container (opaque: it only identifies, it
/// cannot be dereferenced).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackId(usize);

/// The payload of a grid template property in a [`StyleProp`](crate::StyleProp) and a
/// [`StyleValue`](crate::StyleValue): `Copy`, two words.
///
/// - `Static`: a `'static` list.
/// - `Local`: the run-time template that the **container** of the property holds (a
///   [`StyleBuf`](crate::StyleBuf) slot), with its identity. It only means something together
///   with that container: read the tracks through it
///   ([`StyleBuf::grid_tracks`](crate::StyleBuf::grid_tracks),
///   [`resolve_grid_tracks`](crate::resolve_grid_tracks)); there is no API that dereferences a
///   `TracksRef` on its own. A `StyleBuf` accepts a `Local` property only if it holds that very
///   template (copying such a property into another container does nothing).
///
/// Equality: static lists by tracks, local templates by identity (two values compared at the
/// same time refer to live templates, so different identities are different lists).
#[derive(Clone, Copy, Debug)]
pub enum TracksRef {
    /// A `'static` list.
    Static(&'static [GridTrack]),
    /// The template the property's container holds.
    Local(TrackId),
}

impl TracksRef {
    /// The tracks of a `'static` template (`None` for a container's template).
    #[must_use]
    pub const fn as_static(self) -> Option<&'static [GridTrack]> {
        match self {
            Self::Static(t) => Some(t),
            Self::Local(_) => None,
        }
    }
}

impl PartialEq for TracksRef {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Static(a), Self::Static(b)) => core::ptr::eq(a, b) || a == b,
            (Self::Local(a), Self::Local(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for TracksRef {}

impl crate::PropValue for TracksRef {
    #[inline]
    fn to_value(&self) -> crate::StyleValue {
        crate::StyleValue::GridTracks(*self)
    }

    #[inline]
    fn from_value(v: crate::StyleValue) -> Option<Self> {
        match v {
            crate::StyleValue::GridTracks(t) => Some(t),
            _ => None,
        }
    }
}

impl From<&'static [GridTrack]> for TracksRef {
    fn from(t: &'static [GridTrack]) -> Self {
        Self::Static(t)
    }
}

impl<const N: usize> From<&'static [GridTrack; N]> for TracksRef {
    fn from(t: &'static [GridTrack; N]) -> Self {
        Self::Static(t)
    }
}

/// The slot index of a grid template property (columns, rows).
#[inline]
fn slot_index(prop: crate::PropId) -> Option<usize> {
    match prop {
        crate::PropId::GridColumnTracks => Some(0),
        crate::PropId::GridRowTracks => Some(1),
        _ => None,
    }
}

/// The slots of a buffer that holds run-time values, behind a trait object: the code that
/// reads, copies, clones and drops run-time templates and transitions is reached only through
/// the vtable of [`Held`], which only exists once such a value was stored. Firmware that never
/// builds one links none of it.
trait Slots: core::fmt::Debug {
    /// The template of slot `i`.
    fn get(&self, i: usize) -> Option<&SharedTracks>;
    /// The transition held.
    fn transition(&self) -> Option<&Rc<crate::Transition>>;
    /// Releases slot `i` (`TRANSITION_SLOT`: the transition); returns whether every slot is
    /// empty now.
    fn clear(&mut self, i: usize) -> bool;
    /// A copy sharing the values.
    fn clone_box(&self) -> Box<dyn Slots>;
    /// Sets property `prop` of `dst` to the template of slot `i` if it is `id`.
    fn copy_to(&self, i: usize, id: TrackId, prop: crate::PropId, dst: &mut crate::StyleBuf) -> bool;
}

/// The slot index of the transition in [`Slots::clear`].
const TRANSITION_SLOT: usize = 2;

/// The run-time values of a buffer: the two template slots (columns, rows) and the transition.
#[derive(Clone, Debug, Default)]
struct Held {
    tracks: [Option<SharedTracks>; 2],
    transition: Option<Rc<crate::Transition>>,
}

impl Held {
    fn is_empty(&self) -> bool {
        self.tracks.iter().all(Option::is_none) && self.transition.is_none()
    }
}

impl Slots for Held {
    fn get(&self, i: usize) -> Option<&SharedTracks> {
        self.tracks.get(i)?.as_ref()
    }

    fn transition(&self) -> Option<&Rc<crate::Transition>> {
        self.transition.as_ref()
    }

    fn clear(&mut self, i: usize) -> bool {
        if i == TRANSITION_SLOT {
            self.transition = None;
        } else if let Some(s) = self.tracks.get_mut(i) {
            *s = None;
        }
        self.is_empty()
    }

    fn clone_box(&self) -> Box<dyn Slots> {
        Box::new(self.clone())
    }

    fn copy_to(&self, i: usize, id: TrackId, prop: crate::PropId, dst: &mut crate::StyleBuf) -> bool {
        match self.get(i) {
            Some(t) if t.id() == id => dst.set_tracks(prop, GridTracks::Shared(t.clone())),
            _ => false,
        }
    }
}

/// The run-time values of a [`StyleBuf`](crate::StyleBuf): one slot per grid template
/// property and one for the `transition` property. One word (a thin pointer to the boxed trait
/// object, so `StyleBuf` grows by one word only), no allocation until a run-time value is
/// stored.
#[derive(Debug, Default)]
#[allow(
    clippy::box_collection,
    reason = "a thin pointer keeps `StyleBuf` one word larger only"
)]
pub(crate) struct OwnedSlots(Option<Box<Box<dyn Slots>>>);

impl Clone for OwnedSlots {
    fn clone(&self) -> Self {
        Self(self.0.as_ref().map(|s| Box::new(s.clone_box())))
    }
}

impl OwnedSlots {
    /// No slots (does not allocate).
    pub(crate) const fn new() -> Self {
        Self(None)
    }

    /// The template held for `prop`.
    #[inline]
    pub(crate) fn get(&self, prop: crate::PropId) -> Option<&SharedTracks> {
        self.0.as_deref()?.get(slot_index(prop)?)
    }

    /// The transition held.
    #[inline]
    pub(crate) fn transition(&self) -> Option<&Rc<crate::Transition>> {
        self.0.as_deref()?.transition()
    }

    /// A copy of the held values for a replacement of one of them.
    fn take_held(&self) -> Held {
        let mut held = Held::default();
        if let Some(old) = self.0.as_deref() {
            for (j, s) in held.tracks.iter_mut().enumerate() {
                *s = old.get(j).cloned();
            }
            held.transition = old.transition().cloned();
        }
        held
    }

    /// Stores `t` for `prop` (replacing and releasing the previous one).
    pub(crate) fn set(&mut self, prop: crate::PropId, t: SharedTracks) {
        let Some(i) = slot_index(prop) else { return };
        let mut held = self.take_held();
        held.tracks[i] = Some(t);
        self.0 = Some(Box::new(Box::new(held)));
    }

    /// Stores the transition `t` (replacing and releasing the previous one).
    pub(crate) fn set_transition(&mut self, t: Rc<crate::Transition>) {
        let mut held = self.take_held();
        held.transition = Some(t);
        self.0 = Some(Box::new(Box::new(held)));
    }

    /// Sets property `prop` of `dst` to the template held for `prop` if it is `id`.
    pub(crate) fn copy_to(&self, prop: crate::PropId, id: TrackId, dst: &mut crate::StyleBuf) -> bool {
        match (self.0.as_deref(), slot_index(prop)) {
            (Some(s), Some(i)) => s.copy_to(i, id, prop, dst),
            _ => false,
        }
    }

    /// Releases the value held for `prop` (a grid template or the transition); frees the slots
    /// once all are empty.
    #[inline]
    pub(crate) fn clear(&mut self, prop: crate::PropId) {
        let i = match prop {
            crate::PropId::Transition => Some(TRANSITION_SLOT),
            _ => slot_index(prop),
        };
        if let Some(s) = self.0.as_deref_mut()
            && let Some(i) = i
            && s.clear(i)
        {
            self.0 = None;
        }
    }
}

/// Converts `style!` values of the grid template properties in `const` context: a `'static`
/// slice or array (the only templates a `static` style can hold).
#[doc(hidden)]
pub struct __TracksArg<T>(pub T);

impl __TracksArg<&'static [GridTrack]> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> TracksRef {
        TracksRef::Static(self.0)
    }
}

impl<const N: usize> __TracksArg<&'static [GridTrack; N]> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> TracksRef {
        TracksRef::Static(self.0)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn shared_lists_are_counted_and_compared_by_content() {
        let a = SharedTracks::from(vec![GridTrack::Px(5)]);
        let b = a.clone();
        assert_eq!(a.strong_count(), 2);
        drop(b);
        assert_eq!(a.strong_count(), 1);
        let other = SharedTracks::from(vec![GridTrack::Px(5)]);
        assert_eq!(a, other, "same tracks");
        assert_ne!(a.id(), other.id(), "another list");
        assert_eq!(
            GridTracks::from(a.clone()),
            GridTracks::from(vec![GridTrack::Px(5)])
        );
    }

    #[test]
    fn static_lists() {
        static A: [GridTrack; 1] = [GridTrack::Fr(1)];
        static B: [GridTrack; 1] = [GridTrack::Fr(1)];
        const C: TracksRef = __TracksArg(&A).get();
        assert_eq!(TracksRef::from(&A), TracksRef::from(&B)); // same tracks
        assert_eq!(C.as_static(), Some(&A[..]));
        assert_eq!(GridTracks::from(&A), GridTracks::from_static(&B));
    }

    #[test]
    fn slots_release_their_templates() {
        let t = SharedTracks::from(vec![GridTrack::Px(5)]);
        let mut s = OwnedSlots::new();
        assert!(s.0.is_none(), "no allocation without a template");
        s.set(crate::PropId::GridColumnTracks, t.clone());
        s.set(crate::PropId::GridRowTracks, t.clone());
        assert_eq!(t.strong_count(), 3);
        s.clear(crate::PropId::GridColumnTracks);
        assert_eq!(t.strong_count(), 2);
        s.clear(crate::PropId::GridRowTracks);
        assert!(s.0.is_none(), "freed once empty");
        assert_eq!(t.strong_count(), 1);
        // The transition slot lives with the templates.
        let tr = Rc::new(crate::Transition::all(twine_core::Duration::ms(1)));
        s.set(crate::PropId::GridRowTracks, t.clone());
        s.set_transition(tr.clone());
        assert_eq!(Rc::strong_count(&tr), 2);
        assert!(s.transition().is_some_and(|x| Rc::ptr_eq(x, &tr)));
        let c = s.clone();
        assert_eq!(Rc::strong_count(&tr), 3);
        drop(c);
        s.clear(crate::PropId::GridRowTracks);
        assert!(s.0.is_some(), "the transition is still held");
        s.clear(crate::PropId::Transition);
        assert!(s.0.is_none());
        assert_eq!(Rc::strong_count(&tr), 1);
    }

    #[test]
    fn size_is_two_words() {
        // `StyleProp` stays 12 bytes on 32-bit targets (checked at compile time in `prop.rs`).
        assert_eq!(
            core::mem::size_of::<TracksRef>(),
            core::mem::size_of::<&[GridTrack]>()
        );
    }
}
