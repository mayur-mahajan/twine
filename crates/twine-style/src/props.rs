//! [`Props`]: a set of style properties as a bitset, with one constant per property group of
//! the property table.

use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not, Sub};

use crate::prop::{PROP_COUNT, PropId};

/// Number of `u32` words of a [`Props`] bitset (bit `id − 1` per property).
const WORDS: usize = PROP_COUNT.div_ceil(32);

/// A set of style properties: one bit per [`PropId`], `Copy`, `const`-constructible.
///
/// Every property belongs to exactly one **property group** of the property table; each group
/// is a constant generated from the table (so a group always has exactly the table's members):
/// `SIZE`, `POSITION`, `TRANSFORM` (translation, scale, rotation, skew, pivot, transform
/// width/height), `PADDING`, `MARGIN`, `BG`, `BORDER`, `OUTLINE`, `SHADOW`, `DROP_SHADOW`,
/// `BLUR`, `IMAGE`, `LINE`, `ARC`, `TEXT`, `RADIUS`, `OPACITY`, `RECOLOR`, `MISC`, `FLEX` and
/// `GRID` (members: `twine-style/PROPERTIES.md`). Combine them with `|`, add single
/// properties with [`with`](Self::with) or `| PropId::…`, remove with `-`; in `const` /
/// `static` context (operators are not `const`) use [`union`](Self::union),
/// [`with`](Self::with), [`difference`](Self::difference) and [`from_ids`](Self::from_ids). [`GROUPS`](Self::GROUPS)
/// lists every group with its name.
///
/// Used by [`Transition`](crate::Transition) to say which properties animate. Membership is
/// one shift and mask ([`contains`](Self::contains)); iteration ([`iter`](Self::iter)) visits
/// set bits only, so the cost of going through a set is O(members), never O(all properties).
///
/// ```
/// use twine_style::{PropId, Props};
///
/// let p = Props::BG | Props::TRANSFORM | PropId::Radius;
/// assert!(p.contains(PropId::BgColor) && p.contains(PropId::TranslateX));
/// assert!(p.contains(PropId::Radius) && !p.contains(PropId::Width));
/// assert_eq!((p - Props::BG).len(), Props::TRANSFORM.len() + 1);
///
/// // Constants: `|` is not `const`, `union` / `with` / `from_ids` are.
/// const PRESS: Props = Props::BG.union(Props::TRANSFORM).with(PropId::Radius);
/// assert_eq!(PRESS, p);
/// const EXACT: Props = Props::from_ids(&[PropId::BgColor, PropId::TransformScaleX]);
/// assert_eq!(EXACT.iter().collect::<Vec<_>>(), [PropId::TransformScaleX, PropId::BgColor]); // id order
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Props([u32; WORDS]);

impl Props {
    /// No property.
    pub const EMPTY: Props = Props([0; WORDS]);

    /// Every property.
    pub const ALL: Props = Props::from_ids(&PropId::ALL);

    /// Every property whose values change gradually during a transition (integers, lengths,
    /// colors, opacities, angles, scales: see [`is_interpolable`](crate::is_interpolable)).
    /// The others (fonts, images, enums, flags, references, grid templates) switch at once or
    /// at the end of a transition.
    pub const INTERPOLABLE: Props = interpolable();

    /// The set of `id`.
    /// ```
    /// use twine_style::{PropId, Props};
    /// assert_eq!(Props::of(PropId::Radius).len(), 1);
    /// ```
    #[inline]
    #[must_use]
    pub const fn of(id: PropId) -> Props {
        Props::EMPTY.with(id)
    }

    /// The set of `ids` (duplicates are fine).
    /// ```
    /// use twine_style::{PropId, Props};
    /// const P: Props = Props::from_ids(&[PropId::Width, PropId::Height, PropId::Width]);
    /// assert_eq!(P.len(), 2);
    /// ```
    #[must_use]
    pub const fn from_ids(ids: &[PropId]) -> Props {
        let mut p = Props::EMPTY;
        let mut i = 0;
        while i < ids.len() {
            p = p.with(ids[i]);
            i += 1;
        }
        p
    }

    /// This set plus `id`.
    /// ```
    /// use twine_style::{PropId, Props};
    /// assert!(Props::BG.with(PropId::Radius).contains(PropId::Radius));
    /// ```
    #[inline]
    #[must_use]
    pub const fn with(mut self, id: PropId) -> Props {
        let (w, b) = bit(id);
        self.0[w] |= b;
        self
    }

