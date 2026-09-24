//! Style containers: flash-resident [`Style`], heap [`StyleBuf`] and the shared [`StyleRef`].

use alloc::rc::Rc;
use alloc::vec::Vec;

use crate::prop::{PropId, StyleProp};
use crate::value::StyleValue;

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

const fn mask_of(props: &[StyleProp]) -> u16 {
    let mut mask = 0u16;
    let mut i = 0;
    while i < props.len() {
        mask |= props[i].id().group_bit();
        i += 1;
    }
    mask
}

/// An immutable, `const`-constructible style that lives in flash (LVGL `LV_STYLE_CONST_INIT`).
///
/// Usually written with the [`style!`](crate::style!) macro. Lookups scan the properties from
/// the end, so a later duplicate wins; the precomputed group mask ([`Style::has_group`]) lets
/// resolution skip styles that cannot contain a property.
///
/// ```
/// use twine_core::Color;
/// use twine_style::{PropId, Style, StyleProp, StyleValue};
///
/// static S: Style = Style::new(&[StyleProp::BgColor(Color::RED), StyleProp::Radius(4)]);
/// assert_eq!(S.get(PropId::Radius), Some(StyleValue::Int(4)));
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

    /// The properties, in declaration order.
    #[must_use]
    pub fn props(&self) -> &'static [StyleProp] {
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
/// let mut s = StyleBuf::new().bg_color(Color::BLUE).bg_opa(Opa::COVER).width(120);
/// assert_eq!(s.len(), 3);
/// assert!(s.set(StyleProp::BgColor(Color::RED))); // changed
/// assert!(!s.set(StyleProp::BgColor(Color::RED))); // same value
/// assert_eq!(s.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
/// assert!(s.remove(PropId::BgOpa));
/// assert_eq!(s.len(), 2);
/// ```
#[derive(Clone, Debug, Default)]
pub struct StyleBuf {
    props: Vec<StyleProp>,
    has_group: u16,
}

impl StyleBuf {
    /// An empty style (does not allocate).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            props: Vec::new(),
            has_group: 0,
        }
    }

    /// Sets a property, replacing an existing value of the same property in place. Returns
    /// `true` if the style changed (new property or different value).
    pub fn set(&mut self, p: StyleProp) -> bool {
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

    /// Removes a property. Returns `true` if it was set.
    pub fn remove(&mut self, id: PropId) -> bool {
        let Some(i) = self.props.iter().position(|q| q.id() == id) else {
            return false;
        };
        self.props.remove(i);
        self.has_group = mask_of(&self.props);
        true
    }

    /// The value of `id`, if set.
    #[inline]
    #[must_use]
    pub fn get(&self, id: PropId) -> Option<StyleValue> {
        find(&self.props, self.has_group, id)
    }

    /// Removes every property (keeps the allocation).
    pub fn clear(&mut self) {
        self.props.clear();
        self.has_group = 0;
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

    /// The properties in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &StyleProp> {
        self.props.iter()
    }

    /// Bit `g` is set iff a property of group `g` is present.
    #[must_use]
    pub const fn has_group(&self) -> u16 {
        self.has_group
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

/// A style attached to a node: a `static` [`Style`] or a shared [`StyleBuf`] (themes share
/// their runtime styles between many nodes).
///
/// ```
/// use std::rc::Rc;
/// use twine_core::Color;
/// use twine_style::{PropId, Style, StyleBuf, StyleProp, StyleRef};
///
/// static S: Style = Style::new(&[StyleProp::BgColor(Color::RED)]);
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
            StyleProp::Width(Length::Px(10)),                    // group 0
            StyleProp::BgColor(Color::RED),                      // group 2
            StyleProp::GridCellYAlign(crate::GridAlign::Center), // group 8
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
            StyleProp::Radius(1),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(2),
        ]);
        assert_eq!(S.get(PropId::Radius), Some(StyleValue::Int(2)));
    }

    #[test]
    fn stylebuf_set_replaces_and_reports_change() {
        let mut s = StyleBuf::new();
        assert!(s.set(StyleProp::Radius(3)));
        assert!(!s.set(StyleProp::Radius(3)));
        assert!(s.set(StyleProp::Radius(4)));
        assert_eq!(s.len(), 1, "replaced in place, no duplicate");
        assert!(s.set(StyleProp::PadTop(1)));
        assert_eq!(
            s.iter().map(StyleProp::id).collect::<Vec<_>>(),
            [PropId::Radius, PropId::PadTop]
        );
        assert_eq!(s.get(PropId::Radius), Some(StyleValue::Int(4)));
        let collected: StyleBuf = [StyleProp::Radius(1), StyleProp::Radius(9)].into_iter().collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected.get(PropId::Radius), Some(StyleValue::Int(9)));
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
        static S: Style = Style::new(&[StyleProp::BgColor(Color::BLUE), StyleProp::Radius(8)]);
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
}
