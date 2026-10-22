//! [`BtnMap`]: the buttons of a button matrix, row by row — in flash ([`MapBtn`] tables) or
//! built at run time ([`OwnedMap`]).

use alloc::string::String;
use alloc::vec::Vec;

use super::BtnCtrl;

/// A button of a static [`BtnMap`]: its text and initial control bits (relative width,
/// checkable, …).
///
/// ```
/// use twine_widgets::buttonmatrix::{BtnCtrl, MapBtn};
///
/// const OK: MapBtn = MapBtn::new("OK", BtnCtrl::width(2).union(BtnCtrl::CHECKABLE));
/// assert_eq!((OK.text, OK.ctrl.width_units()), ("OK", 2));
/// assert_eq!(MapBtn::text("1").ctrl, BtnCtrl::empty());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MapBtn {
    /// The label (may be empty: a button without text).
    pub text: &'static str,
    /// The control bits the button starts with.
    pub ctrl: BtnCtrl,
}

impl MapBtn {
    /// A button showing `text` with control bits `ctrl`.
    #[must_use]
    pub const fn new(text: &'static str, ctrl: BtnCtrl) -> Self {
        MapBtn { text, ctrl }
    }

    /// A button showing `text`, one width unit, no other control bits.
    #[must_use]
    pub const fn text(text: &'static str) -> Self {
        MapBtn {
            text,
            ctrl: BtnCtrl::empty(),
        }
    }
}

/// A button matrix map built at run time: the buttons' texts and initial control bits in
/// reading order, and the number of buttons of each row. Each text is its own `String`, so a
/// button's text can be replaced in place
/// ([`ButtonMatrix::set_btn_text`](super::ButtonMatrix::set_btn_text) reuses its capacity).
///
/// ```
/// use twine_widgets::buttonmatrix::{BtnCtrl, OwnedMap};
///
/// let mut m = OwnedMap::new();
/// m.push("1", BtnCtrl::empty());
/// m.push("2", BtnCtrl::empty());
/// m.new_row();
/// m.push("OK", BtnCtrl::width(2));
/// assert_eq!((m.btn_count(), m.row_count()), (3, 2));
/// assert_eq!(m.text(2), Some("OK"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct OwnedMap {
    texts: Vec<String>,
    ctrl: Vec<BtnCtrl>,
    /// Buttons per row (a row may be empty).
    rows: Vec<u16>,
}

impl OwnedMap {
    /// An empty map (no rows).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty map with room for `buttons` buttons in `rows` rows.
    #[must_use]
    pub fn with_capacity(buttons: usize, rows: usize) -> Self {
        OwnedMap {
            texts: Vec::with_capacity(buttons),
            ctrl: Vec::with_capacity(buttons),
            rows: Vec::with_capacity(rows),
        }
    }

    /// Appends a button to the last row (starting the first row of an empty map). A map holds
    /// at most `u16::MAX` buttons per row: more are ignored with a warning.
    pub fn push(&mut self, text: &str, ctrl: BtnCtrl) {
        if self.rows.is_empty() {
            self.rows.push(0);
        }
        let Some(n) = self.rows.last_mut() else { return };
        if *n == u16::MAX {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: too many buttons in a row; ignored");
            return;
        }
        *n = n.saturating_add(1); // `*n < u16::MAX` (checked above)
        self.texts.push(String::from(text));
        self.ctrl.push(ctrl);
    }

    /// Starts a new row: the buttons pushed next go there. A row without buttons takes its
    /// height but shows nothing.
    pub fn new_row(&mut self) {
        self.rows.push(0);
    }

    /// The number of buttons.
    #[must_use]
    pub fn btn_count(&self) -> usize {
        self.texts.len()
    }

    /// The number of rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The text of button `i`.
    #[must_use]
    pub fn text(&self, i: usize) -> Option<&str> {
        self.texts.get(i).map(String::as_str)
    }
}

impl From<&BtnMap> for OwnedMap {
    /// A copy of any map (e.g. to edit a static one).
    fn from(m: &BtnMap) -> Self {
        let mut o = OwnedMap::with_capacity(m.btn_count(), m.row_count());
        let mut i = 0;
        for len in m.row_lens() {
            o.new_row();
            for _ in 0..len {
                o.push(m.text(i).unwrap_or(""), m.initial_ctrl(i).unwrap_or_default());
                // At most `btn_count()` buttons: far below `usize::MAX`.
                i = i.saturating_add(1);
            }
        }
        o
    }
}