    /// This set without `id`.
    /// ```
    /// use twine_style::{PropId, Props};
    /// assert!(!Props::BG.without(PropId::BgColor).contains(PropId::BgColor));
    /// ```
    #[inline]
    #[must_use]
    pub const fn without(mut self, id: PropId) -> Props {
        let (w, b) = bit(id);
        self.0[w] &= !b;
        self
    }

    /// The members of either set.
    /// `const` form of `|`.
    ///
    /// ```
    /// use twine_style::{PropId, Props};
    /// const P: Props = Props::BG.union(Props::BORDER);
    /// assert_eq!(P, Props::BG | Props::BORDER);
    /// ```
    #[inline]
    #[must_use]
    pub const fn union(mut self, other: Props) -> Props {
        let mut i = 0;
        while i < WORDS {
            self.0[i] |= other.0[i];
            i += 1;
        }
        self
    }

    /// The members of both sets.
    /// `const` form of `&`.
    ///
    /// ```
    /// use twine_style::{PropId, Props};
    /// let p = Props::BG.with(PropId::Font).intersection(Props::INTERPOLABLE);
    /// assert!(p.contains(PropId::BgColor) && !p.contains(PropId::Font));
    /// ```
    #[inline]
    #[must_use]
    pub const fn intersection(mut self, other: Props) -> Props {
        let mut i = 0;
        while i < WORDS {
            self.0[i] &= other.0[i];
            i += 1;
        }
        self
    }

    /// The members of this set that are not in `other`.
    /// `const` form of `-`.
    ///
    /// ```
    /// use twine_style::{PropId, Props};
    /// const P: Props = Props::ALL.difference(Props::TEXT);
    /// assert!(!P.contains(PropId::Font) && P.contains(PropId::Width));
    /// ```
    #[inline]
    #[must_use]
    pub const fn difference(mut self, other: Props) -> Props {
        let mut i = 0;
        while i < WORDS {
            self.0[i] &= !other.0[i];
            i += 1;
        }
        self
    }

    /// Whether `id` is a member.
    /// ```
    /// use twine_style::{PropId, Props};
    /// assert!(Props::BG.contains(PropId::BgColor));
    /// assert!(!Props::BG.contains(PropId::Width));
    /// ```
    #[inline]
    #[must_use]
    pub const fn contains(&self, id: PropId) -> bool {
        let (w, b) = bit(id);
        self.0[w] & b != 0
    }

    /// Whether the set is empty.
    /// ```
    /// use twine_style::{PropId, Props};
    /// assert!(Props::EMPTY.is_empty());
    /// assert!((Props::BG - Props::BG).is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        let mut i = 0;
        while i < WORDS {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Number of members.
    /// ```
    /// use twine_style::{PropId, Props};
    /// assert_eq!(Props::of(PropId::Width).with(PropId::Height).len(), 2);
    /// ```
    #[must_use]
    pub const fn len(&self) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < WORDS {
            n += self.0[i].count_ones() as usize;
            i += 1;
        }
        n
    }

    /// The members in id order (visits set bits only).
    /// ```
    /// use twine_style::{PropId, Props};
    /// let p = Props::of(PropId::Height).with(PropId::Width);
    /// assert_eq!(p.iter().count(), 2);
    /// assert!(p.iter().all(|id| p.contains(id)));
    /// ```
    pub fn iter(self) -> impl Iterator<Item = PropId> {
        let mut words = self.0;
        let mut w = 0;
        core::iter::from_fn(move || {
            while w < WORDS {
                let word = words[w];
                if word != 0 {
                    let b = word.trailing_zeros();
                    words[w] &= word - 1;
                    return PropId::from_u8((w * 32 + b as usize + 1) as u8);
                }
                w += 1;
            }
            None
        })
    }
}

/// Word index and mask of `id` (bit `id − 1`).
#[inline]
const fn bit(id: PropId) -> (usize, u32) {
    let i = id as usize - 1;
    (i / 32, 1 << (i % 32))
}

/// [`Props::INTERPOLABLE`], computed at compile time from the table's default values.
const fn interpolable() -> Props {
    let mut p = Props::EMPTY;
    let mut i = 0;
    while i < PROP_COUNT {
        if crate::transition::is_interpolable(PropId::ALL[i]) {
            p = p.with(PropId::ALL[i]);
        }
        i += 1;
    }
    p
}

impl Default for Props {
    /// [`Props::EMPTY`].
    fn default() -> Self {
        Props::EMPTY
    }
}

impl From<PropId> for Props {
    fn from(id: PropId) -> Self {
        Props::of(id)
    }
}

