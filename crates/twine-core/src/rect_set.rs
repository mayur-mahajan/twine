//! [`RectSet`]: the fixed-capacity dirty-area set used by the refresh pipeline.

use crate::geometry::Rect;

/// Result of [`RectSet::add`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AddResult {
    /// The rectangle was stored.
    Added,
    /// An existing rectangle already covers it; nothing changed.
    AlreadyCovered,
    /// Nothing remained after clipping to the bounds.
    ClippedAway,
    /// The set was full: to make room, the two rectangles (stored or new) whose union adds
    /// the least area were joined.
    Overflowed,
}

/// Up to `N` dirty rectangles (or a lower [`limit`](RectSet::limit)), clipped to `bounds`,
/// never allocating (LVGL's `inv_areas`).
///
/// - [`add`](RectSet::add) clips, skips rectangles already covered, drops stored rectangles
///   covered by the new one, and when the set is full joins the two rectangles whose union
///   adds the least area: the set always covers everything added, and grows only as much as
///   the limit requires (no jump to one bounding box of everything, which turns a few small
///   changes far apart into a redraw of the screen between them).
/// - [`merge`](RectSet::merge) joins pairs that touch or overlap when their union is smaller
///   than the two areas together (LVGL `lv_refr_join_area`). The result is deterministic.
///
/// `size_of::<RectSet<32>>()` is 552 bytes on 64-bit hosts (`16·32 + 16 + 24`) and 540 on
/// 32-bit MCUs.
///
/// ```
/// use twine_core::{Rect, RectSet, AddResult};
/// let mut set: RectSet = RectSet::new(Rect::new(0, 0, 320, 240));
/// assert_eq!(set.add(Rect::new(10, 10, 50, 50)), AddResult::Added);
/// assert_eq!(set.add(Rect::new(20, 20, 30, 30)), AddResult::AlreadyCovered);
/// assert_eq!(set.add(Rect::new(400, 0, 500, 10)), AddResult::ClippedAway);
/// set.add(Rect::new(40, 10, 80, 50));
/// set.merge();
/// assert_eq!(set.iter().copied().collect::<Vec<_>>(), [Rect::new(10, 10, 80, 50)]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RectSet<const N: usize = 32> {
    bounds: Rect,
    rects: [Rect; N],
    len: usize,
    /// Most rectangles stored (`1..=N`).
    limit: usize,
    overflowed: bool,
}

impl<const N: usize> RectSet<N> {
    const CAPACITY_OK: () = assert!(N >= 1, "RectSet: N must be >= 1");

    /// An empty set whose rectangles are clipped to `bounds`, holding up to `N`.
    #[must_use]
    pub const fn new(bounds: Rect) -> Self {
        Self::with_limit(bounds, N)
    }

    /// An empty set whose rectangles are clipped to `bounds`, holding at most `limit`
    /// (`1..=N`; other values are clamped).
    ///
    /// ```
    /// use twine_core::{AddResult, Rect, RectSet};
    /// let mut set: RectSet = RectSet::with_limit(Rect::new(0, 0, 320, 240), 2);
    /// set.add(Rect::new(0, 0, 10, 10));
    /// set.add(Rect::new(300, 200, 310, 210));
    /// // Full: the new rectangle joins its nearest neighbour, the far one stays apart.
    /// assert_eq!(set.add(Rect::new(20, 0, 30, 10)), AddResult::Overflowed);
    /// assert_eq!(
    ///     set.iter().copied().collect::<Vec<_>>(),
    ///     [Rect::new(0, 0, 30, 10), Rect::new(300, 200, 310, 210)]
    /// );
    /// ```
    #[must_use]
    pub const fn with_limit(bounds: Rect, limit: usize) -> Self {
        let () = Self::CAPACITY_OK;
        RectSet {
            bounds,
            rects: [Rect::ZERO; N],
            len: 0,
            limit: clamp_limit::<N>(limit),
            overflowed: false,
        }
    }