/// The buttons of a [`ButtonMatrix`](super::ButtonMatrix), row by row: each button has its
/// text and the control bits it starts with. Rows are explicit (no `"\n"` entries, no `""`
/// terminator: an empty text is a button without text).
///
/// - [`Static`](BtnMap::Static): a table in flash, stored by reference (no allocation):
///   `&[&[MapBtn]]`, one slice per row.
/// - [`Owned`](BtnMap::Owned): built at run time ([`OwnedMap`]), e.g. by `twine-view`.
///
/// ```
/// use twine_widgets::buttonmatrix::{BtnCtrl, BtnMap, MapBtn};
///
/// static KEYS: [&[MapBtn]; 2] = [
///     &[MapBtn::text("1"), MapBtn::text("2"), MapBtn::text("3")],
///     &[MapBtn::new("OK", BtnCtrl::width(2))],
/// ];
/// let m = BtnMap::from(&KEYS);
/// assert_eq!((m.btn_count(), m.row_count()), (4, 2));
/// assert_eq!(m.text(3), Some("OK"));
/// assert_eq!(m.row_lens().collect::<Vec<_>>(), [3, 1]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BtnMap {
    /// A map in flash.
    Static(&'static [&'static [MapBtn]]),
    /// A map built at run time.
    Owned(OwnedMap),
}

impl Default for BtnMap {
    /// The empty map (no rows, no buttons).
    fn default() -> Self {
        BtnMap::Static(&[])
    }
}

impl From<&'static [&'static [MapBtn]]> for BtnMap {
    fn from(m: &'static [&'static [MapBtn]]) -> Self {
        BtnMap::Static(m)
    }
}

impl<const N: usize> From<&'static [&'static [MapBtn]; N]> for BtnMap {
    fn from(m: &'static [&'static [MapBtn]; N]) -> Self {
        BtnMap::Static(m)
    }
}

impl From<OwnedMap> for BtnMap {
    fn from(m: OwnedMap) -> Self {
        BtnMap::Owned(m)
    }
}

/// The texts of a [`BtnMap`] in reading order ([`BtnMap::texts`]).
#[derive(Clone, Debug)]
pub struct BtnTexts<'a>(TextsInner<'a>);

#[derive(Clone, Debug)]
enum TextsInner<'a> {
    Static {
        rows: core::slice::Iter<'a, &'static [MapBtn]>,
        row: core::slice::Iter<'a, MapBtn>,
    },
    Owned(core::slice::Iter<'a, String>),
}

impl<'a> Iterator for BtnTexts<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        match &mut self.0 {
            TextsInner::Static { rows, row } => loop {
                if let Some(b) = row.next() {
                    return Some(b.text);
                }
                *row = rows.next()?.iter();
            },
            TextsInner::Owned(it) => it.next().map(String::as_str),
        }
    }
}

impl BtnMap {
    /// The number of buttons.
    #[must_use]
    pub fn btn_count(&self) -> usize {
        match self {
            BtnMap::Static(rows) => rows.iter().map(|r| r.len()).sum(),
            BtnMap::Owned(m) => m.btn_count(),
        }
    }