impl FromIterator<PropId> for Props {
    fn from_iter<I: IntoIterator<Item = PropId>>(iter: I) -> Self {
        iter.into_iter().fold(Props::EMPTY, Props::with)
    }
}

impl BitOr for Props {
    type Output = Props;
    fn bitor(self, rhs: Props) -> Props {
        self.union(rhs)
    }
}

impl BitOr<PropId> for Props {
    type Output = Props;
    fn bitor(self, rhs: PropId) -> Props {
        self.with(rhs)
    }
}

impl BitOrAssign for Props {
    fn bitor_assign(&mut self, rhs: Props) {
        *self = self.union(rhs);
    }
}

impl BitOrAssign<PropId> for Props {
    fn bitor_assign(&mut self, rhs: PropId) {
        *self = self.with(rhs);
    }
}

impl BitAnd for Props {
    type Output = Props;
    fn bitand(self, rhs: Props) -> Props {
        self.intersection(rhs)
    }
}

impl BitAndAssign for Props {
    fn bitand_assign(&mut self, rhs: Props) {
        *self = self.intersection(rhs);
    }
}

impl Sub for Props {
    type Output = Props;
    fn sub(self, rhs: Props) -> Props {
        self.difference(rhs)
    }
}

impl Sub<PropId> for Props {
    type Output = Props;
    fn sub(self, rhs: PropId) -> Props {
        self.without(rhs)
    }
}

impl Not for Props {
    type Output = Props;
    /// Every property not in the set.
    fn not(self) -> Props {
        Props::ALL.difference(self)
    }
}

impl core::fmt::Debug for Props {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Props {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Props({} props)", self.len());
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;
    use crate::prop::generated_tests::GROUP_ROWS;

    #[test]
    fn groups_match_the_table() {
        // Every row of the table is in its group's constant and in no other one.
        let mut seen = Props::EMPTY;
        for &(group, members) in GROUP_ROWS {
            let constant = Props::GROUPS
                .iter()
                .find(|g| g.0 == group)
                .map(|g| g.1)
                .expect("group constant");
            assert_eq!(constant, Props::from_ids(members), "{group}");
            assert!((seen & constant).is_empty(), "{group} overlaps an earlier group");
            seen |= constant;
        }
        assert_eq!(seen, Props::ALL, "the groups cover every property");
        assert_eq!(Props::GROUPS.len(), GROUP_ROWS.len());
        assert_eq!(Props::ALL.len(), PROP_COUNT);
        assert!(Props::BG.contains(PropId::BgColor) && Props::BG.contains(PropId::BgImage));
        assert!(Props::TRANSFORM.contains(PropId::TranslateY) && !Props::TRANSFORM.contains(PropId::X));
        assert!(Props::RECOLOR.contains(PropId::Recolor) && Props::RADIUS.contains(PropId::Radius));
    }

    #[test]
    fn set_operations() {
        let a = Props::from_ids(&[PropId::Width, PropId::BgColor, PropId::GridCellYAlign]);
        assert_eq!(a.len(), 3);
        assert_eq!(
            a.iter().collect::<Vec<_>>(),
            [PropId::Width, PropId::BgColor, PropId::GridCellYAlign]
        );
        assert_eq!(
            a - PropId::Width,
            Props::from_ids(&[PropId::BgColor, PropId::GridCellYAlign])
        );
        assert_eq!(a & Props::BG, Props::of(PropId::BgColor));
        assert!((!a).contains(PropId::Height) && !(!a).contains(PropId::Width));
        assert_eq!(!Props::EMPTY, Props::ALL);
        assert_eq!(PropId::ALL.into_iter().collect::<Props>(), Props::ALL);
        assert_eq!(Props::ALL.iter().count(), PROP_COUNT);
        assert!(Props::default().is_empty() && Props::EMPTY.iter().next().is_none());
        let mut b = Props::EMPTY;
        b |= PropId::Radius;
        b |= Props::OPACITY;
        b &= Props::RADIUS | Props::OPACITY;
        assert_eq!(b, Props::from(PropId::Radius) | Props::OPACITY);
        // The last property id is representable.
        let last = PropId::ALL[PROP_COUNT - 1];
        assert!(Props::of(last).contains(last) && Props::of(last).iter().eq([last]));
    }

    #[test]
    fn interpolable_follows_the_payload_types() {
        for id in PropId::ALL {
            assert_eq!(
                Props::INTERPOLABLE.contains(id),
                crate::is_interpolable(id),
                "{id:?}"
            );
        }
        assert!(!Props::INTERPOLABLE.contains(PropId::GridColumnTracks));
        assert!(Props::INTERPOLABLE.contains(PropId::BgColor));
    }
}