    /// The most rectangles the set holds.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Changes the limit (`1..=N`; other values are clamped). Lowering it below the number of
    /// stored rectangles joins the cheapest pairs until they fit (counted as an overflow).
    ///
    /// ```
    /// use twine_core::{Rect, RectSet};
    /// let mut set: RectSet = RectSet::new(Rect::new(0, 0, 320, 240));
    /// set.add(Rect::new(0, 0, 10, 10));
    /// set.add(Rect::new(20, 0, 30, 10));
    /// set.add(Rect::new(300, 200, 310, 210));
    /// set.set_limit(2);
    /// assert_eq!(set.limit(), 2);
    /// assert_eq!(set.len(), 2);
    /// assert!(set.overflowed());
    /// ```
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = clamp_limit::<N>(limit);
        while self.len > self.limit {
            self.join_cheapest(None);
        }
    }

    /// The clip bounds.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Changes the bounds and clears the set.
    pub fn set_bounds(&mut self, b: Rect) {
        self.bounds = b;
        self.clear();
    }

    /// Adds a dirty rectangle (see the type docs for the policy).
    pub fn add(&mut self, r: Rect) -> AddResult {
        let Some(r) = r.intersection(&self.bounds) else {
            return AddResult::ClippedAway;
        };
        if self.as_slice().iter().any(|e| e.contains_rect(&r)) {
            return AddResult::AlreadyCovered;
        }
        self.retain(|e| !r.contains_rect(e));
        if self.len < self.limit {
            self.push(r);
            return AddResult::Added;
        }
        self.join_cheapest(Some(r));
        AddResult::Overflowed
    }

    /// Joins the pair of rectangles — the stored ones and `extra` — whose union adds the least
    /// area; `extra` is stored if it was not part of the pair. The set shrinks by one when
    /// `extra` is `None` and keeps its size otherwise.
    fn join_cheapest(&mut self, extra: Option<Rect>) {
        // Candidate `i`: stored rectangle `i`, or `extra` at index `len`.
        let n = if extra.is_some() {
            self.len.wrapping_add(1)
        } else {
            self.len
        };
        let at = |s: &Self, i: usize| match (s.rects.get(..s.len).and_then(|r| r.get(i)), extra) {
            (Some(&e), _) => e,
            (None, Some(x)) => x,
            (None, None) => Rect::ZERO,
        };
        if n < 2 {
            return;
        }
        let mut best = (0usize, 1usize, u64::MAX);
        // SAFE-ARITH (the index arithmetic): indices are below `n ≤ N + 1`.
        for i in 0..n {
            for j in i.wrapping_add(1)..n {
                let cost = join_cost(&at(self, i), &at(self, j));
                if cost < best.2 {
                    best = (i, j, cost);
                }
            }
        }
        let (i, j, _) = best;
        let u = at(self, i).union(&at(self, j));
        crate::debug!(
            target: "twine::refresh",
            "RectSet full ({} areas): joined {} and {} into {}",
            self.limit,
            at(self, i),
            at(self, j),
            u
        );
        if let Some(slot) = self.rects.get_mut(i) {
            *slot = u;
        }
        if j < self.len {
            // Two stored rectangles joined: `j` leaves (`i < j < len`, so `i` stays put)…
            let last = self.len.wrapping_sub(1);
            self.rects.swap(j, last);
            self.len = last;
            // …and `extra`, if any, takes its place.
            if let Some(x) = extra {
                self.push(x);
            }
        }
        // The union may cover other stored rectangles.
        self.retain(|e| *e == u || !u.contains_rect(e));
        self.overflowed = true;
    }

    /// Keeps the stored rectangles for which `keep` holds (in order).
    fn retain(&mut self, keep: impl Fn(&Rect) -> bool) {
        let mut kept = 0usize;
        for i in 0..self.len {
            let Some(&e) = self.rects.get(i) else { break };
            if !keep(&e) {
                continue;
            }
            if let Some(slot) = self.rects.get_mut(kept) {
                *slot = e;
            }
            // SAFE-ARITH: `kept ≤ i < len ≤ N`.
            kept = kept.wrapping_add(1);
        }
        self.len = kept;
    }

    /// Stores `r` (`len < N`).
    fn push(&mut self, r: Rect) {
        if let Some(slot) = self.rects.get_mut(self.len) {
            *slot = r;
            // SAFE-ARITH: `len < N` (the slot exists).
            self.len = self.len.wrapping_add(1);
        }
    }

    /// Joins pairs of touching/overlapping rectangles while that reduces the covered area
    /// (`union.area() < a.area() + b.area()`), until nothing changes. O(n²) per pass.
    pub fn merge(&mut self) {
        let mut changed = true;
        while changed {
            changed = false;
            // SAFE-INDEX: `i < j < len ≤ N` in the loops below; the `get` fallbacks are never
            // taken. SAFE-ARITH: the indices stay below `len ≤ N`, and `len ≥ 1` when it is
            // decremented (`j < len`).
            let mut i = 0;
            while i < self.len {
                let mut j = i.wrapping_add(1);
                while j < self.len {
                    let (Some(&a), Some(&b)) = (self.rects.get(i), self.rects.get(j)) else {
                        break;
                    };
                    if a.touches_or_overlaps(&b) {
                        let u = a.union(&b);
                        if u.area() < a.area().saturating_add(b.area()) {
                            self.len = self.len.wrapping_sub(1);
                            let last = self.rects.get(self.len).copied().unwrap_or(u);
                            if let Some(slot) = self.rects.get_mut(i) {
                                *slot = u;
                            }
                            if let Some(slot) = self.rects.get_mut(j) {
                                *slot = last;
                            }
                            changed = true;
                            j = i.wrapping_add(1);
                            continue;
                        }
                    }
                    j = j.wrapping_add(1);
                }
                i = i.wrapping_add(1);
            }
        }
    }

    fn as_slice(&self) -> &[Rect] {
        // SAFE-INDEX: `len ≤ N` (every write keeps it so); the fallback is never taken.
        self.rects.get(..self.len).unwrap_or(&[])
    }

    /// The stored rectangles.
    pub fn iter(&self) -> impl Iterator<Item = &Rect> {
        self.as_slice().iter()
    }

    /// Number of stored rectangles.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing is dirty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes every rectangle and resets the overflow flag.
    pub fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }

    /// Bounding box of all stored rectangles ([`Rect::ZERO`] if empty).
    #[must_use]
    pub fn bounding_box(&self) -> Rect {
        self.as_slice().iter().fold(Rect::ZERO, |acc, r| acc.union(r))
    }

    /// Sum of the stored areas (overlaps counted twice), saturating at `u64::MAX`.
    #[must_use]
    pub fn total_area(&self) -> u64 {
        self.as_slice()
            .iter()
            .fold(0, |sum: u64, r| sum.saturating_add(r.area()))
    }

    /// Whether an overflow happened since the last [`clear`](RectSet::clear).
    #[must_use]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }
}

