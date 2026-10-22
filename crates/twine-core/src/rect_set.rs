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
    /// The set was full: everything was replaced by one bounding box.
    Overflowed,
}

/// Up to `N` dirty rectangles, clipped to `bounds`, never allocating (LVGL's `inv_areas`).
///
/// - [`add`](RectSet::add) clips, skips rectangles already covered, drops stored rectangles
///   covered by the new one, and when full replaces everything by the bounding box.
/// - [`merge`](RectSet::merge) joins pairs that touch or overlap when their union is smaller
///   than the two areas together (LVGL `lv_refr_join_area`). The result is deterministic.
///
/// `size_of::<RectSet<32>>()` is 544 bytes on 64-bit hosts (`16·32 + 16 + 16`) and 536 on
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
    overflowed: bool,
}

impl<const N: usize> RectSet<N> {
    const CAPACITY_OK: () = assert!(N >= 1, "RectSet: N must be >= 1");

    /// An empty set whose rectangles are clipped to `bounds`.
    #[must_use]
    pub const fn new(bounds: Rect) -> Self {
        let () = Self::CAPACITY_OK;
        RectSet {
            bounds,
            rects: [Rect::ZERO; N],
            len: 0,
            overflowed: false,
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
        // Drop stored rectangles covered by `r` (stable order).
        let mut kept = 0;
        for i in 0..self.len {
            let Some(&e) = self.rects.get(i) else { break };
            if r.contains_rect(&e) {
                continue;
            }
            if let Some(slot) = self.rects.get_mut(kept) {
                *slot = e;
            }
            // SAFE-ARITH: `kept ≤ i < len ≤ N`.
            kept = kept.wrapping_add(1);
        }
        self.len = kept;
        if self.len >= N {
            let bbox = self.as_slice().iter().fold(r, |acc, e| acc.union(e));
            crate::debug!(
                target: "twine::refresh",
                "RectSet overflow ({} areas): invalidating bounding box {}",
                N,
                bbox
            );
            // `N ≥ 1` (asserted at compile time): the first slot exists.
            if let Some(first) = self.rects.first_mut() {
                *first = bbox;
            }
            self.len = 1;
            self.overflowed = true;
            return AddResult::Overflowed;
        }
        // SAFE-INDEX: `len < N` (checked above); the fallback is never taken.
        if let Some(slot) = self.rects.get_mut(self.len) {
            *slot = r;
            // SAFE-ARITH: `len < N`.
            self.len = self.len.wrapping_add(1);
        }
        AddResult::Added
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
    fn overflow_becomes_bounding_box() {
        let mut s: RectSet<3> = RectSet::new(SCREEN);
        s.add(Rect::new(0, 0, 10, 10));
        s.add(Rect::new(100, 0, 110, 10));
        s.add(Rect::new(0, 100, 10, 110));
        assert!(!s.overflowed());
        assert_eq!(s.add(Rect::new(300, 200, 310, 210)), AddResult::Overflowed);
        assert!(s.overflowed());
        assert_eq!(rects(&s), [Rect::new(0, 0, 310, 210)]);
        s.clear();
        assert!(!s.overflowed() && s.is_empty());
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
        assert_eq!(core::mem::size_of::<RectSet<32>>(), 16 * 32 + 16 + 16);
    }
}
