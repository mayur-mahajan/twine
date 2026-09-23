//! Property tests for `RectSet`.

use proptest::prelude::*;
use twine_core::{Point, Rect, RectSet};

const BOUNDS: Rect = Rect::new(0, 0, 64, 48);

fn rect() -> impl Strategy<Value = Rect> {
    (-8i32..72, -8i32..56, 0i32..40, 0i32..40).prop_map(|(x, y, w, h)| Rect::from_xywh(x, y, w, h))
}

fn covered(rects: &[Rect], p: Point) -> bool {
    rects.iter().any(|r| r.contains(p))
}

proptest! {
    #[test]
    fn coverage_is_preserved(rs in prop::collection::vec(rect(), 0..40)) {
        let mut set: RectSet<8> = RectSet::new(BOUNDS);
        for r in &rs {
            set.add(*r);
        }
        set.merge();
        let stored: Vec<Rect> = set.iter().copied().collect();
        for y in BOUNDS.y0..BOUNDS.y1 {
            for x in BOUNDS.x0..BOUNDS.x1 {
                let p = Point::new(x, y);
                if covered(&rs, p) {
                    prop_assert!(covered(&stored, p), "{} lost", p);
                }
            }
        }
        for r in &stored {
            prop_assert!(BOUNDS.contains_rect(r));
        }
    }

    #[test]
    fn merge_never_increases_total_area_beyond_bounding_box(rs in prop::collection::vec(rect(), 0..40)) {
        let mut set: RectSet<8> = RectSet::new(BOUNDS);
        for r in &rs {
            set.add(*r);
        }
        let before = set.total_area();
        let bbox_before = set.bounding_box();
        set.merge();
        prop_assert!(set.total_area() <= before.max(bbox_before.area()));
        prop_assert!(set.total_area() <= before);
        prop_assert_eq!(set.bounding_box(), bbox_before);
    }

    #[test]
    fn len_le_capacity(rs in prop::collection::vec(rect(), 0..100)) {
        let mut set: RectSet<5> = RectSet::new(BOUNDS);
        for r in &rs {
            set.add(*r);
            prop_assert!(set.len() <= 5);
        }
        set.merge();
        prop_assert!(set.len() <= 5);
    }
}