/// `limit` clamped to `1..=N`.
const fn clamp_limit<const N: usize>(limit: usize) -> usize {
    if limit < 1 {
        1
    } else if limit > N {
        N
    } else {
        limit
    }
}

/// The area the union of `a` and `b` covers beyond `a` and `b` themselves.
fn join_cost(a: &Rect, b: &Rect) -> u64 {
    let both = a.area().saturating_add(b.area());
    let overlap = a.intersection(b).map_or(0, |o| o.area());
    a.union(b).area().saturating_sub(both.saturating_sub(overlap))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_merge_and_total_area_of_huge_rects() {
        // Two areas near 2^64 overflowed their sum.
        let all = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        let mut s: RectSet<4> = RectSet::new(all);
        s.add(Rect::new(i32::MIN, i32::MIN, i32::MAX - 1, i32::MAX));
        s.add(Rect::new(i32::MIN + 1, i32::MIN, i32::MAX, i32::MAX));
        assert_eq!(s.total_area(), u64::MAX);
        s.merge();
        assert_eq!(s.len(), 1);
    }
    use alloc::vec::Vec;

    const SCREEN: Rect = Rect::new(0, 0, 320, 240);

    fn rects<const N: usize>(s: &RectSet<N>) -> Vec<Rect> {
        s.iter().copied().collect()
    }

    #[test]
    fn contained_rect_is_skipped() {
        let mut s: RectSet<4> = RectSet::new(SCREEN);
        assert_eq!(s.add(Rect::new(0, 0, 100, 100)), AddResult::Added);
        assert_eq!(s.add(Rect::new(10, 10, 20, 20)), AddResult::AlreadyCovered);
        assert_eq!(s.add(Rect::new(0, 0, 100, 100)), AddResult::AlreadyCovered);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn container_removes_contained() {
        let mut s: RectSet<4> = RectSet::new(SCREEN);
        s.add(Rect::new(10, 10, 20, 20));
        s.add(Rect::new(200, 200, 210, 210));
        s.add(Rect::new(30, 30, 40, 40));
        assert_eq!(s.add(Rect::new(0, 0, 50, 50)), AddResult::Added);
        assert_eq!(
            rects(&s),
            [Rect::new(200, 200, 210, 210), Rect::new(0, 0, 50, 50)]
        );
    }

    #[test]
    fn overflow_joins_the_cheapest_pair() {
        let mut s: RectSet<3> = RectSet::new(SCREEN);
        s.add(Rect::new(0, 0, 10, 10));
        s.add(Rect::new(100, 0, 110, 10));
        s.add(Rect::new(0, 100, 10, 110));
        assert!(!s.overflowed());
        // The far corner stays apart: the two top rectangles (900 px² extra) are joined.
        assert_eq!(s.add(Rect::new(300, 200, 310, 210)), AddResult::Overflowed);
        assert!(s.overflowed());
        assert_eq!(
            rects(&s),
            [
                Rect::new(0, 0, 110, 10),
                Rect::new(0, 100, 10, 110),
                Rect::new(300, 200, 310, 210)
            ]
        );
        // A new rectangle next to a stored one joins it.
        assert_eq!(s.add(Rect::new(300, 212, 310, 220)), AddResult::Overflowed);
        assert_eq!(rects(&s)[2], Rect::new(300, 200, 310, 220));
        s.clear();
        assert!(!s.overflowed() && s.is_empty());
        // A limit of one is the bounding box.
        let mut s: RectSet<4> = RectSet::with_limit(SCREEN, 1);
        s.add(Rect::new(0, 0, 10, 10));
        s.add(Rect::new(300, 200, 310, 210));
        assert_eq!(rects(&s), [Rect::new(0, 0, 310, 210)]);
        // Limits are clamped to `1..=N` and kept by `clear` and `set_bounds`.
        assert_eq!(RectSet::<4>::with_limit(SCREEN, 0).limit(), 1);
        assert_eq!(RectSet::<4>::with_limit(SCREEN, 9).limit(), 4);
        s.clear();
        s.set_bounds(Rect::new(0, 0, 10, 10));
        assert_eq!(s.limit(), 1);
    }

    proptest::proptest! {
        #[test]
        fn limited_sets_cover_everything_added(
            adds in proptest::collection::vec((-20i32..340, -20i32..260, 1i32..80, 1i32..80), 1..80),
            max in 1usize..8,
        ) {
            let mut s: RectSet<8> = RectSet::with_limit(SCREEN, max);
            let mut added = Vec::new();
            for (k, (x, y, w, h)) in adds.into_iter().enumerate() {
                let r = Rect::from_xywh(x, y, w, h);
                s.add(r);
                if let Some(c) = r.intersection(&SCREEN) {
                    added.push(c);
                }
                // Halfway, the limit is lowered (stored rectangles are joined to fit).
                if k == 40 {
                    s.set_limit(max.div_ceil(2));
                }
                proptest::prop_assert!(s.len() <= s.limit());
            }
            // Every pixel added is covered by a stored rectangle.
            for r in &added {
                for py in r.y0..r.y1 {
                    for px in r.x0..r.x1 {
                        proptest::prop_assert!(
                            s.iter().any(|e| e.contains(twine_point(px, py))),
                            "({px}, {py}) of {r} lost"
                        );
                    }
                }
            }
        }
    }

    fn twine_point(x: i32, y: i32) -> crate::geometry::Point {
        crate::geometry::Point::new(x, y)
    }

    #[test]
    fn merge_joins_overlapping_when_cheaper() {
        let mut s: RectSet = RectSet::new(SCREEN);
        s.add(Rect::new(0, 0, 100, 100));
        s.add(Rect::new(50, 0, 150, 100));
        s.merge();
        assert_eq!(rects(&s), [Rect::new(0, 0, 150, 100)]);
        // A chain collapses completely.
        let mut s: RectSet = RectSet::new(SCREEN);
        for i in 0..5 {
            s.add(Rect::from_xywh(i * 30, i * 30, 50, 50));
        }
        s.merge();
        assert_eq!(s.len(), 5, "diagonal steps are not worth joining");
        let mut s: RectSet = RectSet::new(SCREEN);
        for i in 0..5 {
            s.add(Rect::from_xywh(i * 20, 0, 30, 30));
        }
        s.merge();
        assert_eq!(rects(&s), [Rect::new(0, 0, 110, 30)]);
    }

    #[test]
    fn merge_keeps_far_apart_rects() {
        let mut s: RectSet = RectSet::new(SCREEN);
        s.add(Rect::new(0, 0, 10, 10));
        s.add(Rect::new(300, 200, 310, 210));
        s.add(Rect::new(10, 20, 20, 30)); // touches only at a corner distance: no
        s.merge();
        assert_eq!(s.len(), 3);
        // Touching edge-to-edge with equal height: union area == sum → kept apart.
        let mut s: RectSet = RectSet::new(SCREEN);
        s.add(Rect::new(0, 0, 10, 10));
        s.add(Rect::new(10, 0, 20, 10));
        s.merge();
        assert_eq!(s.len(), 2);
        assert_eq!(s.total_area(), 200);
        assert_eq!(s.bounding_box(), Rect::new(0, 0, 20, 10));
    }

    #[test]
    fn clip_to_bounds() {
        let mut s: RectSet<4> = RectSet::new(SCREEN);
        assert_eq!(s.add(Rect::new(-10, -10, 10, 10)), AddResult::Added);
        assert_eq!(rects(&s), [Rect::new(0, 0, 10, 10)]);
        assert_eq!(s.add(Rect::new(320, 0, 330, 10)), AddResult::ClippedAway);
        assert_eq!(s.add(Rect::new(5, 5, 5, 50)), AddResult::ClippedAway);
        s.set_bounds(Rect::new(0, 0, 5, 5));
        assert!(s.is_empty());
        assert_eq!(s.bounds(), Rect::new(0, 0, 5, 5));
        assert_eq!(s.bounding_box(), Rect::ZERO);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn size_is_inline_only() {
        assert_eq!(core::mem::size_of::<RectSet<32>>(), 16 * 32 + 16 + 24);
    }
}