    /// The number of rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        match self {
            BtnMap::Static(rows) => rows.len(),
            BtnMap::Owned(m) => m.row_count(),
        }
    }

    /// The number of buttons of each row, top to bottom.
    pub fn row_lens(&self) -> impl Iterator<Item = usize> + '_ {
        let (s, o): (&[&[MapBtn]], &[u16]) = match self {
            BtnMap::Static(rows) => (rows, &[]),
            BtnMap::Owned(m) => (&[], &m.rows),
        };
        s.iter().map(|r| r.len()).chain(o.iter().map(|&n| usize::from(n)))
    }

    /// The texts of the buttons in reading order. Allocation-free.
    #[must_use]
    pub fn texts(&self) -> BtnTexts<'_> {
        BtnTexts(match self {
            BtnMap::Static(rows) => TextsInner::Static {
                rows: rows.iter(),
                row: [].iter(),
            },
            BtnMap::Owned(m) => TextsInner::Owned(m.texts.iter()),
        })
    }

    /// The text of button `i` (in reading order).
    #[must_use]
    pub fn text(&self, i: usize) -> Option<&str> {
        match self {
            BtnMap::Static(_) => self.static_btn(i).map(|b| b.text),
            BtnMap::Owned(m) => m.text(i),
        }
    }

    /// The control bits button `i` starts with.
    #[must_use]
    pub fn initial_ctrl(&self, i: usize) -> Option<BtnCtrl> {
        match self {
            BtnMap::Static(_) => self.static_btn(i).map(|b| b.ctrl),
            BtnMap::Owned(m) => m.ctrl.get(i).copied(),
        }
    }

    /// Button `i` of a static map.
    fn static_btn(&self, mut i: usize) -> Option<&MapBtn> {
        let BtnMap::Static(rows) = self else { return None };
        for r in *rows {
            if let Some(b) = r.get(i) {
                return Some(b);
            }
            // `i ≥ r.len()` (no button `i` in this row): exact.
            i = i.saturating_sub(r.len());
        }
        None
    }

    /// Whether `self` and `other` show the same buttons (a static map: the same table, or an
    /// equal one).
    pub(crate) fn same(&self, other: &BtnMap) -> bool {
        match (self, other) {
            (BtnMap::Static(a), BtnMap::Static(b)) => core::ptr::eq(*a, *b) || a == b,
            _ => self == other,
        }
    }

    /// The buffer to write the new text of button `i` into, when `text` differs from its
    /// text (a static map becomes an owned copy first). `None` when unchanged or when `i` is
    /// out of range. Copying a static map is noted in partition `part`'s seal domain (the
    /// widget's engine).
    pub(crate) fn text_buf(
        &mut self,
        i: usize,
        text: &str,
        part: twine_core::Partition,
    ) -> Option<&mut String> {
        if i >= self.btn_count() || self.text(i) == Some(text) {
            return None;
        }
        if let BtnMap::Static(_) = self {
            twine_core::alloc_guard::note(part, twine_core::alloc_guard::AllocSite::WidgetBuffer);
            *self = BtnMap::Owned(OwnedMap::from(&*self));
        }
        let BtnMap::Owned(m) = self else { return None };
        m.texts.get_mut(i)
    }

    /// The owned texts (none for a static map), to bound them.
    pub(crate) fn owned_texts_mut(&mut self) -> &mut [String] {
        match self {
            BtnMap::Owned(m) => &mut m.texts,
            BtnMap::Static(_) => &mut [],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static MAP: [&[MapBtn]; 3] = [
        &[MapBtn::text("a"), MapBtn::text("b")],
        &[],
        &[MapBtn::new("c", BtnCtrl::CHECKABLE)],
    ];

    #[test]
    fn static_and_owned_maps_agree() {
        let s = BtnMap::from(&MAP);
        let o = BtnMap::Owned(OwnedMap::from(&s));
        for m in [&s, &o] {
            assert_eq!((m.btn_count(), m.row_count()), (3, 3));
            assert_eq!(m.row_lens().collect::<Vec<_>>(), [2, 0, 1]);
            assert_eq!(m.texts().collect::<Vec<_>>(), ["a", "b", "c"]);
            assert_eq!(m.text(2), Some("c"));
            assert_eq!(m.text(3), None);
            assert_eq!(m.initial_ctrl(2), Some(BtnCtrl::CHECKABLE));
        }
        assert!(!s.same(&o) && s.same(&BtnMap::from(&MAP)));
    }

    #[test]
    fn text_buf_copies_a_static_map_once_and_reuses_the_strings() {
        /// Writes `t` into button `i` the way `ButtonMatrix::set_btn_text` does.
        fn set(m: &mut BtnMap, i: usize, t: &str) -> bool {
            m.text_buf(i, t, twine_core::Partition::MAIN)
                .map(|b| {
                    b.clear();
                    b.push_str(t);
                })
                .is_some()
        }
        let mut m = BtnMap::from(&MAP);
        assert!(!set(&mut m, 0, "a"), "same text: no copy");
        assert!(matches!(m, BtnMap::Static(_)));
        assert!(set(&mut m, 1, "bee"));
        assert_eq!(m.texts().collect::<Vec<_>>(), ["a", "bee", "c"]);
        let BtnMap::Owned(o) = &m else { panic!("owned") };
        let cap = o.texts[1].capacity();
        assert!(set(&mut m, 1, "b"));
        let BtnMap::Owned(o) = &m else { panic!("owned") };
        assert_eq!(o.texts[1].capacity(), cap);
        assert!(!set(&mut m, 9, "x"));
        assert_eq!(m.owned_texts_mut().len(), 3);
        assert!(BtnMap::from(&MAP).owned_texts_mut().is_empty());
    }

    #[test]
    fn owned_map_rows() {
        let mut m = OwnedMap::new();
        m.new_row();
        m.new_row();
        m.push("x", BtnCtrl::empty());
        assert_eq!((m.row_count(), m.btn_count()), (2, 1));
        assert_eq!(BtnMap::Owned(m).row_lens().collect::<Vec<_>>(), [0, 1]);
    }
}
